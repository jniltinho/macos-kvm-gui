//! A VM's folder: its layout, the per-Workspace lock shared with the Go app, disk and media
//! files (port of `workspace.go` without the recovery download, and of `boot.go`'s layout).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::config::clean;
use crate::{Error, Result, msg};

/// The install media (raw recovery image).
pub const MEDIA_FILE: &str = "BaseSystem.img";
/// The macOS disk (qcow2).
pub const DISK_FILE: &str = "mac_hdd_ng.img";
/// Records the media's macOS version.
pub const MEDIA_MARKER: &str = ".macos-kvm-gui-media";
/// Where a recovery download is staged.
pub const STAGING_DIR: &str = ".macos-kvm-gui-staging";
/// The app's boot folder inside a workspace.
pub const BOOT_DIR: &str = "macos-kvm-gui";
/// OpenCore's EFI, exposed to QEMU as a FAT disk (vvfat).
pub const EFI_DIR: &str = "macos-kvm-gui/efi";
pub const OVMF_CODE: &str = "macos-kvm-gui/OVMF_CODE_4M.fd";
/// The VM's NVRAM: created once, never overwritten.
pub const VARS_FILE: &str = "macos-kvm-gui/OVMF_VARS_4M.fd";
/// The SMBIOS identity.
pub const IDENTITY_FILE: &str = "macos-kvm-gui/smbios.json";
/// Records that the last boot had root-patch mode on.
pub const ROOT_PATCH_MARKER: &str = "macos-kvm-gui/root-patch-on";
/// The generated, read-only copy of a VM's settings.
pub const SNAPSHOT_FILE: &str = "macos-kvm-gui/vm.json";
/// The smallest disk the app creates.
pub const MIN_DISK_GIB: u64 = 40;
/// Free space a recovery download needs.
pub const MIN_FREE_GIB: u64 = 4;
pub const GIB: u64 = 1 << 30;

/// The files and folders the app creates in a workspace; nothing else there is ever deleted or
/// moved.
pub const WORKSPACE_FILES: [&str; 5] = [DISK_FILE, MEDIA_FILE, MEDIA_MARKER, STAGING_DIR, BOOT_DIR];

/// A VM's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub dir: PathBuf,
}

/// An exclusive Workspace lock; released on drop (or by the kernel if the app dies).
#[derive(Debug)]
pub struct WorkspaceLock(#[expect(dead_code, reason = "held for its flock")] File);

impl Workspace {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// A file of the layout, e.g. `w.path(DISK_FILE)`.
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Takes the Workspace's exclusive flock without waiting: [`Error::Busy`] when another
    /// operation, app instance or the Go app holds it. The lock file is [`lock_path`].
    pub fn lock(&self) -> Result<WorkspaceLock> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(lock_path(&self.dir, runtime.as_deref()))?;
        match f.try_lock() {
            Ok(()) => Ok(WorkspaceLock(f)),
            Err(std::fs::TryLockError::WouldBlock) => Err(Error::Busy),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }

    pub(crate) fn with_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let _lock = self.lock()?;
        f()
    }

    /// Whether the folder exists.
    pub fn exists(&self) -> bool {
        self.dir.is_dir()
    }

    pub fn has_media(&self) -> bool {
        self.path(MEDIA_FILE).exists()
    }

    pub fn has_disk(&self) -> bool {
        self.path(DISK_FILE).exists()
    }

    /// The recorded version of the media: empty without media, `unknown` without a marker.
    pub fn media_version(&self) -> String {
        if !self.has_media() {
            return String::new();
        }
        std::fs::read_to_string(self.path(MEDIA_MARKER))
            .map_or_else(|_| "unknown".into(), |v| v.trim().into())
    }

    /// Creates the thin qcow2 disk with qemu-img, under the lock. `Some(warning)`: created, with
    /// a caveat (little free space).
    pub fn create_disk(&self, size_gib: u64) -> Result<Option<String>> {
        if size_gib < MIN_DISK_GIB {
            return Err(msg(format!("the disk must be at least {MIN_DISK_GIB} GiB")));
        }
        self.with_lock(|| {
            if self.has_disk() {
                return Err(msg(format!("{DISK_FILE} already exists; nothing was changed")));
            }
            let out = Command::new("qemu-img")
                .args(["create", "-f", "qcow2"])
                .arg(self.path(DISK_FILE))
                .arg(format!("{size_gib}G"))
                .output()?;
            if !out.status.success() {
                let text = String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
                let last = text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or_default();
                return Err(msg(format!("qemu-img: {} ({last})", out.status)));
            }
            Ok(free_bytes(&self.dir).filter(|&free| free < size_gib * GIB).map(|free| {
                format!(
                    "the host has only {:.1} GiB free: the disk is thin and the VM may stop when space runs out",
                    free as f64 / GIB as f64
                )
            }))
        })
    }

    /// Deletes the install media under the lock and returns the bytes freed.
    pub fn remove_media(&self) -> Result<u64> {
        self.with_lock(|| {
            let p = self.path(MEDIA_FILE);
            if !p.exists() {
                return Err(msg("there is no install media to remove"));
            }
            let freed = allocated(&p);
            std::fs::remove_file(&p)?;
            let _ = std::fs::remove_file(self.path(MEDIA_MARKER));
            Ok(freed)
        })
    }
}

