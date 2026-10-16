package core

import (
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	_ "modernc.org/sqlite" // pure-Go SQLite driver, registered as "sqlite"
)

// The VMs live in a small SQLite database, <config dir>/macos-kvm-gui/macos-kvm-gui.db, which
// is the source of truth. Files are generated from it: each save writes a snapshot to
// <workspace>/macos-kvm-gui/vm.json, and OpenCore's config.plist is generated at every start.

// VMEntry is one VM. Path is set only for a VM read from a standalone JSON file (--config).
type VMEntry struct {
	ID     string
	Path   string
	Config Config
}

// DisplayName is the VM's name, or a name derived from its macOS version.
func (e VMEntry) DisplayName() string {
	if e.Config.Name != "" {
		return e.Config.Name
	}
	return "macOS " + e.Config.Version
}

func appConfigDir() (string, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "macos-kvm-gui"), nil
}

// DBPath is the database file.
func DBPath() (string, error) {
	dir, err := appConfigDir()
	return filepath.Join(dir, "macos-kvm-gui.db"), err
}

const schemaV1 = `
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
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);`

// schemaVersion is the database's PRAGMA user_version: 1 created the tables, 2 added the
// network adapters, 3 root-patch mode, 4 the graphics adapter, 5 the Reims resolution.
const schemaVersion = 5

// migrate brings a database from version v to schemaVersion. Each step is safe to repeat, and
// the version is written last, so an interrupted upgrade runs again on the next start.
func migrate(db *sql.DB, v int) error {
	if v < 1 {
		if _, err := db.Exec(schemaV1); err != nil {
			return err
		}
	}
	if v < 2 {
		var n int
		if err := db.QueryRow(`SELECT count(*) FROM pragma_table_info('vms') WHERE name = 'networks'`).Scan(&n); err != nil {
			return err
		}
		if n == 0 {
			// The app and the CLI may upgrade at the same moment: the column the other one
			// just added is fine.
			if _, err := db.Exec(`ALTER TABLE vms ADD COLUMN networks TEXT NOT NULL DEFAULT ''`); err != nil &&
				!strings.Contains(err.Error(), "duplicate column name") {
				return err
			}
		}
	}
	if v < 3 {
		var n int
		if err := db.QueryRow(`SELECT count(*) FROM pragma_table_info('vms') WHERE name = 'root_patch'`).Scan(&n); err != nil {
			return err
		}
		if n == 0 {
			if _, err := db.Exec(`ALTER TABLE vms ADD COLUMN root_patch INTEGER NOT NULL DEFAULT 0`); err != nil &&
				!strings.Contains(err.Error(), "duplicate column name") {
				return err
			}
		}
	}
	if v < 4 {
		var n int
		if err := db.QueryRow(`SELECT count(*) FROM pragma_table_info('vms') WHERE name = 'graphics'`).Scan(&n); err != nil {
			return err
		}
		if n == 0 {
			if _, err := db.Exec(`ALTER TABLE vms ADD COLUMN graphics TEXT NOT NULL DEFAULT ''`); err != nil &&
				!strings.Contains(err.Error(), "duplicate column name") {
				return err
			}
		}
	}
	if v < 5 {
		var n int
		if err := db.QueryRow(`SELECT count(*) FROM pragma_table_info('vms') WHERE name = 'resolution'`).Scan(&n); err != nil {
			return err
		}
		if n == 0 {
			if _, err := db.Exec(`ALTER TABLE vms ADD COLUMN resolution TEXT NOT NULL DEFAULT ''`); err != nil &&
				!strings.Contains(err.Error(), "duplicate column name") {
				return err
			}
		}
	}
	if v < 1 {
		// Imported files are renamed as they are read, so a retry does not read them twice.
		if err := importFiles(db); err != nil {
			return err
		}
	}
	_, err := db.Exec(fmt.Sprintf(`PRAGMA user_version = %d`, schemaVersion))
	return err
}

