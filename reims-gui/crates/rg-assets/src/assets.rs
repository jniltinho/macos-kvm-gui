//! The files embedded at build time from `reims-gui/assets` (a copy of the Go app's
//! `internal/assets`, refreshed by `scripts/sync-assets.sh`), or a folder with the same layout
//! named by `MKG_ASSETS_DIR`.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use include_dir::{Dir, DirEntry, include_dir};

use crate::{Result, invalid};

/// `files/EFI` (the OpenCore tree), `files/ovmf` (firmware) and `files/VERSIONS`.
static FILES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../assets/files");
static TEMPLATE: &[u8] = include_bytes!("../../../assets/config.plist.tmpl");
static ICONS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../assets/icons");
static VERSION_ICONS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../assets/logo/versions");

/// The app icon (original artwork, BSD-3-Clause like the Go app).
pub static LOGO: &[u8] = include_bytes!("../../../assets/logo/logo.svg");

/// A toolbar, state or section icon by name, e.g. `icon("start")` for `icons/start.svg`.
pub fn icon(name: &str) -> Option<&'static [u8]> {
    ICONS.get_file(format!("{name}.svg")).map(|f| f.contents())
}

/// A macOS version's icon by short name, e.g. `version_icon("sonoma")`.
pub fn version_icon(short_name: &str) -> Option<&'static [u8]> {
    VERSION_ICONS
        .get_file(format!("{short_name}.svg"))
        .map(|f| f.contents())
}

/// Where the boot files come from. Paths are relative to the assets root and use `/`:
/// `config.plist.tmpl`, `files/EFI/...`, `files/ovmf/...`, `files/VERSIONS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootAssets {
    /// Built into the binary.
    Embedded,
    /// A folder with the same layout (`MKG_ASSETS_DIR`).
    Dir(PathBuf),
}

impl BootAssets {
    /// The embedded files, or `dir` (the `MKG_ASSETS_DIR` setting, resolved by the caller)
    /// once checked to have every file the embedded set has.
    pub fn open(dir: Option<&Path>) -> Result<Self> {
        let Some(dir) = dir.filter(|d| !d.as_os_str().is_empty()) else {
            return Ok(Self::Embedded);
        };
        for p in Self::Embedded.files("")? {
            if p == "files/VERSIONS" {
                continue;
            }
            if !fs::metadata(dir.join(&p)).is_ok_and(|m| !m.is_dir()) {
                return invalid(format!("MKG_ASSETS_DIR={} is missing {p}", dir.display()));
            }
        }
        Ok(Self::Dir(dir.to_path_buf()))
    }

    /// One file's contents.
    pub fn read(&self, path: &str) -> Result<Cow<'static, [u8]>> {
        match self {
            Self::Embedded if path == "config.plist.tmpl" => Ok(Cow::Borrowed(TEMPLATE)),
            Self::Embedded => match path.strip_prefix("files/").and_then(|p| FILES.get_file(p)) {
                Some(f) => Ok(Cow::Borrowed(f.contents())),
                None => invalid(format!("{path}: no such embedded file")),
            },
            Self::Dir(d) => Ok(Cow::Owned(fs::read(d.join(path))?)),
        }
    }

    /// The regular files under `prefix` (all of them for ""), as asset paths.
    pub fn files(&self, prefix: &str) -> Result<Vec<String>> {
        Ok(self
            .entries(prefix)?
            .into_iter()
            .filter(|(_, dir)| !dir)
            .map(|(p, _)| p)
            .collect())
    }

    /// Every folder and file under `prefix` (a folder, or "" for everything), parents first.
    pub(crate) fn entries(&self, prefix: &str) -> Result<Vec<(String, bool)>> {
        let mut out = Vec::new();
        match self {
            Self::Embedded => {
                if prefix.is_empty() {
                    out.push(("config.plist.tmpl".into(), false));
                    walk_embedded(&FILES, "files", &mut out);
                } else if prefix == "files" {
                    walk_embedded(&FILES, "files", &mut out);
                } else if let Some(d) = prefix.strip_prefix("files/").and_then(|p| FILES.get_dir(p))
                {
                    walk_embedded(d, "files", &mut out);
                }
            }
            Self::Dir(root) => walk_dir(root, &root.join(prefix), &mut out)?,
        }
        Ok(out)
    }

    /// Go `AssetVersions`: the versions in `files/VERSIONS` and where the files come from.
    pub fn versions(&self) -> String {
        let src = match self {
            Self::Embedded => "built in".to_string(),
            Self::Dir(d) => format!("from MKG_ASSETS_DIR={}", d.display()),
        };
        match self.read("files/VERSIONS") {
            Ok(v) => format!(
                "{} ({src})",
                String::from_utf8_lossy(&v).trim().replace('\n', ", ")
            ),
            Err(_) => format!("unknown versions, {src}"),
        }
    }
}

