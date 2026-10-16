//! The user config folder, settings.yaml, the `.env` fallback and the extra kexts list (port of
//! `settings.go` and the types of `extrakexts.go`; the kexts' download lives in rg-assets).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::workspace::write_file_atomic;
use crate::{Result, msg};

/// settings.yaml as first written: comments only, sets nothing.
pub const DEFAULT_SETTINGS_YAML: &str = include_str!("settings.default.yaml");

/// Environment keys that settings.yaml can also set.
pub const MKG_THEME: &str = "MKG_THEME";
pub const MKG_ASSETS_DIR: &str = "MKG_ASSETS_DIR";
pub const MKG_REIMS_QEMU: &str = "MKG_REIMS_QEMU";

/// One extra kext archive listed in settings.yaml.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraKext {
    /// An `https://` URL.
    pub url: String,
    /// Lowercase hex, or empty = not checked.
    #[serde(default)]
    pub sha256: String,
    /// The kexts to load, in order (`load:`); empty = the archive's only kext.
    #[serde(default, rename = "load")]
    pub kexts: Vec<String>,
}

/// The kexts part of settings.yaml; a start reads it once and uses that snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtraKexts {
    /// Built-in kexts whose entry is disabled (`disable:`).
    pub disabled: Vec<String>,
    /// `kexts:`.
    pub entries: Vec<ExtraKext>,
}

/// settings.yaml. The environment wins over it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSettings {
    pub theme: Option<String>,
    pub assets_dir: Option<String>,
    pub reims_qemu: Option<String>,
    pub kexts: Option<Vec<ExtraKext>>,
    pub disable: Option<Vec<String>>,
}

/// The app's config folder, `~/.config/macos-kvm-gui` (shared with the Go app): the database,
/// settings.yaml and `.env` live here. Tests point it at a temporary folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDir(PathBuf);

impl AppDir {
    /// The user's folder (`dirs::config_dir()/macos-kvm-gui`), `None` without a config dir.
    pub fn user() -> Option<Self> {
        dirs::config_dir().map(|d| Self(d.join("macos-kvm-gui")))
    }

    /// A folder used as the app's config folder.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `macos-kvm-gui.db`.
    pub fn db_path(&self) -> PathBuf {
        self.0.join("macos-kvm-gui.db")
    }

    /// `settings.yaml`.
    pub fn settings_file(&self) -> PathBuf {
        self.0.join("settings.yaml")
    }

    /// `.env`, the older local settings file.
    pub fn env_file(&self) -> PathBuf {
        self.0.join(".env")
    }

    /// Reads settings.yaml (one document, unknown keys refused), first creating it from
    /// [`DEFAULT_SETTINGS_YAML`] when missing.
    pub fn load_settings(&self) -> Result<FileSettings> {
        let path = self.settings_file();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Best effort: the default sets nothing.
                let _ = std::fs::create_dir_all(&self.0)
                    .map_err(Into::into)
                    .and_then(|()| {
                        write_file_atomic(&path, DEFAULT_SETTINGS_YAML.as_bytes(), 0o644)
                    });
                DEFAULT_SETTINGS_YAML.to_owned()
            }
            Err(e) => return Err(e.into()),
        };
        parse_settings(&text).map_err(|e| msg(format!("{}: {e}", path.display())))
    }

    /// Overwrites settings.yaml with the commented default.
    pub fn reset_settings(&self) -> Result<()> {
        std::fs::create_dir_all(&self.0)?;
        write_file_atomic(
            &self.settings_file(),
            DEFAULT_SETTINGS_YAML.as_bytes(),
            0o644,
        )
    }

    /// The `kexts` and `disable` lists of settings.yaml, checked (https URLs, 64-hex sha256
    /// lowercased, safe kext names).
    pub fn load_extra_kexts(&self) -> Result<ExtraKexts> {
        let fs = self.load_settings()?;
        let mut ks = ExtraKexts {
            disabled: fs.disable.unwrap_or_default(),
            entries: fs.kexts.unwrap_or_default(),
        };
        for (i, e) in ks.entries.iter_mut().enumerate() {
            check_kext(e).map_err(|err| {
                msg(format!(
                    "{}, kext {} ({}): {err}",
                    self.settings_file().display(),
                    i + 1,
                    e.url
                ))
            })?;
        }
        Ok(ks)
    }

    /// `key` from the environment (even when set empty), then settings.yaml, then `.env`; empty
    /// when none has it.
    pub fn setting(&self, key: &str) -> String {
        match std::env::var_os(key) {
            Some(v) => v.to_string_lossy().into_owned(),
            None => self.file_setting(key),
        }
    }

    /// `key` from settings.yaml, then `.env` (the environment left out).
    pub fn file_setting(&self, key: &str) -> String {
        if let Ok(fs) = self.load_settings() {
            let v = match key {
                MKG_THEME => fs.theme,
                MKG_ASSETS_DIR => fs.assets_dir,
                MKG_REIMS_QEMU => fs.reims_qemu,
                _ => None,
            };
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                return v;
            }
        }
        let Ok(text) = std::fs::read_to_string(self.env_file()) else {
            return String::new();
        };
        for line in text.lines() {
            let Some((k, v)) = line.trim().split_once('=') else {
                continue;
            };
            if k.starts_with('#') || k.trim() != key {
                continue;
            }
            return v.trim().trim_matches(['"', '\'']).to_owned();
        }
        String::new()
    }

    /// The `reims_qemu` setting made absolute: `MKG_REIMS_QEMU` relative to the current
    /// directory, settings.yaml's or `.env`'s relative to this folder. `None` when unset or empty.
    pub fn reims_qemu_setting(&self) -> Option<PathBuf> {
        let (v, base) = match std::env::var_os(MKG_REIMS_QEMU) {
            Some(v) => (v.to_string_lossy().into_owned(), None),
            None => (self.file_setting(MKG_REIMS_QEMU), Some(&self.0)),
        };
        if v.is_empty() {
            return None;
        }
        let p = match base {
            Some(dir) if Path::new(&v).is_relative() => dir.join(v),
            _ => PathBuf::from(v),
        };
        Some(std::path::absolute(&p).unwrap_or(p))
    }
}

