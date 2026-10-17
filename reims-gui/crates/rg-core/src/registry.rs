//! The VM registry: `macos-kvm-gui.db`, the SQLite database shared with the Go app, with the
//! same schema, migrations 1→5 and legacy file import (port of `vms.go`).
//!
//! Checks that need higher layers stay with the caller: shut-down checks before
//! [`Registry::delete`] with purge and [`Registry::move_vm`] (the Go app's `FindRunning`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::config::{Config, field_err, load_config, save_config, version_display};
use crate::settings::AppDir;
use crate::workspace::{
    SNAPSHOT_FILE, WORKSPACE_FILES, Workspace, remove_all, require_empty, write_file_atomic,
};
use crate::{Error, Result, msg};

/// The database's `PRAGMA user_version`: 1 created the tables, 2 added the network adapters,
/// 3 root-patch mode, 4 the graphics adapter, 5 the Reims resolution.
pub const SCHEMA_VERSION: i64 = 5;

/// The version 1 schema, verbatim from the Go app.
pub const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS vms (
	id         TEXT PRIMARY KEY,
	name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
	workspace  TEXT NOT NULL,
	version    TEXT NOT NULL,
	ram_mib    INTEGER NOT NULL,
	cores      INTEGER NOT NULL,
	vcpus      INTEGER NOT NULL,
	ssh_port   INTEGER NOT NULL,
	display    TEXT NOT NULL DEFAULT '',
	gpu        TEXT NOT NULL DEFAULT '',
	created_at TEXT NOT NULL DEFAULT (datetime('now')),
	updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);";

/// Columns added by migrations 2–5.
const ADDED_COLUMNS: [(&str, &str); 4] = [
    ("networks", "TEXT NOT NULL DEFAULT ''"),
    ("root_patch", "INTEGER NOT NULL DEFAULT 0"),
    ("graphics", "TEXT NOT NULL DEFAULT ''"),
    ("resolution", "TEXT NOT NULL DEFAULT ''"),
];

const VM_COLUMNS: &str = "id, name, workspace, version, ram_mib, cores, vcpus, ssh_port, display, gpu, networks, root_patch, graphics, resolution";

/// The Workspace when none is saved: the parent folder of every VM.
pub const DEFAULT_WORKSPACE: &str = "~/macOS-VMs";

/// Settings keys of the `settings` table.
pub const PREF_SELECTED: &str = "selected";
pub const PREF_WORKSPACE: &str = "workspace";
pub const PREF_THEME: &str = "theme";
pub const PREF_LIST_WIDTH: &str = "list-width";

/// One VM. `path` is set only for a VM read from a standalone JSON file (`--config`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VmEntry {
    pub id: String,
    pub path: Option<PathBuf>,
    pub config: Config,
}

impl VmEntry {
    /// The VM's name, or one derived from its macOS version.
    pub fn display_name(&self) -> String {
        if self.config.name.is_empty() {
            format!("macOS {}", self.config.version)
        } else {
            self.config.name.clone()
        }
    }
}

/// An open registry. WAL and a busy timeout let several processes (the app, the CLI, the Go
/// app) use the database at once. Not `Sync`: wrap it in a mutex to share it.
#[derive(Debug)]
pub struct Registry {
    conn: Connection,
}

/// A file- and id-safe word: "Tahoe lab" → "tahoe-lab"; "vm" when nothing is left.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s = s.trim_matches('-');
    if s.is_empty() { "vm".into() } else { s.into() }
}

/// Looks a VM up by id, then by display name (case-insensitive): an id always wins.
pub fn find_vm<'a>(vms: &'a [VmEntry], key: &str) -> Option<&'a VmEntry> {
    vms.iter().find(|e| e.id == key).or_else(|| {
        vms.iter()
            .find(|e| e.display_name().to_lowercase() == key.to_lowercase())
    })
}

/// The VM's settings as pretty JSON, the format `--config` and [`Registry::import_settings`] read.
pub fn export_vm(e: &VmEntry) -> Result<String> {
    Ok(serde_json::to_string_pretty(&e.config)?)
}

