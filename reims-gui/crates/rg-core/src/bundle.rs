//! VM bundles: an uncompressed tar with a manifest, the disk, the NVRAM, the SMBIOS identity
//! and, when present, the install media (port of `bundle.go`, same layout as the Go app's).
//! OpenCore and OVMF are not included: they are regenerated at every start.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::config::{Config, field_err};
use crate::registry::{Registry, VmEntry};
use crate::workspace::{
    BOOT_DIR, DISK_FILE, IDENTITY_FILE, MEDIA_FILE, MEDIA_MARKER, ROOT_PATCH_MARKER, VARS_FILE,
    Workspace, require_empty,
};
use crate::{Result, msg};

/// The first entry of every bundle.
pub const BUNDLE_MANIFEST: &str = "macos-kvm-gui-vm.json";
/// The only workspace files a bundle may contain, in order; import refuses any other name.
pub const BUNDLE_FILES: [&str; 6] = [
    DISK_FILE,
    VARS_FILE,
    IDENTITY_FILE,
    MEDIA_FILE,
    MEDIA_MARKER,
    ROOT_PATCH_MARKER,
];

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: i64,
    #[serde(default)]
    app: String,
    config: Config,
}

/// Counts bytes for progress (every 64 MiB and at the end) and stops when `cancel` is set.
struct Progress<'a, R, F> {
    r: R,
    done: &'a mut u64,
    total: u64,
    last: u64,
    cancel: &'a AtomicBool,
    report: &'a mut F,
}

impl<R: Read, F: FnMut(u64, u64)> Read for Progress<'_, R, F> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("operation cancelled"));
        }
        let n = self.r.read(b)?;
        *self.done += n as u64;
        if *self.done - self.last > 64 << 20 || (n == 0 && !b.is_empty()) {
            self.last = *self.done;
            (self.report)(*self.done, self.total);
        }
        Ok(n)
    }
}

