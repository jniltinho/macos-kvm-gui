//! Reims vGPU's host checks (port of the Go app's `internal/core/reimshost.go`).
//!
//! The Vulkan ICD listing (sorted `*.json` per folder, no duplicates) is adapted from Reims vGPU's
//! `crates/reims-ctl/src/gpu.rs` (`icd_files`), GPL-2.0-or-later OR LGPL-3.0-or-later, here over
//! the two folders the Go app checks.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};

use rg_core::Config;

use crate::host::{Check, Host, HostError, Status, failed, open_rw};

/// The vm.max_map_count a VM needs: 64 mappings per MiB of RAM plus a reserve.
pub fn reims_map_count(ram_mib: u32) -> u64 {
    u64::from(ram_mib) * 64 + 65536
}

/// What Set up host writes; it covers a VM of up to 30 GiB.
const SETUP_MAP_COUNT: u64 = 2_097_152;

/// path + mtime → whether `<qemu> -device help` lists apple-gfx-pci.
static PROBE_CACHE: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Mutex::default);

/// A Reims VM may start: the QEMU to run and the warnings for the VM log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReimsReady {
    pub qemu: PathBuf,
    pub warnings: Vec<String>,
}

impl Host {
    /// Runs `<qemu> -device help` once per binary version.
    fn has_apple_gfx(&self, qemu: &Path) -> bool {
        let Ok(mtime) = fs::metadata(qemu).and_then(|m| m.modified()) else {
            return false;
        };
        let key = format!("{}\0{mtime:?}", qemu.display());
        let mut cache = PROBE_CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        *cache.entry(key).or_insert_with(|| {
            let out =
                (self.output)(&qemu.to_string_lossy(), &["-device", "help"]).unwrap_or_default();
            String::from_utf8_lossy(&out).contains("\"apple-gfx-pci\"")
        })
    }

    /// The host checks for a Reims vGPU VM with `ram_mib` of RAM.
    pub fn reims_checks(&self, ram_mib: u32) -> Vec<Check> {
        let mut out = vec![
            self.check_reims_qemu(),
            self.check_vulkan(),
            self.check_map_count(ram_mib),
            self.check_udmabuf(),
        ];
        out.extend(self.check_gnome_capture());
        out
    }

    fn check_reims_qemu(&self) -> Check {
        let name = "Reims vGPU QEMU";
        match (self.reims_qemu)() {
            Err(e) => Check::new(name, Status::Fail, e, "see docs/reims-vgpu.md"),
            Ok(q) if !self.has_apple_gfx(&q) => Check::new(
                name,
                Status::Fail,
                format!("{} does not list the apple-gfx-pci device", q.display()),
                "install or build Reims vGPU again",
            ),
            Ok(q) => Check::new(name, Status::Ok, q.display().to_string(), ""),
        }
    }