/// Writes `<workspace>/macos-kvm-gui/vm.json` when the workspace exists; never creates one.
fn write_snapshot(c: &Config) {
    #[derive(Serialize)]
    struct Snapshot<'a> {
        _note: &'a str,
        config: &'a Config,
    }
    let Ok(dir) = c.workspace_dir() else { return };
    if !dir.exists() {
        return;
    }
    let note = "Generated from macos-kvm-gui.db; edit the VM in the app or with the CLI, not here.";
    if let Ok(data) = serde_json::to_vec_pretty(&Snapshot {
        _note: note,
        config: c,
    }) && std::fs::create_dir_all(dir.join(crate::workspace::BOOT_DIR)).is_ok()
    {
        let _ = write_file_atomic(&dir.join(SNAPSHOT_FILE), &data, 0o644);
    }
}

/// `""` keeps a VM on its legacy single NAT adapter; a list (even empty) is stored as JSON.
fn networks_json(c: &Config) -> Result<String> {
    Ok(match &c.networks {
        None => String::new(),
        Some(n) => serde_json::to_string(n)?,
    })
}

fn scan_vm(row: &Row) -> rusqlite::Result<(VmEntry, String)> {
    let e = VmEntry {
        id: row.get(0)?,
        path: None,
        config: Config {
            name: row.get(1)?,
            workspace: row.get(2)?,
            version: row.get(3)?,
            ram_mib: row.get(4)?,
            cores: row.get(5)?,
            vcpus: row.get(6)?,
            ssh_port: row.get(7)?,
            display: row.get(8)?,
            gpu: row.get(9)?,
            root_patch: row.get(11)?,
            graphics: row.get(12)?,
            resolution: row.get(13)?,
            ..Config::default()
        },
    };
    Ok((e, row.get(10)?))
}

fn is_unique_err(e: &rusqlite::Error) -> bool {
    e.to_string().contains("UNIQUE constraint failed")
}

fn name_taken() -> Error {
    field_err("name", "a VM with this name already exists")
}