/// `qemu/current/bin/qemu-system-x86_64` under a Reims install folder.
const REIMS_QEMU_BIN: &str = "qemu/current/bin/qemu-system-x86_64";

/// Go `ReimsQEMU`: the `reims_qemu` setting (`setting`, which must exist; `settings_file` is
/// named when it does not), else the first install folder in `roots` (`<Workspace>/.reims`,
/// `$XDG_DATA_HOME/reims-ctl`) with a regular `qemu/current/bin/qemu-system-x86_64`.
pub fn find_reims_qemu(
    setting: Option<&Path>,
    settings_file: &Path,
    roots: &[PathBuf],
) -> Result<PathBuf> {
    if let Some(p) = setting {
        return match std::fs::metadata(p) {
            Ok(_) => Ok(p.to_path_buf()),
            Err(e) => Err(msg(format!(
                "reims_qemu in {}: {}: {e}",
                settings_file.display(),
                p.display()
            ))),
        };
    }
    let tried: Vec<PathBuf> = roots.iter().map(|r| r.join(REIMS_QEMU_BIN)).collect();
    if let Some(p) = tried
        .iter()
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()))
    {
        return Ok(p.clone());
    }
    Err(msg(format!(
        "Reims vGPU is not installed (looked for {}): install it in the Host tab or with `reims-gui reims install`",
        tried
            .first()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    )))
}

/// The upstream reims-ctl install folder, `$XDG_DATA_HOME/reims-ctl`.
pub fn reims_ctl_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("reims-ctl"))
}

fn parse_settings(text: &str) -> Result<FileSettings, String> {
    let mut docs = serde_yaml_ng::Deserializer::from_str(text);
    let fs = match docs.next() {
        Some(doc) => Option::<FileSettings>::deserialize(doc)
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
        None => FileSettings::default(),
    };
    if docs.next().is_some() {
        return Err("only one YAML document is allowed (remove the --- separators)".into());
    }
    Ok(fs)
}

fn valid_kext_name(n: &str) -> bool {
    (1..=64).contains(&n.len())
        && n.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
}