// openDB opens the database, creating and migrating it as needed. WAL and a busy timeout let
// the app and the CLI use it at the same time.
func openDB() (*sql.DB, error) {
	path, err := DBPath()
	if err != nil {
		return nil, err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return nil, err
	}
	db, err := sql.Open("sqlite", "file:"+path+"?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)")
	if err != nil {
		return nil, err
	}
	var v int
	if err := db.QueryRow(`PRAGMA user_version`).Scan(&v); err != nil {
		db.Close()
		return nil, fmt.Errorf("database %s: %w", path, err)
	}
	if v < schemaVersion {
		if err := migrate(db, v); err != nil {
			db.Close()
			return nil, fmt.Errorf("upgrading %s: %w", path, err)
		}
	}
	return db, nil
}

const vmColumns = `id, name, workspace, version, ram_mib, cores, vcpus, ssh_port, display, gpu, networks, root_patch, graphics, resolution`

// networksJSON stores the adapters; "" keeps a VM on its legacy single NAT adapter.
func networksJSON(c Config) string {
	if c.Networks == nil {
		return ""
	}
	b, _ := json.Marshal(c.Networks)
	return string(b)
}

func scanVM(row interface{ Scan(...any) error }) (VMEntry, error) {
	var e VMEntry
	c := &e.Config
	var nets string
	err := row.Scan(&e.ID, &c.Name, &c.Workspace, &c.Version, &c.RAMMiB, &c.Cores, &c.VCPUs, &c.SSHPort, &c.Display, &c.GPU, &nets, &c.RootPatch, &c.Graphics, &c.Resolution)
	if err == nil && nets != "" {
		if jerr := json.Unmarshal([]byte(nets), &c.Networks); jerr != nil {
			return e, fmt.Errorf("VM %s: unreadable network adapters: %w", e.ID, jerr)
		}
	}
	return e, err
}