/// Go `AssetVersions()` for the `MKG_ASSETS_DIR` setting (`None` or empty = built in).
pub fn asset_versions(dir: Option<&Path>) -> String {
    match BootAssets::open(dir) {
        Ok(a) => a.versions(),
        Err(e) => e.to_string(),
    }
}

/// include_dir paths are relative to the included folder; `base` is that folder's asset path.
fn walk_embedded(d: &Dir<'static>, base: &str, out: &mut Vec<(String, bool)>) {
    if !d.path().as_os_str().is_empty() {
        out.push((format!("{base}/{}", d.path().display()), true));
    }
    for e in d.entries() {
        match e {
            DirEntry::Dir(sub) => walk_embedded(sub, base, out),
            DirEntry::File(f) => out.push((format!("{base}/{}", f.path().display()), false)),
        }
    }
}

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<(String, bool)>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    if dir != root {
        out.push((rel(root, dir), true));
    }
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk_dir(root, &p, out)?;
        } else {
            out.push((rel(root, &p), false));
        }
    }
    Ok(())
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testutil::tempdir;

    fn assets_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    #[test]
    fn embedded_set_has_the_boot_files() -> Result<()> {
        let a = BootAssets::Embedded;
        let files = a.files("")?;
        for f in [
            "config.plist.tmpl",
            "files/EFI/BOOT/BOOTx64.efi",
            "files/EFI/OC/OpenCore.efi",
            "files/EFI/OC/Kexts/Lilu.kext/Contents/MacOS/Lilu",
            "files/ovmf/OVMF_CODE_4M.fd",
            "files/ovmf/OVMF_VARS_4M.fd",
        ] {
            assert!(files.iter().any(|p| p == f), "{f} missing");
            assert!(!a.read(f)?.is_empty());
        }
        assert!(
            a.entries("files/EFI")?
                .contains(&("files/EFI/OC/Kexts".into(), true))
        );
        assert!(icon("start").is_some() && version_icon("sonoma").is_some() && !LOGO.is_empty());
        Ok(())
    }

    #[test]
    fn assets_dir_override_is_reported_and_checked() -> Result<()> {
        let v = asset_versions(None);
        assert!(v.contains("OpenCore 1.") && v.contains("built in"), "{v}");
        let dir = assets_dir();
        let v = asset_versions(Some(&dir));
        assert!(
            v.contains(&format!("from MKG_ASSETS_DIR={}", dir.display())),
            "{v}"
        );
        let a = BootAssets::open(Some(&dir))?;
        assert_eq!(
            a.read("files/EFI/OC/OpenCore.efi")?,
            BootAssets::Embedded.read("files/EFI/OC/OpenCore.efi")?
        );

        let empty = tempdir();
        let err = BootAssets::open(Some(empty.path()))
            .err()
            .map(|e| e.to_string());
        assert!(err.unwrap_or_default().contains("missing"));

        // A copy without the firmware's fallback loader is refused too.
        let partial = tempdir();
        crate::util::copy_tree(&dir, partial.path())?;
        fs::remove_file(partial.path().join("files/EFI/BOOT/BOOTx64.efi"))?;
        let err = BootAssets::open(Some(partial.path()))
            .err()
            .map(|e| e.to_string());
        assert!(err.unwrap_or_default().contains("BOOTx64.efi"));
        Ok(())
    }
}