    fn check_vulkan(&self) -> Check {
        let name = "Vulkan driver";
        let found = icd_files(&[
            self.share_root.join("vulkan/icd.d"),
            self.etc_root.join("vulkan/icd.d"),
        ]);
        if found.is_empty() {
            return Check::new(
                name,
                Status::Fail,
                "no Vulkan ICD manifest in /usr/share/vulkan/icd.d or /etc/vulkan/icd.d",
                "install your GPU's Vulkan driver (mesa-vulkan-drivers, or the NVIDIA driver)",
            );
        }
        let names: Vec<_> = found
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy())
            .collect();
        Check::new(name, Status::Ok, names.join(", "), "")
    }

    fn check_map_count(&self, ram_mib: u32) -> Check {
        let name = "vm.max_map_count";
        let need = reims_map_count(ram_mib);
        let n = fs::read_to_string(self.proc_root.join("sys/vm/max_map_count"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok());
        match n {
            None => Check::new(
                name,
                Status::Unknown,
                "could not read /proc/sys/vm/max_map_count",
                "",
            ),
            Some(n) if n < need => {
                let to = need.max(SETUP_MAP_COUNT);
                Check::new(
                    name,
                    Status::Fail,
                    format!("{n}, below the {need} a VM with {ram_mib} MiB of RAM needs"),
                    format!(
                        "press Set up host, or: sudo sysctl -w vm.max_map_count={to}, and to keep it: echo vm.max_map_count={to} | sudo tee /etc/sysctl.d/90-macos-kvm-gui.conf"
                    ),
                )
            }
            Some(n) => Check::new(
                name,
                Status::Ok,
                format!("{n} ({need} needed for {ram_mib} MiB)"),
                "",
            ),
        }
    }

    /// Only warns: QEMU may render on another GPU than the AMD one.
    fn check_udmabuf(&self) -> Check {
        let name = "/dev/udmabuf (AMD GPUs)";
        let amd = fs::read_dir(self.sys_root.join("class/drm")).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                e.file_name().to_string_lossy().starts_with("card")
                    && fs::read_to_string(e.path().join("device/vendor"))
                        .is_ok_and(|v| v.trim() == "0x1002")
            })
        });
        if !amd {
            return Check::new(name, Status::Ok, "no AMD GPU", "");
        }
        match open_rw(&self.dev_udmabuf) {
            Ok(()) => Check::new(name, Status::Ok, "read and write allowed", ""),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Check::new(
                name,
                Status::Warn,
                "does not exist: Vulkan on the AMD GPU may fail",
                "sudo modprobe udmabuf",
            ),
            Err(_) => Check::new(
                name,
                Status::Warn,
                "cannot open it read-write: Vulkan on the AMD GPU may fail",
                format!("join the device's group: {}", self.udmabuf_hint()),
            ),
        }
    }

    /// Names /dev/udmabuf's group and the command that joins it.
    pub fn udmabuf_hint(&self) -> String {
        let group = fs::metadata(&self.dev_udmabuf).map_or_else(
            |_| "the group of /dev/udmabuf".to_string(),
            |m| group_name(&self.etc_root.join("group"), m.gid()),
        );
        format!("sudo usermod -aG {group} $USER, then log out and back in")
    }

    /// The checks before a Reims VM starts: the first failure is the error, warnings and
    /// inconclusive checks go to the VM log. Returns the QEMU to run.
    pub fn reims_ready(&self, c: &Config) -> Result<ReimsReady, HostError> {
        let mut warnings = Vec::new();
        for ch in self.reims_checks(c.ram_mib) {
            let mut msg = format!("{}: {}", ch.name, ch.detail);
            if !ch.hint.is_empty() {
                msg += &format!(" ({})", ch.hint);
            }
            match ch.status {
                Status::Fail => return Err(failed(format!("Reims vGPU: {msg}"))),
                Status::Warn | Status::Unknown => warnings.push(msg),
                Status::Ok => {}
            }
        }
        let qemu = (self.reims_qemu)().map_err(failed)?;
        Ok(ReimsReady { qemu, warnings })
    }

    /// Explains a QEMU exit caused by /dev/udmabuf (`None` when `out` is not one).
    pub fn udmabuf_exit(&self, out: &str) -> Option<String> {
        out.contains("udmabuf").then(|| {
            format!(
                "Reims vGPU could not use /dev/udmabuf (needed to render on an AMD GPU): {}, or load it with sudo modprobe udmabuf.",
                self.udmabuf_hint()
            )
        })
    }
}

/// The `*.json` manifests in `dirs`, sorted per folder, without duplicates.
fn icd_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(d) else { continue };
        let mut here: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        here.sort();
        for p in here {
            if !files.contains(&p) {
                files.push(p);
            }
        }
    }
    files
}

