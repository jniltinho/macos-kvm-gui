//! GPU listing from sysfs, macOS compatibility, passthrough eligibility and the vfio-pci
//! reservation (port of the Go app's `internal/core/gpu.go`, and `keplerIDs` from rootpatch.go).

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::PathBuf;

use rg_core::Config;

use crate::compat::{root_patch_version, version_display, version_index};
use crate::host::{Host, HostError, failed, open_rw};

/// Whether macOS drives a GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compat {
    Unknown,
    Supported,
    Unsupported,
    /// NVIDIA Kepler: native up to Big Sur, root-patch mode on Monterey to Sequoia.
    Kepler,
}

impl fmt::Display for Compat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Compat::Unknown => "unknown",
            Compat::Supported => "supported by macOS",
            Compat::Unsupported => "not supported by macOS",
            Compat::Kepler => "NVIDIA Kepler: native up to Big Sur; Monterey to Sequoia with root-patch mode (OCLP)",
        })
    }
}

/// AMD (vendor 1002) device IDs, following the Dortania GPU buyers guide. Anything else from
/// AMD, including every APU iGPU, is unknown and therefore not eligible.
fn amd_compat(device: &str) -> Compat {
    match device {
        // Polaris; Vega, Radeon VII; Navi 10/14; Navi 21 (RX 6800/6800 XT/6900 XT), Navi 23 (RX 6600/6600 XT)
        "67df" | "67ef" | "67ff" | "687f" | "66af" | "731f" | "7340" | "73bf" | "73ff" => {
            Compat::Supported
        }
        // Navi 24, RDNA3: no macOS driver
        "743f" | "744c" | "7448" | "747e" | "7480" => Compat::Unsupported,
        _ => Compat::Unknown,
    }
}

/// AMD GPUs that only work with a device-id spoof or the NootRX kext, which this app does not
/// set up: Lexa, RX 580 2048SP, Navi 21 XTXH/RX 6950 XT, Navi 22 (RX 6700/6750), Navi 23 RX 6650 XT.
fn amd_needs_patch(device: &str) -> bool {
    matches!(device, "699f" | "6fdf" | "73af" | "73a5" | "73df" | "73ef")
}

/// OCLP 2.5.1's kepler_ids (datasets/pci_data.py), NVIDIA vendor 10de: an explicit list, never
/// ranges or marketing names (a GT 730 can be Fermi, a GTX 750 is Maxwell).
const KEPLER_IDS: [&str; 159] = [
    "0fc0", "0fc1", "0fc2", "0fc5", "0fc6", "0fc8", "0fc9", "0fcd", "0fce", "0fd1", "0fd2", "0fd3",
    "0fd4", "0fd5", "0fd6", "0fd8", "0fd9", "0fdb", "0fdf", "0fe0", "0fe1", "0fe2", "0fe3", "0fe4",
    "0fe5", "0fe6", "0fe7", "0fe8", "0fe9", "0fea", "0fec", "0fed", "0fee", "0fef", "0ff1", "0ff2",
    "0ff3", "0ff5", "0ff6", "0ff7", "0ff8", "0ff9", "0ffa", "0ffb", "0ffc", "0ffd", "0ffe", "0fff",
    "1001", "1003", "1004", "1005", "1007", "1008", "100a", "100c", "101e", "101f", "1020", "1021",
    "1022", "1023", "1024", "1026", "1027", "1028", "1029", "102a", "102d", "102e", "102f", "103a",
    "103c", "103f", "1180", "1182", "1183", "1184", "1185", "1186", "1187", "1188", "1189", "118a",
    "118b", "118c", "118d", "118e", "118f", "1191", "1193", "1194", "1195", "1198", "1199", "119a",
    "119d", "119e", "119f", "11a0", "11a1", "11a2", "11a3", "11a7", "11a8", "11a9", "11af", "11b0",
    "11b1", "11b4", "11b6", "11b7", "11b8", "11b9", "11ba", "11bc", "11bd", "11be", "11bf", "11c0",
    "11c2", "11c3", "11c4", "11c5", "11c6", "11c7", "11c8", "11cb", "11e0", "11e1", "11e2", "11e3",
    "11e7", "11fa", "11fc", "1280", "1281", "1282", "1284", "1286", "1287", "1288", "1289", "128a",
    "128b", "128c", "1290", "1291", "1292", "1293", "1294", "1295", "1296", "1298", "1299", "129a",
    "12a0", "12b9", "12ba",
];

/// Whether a PCI ID (vendor:device) is an NVIDIA Kepler GPU.
pub fn is_kepler(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(v, d)| v == "10de" && KEPLER_IDS.contains(&d))
}