impl Registry {
    /// Opens the database in `app`, creating and migrating it as needed (and importing the VMs
    /// of older file-based versions on creation).
    pub fn open(app: &AppDir) -> Result<Self> {
        let path = app.db_path();
        std::fs::create_dir_all(app.path())?;
        let conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| msg(format!("database {}: {e}", path.display())))?;
        let reg = Self { conn };
        if v < SCHEMA_VERSION {
            reg.migrate(v, app.path())
                .map_err(|e| msg(format!("upgrading {}: {e}", path.display())))?;
        }
        Ok(reg)
    }

    /// Brings the database from version `v` to [`SCHEMA_VERSION`]. Each step is safe to repeat
    /// and the version is written last, so an interrupted upgrade runs again.
    fn migrate(&self, v: i64, app_dir: &Path) -> Result<()> {
        if v < 1 {
            self.conn.execute_batch(SCHEMA_V1)?;
        }
        for (step, (col, def)) in (2..).zip(ADDED_COLUMNS) {
            if v >= step {
                continue;
            }
            let n: i64 = self.conn.query_row(
                "SELECT count(*) FROM pragma_table_info('vms') WHERE name = ?1",
                [col],
                |r| r.get(0),
            )?;
            if n == 0 {
                // Another process may have just added it: that is fine.
                match self
                    .conn
                    .execute_batch(&format!("ALTER TABLE vms ADD COLUMN {col} {def}"))
                {
                    Err(e) if !e.to_string().contains("duplicate column name") => {
                        return Err(e.into());
                    }
                    _ => {}
                }
            }
        }
        if v < 1 {
            self.import_files(app_dir)?;
        }
        self.conn
            .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        Ok(())
    }

    /// Moves VMs stored as files by older versions (`vms/*.json`, `config.json`, `selected`)
    /// into a new database. The originals are renamed `*.migrated`, not deleted.
    fn import_files(&self, dir: &Path) -> Result<()> {
        let vms_dir = dir.join("vms");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&vms_dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        let legacy = dir.join("config.json");
        if legacy.exists() {
            files.push(legacy.clone());
        }
        for f in &files {
            let mut c =
                load_config(f).map_err(|e| msg(format!("importing {}: {e}", f.display())))?;
            if c.name.is_empty() {
                c.name = format!("macOS {}", version_display(&c.version));
            }
            match self.insert(c) {
                Ok(_) | Err(Error::Field(_)) => {} // a duplicate name is skipped
                Err(e) => return Err(msg(format!("importing {}: {e}", f.display()))),
            }
        }
        let sel = dir.join("selected");
        if let Ok(id) = std::fs::read_to_string(&sel) {
            let _ = self.conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('selected', ?1)",
                [id.trim()],
            );
            let _ = std::fs::rename(&sel, dir.join("selected.migrated"));
        }
        for p in [vms_dir, legacy] {
            if p.exists() {
                let mut to = p.clone().into_os_string();
                to.push(".migrated");
                let _ = std::fs::rename(&p, to);
            }
        }
        Ok(())
    }

    /// The VMs sorted by name.
    pub fn list(&self) -> Result<Vec<VmEntry>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {VM_COLUMNS} FROM vms ORDER BY name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([], scan_vm)?;
        let mut vms = Vec::new();
        for row in rows {
            let (mut e, nets) = row?;
            if !nets.is_empty() {
                e.config.networks = Some(serde_json::from_str(&nets).map_err(|err| {
                    msg(format!("VM {}: unreadable network adapters: {err}", e.id))
                })?);
            }
            vms.push(e);
        }
        Ok(vms)
    }

    /// Validates `c` (which must have a name) and stores it as a new VM.
    pub fn create(&self, mut c: Config) -> Result<VmEntry> {
        c.name = c.name.trim().to_owned();
        if c.name.is_empty() {
            return Err(field_err("name", "give the VM a name"));
        }
        c.validate()?;
        self.insert(c)
    }

    fn insert(&self, c: Config) -> Result<VmEntry> {
        let base = slug(&c.name);
        let mut id = base.clone();
        for i in 2.. {
            let n: i64 =
                self.conn
                    .query_row("SELECT count(*) FROM vms WHERE id = ?1", [&id], |r| {
                        r.get(0)
                    })?;
            if n == 0 {
                break;
            }
            id = format!("{base}-{i}");
        }
        let res = self.conn.execute(
            &format!("INSERT INTO vms ({VM_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"),
            params![
                id, c.name, c.workspace, c.version, c.ram_mib, c.cores, c.vcpus, c.ssh_port, c.display, c.gpu,
                networks_json(&c)?, c.root_patch, c.graphics, c.resolution
            ],
        );
        match res {
            Err(e) if is_unique_err(&e) => return Err(name_taken()),
            r => r?,
        };
        write_snapshot(&c);
        Ok(VmEntry {
            id,
            path: None,
            config: c,
        })
    }

    /// Validates and stores new settings for `e` and refreshes its workspace snapshot. A
    /// standalone `--config` entry is written to its file instead.
    pub fn save(&self, e: &VmEntry, mut c: Config) -> Result<()> {
        if let Some(path) = &e.path {
            return save_config(path, &c);
        }
        c.name = c.name.trim().to_owned();
        if c.name.is_empty() {
            return Err(field_err("name", "give the VM a name"));
        }
        c.validate()?;
        let res = self.conn.execute(
            "UPDATE vms SET name = ?1, workspace = ?2, version = ?3, ram_mib = ?4, cores = ?5, vcpus = ?6, ssh_port = ?7,
		display = ?8, gpu = ?9, networks = ?10, root_patch = ?11, graphics = ?12, resolution = ?13, updated_at = datetime('now') WHERE id = ?14",
            params![
                c.name, c.workspace, c.version, c.ram_mib, c.cores, c.vcpus, c.ssh_port, c.display, c.gpu,
                networks_json(&c)?, c.root_patch, c.graphics, c.resolution, e.id
            ],
        );
        let n = match res {
            Err(err) if is_unique_err(&err) => return Err(name_taken()),
            r => r?,
        };
        if n == 0 {
            return Err(msg(format!("VM {} no longer exists", e.id)));
        }
        write_snapshot(&c);
        Ok(())
    }

    /// Removes the VM. With `purge` it also deletes, under the workspace lock, the files the app
    /// created there ([`WORKSPACE_FILES`]) and the folder if it ends up empty. Returns what was
    /// deleted. The caller makes sure the VM is shut down first.
    pub fn delete(&self, e: &VmEntry, purge: bool) -> Result<Vec<String>> {
        let mut removed = Vec::new();
        if purge {
            let w = Workspace::new(e.config.workspace_dir()?);
            let res = w.with_lock(|| {
                for f in WORKSPACE_FILES {
                    let p = w.path(f);
                    if p.exists() {
                        remove_all(&p)?;
                        removed.push(p.display().to_string());
                    }
                }
                if std::fs::read_dir(&w.dir).is_ok_and(|mut d| d.next().is_none())
                    && std::fs::remove_dir(&w.dir).is_ok()
                {
                    removed.push(w.dir.display().to_string());
                }
                Ok(())
            });
            res?;
        }
        if let Some(path) = &e.path {
            match std::fs::remove_file(path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err.into()),
                _ => {}
            }
            removed.push(path.display().to_string());
            return Ok(removed);
        }
        self.conn
            .execute("DELETE FROM vms WHERE id = ?1", [&e.id])?;
        removed.push(format!("VM {} (database)", e.id));
        Ok(removed)
    }

    /// The remembered VM, or the first one.
    pub fn selected(&self, vms: &[VmEntry]) -> Option<VmEntry> {
        let id = self.preference(PREF_SELECTED);
        vms.iter().find(|e| e.id == id).or(vms.first()).cloned()
    }

    /// Remembers the VM the app and the CLI act on.
    pub fn select(&self, id: &str) -> Result<()> {
        self.set_preference(PREF_SELECTED, id)
    }

    /// An app-wide preference from the `settings` table; empty when unset.
    pub fn preference(&self, key: &str) -> String {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    /// The folder that holds one subfolder per VM (may start with `~`).
    pub fn workspace_root(&self) -> String {
        let r = self.preference(PREF_WORKSPACE);
        if r.is_empty() {
            DEFAULT_WORKSPACE.into()
        } else {
            r
        }
    }

    /// Validates and saves the Workspace. Existing VMs are not moved.
    pub fn set_workspace_root(&self, root: &str) -> Result<()> {
        let root = root.trim();
        let dir = Config {
            workspace: root.into(),
            ..Config::default()
        }
        .workspace_dir()?;
        // A path with ".." is stored expanded so vm_folder stays absolute.
        let stored = if root.contains("..") {
            dir.display().to_string()
        } else {
            crate::config::clean(Path::new(root)).display().to_string()
        };
        self.set_preference(PREF_WORKSPACE, &stored)
    }

    /// The default folder for a new VM: `<Workspace>/<slug of its name>`.
    pub fn vm_folder(&self, name: &str) -> String {
        Path::new(&self.workspace_root())
            .join(slug(name))
            .display()
            .to_string()
    }

    /// Moves the files the app created for `e` into `dst` (new or empty, same filesystem) and
    /// points the VM there; on any error everything is put back. Both folders are locked. The
    /// caller makes sure the VM is shut down first.
    pub fn move_vm(&self, e: &VmEntry, dst: &str) -> Result<VmEntry> {
        let mut e = e.clone();
        if e.path.is_none() {
            // The stored entry, not a copy that may be out of date.
            let vms = self.list()?;
            e = find_vm(&vms, &e.id)
                .cloned()
                .ok_or_else(|| msg(format!("VM {} no longer exists", e.id)))?;
        }
        let mut c = e.config.clone();
        c.workspace = dst.trim().into();
        let to = c.workspace_dir()?;
        let from = e.config.workspace_dir()?;
        if from == to {
            return Ok(e);
        }
        if to.starts_with(&from) {
            return Err(msg("the new folder cannot be inside the current one"));
        }
        let src = Workspace::new(&from);
        src.with_lock(|| {
            Workspace::new(&to).with_lock(|| {
                require_empty(&to)?;
                std::fs::metadata(&from)
                    .map_err(|err| msg(format!("cannot read the current folder: {err}")))?;
                let created = !to.exists();
                std::fs::create_dir_all(&to)?;
                let mut moved: Vec<&str> = Vec::new();
                let undo = |moved: &[&str]| {
                    for f in moved {
                        let _ = std::fs::rename(to.join(f), src.path(f));
                    }
                    if created {
                        let _ = std::fs::remove_dir(&to);
                    }
                };
                for f in WORKSPACE_FILES {
                    let step = (|| -> Result<bool> {
                        match std::fs::symlink_metadata(src.path(f)) {
                            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                                return Ok(false);
                            }
                            Err(err) => return Err(err.into()),
                            Ok(_) => {}
                        }
                        if std::fs::symlink_metadata(to.join(f)).is_ok() {
                            return Err(msg(format!("{f} already exists in the new folder")));
                        }
                        std::fs::rename(src.path(f), to.join(f)).map_err(|err| {
                            msg(format!(
                                "moving {f}: {err} (the new folder must be on the same disk)"
                            ))
                        })?;
                        Ok(true)
                    })();
                    match step {
                        Ok(true) => moved.push(f),
                        Ok(false) => {}
                        Err(err) => {
                            undo(&moved);
                            return Err(err);
                        }
                    }
                }
                if let Err(err) = self.save(&e, c.clone()) {
                    undo(&moved);
                    return Err(err);
                }
                let _ = std::fs::remove_dir(&from); // only when nothing else is left there
                Ok(())
            })
        })?;
        e.config = c;
        Ok(e)
    }

    /// Adds a VM from a settings file: an export, a `--config` file or a workspace snapshot
    /// (`vm.json`). A name in use gets " (imported)" appended.
    pub fn import_settings(&self, path: &Path) -> Result<VmEntry> {
        #[derive(serde::Deserialize)]
        struct Wrapped {
            config: Option<Config>,
        }
        let b = std::fs::read(path)?;
        let mut c = match serde_json::from_slice::<Wrapped>(&b) {
            Ok(Wrapped { config: Some(c) }) => c,
            _ => serde_json::from_slice::<Config>(&b)
                .map_err(|e| msg(format!("{} is not a VM settings file: {e}", path.display())))?,
        };
        if c.name.trim().is_empty() {
            c.name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        self.create_or_rename(c)
    }

    /// [`Registry::create`], retrying with " (imported)" appended when the name is taken.
    pub(crate) fn create_or_rename(&self, mut c: Config) -> Result<VmEntry> {
        match self.create(c.clone()) {
            Err(Error::Field(f)) if f.field == "name" && !c.name.trim().is_empty() => {
                c.name = format!("{} (imported)", c.name.trim());
                self.create(c)
            }
            r => r,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Result<(tempfile::TempDir, AppDir)> {
        let tmp = tempfile::tempdir()?;
        let app = AppDir::at(tmp.path().join("macos-kvm-gui"));
        Ok((tmp, app))
    }

    #[test]
    fn legacy_files_are_imported_once() -> Result<()> {
        let (tmp, app) = setup()?;
        let dir = app.path();
        save_config(
            &dir.join("config.json"),
            &Config {
                ram_mib: 12288,
                ..Config::defaults()
            },
        )?;
        let old = Config {
            name: "Old Tahoe".into(),
            version: "tahoe".into(),
            ..Config::defaults()
        };
        save_config(&dir.join("vms/old-tahoe.json"), &old)?;
        std::fs::write(dir.join("selected"), "old-tahoe\n")?;

        let reg = Registry::open(&app)?;
        let vms = reg.list()?;
        assert_eq!(vms.len(), 2);
        assert!(dir.join("config.json.migrated").exists() && dir.join("vms.migrated").exists());
        assert!(!dir.join("config.json").exists());
        assert_eq!(reg.selected(&vms).map(|e| e.id), Some("old-tahoe".into()));
        assert_eq!(
            find_vm(&vms, "macos sonoma").map(|e| e.config.ram_mib),
            Some(12288)
        );

        // Create, refuse duplicates (case-insensitive) and bad settings.
        let ws = tmp.path().join("lab");
        std::fs::create_dir_all(&ws)?;
        let mut c = Config {
            name: "Lab".into(),
            workspace: ws.display().to_string(),
            ..Config::defaults()
        };
        let e = reg.create(c.clone())?;
        assert_eq!(e.id, "lab");
        let dup = Config {
            name: "LAB".into(),
            ..c.clone()
        };
        assert!(matches!(reg.create(dup), Err(Error::Field(f)) if f.field == "name"));
        let bad = Config {
            name: "Bad".into(),
            cores: 4,
            vcpus: 6,
            ..c.clone()
        };
        assert!(matches!(reg.create(bad), Err(Error::Field(f)) if f.field == "vcpus"));

        // Save updates the row and regenerates the workspace snapshot.
        c.ram_mib = 16384;
        reg.save(&e, c.clone())?;
        assert_eq!(
            find_vm(&reg.list()?, "lab").map(|e| e.config.ram_mib),
            Some(16384)
        );
        let snap = std::fs::read_to_string(ws.join(SNAPSHOT_FILE))?;
        let v: serde_json::Value = serde_json::from_str(&snap)?;
        assert_eq!(v["config"]["ram_mib"], 16384);
        assert!(snap.contains("Generated from"));
        assert!(export_vm(&e)?.contains(r#""name": "Lab""#));

        // Select, then delete with purge: only the app's files go, the user's stay.
        reg.select(&e.id)?;
        let w = Workspace::new(&ws);
        std::fs::write(w.path(crate::workspace::DISK_FILE), "disk")?;
        std::fs::write(w.path("notes.txt"), "the user's own file")?;
        reg.delete(&e, true)?;
        assert!(
            !w.path(crate::workspace::DISK_FILE).exists()
                && !w.path(crate::workspace::BOOT_DIR).exists()
        );
        assert_eq!(
            std::fs::read_to_string(w.path("notes.txt"))?,
            "the user's own file"
        );
        assert_eq!(reg.list()?.len(), 2);

        // Opening again does not import twice.
        drop(reg);
        assert_eq!(Registry::open(&app)?.list()?.len(), 2);
        Ok(())
    }

    #[test]
    fn find_vm_prefers_the_id() {
        let vms = [
            VmEntry {
                id: "beta".into(),
                config: Config {
                    name: "alpha".into(),
                    ..Config::default()
                },
                path: None,
            },
            VmEntry {
                id: "alpha".into(),
                config: Config {
                    name: "Zebra".into(),
                    ..Config::default()
                },
                path: None,
            },
        ];
        assert_eq!(find_vm(&vms, "alpha").map(|e| e.id.as_str()), Some("alpha"));
        assert_eq!(find_vm(&vms, "ZEBRA").map(|e| e.id.as_str()), Some("alpha"));
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Tahoe lab"), "tahoe-lab");
        assert_eq!(slug("  --Ça va? 2 --"), "a-va-2");
        assert_eq!(slug("!!!"), "vm");
    }

    #[test]
    fn move_vm_moves_only_the_apps_files() -> Result<()> {
        let (tmp, app) = setup()?;
        let reg = Registry::open(&app)?;
        let old = tmp.path().join("old");
        std::fs::create_dir_all(old.join(crate::workspace::BOOT_DIR))?;
        std::fs::write(old.join(crate::workspace::VARS_FILE), "nvram")?;
        std::fs::write(old.join(crate::workspace::DISK_FILE), "disk")?;
        std::fs::write(old.join("OpenCore-Boot.sh"), "not ours")?;
        let e = reg.create(Config {
            name: "Mover".into(),
            workspace: old.display().to_string(),
            ..Config::defaults()
        })?;
        reg.set_workspace_root(&tmp.path().join("root").display().to_string())?;
        let dst = reg.vm_folder(&e.config.name);
        assert!(dst.ends_with("/root/mover"));
        let got = reg.move_vm(&e, &dst)?;
        let dstp = Path::new(&dst);
        assert_eq!(
            std::fs::read_to_string(dstp.join(crate::workspace::DISK_FILE))?,
            "disk"
        );
        assert!(dstp.join(crate::workspace::VARS_FILE).exists());
        assert!(
            !old.join(crate::workspace::DISK_FILE).exists()
                && old.join("OpenCore-Boot.sh").exists()
        );
        let vms = reg.list()?;
        assert_eq!(vms.len(), 1);
        assert_eq!(vms[0].config.workspace, dst);
        assert_eq!(got.config.workspace, dst);

        let full = tmp.path().join("full");
        std::fs::create_dir_all(&full)?;
        std::fs::write(full.join(crate::workspace::DISK_FILE), "other")?;
        assert!(reg.move_vm(&got, &full.display().to_string()).is_err());
        assert!(
            dstp.join(crate::workspace::DISK_FILE).exists(),
            "nothing moved"
        );
        assert!(
            reg.move_vm(&got, &format!("{dst}/inner")).is_err(),
            "moving into itself"
        );
        Ok(())
    }

    #[test]
    fn migration_retries_after_a_failed_import() -> Result<()> {
        let (_tmp, app) = setup()?;
        let dir = app.path().join("vms");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("a.json"), r#"{"name":"A","workspace":"/tmp/a"}"#)?;
        std::fs::write(dir.join("b.json"), "{broken")?;
        assert!(
            Registry::open(&app).is_err(),
            "a malformed legacy file stops the import"
        );
        std::fs::write(dir.join("b.json"), r#"{"name":"B","workspace":"/tmp/b"}"#)?;
        assert_eq!(Registry::open(&app)?.list()?.len(), 2);
        Ok(())
    }

    #[test]
    fn migration_from_version_1() -> Result<()> {
        let (_tmp, app) = setup()?;
        std::fs::create_dir_all(app.path())?;
        let old = Connection::open(app.db_path())?;
        old.execute_batch(&format!(
            "{SCHEMA_V1} PRAGMA user_version = 1;
             INSERT INTO vms (id, name, workspace, version, ram_mib, cores, vcpus, ssh_port) VALUES ('old', 'Old', '/tmp/old', 'sonoma', 8192, 4, 8, 2222);"
        ))?;
        drop(old);

        let reg = Registry::open(&app)?;
        let vms = reg.list()?;
        assert_eq!(vms.len(), 1);
        assert!(vms[0].config.networks.is_none() && !vms[0].config.root_patch);
        assert_eq!(vms[0].config.adapters()[0].mac, crate::network::LEGACY_MAC);
        let v: i64 = reg
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        assert_eq!(v, SCHEMA_VERSION);
        // Repeating the migration is harmless.
        reg.migrate(1, app.path())?;

        let mut c = vms[0].config.clone();
        c.networks = Some(crate::network::new_networks("sonoma"));
        reg.save(&vms[0], c.clone())?;
        assert_eq!(reg.list()?[0].config.networks, c.networks);
        c.version = "sequoia".into();
        c.root_patch = true;
        reg.save(&vms[0], c)?;
        assert!(reg.list()?[0].config.root_patch);
        Ok(())
    }

    #[test]
    fn legacy_and_empty_networks_survive_the_database() -> Result<()> {
        let (_tmp, app) = setup()?;
        let reg = Registry::open(&app)?;
        let legacy = reg.create(Config {
            name: "Legacy".into(),
            ..Config::defaults()
        })?;
        let none = reg.create(Config {
            name: "None".into(),
            networks: Some(vec![]),
            ..Config::defaults()
        })?;
        let vms = reg.list()?;
        assert_eq!(
            find_vm(&vms, &legacy.id).map(|e| e.config.networks.clone()),
            Some(None)
        );
        assert_eq!(
            find_vm(&vms, &none.id).map(|e| e.config.networks.clone()),
            Some(Some(vec![]))
        );
        Ok(())
    }

    #[test]
    fn preferences_and_workspace_root() -> Result<()> {
        let (_tmp, app) = setup()?;
        let reg = Registry::open(&app)?;
        assert_eq!(reg.preference(PREF_THEME), "");
        assert_eq!(reg.workspace_root(), DEFAULT_WORKSPACE);
        reg.set_preference(PREF_THEME, "dark")?;
        reg.set_preference(PREF_THEME, "light")?;
        assert_eq!(reg.preference(PREF_THEME), "light");
        reg.set_workspace_root(" ~/VMs/ ")?;
        assert_eq!(reg.workspace_root(), "~/VMs");
        assert!(reg.set_workspace_root("relative").is_err());
        reg.set_workspace_root("/a/b/../c")?;
        assert_eq!(reg.workspace_root(), "/a/c");
        assert!(reg.selected(&[]).is_none());
        Ok(())
    }

    #[test]
    fn import_settings_renames_a_taken_name() -> Result<()> {
        let (tmp, app) = setup()?;
        let reg = Registry::open(&app)?;
        let e = reg.create(Config {
            name: "From JSON".into(),
            ..Config::defaults()
        })?;
        let p = tmp.path().join("vm.json");
        std::fs::write(&p, export_vm(&e)?)?;
        assert_eq!(reg.import_settings(&p)?.config.name, "From JSON (imported)");
        // A workspace snapshot, wrapped in {"config": ...}, without a name.
        std::fs::write(
            &p,
            r#"{"_note":"x","config":{"workspace":"/tmp/s","version":"tahoe"}}"#,
        )?;
        let got = reg.import_settings(&p)?;
        assert_eq!(
            (got.config.name.as_str(), got.config.version.as_str()),
            ("vm", "tahoe")
        );
        Ok(())
    }
}