fn check_kext(e: &mut ExtraKext) -> Result<(), String> {
    if !e.url.starts_with("https://")
        || e.url.len() == "https://".len()
        || e.url.contains([' ', '\t', '\n'])
    {
        return Err("url must be an https:// URL".into());
    }
    e.sha256 = e.sha256.to_lowercase();
    if !e.sha256.is_empty()
        && !(e.sha256.len() == 64 && e.sha256.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err("sha256 must be 64 hex digits".into());
    }
    if let Some(n) = e.kexts.iter().find(|n| !valid_kext_name(n)) {
        return Err(format!("invalid kext name {n:?} in load"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> std::io::Result<(tempfile::TempDir, AppDir)> {
        let tmp = tempfile::tempdir()?;
        let app = AppDir::at(tmp.path().join("macos-kvm-gui"));
        Ok((tmp, app))
    }

    #[test]
    fn env_file_values() -> Result<()> {
        let (_tmp, app) = app()?;
        std::fs::create_dir_all(app.path())?;
        std::fs::write(
            app.env_file(),
            "# comment\nMKG_FOO = \"from-file\"\nMKG_BAR=x\n",
        )?;
        assert_eq!(app.file_setting("MKG_FOO"), "from-file");
        assert_eq!(app.file_setting("MKG_MISSING"), "");
        // The environment wins: PATH is always set.
        std::fs::write(app.env_file(), "PATH=from-file\n")?;
        assert_ne!(app.setting("PATH"), "from-file");
        Ok(())
    }

    #[test]
    fn settings_yaml_over_env_file() -> Result<()> {
        let (_tmp, app) = app()?;
        std::fs::create_dir_all(app.path())?;
        std::fs::write(
            app.env_file(),
            "MKG_THEME=dark\nMKG_ASSETS_DIR=/from-env-file\n",
        )?;
        assert_eq!(
            app.file_setting(MKG_THEME),
            "dark",
            "the default settings.yaml sets nothing"
        );
        assert_eq!(
            std::fs::read_to_string(app.settings_file())?,
            DEFAULT_SETTINGS_YAML
        );
        std::fs::write(app.settings_file(), "theme: light\nkexts: []\n")?;
        assert_eq!(app.file_setting(MKG_THEME), "light");
        assert_eq!(app.file_setting(MKG_ASSETS_DIR), "/from-env-file");
        std::fs::write(app.settings_file(), "x")?;
        app.reset_settings()?;
        assert_eq!(
            std::fs::read_to_string(app.settings_file())?,
            DEFAULT_SETTINGS_YAML
        );
        Ok(())
    }

    #[test]
    fn extra_kexts() -> Result<()> {
        let (_tmp, app) = app()?;
        let ks = app.load_extra_kexts()?;
        assert!(ks.entries.is_empty() && ks.disabled.is_empty());
        std::fs::write(app.settings_file(), "")?;
        assert!(
            app.load_extra_kexts()?.entries.is_empty(),
            "an empty file is an empty list"
        );
        std::fs::write(app.settings_file(), "# only comments\n")?;
        assert!(app.load_extra_kexts()?.entries.is_empty());
        let sha = "AB".repeat(32);
        std::fs::write(
            app.settings_file(),
            format!(
                "disable: [WhateverGreen]\nkexts:\n  - url: https://example.com/a.zip\n    sha256: {sha}\n  - url: https://example.com/b.zip\n    load: [B, A]\n"
            ),
        )?;
        let ks = app.load_extra_kexts()?;
        assert_eq!(ks.entries.len(), 2);
        assert_eq!(ks.entries[0].sha256, "ab".repeat(32));
        assert_eq!(ks.entries[1].kexts, ["B", "A"]);
        assert_eq!(ks.disabled, ["WhateverGreen"]);
        for (body, want) in [
            ("kexts:\n  - url: http://example.com/a.zip\n", "https://"),
            (
                "kexts:\n  - url: https://example.com/a.zip\n    sha256: abc\n",
                "64 hex",
            ),
            (
                "kexts:\n  - url: https://example.com/a.zip\n    load: [a/b]\n",
                "invalid kext name",
            ),
            (
                "kexts:\n  - url: https://example.com/a.zip\n    sha: x\n",
                "unknown field",
            ),
            (
                "kext:\n  - url: https://example.com/a.zip\n",
                "unknown field",
            ),
            (
                "kexts: []\n---\nkexts:\n  - url: https://example.com/a.zip\n",
                "one YAML document",
            ),
        ] {
            std::fs::write(app.settings_file(), body)?;
            let err = app
                .load_extra_kexts()
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(err.contains(want), "{body:?}: want {want:?}, got {err:?}");
        }
        std::fs::write(
            app.settings_file(),
            "kexts:\n  - url: https://example.com/a.zip\n    load: []\n",
        )?;
        assert!(app.load_extra_kexts()?.entries[0].kexts.is_empty());
        Ok(())
    }

    #[test]
    fn reims_qemu_setting_then_install_folders() -> Result<()> {
        let (tmp, app) = app()?;
        std::fs::create_dir_all(app.path())?;
        let file = app.settings_file();
        if std::env::var_os(MKG_REIMS_QEMU).is_none() {
            assert_eq!(app.reims_qemu_setting(), None);
            std::fs::write(&file, "reims_qemu: q/bin\n")?;
            assert_eq!(app.reims_qemu_setting(), Some(app.path().join("q/bin")));
        }
        let missing = tmp.path().join("missing");
        let err = find_reims_qemu(Some(&missing), &file, &[])
            .err()
            .map(|e| e.to_string());
        assert!(err.unwrap_or_default().starts_with("reims_qemu in "));
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        let err = find_reims_qemu(None, &file, &[a.clone(), b.clone()])
            .err()
            .map(|e| e.to_string());
        assert!(
            err.unwrap_or_default()
                .contains(&a.join(REIMS_QEMU_BIN).display().to_string())
        );
        std::fs::create_dir_all(b.join("qemu/current/bin"))?;
        std::fs::write(b.join(REIMS_QEMU_BIN), "")?;
        assert_eq!(
            find_reims_qemu(None, &file, &[a, b.clone()])?,
            b.join(REIMS_QEMU_BIN)
        );
        Ok(())
    }
}