fn compat_of(vendor: &str, device: &str) -> Compat {
    match vendor {
        "1002" if amd_needs_patch(device) => Compat::Unsupported,
        "1002" => amd_compat(device),
        "10de" if is_kepler(&format!("{vendor}:{device}")) => Compat::Kepler,
        // Maxwell and newer: no Metal driver since High Sierra's Web Drivers.
        "10de" | "8086" => Compat::Unsupported,
        _ => Compat::Unknown,
    }
}

/// One function of the GPU's slot (the GPU itself, its HDMI audio, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PciFunc {
    /// 0000:03:00.1
    pub addr: String,
    /// 1002:ab28
    pub id: String,
    pub driver: String,
}

/// A display adapter (PCI class 0x03xxxx).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gpu {
    /// Address of the display function, 0000:03:00.0.
    pub addr: String,
    /// vendor:device
    pub id: String,
    pub name: String,
    pub driver: String,
    pub group: String,
    pub boot_vga: bool,
    /// A DRM connector of this GPU has a monitor attached.
    pub monitor: bool,
    pub compat: Compat,
    /// Every function of the slot, display function first.
    pub funcs: Vec<PciFunc>,
    pub eligible: bool,
    /// Why it is not eligible.
    pub reason: String,
}

/// Go `pciIDRe`: `^[0-9a-f]{4}:[0-9a-f]{4}$`.
fn valid_pci_id(id: &str) -> bool {
    let hex4 = |s: &str| {
        s.len() == 4
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    };
    id.split_once(':').is_some_and(|(v, d)| hex4(v) && hex4(d))
}

fn slot_of(addr: &str) -> &str {
    addr.rfind('.').map_or(addr, |i| &addr[..i])
}

/// Files written by the reservation, under /etc.
pub(crate) const VFIO_MODPROBE: &str = "modprobe.d/macos-kvm-gui-vfio.conf";

/// Constant: the IDs arrive as $1 and are checked again here. It holds a host-wide lock and
/// rolls the files back if the initramfs cannot be rebuilt.
pub(crate) const RESERVE_SCRIPT: &str = r#"case "$1" in ""|*[!0-9a-f:,]*) echo "invalid PCI IDs" >&2; exit 2;; esac
exec 9>/run/lock/macos-kvm-gui-host.lock && flock -n 9 || { echo "another GPU change is running" >&2; exit 3; }
printf 'options vfio-pci ids=%s disable_vga=1\nsoftdep amdgpu pre: vfio-pci\nsoftdep radeon pre: vfio-pci\nsoftdep nouveau pre: vfio-pci\nsoftdep nvidia pre: vfio-pci\nsoftdep snd_hda_intel pre: vfio-pci\n' "$1" > /etc/modprobe.d/macos-kvm-gui-vfio.conf || exit 1
printf 'SUBSYSTEM=="vfio", GROUP="kvm", MODE="0660"\n' > /etc/udev/rules.d/99-macos-kvm-gui-vfio.rules || exit 1
if ! update-initramfs -u -k all; then
  rm -f /etc/modprobe.d/macos-kvm-gui-vfio.conf /etc/udev/rules.d/99-macos-kvm-gui-vfio.rules
  update-initramfs -u -k all
  echo "initramfs rebuild failed; the reservation was rolled back" >&2
  exit 1
fi"#;

pub(crate) const RELEASE_SCRIPT: &str = r#"exec 9>/run/lock/macos-kvm-gui-host.lock && flock -n 9 || { echo "another GPU change is running" >&2; exit 3; }
rm -f /etc/modprobe.d/macos-kvm-gui-vfio.conf /etc/udev/rules.d/99-macos-kvm-gui-vfio.rules || exit 1
update-initramfs -u -k all || { echo "files removed, but the initramfs rebuild failed: run sudo update-initramfs -u -k all" >&2; exit 1; }"#;

/// The oldest macOS with a native driver for an AMD GPU (Dortania).
fn gpu_min_version(device: &str) -> Option<&'static str> {
    match device {
        "731f" | "7340" => Some("catalina"), // Navi 10/14: 10.15.1+
        "73bf" => Some("big-sur"),           // Navi 21: 11.4+
        "73ff" => Some("monterey"),          // Navi 23: 12.1+
        "66af" => Some("mojave"),            // Radeon VII: 10.14.5+
        _ => None,
    }
}

impl Host {
    fn pci_dir(&self) -> PathBuf {
        self.sys_root.join("bus/pci/devices")
    }

    fn pci_attr(&self, addr: &str, name: &str) -> String {
        let s = fs::read_to_string(self.pci_dir().join(addr).join(name)).unwrap_or_default();
        let s = s.trim();
        s.strip_prefix("0x").unwrap_or(s).to_string()
    }