/// The lock file of a workspace, the Go app's exact protocol:
/// `<$XDG_RUNTIME_DIR or temp dir>/macos-kvm-gui-<hex of the first 8 bytes of
/// sha256(real_path(dir))>.lock`.
pub fn lock_path(dir: &Path, runtime_dir: Option<&Path>) -> PathBuf {
    let sum = Sha256::digest(real_path(dir).as_os_str().as_bytes());
    let hex: String = sum[..8].iter().map(|b| format!("{b:02x}")).collect();
    runtime_dir
        .map_or_else(std::env::temp_dir, Path::to_path_buf)
        .join(format!("macos-kvm-gui-{hex}.lock"))
}

/// Resolves symlinks so two spellings of one folder get one lock; a missing path is resolved as
/// far as it exists. Symlinks are resolved before `..`, as the kernel does.
pub fn real_path(p: &Path) -> PathBuf {
    if let Ok(r) = std::fs::canonicalize(p) {
        return r;
    }
    let bytes = p.as_os_str().as_bytes();
    if let Some(i) = bytes.iter().rposition(|&b| b == b'/') {
        let (dir, base) = bytes.split_at(i + 1);
        if dir != bytes {
            let dir = Path::new(std::ffi::OsStr::from_bytes(dir));
            return clean(&real_path(dir).join(std::ffi::OsStr::from_bytes(base)));
        }
    }
    clean(p)
}

/// Bytes actually used on the host (sparse files count less), 0 on error.
pub fn allocated(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.blocks() * 512)
}

/// Free bytes for unprivileged users on the filesystem of `dir`.
pub fn free_bytes(dir: &Path) -> Option<u64> {
    rustix::fs::statvfs(dir)
        .ok()
        .map(|s| s.f_bavail * s.f_frsize)
}

/// Fails unless `dir` is missing or an empty, readable folder.
pub fn require_empty(dir: &Path) -> Result<()> {
    match std::fs::read_dir(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(msg(format!("cannot check {}: {e}", dir.display()))),
        Ok(mut entries) => match entries.next() {
            Some(_) => Err(msg(format!(
                "{} is not empty: choose a new folder",
                dir.display()
            ))),
            None => Ok(()),
        },
    }
}

/// Writes through a temporary file in the same folder, then renames it over `path`.
pub fn write_file_atomic(path: &Path, data: &[u8], mode: u32) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".tmp-{}", fastrand::u64(..)));
    let res = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(data)?;
        f.set_permissions(std::fs::Permissions::from_mode(mode))?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(res?)
}