// ListVMs returns the VMs sorted by name.
func ListVMs() ([]VMEntry, error) {
	db, err := openDB()
	if err != nil {
		return nil, err
	}
	defer db.Close()
	rows, err := db.Query(`SELECT ` + vmColumns + ` FROM vms ORDER BY name COLLATE NOCASE`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var vms []VMEntry
	for rows.Next() {
		e, err := scanVM(rows)
		if err != nil {
			return nil, err
		}
		vms = append(vms, e)
	}
	return vms, rows.Err()
}

var nonSlug = regexp.MustCompile(`[^a-z0-9]+`)

// Slug turns a name into a file- and id-safe word, for example "Tahoe lab" → "tahoe-lab".
func Slug(name string) string {
	s := strings.Trim(nonSlug.ReplaceAllString(strings.ToLower(name), "-"), "-")
	if s == "" {
		return "vm"
	}
	return s
}

// DefaultWorkspace is the app's Workspace when none is saved: the parent folder of every VM.
const DefaultWorkspace = "~/macOS-VMs"

// WorkspaceRoot is the folder that holds one subfolder per VM. VMs added before it existed (for
// example an OSX-KVM folder) keep their own folder.
func WorkspaceRoot() string {
	if r := Preference("workspace"); r != "" {
		return r
	}
	return DefaultWorkspace
}

// SetWorkspaceRoot validates and saves the Workspace. Existing VMs are not moved.
func SetWorkspaceRoot(root string) error {
	root = strings.TrimSpace(root)
	dir, err := (Config{Workspace: root}).WorkspaceDir()
	if err != nil {
		return err
	}
	if root = filepath.Clean(root); strings.Contains(root, "..") {
		root = dir // "~/../x" is stored expanded so VMFolder stays absolute
	}
	return SetPreference("workspace", root)
}

// VMFolder is the default folder for a new VM: <Workspace>/<slug of its name>.
func VMFolder(name string) string { return filepath.Join(WorkspaceRoot(), Slug(name)) }

func isUniqueErr(err error) bool {
	return err != nil && strings.Contains(err.Error(), "UNIQUE constraint failed")
}

// CreateVM validates c, stores it as a new VM and returns its entry.
func CreateVM(c Config) (VMEntry, error) {
	c.Name = strings.TrimSpace(c.Name)
	if c.Name == "" {
		return VMEntry{}, &FieldError{"name", "give the VM a name"}
	}
	if err := c.Validate(); err != nil {
		return VMEntry{}, err
	}
	db, err := openDB()
	if err != nil {
		return VMEntry{}, err
	}
	defer db.Close()
	return insertVM(db, c)
}

func insertVM(db *sql.DB, c Config) (VMEntry, error) {
	base := Slug(c.Name)
	id := base
	for i := 2; ; i++ {
		var n int
		if err := db.QueryRow(`SELECT count(*) FROM vms WHERE id = ?`, id).Scan(&n); err != nil {
			return VMEntry{}, err
		}
		if n == 0 {
			break
		}
		id = fmt.Sprintf("%s-%d", base, i)
	}
	_, err := db.Exec(`INSERT INTO vms (`+vmColumns+`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		id, c.Name, c.Workspace, c.Version, c.RAMMiB, c.Cores, c.VCPUs, c.SSHPort, c.Display, c.GPU, networksJSON(c), c.RootPatch, c.Graphics, c.Resolution)
	if isUniqueErr(err) {
		return VMEntry{}, &FieldError{"name", "a VM with this name already exists"}
	}
	if err != nil {
		return VMEntry{}, err
	}
	writeSnapshot(c)
	return VMEntry{ID: id, Config: c}, nil
}

// SaveVM validates and stores new settings for e, and refreshes its generated snapshot.
func SaveVM(e VMEntry, c Config) error {
	if e.Path != "" { // a standalone --config file
		return SaveConfig(e.Path, c)
	}
	c.Name = strings.TrimSpace(c.Name)
	if c.Name == "" {
		return &FieldError{"name", "give the VM a name"}
	}
	if err := c.Validate(); err != nil {
		return err
	}
	db, err := openDB()
	if err != nil {
		return err
	}
	defer db.Close()
	res, err := db.Exec(`UPDATE vms SET name = ?, workspace = ?, version = ?, ram_mib = ?, cores = ?, vcpus = ?, ssh_port = ?,
		display = ?, gpu = ?, networks = ?, root_patch = ?, graphics = ?, resolution = ?, updated_at = datetime('now') WHERE id = ?`,
		c.Name, c.Workspace, c.Version, c.RAMMiB, c.Cores, c.VCPUs, c.SSHPort, c.Display, c.GPU, networksJSON(c), c.RootPatch, c.Graphics, c.Resolution, e.ID)
	if isUniqueErr(err) {
		return &FieldError{"name", "a VM with this name already exists"}
	}
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return fmt.Errorf("VM %s no longer exists", e.ID)
	}
	writeSnapshot(c)
	return nil
}

// snapshotFile is the generated, read-only copy of a VM's settings inside its workspace.
const snapshotFile = BootDir + "/vm.json"

// writeSnapshot writes the generated copy when the workspace exists; it never creates one.
func writeSnapshot(c Config) {
	dir, err := c.WorkspaceDir()
	if err != nil || !exists(dir) {
		return
	}
	data, _ := json.MarshalIndent(struct {
		Note   string `json:"_note"`
		Config Config `json:"config"`
	}{"Generated from macos-kvm-gui.db; edit the VM in the app or with the CLI, not here.", c}, "", "  ")
	if os.MkdirAll(filepath.Join(dir, BootDir), 0o755) == nil {
		writeFileAtomic(filepath.Join(dir, snapshotFile), data, 0o644)
	}
}

// ExportVM returns e's settings as JSON, the format --config reads.
func ExportVM(e VMEntry) ([]byte, error) { return json.MarshalIndent(e.Config, "", "  ") }

// workspaceFiles are the files and folders the app itself creates in a workspace; nothing
// else there is ever deleted.
var workspaceFiles = []string{DiskFile, MediaFile, mediaMarker, stagingDir, BootDir}

// DeleteVM removes the VM. With purge it also deletes, under the workspace lock, the files the
// app created in the workspace, and the workspace folder if it ends up empty. It returns what
// was deleted.
func DeleteVM(e VMEntry, purge bool) ([]string, error) {
	var removed []string
	if purge {
		if _, ok := FindRunning(e.Config); ok {
			return nil, errors.New("shut down the VM before deleting its files")
		}
		dir, err := e.Config.WorkspaceDir()
		if err != nil {
			return nil, err
		}
		w := NewWorkspace(dir)
		err = w.withLock(func() error {
			for _, f := range workspaceFiles {
				p := w.Path(f)
				if !exists(p) {
					continue
				}
				if err := os.RemoveAll(p); err != nil {
					return err
				}
				removed = append(removed, p)
			}
			if entries, err := os.ReadDir(w.Dir); err == nil && len(entries) == 0 && os.Remove(w.Dir) == nil {
				removed = append(removed, w.Dir)
			}
			return nil
		})
		if err != nil {
			return removed, err
		}
	}
	if e.Path != "" {
		if err := os.Remove(e.Path); err != nil && !errors.Is(err, os.ErrNotExist) {
			return removed, err
		}
		return append(removed, e.Path), nil
	}
	db, err := openDB()
	if err != nil {
		return removed, err
	}
	defer db.Close()
	if _, err := db.Exec(`DELETE FROM vms WHERE id = ?`, e.ID); err != nil {
		return removed, err
	}
	return append(removed, "VM "+e.ID+" (database)"), nil
}

// SelectedVM returns the remembered VM, or the first one.
func SelectedVM(vms []VMEntry) (VMEntry, bool) {
	if len(vms) == 0 {
		return VMEntry{}, false
	}
	if db, err := openDB(); err == nil {
		defer db.Close()
		var id string
		if db.QueryRow(`SELECT value FROM settings WHERE key = 'selected'`).Scan(&id) == nil {
			for _, e := range vms {
				if e.ID == id {
					return e, true
				}
			}
		}
	}
	return vms[0], true
}

// SelectVM remembers the VM the app and the CLI act on.
func SelectVM(id string) error {
	db, err := openDB()
	if err != nil {
		return err
	}
	defer db.Close()
	_, err = db.Exec(`INSERT INTO settings (key, value) VALUES ('selected', ?)
		ON CONFLICT(key) DO UPDATE SET value = excluded.value`, id)
	return err
}

// FindVM looks a VM up by id, then by name (case-insensitive): an id always wins, so a VM
// later named like another VM's id cannot be picked instead of it.
func FindVM(vms []VMEntry, key string) (VMEntry, bool) {
	for _, e := range vms {
		if e.ID == key {
			return e, true
		}
	}
	for _, e := range vms {
		if strings.EqualFold(e.DisplayName(), key) {
			return e, true
		}
	}
	return VMEntry{}, false
}

// importFiles moves VMs stored as files by older versions into a new database: the JSON files
// under vms/ and the single-VM config.json. The originals are renamed, not deleted.
func importFiles(db *sql.DB) error {
	dir, err := appConfigDir()
	if err != nil {
		return err
	}
	vmsDir := filepath.Join(dir, "vms")
	files, _ := filepath.Glob(filepath.Join(vmsDir, "*.json"))
	legacy := filepath.Join(dir, "config.json")
	if exists(legacy) {
		files = append(files, legacy)
	}
	for _, f := range files {
		c, err := LoadConfig(f)
		if err != nil {
			return fmt.Errorf("importing %s: %w", f, err)
		}
		if c.Name == "" {
			c.Name = "macOS " + versionDisplay(c.Version)
		}
		if _, err := insertVM(db, c); err != nil {
			var fe *FieldError
			if !errors.As(err, &fe) { // a duplicate name is skipped; anything else stops the import
				return fmt.Errorf("importing %s: %w", f, err)
			}
		}
	}
	if sel, err := os.ReadFile(filepath.Join(dir, "selected")); err == nil {
		db.Exec(`INSERT OR REPLACE INTO settings (key, value) VALUES ('selected', ?)`, strings.TrimSpace(string(sel)))
		os.Rename(filepath.Join(dir, "selected"), filepath.Join(dir, "selected.migrated"))
	}
	if exists(vmsDir) {
		os.Rename(vmsDir, vmsDir+".migrated")
	}
	if exists(legacy) {
		os.Rename(legacy, legacy+".migrated")
	}
	return nil
}

func versionDisplay(short string) string {
	for _, v := range Versions {
		if v.Short == short {
			name, _, _ := strings.Cut(v.Name, " (")
			return name
		}
	}
	return short
}

// MoveVM moves the files the app created for e (disk, install media, NVRAM, identity, boot
// files) into dst, which must be new or empty, and points the VM there. Files are renamed, so dst
// must be on the same filesystem; on any error everything is put back. Other files in the old
// folder (for example an OSX-KVM checkout) stay where they are. Both folders are locked.
func MoveVM(e VMEntry, dst string) (VMEntry, error) {
	if e.Path == "" { // use the stored entry, not a copy that may be out of date
		vms, err := ListVMs()
		if err != nil {
			return e, err
		}
		cur, ok := FindVM(vms, e.ID)
		if !ok {
			return e, fmt.Errorf("VM %s no longer exists", e.ID)
		}
		e = cur
	}
	if _, ok := FindRunning(e.Config); ok {
		return e, errors.New("shut down the VM before moving it")
	}
	c := e.Config
	c.Workspace = strings.TrimSpace(dst)
	to, err := c.WorkspaceDir()
	if err != nil {
		return e, err
	}
	from, err := e.Config.WorkspaceDir()
	if err != nil {
		return e, err
	}
	if filepath.Clean(from) == to {
		return e, nil
	}
	if rel, err := filepath.Rel(from, to); err == nil && !strings.HasPrefix(rel, "..") {
		return e, errors.New("the new folder cannot be inside the current one")
	}
	src := NewWorkspace(from)
	err = src.withLock(func() error {
		return NewWorkspace(to).withLock(func() error {
			if err := requireEmpty(to); err != nil {
				return err
			}
			if _, err := os.Stat(from); err != nil {
				return fmt.Errorf("cannot read the current folder: %w", err)
			}
			created := !exists(to)
			if err := os.MkdirAll(to, 0o755); err != nil {
				return err
			}
			var moved []string
			undo := func() {
				for _, f := range moved {
					os.Rename(filepath.Join(to, f), src.Path(f))
				}
				if created {
					os.Remove(to)
				}
			}
			for _, f := range workspaceFiles {
				if _, err := os.Lstat(src.Path(f)); errors.Is(err, os.ErrNotExist) {
					continue
				} else if err != nil {
					undo()
					return err
				}
				if _, err := os.Lstat(filepath.Join(to, f)); !errors.Is(err, os.ErrNotExist) {
					undo()
					return fmt.Errorf("%s already exists in the new folder", f)
				}
				if err := os.Rename(src.Path(f), filepath.Join(to, f)); err != nil {
					undo()
					return fmt.Errorf("moving %s: %w (the new folder must be on the same disk)", f, err)
				}
				moved = append(moved, f)
			}
			if err := SaveVM(e, c); err != nil {
				undo()
				return err
			}
			os.Remove(from) // only when nothing else is left there
			return nil
		})
	})
	if err != nil {
		return e, err
	}
	e.Config = c
	return e, nil
}

// requireEmpty fails unless dir is missing or an empty, readable folder.
func requireEmpty(dir string) error {
	entries, err := os.ReadDir(dir)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("cannot check %s: %w", dir, err)
	}
	if len(entries) > 0 {
		return fmt.Errorf("%s is not empty: choose a new folder", dir)
	}
	return nil
}

// RequireEmpty is requireEmpty for the GUI's checks before creating a VM.
func RequireEmpty(dir string) error { return requireEmpty(dir) }