    fn pci_link(&self, addr: &str, name: &str) -> String {
        fs::read_link(self.pci_dir().join(addr).join(name))
            .ok()
            .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    fn pci_func(&self, addr: &str) -> PciFunc {
        PciFunc {
            addr: addr.into(),
            id: format!(
                "{}:{}",
                self.pci_attr(addr, "vendor"),
                self.pci_attr(addr, "device")
            ),
            driver: self.pci_link(addr, "driver"),
        }
    }

    /// The host's display adapters with their eligibility (settings.yaml's kexts read through
    /// [`Host::extra_kexts`]).
    pub fn gpus(&self) -> Vec<Gpu> {
        self.list_gpus(None)
    }

    /// `kexts`: a start's snapshot of how many extra kexts settings.yaml lists, else `None`.
    fn list_gpus(&self, kexts: Option<usize>) -> Vec<Gpu> {
        let mut all: Vec<String> = fs::read_dir(self.pci_dir())
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        all.sort();
        let iommu = fs::read_dir(self.sys_root.join("kernel/iommu_groups"))
            .is_ok_and(|mut rd| rd.next().is_some());
        let connected = self.connected_gpus();

        let mut gpus: Vec<Gpu> = all
            .iter()
            .filter(|a| self.pci_attr(a, "class").starts_with("03"))
            .map(|addr| {
                let f = self.pci_func(addr);
                let (vendor, device) = f.id.split_once(':').unwrap_or((&f.id, ""));
                let mut funcs = vec![f.clone()];
                funcs.extend(
                    all.iter()
                        .filter(|o| *o != addr && slot_of(o) == slot_of(addr))
                        .map(|o| self.pci_func(o)),
                );
                Gpu {
                    addr: addr.clone(),
                    compat: compat_of(vendor, device),
                    name: self.pci_name(addr, &f.id),
                    group: self.pci_link(addr, "iommu_group"),
                    boot_vga: self.pci_attr(addr, "boot_vga") == "1",
                    monitor: connected.contains(addr.as_str()),
                    id: f.id,
                    driver: f.driver,
                    funcs,
                    eligible: false,
                    reason: String::new(),
                }
            })
            .collect();
        let verdicts: Vec<_> = gpus
            .iter()
            .map(|g| self.eligibility(g, &gpus, &all, iommu, kexts))
            .collect();
        for (g, v) in gpus.iter_mut().zip(verdicts) {
            match v {
                Ok(()) => g.eligible = true,
                Err(r) => g.reason = r,
            }
        }
        gpus
    }

    /// PCI addresses with a DRM connector that has a monitor attached.
    fn connected_gpus(&self) -> HashSet<String> {
        let drm = self.sys_root.join("class/drm");
        let Ok(rd) = fs::read_dir(&drm) else {
            return HashSet::new();
        };
        rd.flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let (card, _) = name
                    .split_once('-')
                    .filter(|(c, _)| c.starts_with("card"))?;
                let st = fs::read_to_string(e.path().join("status")).ok()?;
                if st.trim() != "connected" {
                    return None;
                }
                let dev = fs::canonicalize(drm.join(card).join("device")).ok()?;
                Some(dev.file_name()?.to_string_lossy().into_owned())
            })
            .collect()
    }

    /// The group's devices, sorted like Go's `os.ReadDir`.
    fn group_members(&self, group: &str) -> Vec<String> {
        let mut m: Vec<String> = fs::read_dir(
            self.sys_root
                .join("kernel/iommu_groups")
                .join(group)
                .join("devices"),
        )
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
        m.sort();
        m
    }

    fn pci_name(&self, addr: &str, id: &str) -> String {
        (self.output)("lspci", &["-s", addr])
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o)
                    .trim()
                    .split_once(": ")
                    .map(|(_, n)| n.to_string())
            })
            .unwrap_or_else(|| format!("PCI device {id}"))
    }

    /// Why a patch-dependent GPU cannot be used ("" = it can).
    fn patch_kext_reason(&self, kexts: Result<usize, String>) -> String {
        match kexts {
            Err(e) => e,
            Ok(0) => format!(
                "macOS drives this GPU only with a patch kext: list one in {} (see docs/gpu-passthrough.md)",
                self.settings_file
            ),
            Ok(_) => String::new(),
        }
    }

    fn eligibility(
        &self,
        g: &Gpu,
        gpus: &[Gpu],
        all: &[String],
        iommu: bool,
        kexts: Option<usize>,
    ) -> Result<(), String> {
        let patch = g.id.strip_prefix("1002:").is_some_and(amd_needs_patch);
        let patch_reason = match (patch, kexts) {
            (false, _) => String::new(),
            (true, Some(n)) => self.patch_kext_reason(Ok(n)),
            (true, None) => self.patch_kext_reason((self.extra_kexts)()),
        };
        if !iommu {
            return Err("IOMMU is disabled: enable AMD-Vi (or VT-d) in the BIOS".into());
        }
        if !patch_reason.is_empty() {
            return Err(patch_reason);
        }
        if g.compat == Compat::Unsupported && !patch {
            return Err("macOS has no driver for this GPU".into());
        }
        if g.compat == Compat::Unknown {
            return Err("not in the list of macOS-supported GPUs; AMD Ryzen iGPUs have no macOS driver (see the Dortania GPU buyers guide)".into());
        }
        if g.boot_vga {
            return Err("this GPU shows the host's boot screen: make the integrated GPU the primary display in the BIOS".into());
        }
        if g.monitor {
            return Err("a monitor is connected to this GPU and the host is using it: move the host's monitor to another GPU first".into());
        }
        let host_display = gpus.iter().any(|o| {
            slot_of(&o.addr) != slot_of(&g.addr) && !o.driver.is_empty() && o.driver != "vfio-pci"
        });
        if !host_display {
            return Err("no other GPU with a driver is left for the host's display".into());
        }
        let mut ids = HashSet::new();
        for f in &g.funcs {
            if !valid_pci_id(&f.id) {
                return Err(format!("unexpected PCI ID {}", f.id));
            }
            let class = self.pci_attr(&f.addr, "class");
            if !class.starts_with("03") && !class.starts_with("0403") {
                return Err(format!(
                    "{} (class {class}, for example a USB-C controller) is not a GPU or audio function; passing it through is not supported",
                    f.addr
                ));
            }
            ids.insert(f.id.as_str());
            for m in self.group_members(&self.pci_link(&f.addr, "iommu_group")) {
                if slot_of(&m) != slot_of(&g.addr)
                    && !self.pci_attr(&m, "class").starts_with("0604")
                {
                    return Err(format!(
                        "the IOMMU group of {} also contains {m}, from another slot: move the card to another slot",
                        f.addr
                    ));
                }
            }
        }
        if let Some(other) = all
            .iter()
            .find(|o| slot_of(o) != slot_of(&g.addr) && ids.contains(self.pci_func(o).id.as_str()))
        {
            return Err(format!(
                "{other} has the same PCI ID, and vfio-pci would claim it too"
            ));
        }
        Ok(())
    }

    /// The IDs currently reserved for vfio-pci by this app ("" = none).
    pub fn reserved_ids(&self) -> String {
        let conf = fs::read_to_string(self.etc_root.join(VFIO_MODPROBE)).unwrap_or_default();
        conf.find("ids=")
            .map(|i| {
                conf[i + 4..]
                    .chars()
                    .take_while(|c| {
                        c.is_ascii_digit() || ('a'..='f').contains(c) || matches!(c, ':' | ',')
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Makes vfio-pci claim the GPU's slot at the next boot.
    pub fn reserve_gpu(&self, g: &Gpu) -> Result<(), HostError> {
        if !g.eligible {
            return Err(failed(format!(
                "this GPU cannot be passed through: {}",
                g.reason
            )));
        }
        let ids = vfio_ids(g)?;
        match self.run_privileged(RESERVE_SCRIPT, &[&ids]) {
            Err(HostError::Failed(e)) => return Err(failed(format!("reservation failed: {e}"))),
            Err(e) => return Err(e),
            Ok(()) => {}
        }
        if self.reserved_ids() != ids {
            return Err(failed(format!(
                "reservation did not take effect: /etc/{VFIO_MODPROBE} was not written"
            )));
        }
        Ok(())
    }

    /// Removes the reservation; the GPU gets its normal driver back after a reboot.
    pub fn release_gpu(&self) -> Result<(), HostError> {
        match self.run_privileged(RELEASE_SCRIPT, &[]) {
            Err(HostError::Failed(e)) => return Err(failed(format!("release failed: {e}"))),
            Err(e) => return Err(e),
            Ok(()) => {}
        }
        if !self.reserved_ids().is_empty() {
            return Err(failed(format!(
                "release did not take effect: /etc/{VFIO_MODPROBE} still exists"
            )));
        }
        Ok(())
    }

    /// Checks, right before QEMU starts, that `c.gpu` can really be used. `extra_kexts` is how
    /// many kexts the start's settings.yaml snapshot lists; `memlock` the soft RLIMIT_MEMLOCK in
    /// bytes. Returns the slot's functions, display function first.
    pub fn gpu_ready(
        &self,
        c: &Config,
        extra_kexts: usize,
        memlock: u64,
    ) -> Result<Vec<String>, HostError> {
        let gpus = self.list_gpus(Some(extra_kexts));
        let Some(g) = gpus.iter().find(|g| g.addr == c.gpu) else {
            return Err(failed(format!("GPU {} not found on this host", c.gpu)));
        };
        if !g.eligible {
            return Err(failed(format!(
                "GPU {} cannot be passed through: {}",
                c.gpu, g.reason
            )));
        }
        if let Some(min) = g.id.strip_prefix("1002:").and_then(gpu_min_version)
            && version_index(&c.version) < version_index(min)
        {
            return Err(failed(format!("this GPU needs macOS {min} or newer")));
        }
        kepler_ready(g, c)?;
        let mut funcs = Vec::new();
        for f in &g.funcs {
            if f.driver != "vfio-pci" {
                return Err(failed(format!(
                    "{} is bound to {:?}, not vfio-pci: reserve the GPU in the GPU tab and reboot the host",
                    f.addr, f.driver
                )));
            }
            let node = self.dev_vfio.join(self.pci_link(&f.addr, "iommu_group"));
            if let Err(e) = open_rw(&node) {
                return Err(failed(format!(
                    "cannot open {} ({e}): check the udev rule from the reservation and that you are in the kvm group (log in again after joining it)",
                    node.display()
                )));
            }
            funcs.push(f.addr.clone());
        }
        let need = u64::from(c.ram_mib) << 20;
        if memlock < need {
            return Err(failed(format!(
                "the soft locked-memory limit is {} MiB but the VM needs {} MiB (VFIO pins all guest RAM): raise memlock in /etc/security/limits.d and log in again",
                memlock >> 20,
                c.ram_mib
            )));
        }
        Ok(funcs)
    }
}

/// The slot's unique PCI IDs, comma-separated, each checked.
fn vfio_ids(g: &Gpu) -> Result<String, HostError> {
    let mut ids: Vec<&str> = Vec::new();
    for f in &g.funcs {
        if !valid_pci_id(&f.id) {
            return Err(failed(format!("refusing unexpected PCI ID {:?}", f.id)));
        }
        if !ids.contains(&f.id.as_str()) {
            ids.push(&f.id);
        }
    }
    Ok(ids.join(","))
}

/// A Kepler GPU against the VM's macOS: native up to Big Sur, root-patch mode from Monterey to
/// Sequoia, nothing on Tahoe.
fn kepler_ready(g: &Gpu, c: &Config) -> Result<(), HostError> {
    if g.compat != Compat::Kepler || version_index(&c.version) <= version_index("big-sur") {
        return Ok(());
    }
    let v = version_display(&c.version);
    if !root_patch_version(&c.version) {
        return Err(failed(format!(
            "NVIDIA Kepler GPUs have no driver on {v}, and OCLP does not patch it yet: use Big Sur to Sequoia"
        )));
    }
    if !c.root_patch {
        return Err(failed(format!(
            "on {v} a Kepler GPU needs root-patch mode (GPU tab) and OCLP's root patch inside macOS"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::CmdError;
    use crate::testutil::{TestResult, cmd_err, fake_host, is_root_for, write};
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[derive(Clone)]
    struct Dev {
        addr: &'static str,
        class: &'static str,
        vendor: &'static str,
        device: &'static str,
        driver: &'static str,
        group: &'static str,
        boot_vga: &'static str,
        monitor: bool,
    }

    #[expect(clippy::too_many_arguments, reason = "one row of the Go test table")]
    const fn dev(
        addr: &'static str,
        class: &'static str,
        vendor: &'static str,
        device: &'static str,
        driver: &'static str,
        group: &'static str,
        boot_vga: &'static str,
        monitor: bool,
    ) -> Dev {
        Dev {
            addr,
            class,
            vendor,
            device,
            driver,
            group,
            boot_vga,
            monitor,
        }
    }

    /// The real host, plus a hypothetical RX 6600 in slot 03:00.
    fn reference_host() -> Vec<Dev> {
        vec![
            dev(
                "0000:01:00.0",
                "030000",
                "10de",
                "2786",
                "nvidia",
                "13",
                "1",
                true,
            ),
            dev(
                "0000:01:00.1",
                "040300",
                "10de",
                "22bc",
                "snd_hda_intel",
                "13",
                "",
                false,
            ),
            dev(
                "0000:03:00.0",
                "030000",
                "1002",
                "73ff",
                "amdgpu",
                "14",
                "0",
                false,
            ),
            dev(
                "0000:03:00.1",
                "040300",
                "1002",
                "ab28",
                "snd_hda_intel",
                "14",
                "",
                false,
            ),
            dev(
                "0000:04:04.0",
                "060400",
                "1022",
                "43f5",
                "pcieport",
                "17",
                "",
                false,
            ),
            dev(
                "0000:08:00.0",
                "028000",
                "8086",
                "2725",
                "iwlwifi",
                "17",
                "",
                false,
            ),
            dev(
                "0000:0a:00.0",
                "030200",
                "10de",
                "1db6",
                "nvidia",
                "17",
                "",
                false,
            ),
            dev(
                "0000:75:00.0",
                "030000",
                "1002",
                "13c0",
                "amdgpu",
                "29",
                "0",
                false,
            ),
        ]
    }

    /// Builds /sys/bus/pci/devices and /sys/kernel/iommu_groups for `devs`.
    fn fake_sysfs(devs: &[Dev]) -> std::io::Result<(TempDir, Host)> {
        let (d, h) = fake_host("svm", "Y", &[])?;
        for x in devs {
            let dir = h.pci_dir().join(x.addr);
            write(&dir.join("class"), &format!("0x{}", x.class))?;
            write(&dir.join("vendor"), &format!("0x{}", x.vendor))?;
            write(&dir.join("device"), &format!("0x{}", x.device))?;
            if !x.boot_vga.is_empty() {
                write(&dir.join("boot_vga"), x.boot_vga)?;
            }
            if !x.driver.is_empty() {
                symlink(
                    format!("../../../bus/pci/drivers/{}", x.driver),
                    dir.join("driver"),
                )?;
            }
            if x.monitor {
                let card = h.sys_root.join(format!("class/drm/card{}", x.group));
                fs::create_dir_all(&card)?;
                symlink(&dir, card.join("device"))?;
                write(
                    &h.sys_root
                        .join(format!("class/drm/card{}-DP-1/status", x.group)),
                    "connected",
                )?;
            }
            if x.driver == "vfio-pci" {
                write(&h.dev_vfio.join(x.group), "")?;
            }
            let gdir = h.sys_root.join("kernel/iommu_groups").join(x.group);
            fs::create_dir_all(gdir.join("devices"))?;
            symlink(&gdir, dir.join("iommu_group"))?;
            symlink(&dir, gdir.join("devices").join(x.addr))?;
        }
        Ok((d, h))
    }

    fn gpu(h: &Host, addr: &str) -> Result<Gpu, String> {
        h.gpus()
            .into_iter()
            .find(|g| g.addr == addr)
            .ok_or_else(|| format!("no GPU {addr}"))
    }

    fn refused(devs: &[Dev], want: &str) -> TestResult {
        let (_d, h) = fake_sysfs(devs)?;
        let g = gpu(&h, "0000:03:00.0")?;
        assert!(
            !g.eligible && g.reason.contains(want),
            "want {want:?}, got {:?}",
            g.reason
        );
        Ok(())
    }

    #[test]
    fn gpus_are_listed_with_eligibility() -> TestResult {
        let (_d, h) = fake_sysfs(&reference_host())?;
        let gs = h.gpus();
        assert_eq!(gs.len(), 4);
        for (addr, compat, eligible, reason) in [
            ("0000:01:00.0", Compat::Unsupported, false, "no driver"), // RTX 4070
            ("0000:0a:00.0", Compat::Unsupported, false, "no driver"), // Tesla V100
            ("0000:75:00.0", Compat::Unknown, false, "Dortania"), // Ryzen iGPU must never be reservable
            ("0000:03:00.0", Compat::Supported, true, ""),        // RX 6600
        ] {
            let g = gpu(&h, addr)?;
            assert!(
                g.compat == compat && g.eligible == eligible && g.reason.contains(reason),
                "{addr}: {:?} {} {:?}",
                g.compat,
                g.eligible,
                g.reason
            );
        }
        let rx = gpu(&h, "0000:03:00.0")?;
        assert_eq!(rx.funcs.len(), 2);
        assert_eq!(rx.funcs[0].addr, "0000:03:00.0");
        assert_eq!(rx.funcs[1].id, "1002:ab28");
        assert!(gpu(&h, "0000:01:00.0")?.monitor);
        Ok(())
    }

    #[test]
    fn compat_table_follows_dortania() {
        for (id, want) in [
            ("1002:73ff", Compat::Supported),   // RX 6600
            ("1002:67df", Compat::Supported),   // RX 580
            ("1002:73df", Compat::Unsupported), // RX 6700 XT: needs a patch kext
            ("1002:73ef", Compat::Unsupported), // RX 6650 XT: needs spoof
            ("1002:744c", Compat::Unsupported), // RX 7900
            ("1002:13c0", Compat::Unknown),     // Ryzen 9000 iGPU
            ("10de:2786", Compat::Unsupported), // RTX 4070
            ("10de:128b", Compat::Kepler),      // GT 710
        ] {
            let (v, d) = id.split_once(':').unwrap_or_default();
            assert_eq!(compat_of(v, d), want, "{id}");
        }
    }

    #[test]
    fn boot_display_is_refused() -> TestResult {
        let mut d = reference_host();
        d[2].boot_vga = "1";
        refused(&d, "primary display")
    }

    #[test]
    fn shared_group_names_the_foreign_device() -> TestResult {
        let mut d = reference_host();
        (d[2].group, d[3].group) = ("17", "17"); // same group as the Wi-Fi card
        refused(&d, "0000:08:00.0")
    }

    #[test]
    fn gpu_driving_a_monitor_is_refused() -> TestResult {
        let mut d = reference_host();
        d[2].monitor = true;
        refused(&d, "monitor")
    }

    #[test]
    fn usb_function_in_the_slot_is_refused() -> TestResult {
        let mut d = reference_host();
        d.push(dev(
            "0000:03:00.2",
            "0c0330",
            "1002",
            "73a6",
            "xhci_hcd",
            "14",
            "",
            false,
        ));
        refused(&d, "0000:03:00.2")
    }

    #[test]
    fn every_functions_group_is_checked() -> TestResult {
        let mut d = reference_host();
        d[3].group = "17"; // audio function shares the Wi-Fi card's group
        refused(&d, "0000:03:00.1")
    }

    #[test]
    fn identical_second_card_is_refused() -> TestResult {
        let mut d = reference_host();
        d.push(dev(
            "0000:05:00.0",
            "030000",
            "1002",
            "73ff",
            "amdgpu",
            "15",
            "0",
            false,
        ));
        refused(&d, "same PCI ID")
    }

    #[test]
    fn hosts_only_gpu_is_refused() -> TestResult {
        let d = reference_host();
        refused(&[d[2].clone(), d[3].clone()], "host's display")
    }

    #[test]
    fn iommu_off_is_refused() -> TestResult {
        let (_d, h) = fake_sysfs(&reference_host())?;
        fs::remove_dir_all(h.sys_root.join("kernel/iommu_groups"))?;
        let g = gpu(&h, "0000:03:00.0")?;
        assert!(!g.eligible && g.reason.contains("IOMMU"), "{:?}", g.reason);
        Ok(())
    }

    #[test]
    fn reserve_and_release() -> TestResult {
        let (_d, mut h) = fake_sysfs(&reference_host())?;
        let rx = gpu(&h, "0000:03:00.0")?;
        let conf = h.etc_root.join(VFIO_MODPROBE);

        let c = conf.clone();
        h.run = Arc::new(move |name, a| {
            assert!(
                name == "pkexec"
                    && a[2] == RESERVE_SCRIPT
                    && a[3] == "sh"
                    && a[4] == "1002:73ff,1002:ab28",
                "{name} {a:?}"
            );
            write(
                &c,
                &format!("options vfio-pci ids={} disable_vga=1\n", a[4]),
            )
            .map_err(|e| cmd_err(1, &e.to_string()))
        });
        h.reserve_gpu(&rx)?;
        assert_eq!(h.reserved_ids(), "1002:73ff,1002:ab28");
        assert!(
            !RESERVE_SCRIPT.contains("blacklist"),
            "never blacklist a driver (the host iGPU uses amdgpu too)"
        );

        h.run = Arc::new(|_, _| Err(cmd_err(126, "")));
        assert_eq!(h.release_gpu(), Err(HostError::Cancelled));
        let c = conf.clone();
        h.run = Arc::new(move |_, _| fs::remove_file(&c).map_err(|e| cmd_err(1, &e.to_string())));
        h.release_gpu()?;
        assert_eq!(h.reserved_ids(), "");

        let mut bad = rx.clone();
        bad.funcs = vec![PciFunc {
            addr: "0000:03:00.0".into(),
            id: "1002:73ff;reboot".into(),
            driver: "amdgpu".into(),
        }];
        h.run =
            Arc::new(|_: &str, _: &[&str]| -> Result<(), CmdError> { panic!("must not elevate") });
        assert!(h.reserve_gpu(&bad).is_err());
        assert!(
            h.reserve_gpu(&gpu(&h, "0000:75:00.0")?).is_err(),
            "the iGPU must never be reservable"
        );
        Ok(())
    }

    #[test]
    fn gpu_ready_checks_driver_memlock_version_and_vfio_node() -> TestResult {
        let c = Config {
            gpu: "0000:03:00.0".into(),
            ram_mib: 16384,
            ..Config::defaults()
        };
        let mut devs = reference_host();
        let (_d, h) = fake_sysfs(&devs)?;
        assert!(
            h.gpu_ready(&c, 0, 1 << 40)
                .is_err_and(|e| e.to_string().contains("reserve the GPU"))
        );

        (devs[2].driver, devs[3].driver) = ("vfio-pci", "vfio-pci");
        let (_d, h) = fake_sysfs(&devs)?;
        assert!(
            h.gpu_ready(&c, 0, 8 << 30)
                .is_err_and(|e| e.to_string().contains("16384 MiB"))
        );
        let old = Config {
            version: "big-sur".into(),
            ..c.clone()
        };
        assert!(
            h.gpu_ready(&old, 0, u64::MAX)
                .is_err_and(|e| e.to_string().contains("monterey"))
        );
        assert_eq!(
            h.gpu_ready(&c, 0, u64::MAX)?,
            ["0000:03:00.0", "0000:03:00.1"]
        );

        let node = h.dev_vfio.join("14");
        fs::set_permissions(&node, fs::Permissions::from_mode(0o000))?;
        if !is_root_for(&node) {
            assert!(
                h.gpu_ready(&c, 0, u64::MAX)
                    .is_err_and(|e| e.to_string().contains("kvm group"))
            );
        }
        Ok(())
    }

    /// The real shell must reject hostile IDs before touching anything.
    #[test]
    fn reserve_script_rejects_injection() -> TestResult {
        for bad in ["", "1002:73ff;reboot", "$(id)", "1002:73ff ab28", "../x"] {
            let st = std::process::Command::new("/bin/sh")
                .args(["-c", RESERVE_SCRIPT, "sh", bad])
                .output()?
                .status;
            assert_eq!(st.code(), Some(2), "{bad:?}");
        }
        Ok(())
    }

    /// A GT 710 (GK208, 10de:128b) in slot 05:00, bound to vfio-pci, next to the host's RX 6600.
    #[test]
    fn kepler_gpu_follows_the_macos_version() -> TestResult {
        let mut devs = reference_host();
        devs.push(dev(
            "0000:05:00.0",
            "030000",
            "10de",
            "128b",
            "vfio-pci",
            "21",
            "0",
            false,
        ));
        devs.push(dev(
            "0000:05:00.1",
            "040300",
            "10de",
            "0e0f",
            "vfio-pci",
            "21",
            "",
            false,
        ));
        let (_d, h) = fake_sysfs(&devs)?;
        let g = gpu(&h, "0000:05:00.0")?;
        assert!(g.compat == Compat::Kepler && g.eligible, "{g:?}");
        assert_eq!(
            compat_of("10de", "1380"),
            Compat::Unsupported,
            "Maxwell GTX 750 Ti is not Kepler"
        );
        assert_eq!(
            compat_of("10de", "0f02"),
            Compat::Unsupported,
            "Fermi GT 730 is not Kepler"
        );

        let mut c = Config {
            gpu: "0000:05:00.0".into(),
            ram_mib: 8192,
            ..Config::defaults()
        };
        for (v, want) in [
            ("big-sur", ""),
            ("catalina", ""),
            ("sequoia", "root-patch mode"),
            ("tahoe", "no driver"),
        ] {
            c.version = v.into();
            let r = h.gpu_ready(&c, 0, u64::MAX);
            match want {
                "" => assert!(r.is_ok(), "{v}: {r:?}"),
                w => assert!(
                    r.as_ref().is_err_and(|e| e.to_string().contains(w)),
                    "{v}: {r:?}"
                ),
            }
        }
        c.version = "sequoia".into();
        c.root_patch = true;
        assert_eq!(h.gpu_ready(&c, 0, u64::MAX)?.len(), 2);
        assert!(
            RESERVE_SCRIPT.contains("softdep nouveau pre: vfio-pci")
                && RESERVE_SCRIPT.contains("softdep nvidia pre: vfio-pci")
        );
        Ok(())
    }

    #[test]
    fn patch_kext_gpu_needs_a_kext_in_settings_yaml() -> TestResult {
        let mut devs = reference_host();
        devs[2].device = "73df"; // RX 6700 XT
        let (_d, mut h) = fake_sysfs(&devs)?;
        let g = gpu(&h, "0000:03:00.0")?;
        assert!(
            !g.eligible && g.reason.contains("settings.yaml"),
            "{:?}",
            g.reason
        );

        h.extra_kexts = Arc::new(|| Ok(1));
        assert!(gpu(&h, "0000:03:00.0")?.eligible);

        h.extra_kexts = Arc::new(|| {
            Err("/x/settings.yaml: yaml: line 1: did not find expected node content".into())
        });
        let g = gpu(&h, "0000:03:00.0")?;
        assert!(
            !g.eligible && g.reason.contains("settings.yaml"),
            "a broken file is the reason: {:?}",
            g.reason
        );

        // The start's snapshot (no kexts), taken before settings.yaml was edited, wins.
        h.extra_kexts = Arc::new(|| Ok(1));
        let c = Config {
            gpu: "0000:03:00.0".into(),
            version: "sequoia".into(),
            ..Config::defaults()
        };
        assert!(
            h.gpu_ready(&c, 0, 1 << 40)
                .is_err_and(|e| e.to_string().contains("settings.yaml"))
        );
        Ok(())
    }

    #[test]
    fn pci_ids_are_validated() {
        assert!(valid_pci_id("1002:73ff"));
        assert!(
            !valid_pci_id("1002:73FF") && !valid_pci_id("1002:73ff;x") && !valid_pci_id("100:73ff")
        );
    }
}