/// Removes a file or a whole folder (Go's `os.RemoveAll` on one path).
pub(crate) fn remove_all(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_outside_the_workspace() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let w = Workspace::new(tmp.path());
        let lock = w.lock()?;
        assert!(matches!(w.lock(), Err(Error::Busy)));
        assert_eq!(std::fs::read_dir(tmp.path())?.count(), 0);
        drop(lock);
        w.lock()?;
        Ok(())
    }

    #[test]
    fn lock_follows_symlinks() -> Result<()> {
        let real = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        let link = other.path().join("link");
        std::os::unix::fs::symlink(real.path(), &link)?;
        let _lock = Workspace::new(real.path()).lock()?;
        assert!(matches!(Workspace::new(&link).lock(), Err(Error::Busy)));
        Ok(())
    }

    #[test]
    fn lock_path_matches_go() {
        // Generated by a throwaway Go program calling workspace.go's realPath and Lock's naming.
        let p = lock_path(
            Path::new("/nonexistent-rg-fixture/vms/../lab"),
            Some(Path::new("/run/user/1000")),
        );
        assert_eq!(p, Path::new(GO_LOCK_PATH));
    }
    const GO_LOCK_PATH: &str = "/run/user/1000/macos-kvm-gui-fcd949a3d4c9ef1b.lock";

    #[test]
    fn real_path_resolves_symlinks_before_dotdot() -> Result<()> {
        let base = tempfile::tempdir()?;
        std::fs::create_dir_all(base.path().join("srv/subdir"))?;
        std::fs::create_dir_all(base.path().join("srv/mac"))?;
        let alias = base.path().join("alias");
        std::os::unix::fs::symlink(base.path().join("srv/subdir"), &alias)?;
        let want = std::fs::canonicalize(base.path())?
            .join("srv/mac")
            .join(DISK_FILE);
        let got = real_path(Path::new(&format!(
            "{}/../mac/{DISK_FILE}",
            alias.display()
        )));
        assert_eq!(got, want);
        Ok(())
    }

    #[test]
    fn media_removal_and_disk_checks() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let w = Workspace::new(tmp.path());
        assert!(w.create_disk(39).is_err(), "39 GiB must be refused");
        std::fs::write(w.path(DISK_FILE), "disk")?;
        assert!(
            w.create_disk(80).is_err(),
            "an existing disk is never overwritten"
        );
        assert_eq!(std::fs::read(w.path(DISK_FILE))?, b"disk");

        assert_eq!(w.media_version(), "");
        std::fs::write(w.path(MEDIA_FILE), "m".repeat(1 << 20))?;
        assert_eq!(w.media_version(), "unknown");
        std::fs::write(w.path(MEDIA_MARKER), "sonoma\n")?;
        assert_eq!(w.media_version(), "sonoma");
        let freed = w.remove_media()?;
        assert!(freed >= 1 << 20 && !w.has_media() && !w.path(MEDIA_MARKER).exists());
        let _lock = w.lock()?;
        std::fs::write(w.path(MEDIA_FILE), "m")?;
        assert!(matches!(w.remove_media(), Err(Error::Busy)));
        Ok(())
    }

    #[test]
    fn create_disk_with_qemu_img() -> Result<()> {
        if Command::new("qemu-img").arg("--version").output().is_err() {
            return Ok(()); // qemu-img missing (the build image has none)
        }
        let tmp = tempfile::tempdir()?;
        let w = Workspace::new(tmp.path());
        w.create_disk(80)?;
        assert!(allocated(&w.path(DISK_FILE)) < 10 << 20);
        assert_eq!(
            crate::network::disk_virtual_size(&w.path(DISK_FILE)),
            80 * GIB
        );
        Ok(())
    }

    #[test]
    fn require_empty_folders() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        require_empty(&tmp.path().join("missing"))?;
        require_empty(tmp.path())?;
        std::fs::write(tmp.path().join("f"), "")?;
        assert!(require_empty(tmp.path()).is_err());
        Ok(())
    }
}