/// Writes `e` as a bundle to `dst` (through `dst.part`), holding the workspace lock while the
/// disk is copied. `progress(done, total)` gets bytes. The caller makes sure the VM is shut down.
pub fn export_bundle(
    e: &VmEntry,
    dst: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<()> {
    let w = Workspace::new(e.config.workspace_dir()?);
    if !w.has_disk() {
        return Err(msg(format!(
            "{} has no disk yet: nothing to export",
            e.display_name()
        )));
    }
    w.with_lock(|| {
        let mut files = Vec::new();
        let mut total = 0;
        for f in BUNDLE_FILES {
            if let Ok(m) = std::fs::metadata(w.path(f))
                && m.is_file()
            {
                files.push(f);
                total += m.len();
            }
        }
        let mut tmp = dst.as_os_str().to_owned();
        tmp.push(".part");
        let tmp = std::path::PathBuf::from(tmp);
        let res = (|| -> Result<()> {
            let mut tw = tar::Builder::new(File::create(&tmp)?);
            let m = serde_json::to_vec_pretty(&Manifest {
                format: 1,
                app: "macos-kvm-gui".into(),
                config: e.config.clone(),
            })?;
            tw.append(&header(BUNDLE_MANIFEST, m.len() as u64, 0)?, m.as_slice())?;
            let mut done = 0;
            for f in files {
                let file = File::open(w.path(f))?;
                let meta = file.metadata()?;
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
                let h = header(f, meta.len(), mtime.map_or(0, |d| d.as_secs()))?;
                let r = Progress {
                    r: file,
                    done: &mut done,
                    total,
                    last: 0,
                    cancel,
                    report: &mut progress,
                };
                tw.append(&h, r)
                    .map_err(|err| msg(format!("exporting {f}: {err}")))?;
            }
            tw.into_inner()?.sync_all()?;
            Ok(std::fs::rename(&tmp, dst)?)
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    })
}

fn header(name: &str, size: u64, mtime: u64) -> io::Result<tar::Header> {
    let mut h = tar::Header::new_gnu();
    h.set_path(name)?;
    h.set_mode(0o644);
    h.set_size(size);
    h.set_mtime(mtime);
    h.set_entry_type(tar::EntryType::Regular);
    h.set_cksum();
    Ok(h)
}

/// Refuses a qcow2 that points at other files (a backing file or an external data file): a
/// crafted bundle could otherwise make QEMU read or write host files. Reads the qcow2 header.
pub fn check_standalone_disk(path: &Path) -> Result<()> {
    let mut h = [0u8; 80];
    let n = File::open(path)
        .and_then(|mut f| f.read(&mut h))
        .map_err(|e| msg(format!("checking the imported disk: {e}")))?;
    let be64 = |o: usize| u64::from_be_bytes(h[o..o + 8].try_into().unwrap_or_default());
    let version = u32::from_be_bytes(h[4..8].try_into().unwrap_or_default());
    let standalone = n >= 72
        && &h[..4] == b"QFI\xfb"
        && (version == 2 || version == 3)
        && be64(8) == 0 // backing_file_offset
        && (version == 2 || (n >= 80 && be64(72) & 0b100 == 0)); // incompatible: external data file
    if standalone {
        Ok(())
    } else {
        Err(msg(
            "the bundle's disk is not a standalone qcow2 (it references other files): refusing it",
        ))
    }
}

impl Registry {
    /// Unpacks a bundle into `workspace` (absolute; missing or empty) and adds the VM, without
    /// its GPU. A name in use gets " (imported)" appended. On any error nothing is left behind.
    pub fn import_bundle(
        &self,
        src: &Path,
        workspace: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<VmEntry> {
        if !workspace.is_absolute() {
            return Err(field_err("workspace", "must be an absolute path"));
        }
        // The lock keeps two imports into the same folder from cleaning up each other's files.
        Workspace::new(workspace).with_lock(|| {
            require_empty(workspace)?;
            let created = !workspace.exists();
            let res = self.unpack(src, workspace, cancel, &mut progress);
            if res.is_err() {
                for f in BUNDLE_FILES {
                    let _ = std::fs::remove_file(workspace.join(f));
                }
                let _ = std::fs::remove_dir_all(workspace.join(BOOT_DIR));
                if created {
                    let _ = std::fs::remove_dir(workspace);
                }
            }
            res
        })
    }

    fn unpack(
        &self,
        src: &Path,
        workspace: &Path,
        cancel: &AtomicBool,
        progress: &mut impl FnMut(u64, u64),
    ) -> Result<VmEntry> {
        let file = File::open(src)?;
        let total = file.metadata()?.len();
        let mut ar = tar::Archive::new(file);
        let mut entries = ar.entries()?;
        let mut first = match entries.next() {
            Some(Ok(h)) if h.path_bytes().as_ref() == BUNDLE_MANIFEST.as_bytes() => h,
            _ => return Err(msg("not a macos-kvm-gui VM bundle (no manifest)")),
        };
        let mut m = Vec::new();
        (&mut first).take(1 << 20).read_to_end(&mut m)?;
        let manifest = serde_json::Deserializer::from_slice(&m)
            .into_iter::<Manifest>()
            .next();
        let manifest = match manifest {
            Some(Ok(mf)) if mf.format == 1 => mf,
            Some(Ok(mf)) => {
                return Err(msg(format!(
                    "unsupported bundle manifest: format {}",
                    mf.format
                )));
            }
            Some(Err(e)) => return Err(msg(format!("unsupported bundle manifest: {e}"))),
            None => return Err(msg("unsupported bundle manifest: empty")),
        };
        std::fs::create_dir_all(workspace.join(BOOT_DIR))?;
        let mut done = 0;
        for entry in entries {
            let mut entry = entry?;
            let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
            if !BUNDLE_FILES.contains(&name.as_str())
                || entry.header().entry_type() != tar::EntryType::Regular
            {
                return Err(msg(format!("refusing unexpected bundle entry {name:?}")));
            }
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(workspace.join(&name))?;
            let mut r = Progress {
                r: &mut entry,
                done: &mut done,
                total,
                last: 0,
                cancel,
                report: progress,
            };
            io::copy(&mut r, &mut out)?;
            out.sync_all()?;
        }
        if !workspace.join(DISK_FILE).exists() {
            return Err(msg("the bundle has no macOS disk"));
        }
        check_standalone_disk(&workspace.join(DISK_FILE))?;
        let mut c = manifest.config;
        c.workspace = workspace.display().to_string();
        c.gpu.clear(); // a GPU address from another host means nothing here
        self.create_or_rename(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppDir;

    /// A minimal standalone qcow2 v3 header, optionally with a backing file offset.
    fn qcow2(path: &Path, backing: bool) -> io::Result<()> {
        let mut h = vec![0u8; 512];
        h[..4].copy_from_slice(b"QFI\xfb");
        h[4..8].copy_from_slice(&3u32.to_be_bytes());
        if backing {
            h[8..16].copy_from_slice(&200u64.to_be_bytes());
        }
        h[24..32].copy_from_slice(&(64u64 << 20).to_be_bytes());
        std::fs::write(path, h)
    }

    fn setup() -> Result<(tempfile::TempDir, Registry)> {
        let tmp = tempfile::tempdir()?;
        let reg = Registry::open(&AppDir::at(tmp.path().join("cfg")))?;
        Ok((tmp, reg))
    }

    fn vm_with_disk(reg: &Registry, dir: &Path, name: &str, backing: bool) -> Result<VmEntry> {
        let w = Workspace::new(dir);
        std::fs::create_dir_all(w.path(BOOT_DIR))?;
        std::fs::write(w.path(VARS_FILE), "nvram")?;
        std::fs::write(w.path(IDENTITY_FILE), "{}")?;
        std::fs::create_dir_all(w.path(crate::workspace::EFI_DIR))?;
        qcow2(&w.path(DISK_FILE), backing)?;
        let c = Config {
            name: name.into(),
            workspace: dir.display().to_string(),
            gpu: "0000:03:00.0".into(),
            ..Config::defaults()
        };
        reg.create(c)
    }

    #[test]
    fn bundle_round_trip() -> Result<()> {
        let (tmp, reg) = setup()?;
        let src = tmp.path().join("lab");
        let e = vm_with_disk(&reg, &src, "Lab", false)?;
        std::fs::write(src.join(MEDIA_FILE), "media")?;
        let dst = tmp.path().join("lab.tar");
        let mut calls = 0;
        export_bundle(&e, &dst, &AtomicBool::new(false), |_, _| calls += 1)?;
        assert!(calls > 0, "export reports progress");

        let ws2 = tmp.path().join("copy");
        let got = reg.import_bundle(&dst, &ws2, &AtomicBool::new(false), |_, _| {})?;
        assert_eq!(got.config.name, "Lab (imported)");
        assert_eq!(Path::new(&got.config.workspace), ws2);
        assert!(got.config.gpu.is_empty());
        for f in [DISK_FILE, VARS_FILE, IDENTITY_FILE, MEDIA_FILE] {
            assert_eq!(
                std::fs::read(src.join(f))?,
                std::fs::read(ws2.join(f))?,
                "{f}"
            );
        }
        assert!(
            !ws2.join(crate::workspace::EFI_DIR).exists(),
            "OpenCore is not shipped"
        );
        assert!(
            reg.import_bundle(&dst, &ws2, &AtomicBool::new(false), |_, _| {})
                .is_err(),
            "non-empty folder"
        );

        let cancelled = export_bundle(
            &e,
            &tmp.path().join("c.tar"),
            &AtomicBool::new(true),
            |_, _| {},
        );
        assert!(cancelled.is_err() && !tmp.path().join("c.tar.part").exists());
        Ok(())
    }

    #[test]
    fn bundle_rejects_a_disk_with_a_backing_file() -> Result<()> {
        let (tmp, reg) = setup()?;
        let e = vm_with_disk(&reg, &tmp.path().join("src"), "Crafted", true)?;
        let dst = tmp.path().join("crafted.tar");
        export_bundle(&e, &dst, &AtomicBool::new(false), |_, _| {})?;
        let ws = tmp.path().join("imported");
        let err = reg
            .import_bundle(&dst, &ws, &AtomicBool::new(false), |_, _| {})
            .err();
        assert!(err.is_some_and(|e| e.to_string().contains("standalone")));
        assert!(!ws.exists());
        Ok(())
    }

    #[test]
    fn bundle_rejects_unexpected_entries() -> Result<()> {
        let (tmp, reg) = setup()?;
        let evil = tmp.path().join("evil.tar");
        let mut tw = tar::Builder::new(File::create(&evil)?);
        let m = br#"{"format":1,"config":{"name":"Evil","workspace":"/tmp/x","version":"sonoma","ram_mib":8192,"cores":4,"vcpus":8,"ssh_port":2222}}"#;
        tw.append(&header(BUNDLE_MANIFEST, m.len() as u64, 0)?, &m[..])?;
        let mut h = tar::Header::new_gnu();
        // set_path refuses "..", as an attacker's tool would not: write the raw name.
        h.as_old_mut().name[..16].copy_from_slice(b"../../escape.txt");
        h.set_size(2);
        h.set_mode(0o644);
        h.set_cksum();
        tw.append(&h, &b"hi"[..])?;
        tw.into_inner()?;

        let ws = tmp.path().join("parent/ws");
        let err = reg
            .import_bundle(&evil, &ws, &AtomicBool::new(false), |_, _| {})
            .err();
        assert!(err.is_some_and(|e| e.to_string().contains("unexpected bundle entry")));
        assert!(!tmp.path().join("escape.txt").exists() && !ws.exists());
        assert!(reg.list()?.is_empty());
        Ok(())
    }

    #[test]
    fn reads_a_bundle_written_by_go() -> Result<()> {
        // Same layout as Go's archive/tar output (USTAR headers, manifest first).
        let (tmp, reg) = setup()?;
        let tarp = tmp.path().join("go.tar");
        let mut tw = tar::Builder::new(File::create(&tarp)?);
        let m = br#"{"format":1,"app":"macos-kvm-gui","config":{"name":"Go VM","workspace":"/x","version":"tahoe","ram_mib":8192,"cores":4,"vcpus":8,"ssh_port":2222,"networks":[]}}"#;
        let mut disk = Vec::new();
        let d = tmp.path().join("d");
        qcow2(&d, false)?;
        File::open(&d)?.read_to_end(&mut disk)?;
        for (name, data) in [
            (BUNDLE_MANIFEST, &m[..]),
            (DISK_FILE, &disk[..]),
            (VARS_FILE, &b"nv"[..]),
        ] {
            let mut h = tar::Header::new_ustar();
            h.set_path(name)?;
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tw.append(&h, data)?;
        }
        tw.into_inner()?;
        let e = reg.import_bundle(
            &tarp,
            &tmp.path().join("ws"),
            &AtomicBool::new(false),
            |_, _| {},
        )?;
        assert_eq!(e.config.version, "tahoe");
        assert_eq!(e.config.networks, Some(vec![]));
        Ok(())
    }
}