/// Looks `gid` up in an /etc/group file, falling back to the number.
fn group_name(file: &Path, gid: u32) -> String {
    let data = fs::read_to_string(file).unwrap_or_default();
    let gid = gid.to_string();
    data.lines()
        .map(|l| l.split(':').collect::<Vec<_>>())
        .find(|f| f.len() >= 3 && f[2] == gid)
        .map_or(gid.clone(), |f| f[0].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TestResult, by_name, fake_host, is_root_for, write};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn map_count_formula() {
        assert_eq!(reims_map_count(16384), 1_114_112);
        assert_eq!(reims_map_count(8192), 589_824);
    }

    #[test]
    fn reims_checks_cover_qemu_vulkan_map_count_and_udmabuf() -> TestResult {
        let (_d, mut h) = fake_host("svm", "Y", &[])?;
        let probes = Arc::new(AtomicUsize::new(0));
        let p = probes.clone();
        h.output = Arc::new(move |_, _| {
            p.fetch_add(1, Ordering::SeqCst);
            Ok(br#"name "apple-gfx-pci", bus PCI, desc "macOS Paravirtualized Graphics""#.to_vec())
        });
        write(
            &h.sys_root.join("class/drm/card1/device/vendor"),
            "0x1002\n",
        )?;
        write(&h.dev_udmabuf, "")?;

        for ch in h.reims_checks(16384) {
            assert_eq!(ch.status, Status::Ok, "{}: {}", ch.name, ch.detail);
        }
        h.reims_checks(16384);
        assert_eq!(
            probes.load(Ordering::SeqCst),
            1,
            "the -device help probe is cached by path and mtime"
        );

        // Low map count: blocking, with the sysctl command and a file to keep it.
        write(&h.proc_root.join("sys/vm/max_map_count"), "65530\n")?;
        let c16 = Config {
            ram_mib: 16384,
            ..Config::default()
        };
        let err = h
            .reims_ready(&c16)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            err.contains("sudo sysctl -w vm.max_map_count=2097152")
                && err.contains("/etc/sysctl.d/"),
            "{err}"
        );
        assert!(err.contains("Set up host"), "{err}");
        write(&h.proc_root.join("sys/vm/max_map_count"), "1114112\n")?;

        // AMD GPU without udmabuf access: a warning naming the group, not an error.
        fs::set_permissions(&h.dev_udmabuf, fs::Permissions::from_mode(0o000))?;
        let gid = fs::metadata(&h.dev_udmabuf)?.gid();
        write(
            &h.etc_root.join("group"),
            &format!("root:x:0:\nkvm:x:{gid}:nilton\n"),
        )?;
        if !is_root_for(&h.dev_udmabuf) {
            let r = h.reims_ready(&c16)?;
            assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
            assert!(
                r.warnings[0].contains("usermod -aG kvm"),
                "{:?}",
                r.warnings
            );
            assert!(!r.qemu.as_os_str().is_empty());
        }

        // No Vulkan driver.
        fs::remove_dir_all(h.share_root.join("vulkan"))?;
        let c8 = Config {
            ram_mib: 8192,
            ..Config::default()
        };
        assert!(
            h.reims_ready(&c8)
                .is_err_and(|e| e.to_string().contains("Vulkan"))
        );

        // In the Host tab they are warnings while the VM is not on Reims vGPU.
        assert_eq!(
            by_name(h.checks("sequoia"))["Vulkan driver"].status,
            Status::Warn
        );
        let reims = Config {
            version: "sequoia".into(),
            ram_mib: 8192,
            graphics: rg_core::config::GRAPHICS_REIMS.into(),
            ..Config::default()
        };
        assert_eq!(
            by_name(h.checks_for(&reims))["Vulkan driver"].status,
            Status::Fail
        );

        // A build without the device names the binary.
        let other = tempfile::tempdir()?;
        let other = other.path().join("qemu-system-x86_64");
        write(&other, "")?;
        let o = other.clone();
        h.reims_qemu = Arc::new(move || Ok(o.clone()));
        h.output = Arc::new(|_, _| Ok(br#"name "vmware-svga", bus PCI"#.to_vec()));
        let c = h.check_reims_qemu();
        assert!(
            c.status == Status::Fail && c.detail.contains(&*other.to_string_lossy()),
            "{c:?}"
        );
        Ok(())
    }

    #[test]
    fn missing_udmabuf_warns_to_load_the_module() -> TestResult {
        let (_d, h) = fake_host("svm", "Y", &[])?;
        write(
            &h.sys_root.join("class/drm/card0/device/vendor"),
            "0x1002\n",
        )?;
        let c = h.check_udmabuf();
        assert_eq!(
            (c.status, c.hint.as_str()),
            (Status::Warn, "sudo modprobe udmabuf")
        );
        Ok(())
    }

    #[test]
    fn udmabuf_exit_is_explained() -> TestResult {
        let (_d, h) = fake_host("svm", "Y", &[])?;
        let m = h.udmabuf_exit("reims-vgpu: fatal: cannot open /dev/udmabuf: Permission denied");
        assert!(m.is_some_and(|m| m.contains("usermod -aG")));
        assert_eq!(h.udmabuf_exit("other"), None);
        Ok(())
    }

    #[test]
    fn group_name_falls_back_to_the_number() -> TestResult {
        let d = tempfile::tempdir()?;
        let f = d.path().join("group");
        write(&f, "root:x:0:\nkvm:x:993:nilton\n")?;
        assert_eq!(group_name(&f, 993), "kvm");
        assert_eq!(group_name(&f, 5), "5");
        Ok(())
    }
}
