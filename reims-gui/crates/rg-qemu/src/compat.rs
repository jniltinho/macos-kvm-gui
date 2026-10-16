//! Workspace paths, version profiles and host lookups this crate needs from rg-core, rg-host and
//! rg-assets, which are written in parallel.
//!
//! NOTE(integration): move each item to its owner (constants and `Config` helpers to rg-core,
//! `udmabuf_hint` to rg-host) and re-export or call it from there; keep the behavior, which is
//! the Go app's (`boot.go`, `workspace.go`, `config.go`, `network.go`, `reimshost.go`).

use rg_core::{Config, Nic};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The extracted OpenCore EFI, exposed to QEMU as a FAT disk (vvfat).
pub const EFI_DIR: &str = "macos-kvm-gui/efi";
pub const OVMF_CODE: &str = "macos-kvm-gui/OVMF_CODE_4M.fd";
/// The VM's NVRAM.
pub const VARS_FILE: &str = "macos-kvm-gui/OVMF_VARS_4M.fd";
/// The recovery image.
pub const MEDIA_FILE: &str = "BaseSystem.img";
/// The VM's disk.
pub const DISK_FILE: &str = "mac_hdd_ng.img";

/// The MAC every VM used before adapters were configurable (from OSX-KVM).
const LEGACY_MAC: &str = "52:54:00:c9:18:27";
pub const NET_NAT: &str = "nat";
pub const NET_BRIDGED: &str = "bridged";

/// `config.go`'s `Versions`, in release order.
const VERSIONS: [&str; 9] = [
    "high-sierra",
    "mojave",
    "catalina",
    "big-sur",
    "monterey",
    "ventura",
    "sonoma",
    "sequoia",
    "tahoe",
];
const IDX_BIG_SUR: isize = 3;
const IDX_MONTEREY: isize = 4;

/// `cpuFlags`: MY_OPTIONS in OpenCore-Boot.sh.
const CPU_FLAGS: &str = "kvm=on,vendor=GenuineIntel,+invtsc,vmware-cpuid-freq=on,+ssse3,+sse4.2,+popcnt,+avx,+aes,+xsave,+xsaveopt,check";

fn version_index(short: &str) -> isize {
    VERSIONS
        .iter()
        .position(|v| *v == short)
        .map_or(-1, |i| i as isize)
}

/// `ProfileFor(version).CPU`.
pub fn profile_cpu(version: &str) -> String {
    if version_index(version) <= IDX_MONTEREY {
        format!("Penryn,{CPU_FLAGS}")
    } else {
        format!("Skylake-Client,-hle,-rtm,{CPU_FLAGS}")
    }
}

/// `Config.Adapters`: the VM's adapters, or for a VM saved before adapters existed, one NAT
/// adapter with the version's model and the legacy MAC.
pub fn adapters(c: &Config) -> Vec<Nic> {
    if let Some(n) = &c.networks {
        return n.clone();
    }
    let model = if version_index(&c.version) >= IDX_BIG_SUR {
        "virtio-net-pci"
    } else {
        "vmxnet3"
    };
    vec![Nic {
        enabled: true,
        mode: NET_NAT.into(),
        bridge: String::new(),
        model: model.into(),
        mac: LEGACY_MAC.into(),
    }]
}

/// `Config.WorkspaceDir`: `~` expanded; must be absolute.
pub fn workspace_dir(c: &Config) -> Option<PathBuf> {
    let p = &c.workspace;
    let p = if p == "~" || p.starts_with("~/") {
        dirs_home()?.join(p.trim_start_matches('~').trim_start_matches('/'))
    } else {
        PathBuf::from(p)
    };
    p.is_absolute().then(|| clean(&p))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Lexical clean like Go's `filepath.Clean` for absolute paths.
fn clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for c in p.components() {
        match c {
            std::path::Component::Normal(s) => out.push(s),
            std::path::Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    out
}

/// `realPath`: symlinks resolved as far as the path exists.
pub fn real_path(p: &Path) -> PathBuf {
    if let Ok(r) = std::fs::canonicalize(p) {
        return r;
    }
    match (p.parent(), p.file_name()) {
        (Some(dir), Some(base)) if !dir.as_os_str().is_empty() && dir != p => {
            real_path(dir).join(base)
        }
        _ => clean(p),
    }
}

/// Where distributions install qemu-bridge-helper (Debian/Ubuntu, Fedora).
const BRIDGE_HELPERS: [&str; 2] = [
    "/usr/lib/qemu/qemu-bridge-helper",
    "/usr/libexec/qemu-bridge-helper",
];

/// `BridgeHelper`: the installed qemu-bridge-helper, or "".
pub fn bridge_helper() -> String {
    BRIDGE_HELPERS
        .iter()
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()))
        .map_or_else(String::new, |p| (*p).to_string())
}

/// `reimsAudio`: the host's sound server, PipeWire, PulseAudio or none.
pub fn reims_audio() -> &'static str {
    let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|r| !r.is_empty()) else {
        return "none";
    };
    let run = PathBuf::from(run);
    if run.join("pipewire-0").exists() {
        "pipewire"
    } else if run.join("pulse/native").exists() {
        "pa"
    } else {
        "none"
    }
}

/// `udmabufHint`: names /dev/udmabuf's group and the command that joins it.
pub fn udmabuf_hint() -> String {
    let group = std::fs::metadata("/dev/udmabuf").map_or_else(
        |_| "the group of /dev/udmabuf".to_string(),
        |m| group_name(Path::new("/etc/group"), m.gid()),
    );
    format!("sudo usermod -aG {group} $USER, then log out and back in")
}

fn group_name(file: &Path, gid: u32) -> String {
    let data = std::fs::read_to_string(file).unwrap_or_default();
    let gid_s = gid.to_string();
    data.lines()
        .map(|l| l.split(':').collect::<Vec<_>>())
        .find(|f| f.len() >= 3 && f[2] == gid_s)
        .map_or(gid_s.clone(), |f| f[0].to_string())
}

/// The base of the private run directories: `$XDG_RUNTIME_DIR`, or the temporary directory.
pub fn runtime_base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|r| !r.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_split_at_monterey() {
        assert!(profile_cpu("monterey").starts_with("Penryn,"));
        assert!(profile_cpu("ventura").starts_with("Skylake-Client,"));
    }

    #[test]
    fn legacy_vm_gets_one_nat_adapter_with_the_legacy_mac() {
        let mut c = Config::defaults();
        c.version = "catalina".into();
        let a = adapters(&c);
        assert_eq!(
            (a.len(), a[0].model.as_str(), a[0].mac.as_str()),
            (1, "vmxnet3", LEGACY_MAC)
        );
    }

    #[test]
    fn workspace_dir_expands_home_and_refuses_relative() {
        let mut c = Config::defaults();
        c.workspace = "/vm/../vm/a/".into();
        assert_eq!(workspace_dir(&c), Some(PathBuf::from("/vm/a")));
        c.workspace = "rel".into();
        assert_eq!(workspace_dir(&c), None);
    }

    #[test]
    fn group_name_falls_back_to_the_number() -> std::io::Result<()> {
        let d = tempfile::tempdir()?;
        let f = d.path().join("group");
        std::fs::write(&f, "root:x:0:\nrender:x:992:me\n")?;
        assert_eq!(
            (group_name(&f, 992), group_name(&f, 5)),
            ("render".into(), "5".into())
        );
        Ok(())
    }
}
