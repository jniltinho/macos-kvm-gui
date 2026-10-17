//! A VM's settings, serialized like the Go app's `core.Config` (port of `config.go`).

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::network::{Nic, validate_nics};
use crate::settings::ExtraKexts;
use crate::workspace::write_file_atomic;
use crate::{Error, Result, msg};

/// A macOS version: the short name fetch-macOS-v2.py takes (`-s`) and its display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    pub short: &'static str,
    pub name: &'static str,
}

/// The versions in release order; the index decides the hardware profile.
pub const VERSIONS: [Version; 9] = [
    Version {
        short: "high-sierra",
        name: "High Sierra (10.13)",
    },
    Version {
        short: "mojave",
        name: "Mojave (10.14)",
    },
    Version {
        short: "catalina",
        name: "Catalina (10.15)",
    },
    Version {
        short: "big-sur",
        name: "Big Sur (11)",
    },
    Version {
        short: "monterey",
        name: "Monterey (12)",
    },
    Version {
        short: "ventura",
        name: "Ventura (13)",
    },
    Version {
        short: "sonoma",
        name: "Sonoma (14)",
    },
    Version {
        short: "sequoia",
        name: "Sequoia (15)",
    },
    Version {
        short: "tahoe",
        name: "Tahoe (26)",
    },
];

pub(crate) const IDX_BIG_SUR: i32 = 3;
pub(crate) const IDX_MONTEREY: i32 = 4;
pub(crate) const IDX_VENTURA: i32 = 5;
/// The only version tested end to end on the reference host; the others are experimental.
const VALIDATED: &str = "sonoma";

/// The version's index in [`VERSIONS`], or -1 when unknown.
pub fn version_index(short: &str) -> i32 {
    VERSIONS
        .iter()
        .position(|v| v.short == short)
        .map_or(-1, |i| i as i32)
}

/// The version's name without the number ("Sonoma"), or `short` when unknown.
pub fn version_display(short: &str) -> &str {
    VERSIONS
        .iter()
        .find(|v| v.short == short)
        .map_or(short, |v| {
            v.name.split_once(" (").map_or(v.name, |(n, _)| n)
        })
}

/// The Screen in the app over VNC (the default).
pub const DISPLAY_APP: &str = "app";
/// QEMU's own window.
pub const DISPLAY_WINDOW: &str = "window";
/// VMware SVGA II, drawn on the CPU (the default graphics adapter).
pub const GRAPHICS_SVGA: &str = "vmware-svga";
/// Reims vGPU: Metal in macOS through the host's Vulkan driver.
pub const GRAPHICS_REIMS: &str = "reims-vgpu";
/// Reims vGPU's display size when `resolution` is empty.
pub const DEFAULT_RESOLUTION: &str = "1920x1080";
/// The display sizes offered for Reims vGPU. See [`wide_resolution`].
pub const RESOLUTIONS: [&str; 7] = [
    "1920x1080",
    "1920x1200",
    "2048x1152",
    "2304x1296",
    "2560x1440",
    "2560x1600",
    "3840x2160",
];
/// The least RAM a VM may have.
pub const MIN_RAM_MIB: u32 = 4096;
/// The first SSH port a new VM may get (Go `FirstSSHPort`).
pub const FIRST_SSH_PORT: u16 = 2222;

/// A VM's settings. Field names and omission rules match the Go JSON exactly; a field missing
/// from the JSON takes its [`Config::defaults`] value, as Go's `LoadConfig` does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default = "Config::defaults")]
pub struct Config {
    /// Shown in the VM list.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub workspace: String,
    pub version: String,
    pub ram_mib: u32,
    pub cores: u32,
    pub vcpus: u32,
    pub ssh_port: u16,
    /// PCI address of a passed-through GPU; empty = emulated display.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub gpu: String,
    /// `app` (default) or `window`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub display: String,
    /// `None` (JSON null or absent, VMs saved before adapters existed) means one NAT adapter
    /// with the legacy MAC (see [`Config::adapters`]); an empty list means no adapter. Never
    /// omitted when serializing.
    pub networks: Option<Vec<Nic>>,
    /// Experimental root-patch mode (OCLP on a passed-through Kepler GPU).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub root_patch: bool,
    /// Empty or `vmware-svga` (the default), or `reims-vgpu`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub graphics: String,
    /// Reims vGPU display size (one of [`RESOLUTIONS`]); empty = [`DEFAULT_RESOLUTION`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub resolution: String,
    /// A start's snapshot of settings.yaml's kexts, never saved; `None` loads no extra kext.
    #[serde(skip)]
    pub extras: Option<ExtraKexts>,
    /// A one-off start option, never saved: macOS prints its boot log (`-v debug=0x100`).
    #[serde(skip)]
    pub verbose_boot: bool,
}

/// The invalid field, for the UI to highlight. Displays as `field: msg`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {msg}")]
pub struct FieldError {
    pub field: String,
    pub msg: String,
}

pub(crate) fn field_err(field: impl Into<String>, msg: impl Into<String>) -> Error {
    Error::Field(FieldError {
        field: field.into(),
        msg: msg.into(),
    })
}

/// The virtual hardware derived from the macOS version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub cpu: String,
    /// Every version but the validated one (Sonoma).
    pub experimental: bool,
    pub needs_avx2: bool,
}

/// From MY_OPTIONS in OpenCore-Boot.sh.
const CPU_FLAGS: &str = "kvm=on,vendor=GenuineIntel,+invtsc,vmware-cpuid-freq=on,+ssse3,+sse4.2,+popcnt,+avx,+aes,+xsave,+xsaveopt,check";

/// The CPU model and flags for a macOS version.
pub fn profile_for(version: &str) -> Profile {
    let i = version_index(version);
    let cpu = if i <= IDX_MONTEREY {
        format!("Penryn,{CPU_FLAGS}")
    } else {
        format!("Skylake-Client,-hle,-rtm,{CPU_FLAGS}")
    };
    Profile {
        cpu,
        experimental: version != VALIDATED,
        needs_avx2: i >= IDX_VENTURA,
    }
}

/// Whether root-patch mode applies: Monterey to Sequoia (Big Sur drives Kepler natively, Tahoe
/// has no stable OCLP support yet).
pub fn root_patch_version(version: &str) -> bool {
    matches!(version, "monterey" | "ventura" | "sonoma" | "sequoia")
}

/// Go `SMBIOSModel`: a Mac that supports the macOS version. iMac models also keep
/// AppleMCEReporter (which panics on AMD hosts) from loading.
pub fn smbios_model(version: &str) -> &'static str {
    match version {
        "high-sierra" => "iMac18,3",
        "mojave" => "iMac19,1",
        _ => "iMac20,1", // Catalina 10.15.6 to Tahoe
    }
}

/// Go `ReimsDir`: `<Workspace>/.reims` for the Workspace root (`~` expanded, absolute).
pub fn reims_dir(workspace_root: &str) -> Result<PathBuf> {
    Ok(expand_workspace(workspace_root, dirs::home_dir().as_deref())?.join(".reims"))
}

/// Whether WindowServer is expected to crash at login on this resolution without
/// ReimsVgpu.kext (macOS 26 only). Measured on Sequoia 15.8.1: wider than 2048 px crashes,
/// while 3840x2160 boots as a HiDPI 1920x1080 desktop, so 4K is not counted.
pub fn wide_resolution(r: &str) -> bool {
    let w: i64 = r
        .split('x')
        .next()
        .and_then(|w| w.parse().ok())
        .unwrap_or(0);
    w > 2048 && w < 3840
}

/// Whether `s` is a PCI address such as `0000:03:00.0` (lowercase hex).
pub fn is_pci_addr(s: &str) -> bool {
    let b = s.as_bytes();
    let hex = |r: std::ops::Range<usize>| {
        b[r].iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    };
    b.len() == 12
        && hex(0..4)
        && b[4] == b':'
        && hex(5..7)
        && b[7] == b':'
        && hex(8..10)
        && b[10] == b'.'
        && (b'0'..=b'7').contains(&b[11])
}

impl Config {
    /// The Go app's `Defaults()`.
    pub fn defaults() -> Self {
        Self {
            workspace: "~/OSX-KVM".into(),
            version: VALIDATED.into(),
            ram_mib: 8192,
            cores: 4,
            vcpus: 8,
            // Automatic: `Registry::create` gives the VM the next free port.
            ssh_port: 0,
            ..Self::default()
        }
    }

    /// Fits CPU and RAM to the host: about half its threads as cores × 2 threads with a
    /// power-of-two core count (24 threads give 4 × 2 = 8 vCPUs), and a quarter of its RAM
    /// between 4 and 16 GiB.
    pub fn recommended(mut self, host_threads: u32, host_mem_mib: u64) -> Self {
        let want = (host_threads / 2).clamp(2, 16);
        let mut cores = 1;
        while cores * 2 * 2 <= want {
            cores *= 2;
        }
        self.vcpus = cores * 2;
        self.cores = cores;
        self.ram_mib = (host_mem_mib / 4 / 1024 * 1024).clamp(MIN_RAM_MIB.into(), 16384) as u32;
        self
    }

    /// Whether the VM runs on Reims vGPU.
    pub fn reims(&self) -> bool {
        self.graphics == GRAPHICS_REIMS
    }

    /// Whether the VM loads ReimsVgpu.kext: Reims vGPU on Tahoe, the only version it loads on.
    pub fn reims_kext(&self) -> bool {
        self.reims() && self.version == "tahoe"
    }

    /// Whether the screen is shown inside the app.
    pub fn in_app(&self) -> bool {
        self.display != DISPLAY_WINDOW
    }

    /// The Reims vGPU display size.
    pub fn reims_resolution(&self) -> &str {
        if self.resolution.is_empty() {
            DEFAULT_RESOLUTION
        } else {
            &self.resolution
        }
    }

    /// The workspace folder: `~` expanded, absolute and cleaned.
    pub fn workspace_dir(&self) -> Result<PathBuf> {
        expand_workspace(&self.workspace, dirs::home_dir().as_deref())
    }

    /// Checks every setting, in the Go app's order and with its messages.
    pub fn validate(&self) -> Result<()> {
        self.workspace_dir()?;
        if version_index(&self.version) < 0 {
            return Err(field_err(
                "version",
                format!("unknown version: {}", self.version),
            ));
        }
        if self.ram_mib < MIN_RAM_MIB {
            return Err(field_err("ram", format!("minimum is {MIN_RAM_MIB} MiB")));
        }
        if self.cores < 1 {
            return Err(field_err("cores", "minimum is 1 core"));
        }
        if self.vcpus < 1 || !self.vcpus.is_multiple_of(self.cores) {
            return Err(field_err(
                "vcpus",
                "must be a multiple of the number of cores",
            ));
        }
        // macOS stops at the Apple logo when the core count is not a power of two.
        if !self.cores.is_power_of_two() {
            return Err(field_err(
                "cores",
                format!(
                    "{} cores: use 1, 2, 4, 8 or 16 (macOS hangs at the Apple logo with other core counts)",
                    self.cores
                ),
            ));
        }
        if !self.display.is_empty() && self.display != DISPLAY_APP && self.display != DISPLAY_WINDOW
        {
            return Err(field_err("display", "expected app or window"));
        }
        if self.root_patch && !root_patch_version(&self.version) {
            return Err(field_err(
                "root_patch",
                "root-patch mode is for Monterey 12 to Sequoia 15: Big Sur drives Kepler GPUs natively, and Tahoe has no stable OCLP support yet",
            ));
        }
        if !self.graphics.is_empty()
            && self.graphics != GRAPHICS_SVGA
            && self.graphics != GRAPHICS_REIMS
        {
            return Err(field_err(
                "graphics",
                format!("expected {GRAPHICS_SVGA} or {GRAPHICS_REIMS}"),
            ));
        }
        if !self.resolution.is_empty() && !RESOLUTIONS.contains(&self.resolution.as_str()) {
            return Err(field_err(
                "resolution",
                format!("expected one of {}", RESOLUTIONS.join(", ")),
            ));
        }
        if self.reims() && !self.gpu.is_empty() {
            return Err(field_err(
                "graphics",
                format!(
                    "Reims vGPU and GPU passthrough cannot be used together: choose one of them (clear the GPU, or set graphics to {GRAPHICS_SVGA})"
                ),
            ));
        }
        if !self.gpu.is_empty() && !is_pci_addr(&self.gpu) {
            return Err(field_err(
                "gpu",
                "expected a PCI address such as 0000:03:00.0",
            ));
        }
        if self.ssh_port < 1024 {
            return Err(field_err("ssh_port", "use a port between 1024 and 65535"));
        }
        validate_nics(&self.adapters())
    }
}

pub(crate) fn expand_workspace(p: &str, home: Option<&Path>) -> Result<PathBuf> {
    let path = if p == "~" || p.starts_with("~/") {
        let home = home.ok_or_else(|| msg("$HOME is not defined"))?;
        home.join(p.trim_start_matches('~').trim_start_matches('/'))
    } else {
        PathBuf::from(p)
    };
    if !path.is_absolute() {
        return Err(field_err("workspace", "must be an absolute path"));
    }
    Ok(clean(&path))
}

/// Lexical path cleaning, like Go's `filepath.Clean`.
pub fn clean(p: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) => {}
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    if out.is_empty() {
        PathBuf::from(".")
    } else {
        out.iter().collect()
    }
}

/// Reads a standalone settings file (`--config`, a legacy `config.json`); defaults when missing.
pub fn load_config(path: &Path) -> Result<Config> {
    let mut c: Config = match std::fs::read(path) {
        Ok(data) => serde_json::from_slice(&data)
            .map_err(|e| msg(format!("invalid config in {}: {e}", path.display())))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::defaults(),
        Err(e) => return Err(e.into()),
    };
    // A standalone file is not in the VM list, so "automatic" is simply the first port.
    if c.ssh_port == 0 {
        c.ssh_port = FIRST_SSH_PORT;
    }
    Ok(c)
}

/// Validates and writes a standalone settings file atomically.
pub fn save_config(path: &Path, c: &Config) -> Result<()> {
    c.validate()?;
    let data = serde_json::to_vec_pretty(c)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_file_atomic(path, &data, 0o644)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Case = (&'static str, fn(&mut Config));

    /// The defaults with a port: valid as they are.
    fn valid() -> Config {
        Config {
            ssh_port: FIRST_SSH_PORT,
            ..Config::defaults()
        }
    }

    fn field_of(c: &Config) -> Option<String> {
        match c.validate() {
            Err(Error::Field(f)) => Some(f.field),
            _ => None,
        }
    }

    #[test]
    fn json_matches_go() -> Result<(), serde_json::Error> {
        // As the Go app writes it (fields in struct order, empty optional ones left out).
        let go = r#"{"workspace":"/vms/n","version":"sequoia","ram_mib":16384,"cores":2,"vcpus":4,"ssh_port":2222,"networks":null,"graphics":"reims-vgpu"}"#;
        let c: Config = serde_json::from_str(go)?;
        assert!(c.reims() && c.in_app() && c.networks.is_none());
        assert_eq!(c.reims_resolution(), DEFAULT_RESOLUTION);
        assert_eq!(serde_json::to_string(&c)?, go);
        Ok(())
    }

    #[test]
    fn missing_json_fields_take_the_defaults() -> Result<(), serde_json::Error> {
        let c: Config = serde_json::from_str(r#"{"name":"A","workspace":"/tmp/a"}"#)?;
        assert_eq!(
            c,
            Config {
                name: "A".into(),
                workspace: "/tmp/a".into(),
                ..Config::defaults()
            }
        );
        Ok(())
    }

    #[test]
    fn no_adapters_survive_json() -> Result<(), serde_json::Error> {
        let c = Config {
            networks: Some(vec![]),
            verbose_boot: true,
            ..Config::defaults()
        };
        let s = serde_json::to_string(&c)?;
        assert!(!s.contains("verbose"));
        let back: Config = serde_json::from_str(&s)?;
        assert_eq!(back.networks, Some(vec![]));
        let legacy: Config = serde_json::from_str(r#"{"version":"sonoma"}"#)?;
        assert!(legacy.networks.is_none());
        Ok(())
    }

    #[test]
    fn config_round_trip_and_defaults() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("sub/config.json");
        assert_eq!(
            load_config(&path)?,
            valid(),
            "a missing file gets port 2222"
        );
        let c = Config {
            ram_mib: 16384,
            ..valid()
        };
        save_config(&path, &c)?;
        assert_eq!(load_config(&path)?.ram_mib, 16384);
        std::fs::write(&path, "{broken")?;
        assert!(load_config(&path).is_err());
        Ok(())
    }

    #[test]
    fn validate_names_the_field() {
        let cases: [Case; 6] = [
            ("ram", |c| c.ram_mib = 2048),
            ("cores", |c| c.cores = 0),
            ("vcpus", |c| (c.cores, c.vcpus) = (4, 6)),
            ("ssh_port", |c| c.ssh_port = 80),
            ("workspace", |c| c.workspace = "relative/dir".into()),
            ("version", |c| c.version = "lion".into()),
        ];
        for (field, mutate) in cases {
            let mut c = valid();
            mutate(&mut c);
            assert_eq!(field_of(&c).as_deref(), Some(field));
        }
        assert!(valid().validate().is_ok());
    }

    #[test]
    fn validate_graphics_resolution_gpu_root_patch() {
        let cases: [Case; 6] = [
            ("display", |c| c.display = "tv".into()),
            ("graphics", |c| c.graphics = "virtio".into()),
            ("resolution", |c| c.resolution = "800x600".into()),
            ("graphics", |c| {
                (c.graphics, c.gpu) = (GRAPHICS_REIMS.into(), "0000:03:00.0".into())
            }),
            ("gpu", |c| c.gpu = "03:00.0".into()),
            ("root_patch", |c| {
                (c.root_patch, c.version) = (true, "tahoe".into())
            }),
        ];
        for (field, mutate) in cases {
            let mut c = valid();
            mutate(&mut c);
            assert_eq!(field_of(&c).as_deref(), Some(field), "{c:?}");
        }
        let ok = Config {
            gpu: "0000:03:00.0".into(),
            root_patch: true,
            ..valid()
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn recommended_fits_the_host() {
        for (threads, mem, vcpus, cores, ram) in [
            (24, 186 * 1024, 8, 4, 16384), // 6 cores would hang macOS at the logo
            (4, 8 * 1024, 2, 1, 4096),
            (64, 512 * 1024, 16, 8, 16384),
            (6, 24 * 1024, 2, 1, 6144),
        ] {
            let c = valid().recommended(threads, mem);
            assert_eq!(
                (c.vcpus, c.cores, c.ram_mib),
                (vcpus, cores, ram),
                "{threads} threads"
            );
            assert!(c.validate().is_ok());
        }
    }

    #[test]
    fn workspace_tilde_expands() -> Result<()> {
        let home = Path::new("/home/x");
        assert_eq!(
            expand_workspace("~/OSX-KVM", Some(home))?,
            Path::new("/home/x/OSX-KVM")
        );
        assert_eq!(expand_workspace("~", Some(home))?, home);
        assert_eq!(expand_workspace("/a/./b/../c/", None)?, Path::new("/a/c"));
        Ok(())
    }

    #[test]
    fn profile_per_version() {
        for v in VERSIONS {
            let p = profile_for(v.short);
            let old = version_index(v.short) <= IDX_MONTEREY;
            assert_eq!(old, p.cpu.starts_with("Penryn"), "{}", v.short);
            assert_eq!(p.needs_avx2, !old, "{}", v.short);
            assert_eq!(p.experimental, v.short != "sonoma");
            let m = crate::network::default_nic_model(v.short);
            assert!(crate::network::nic_supported(v.short, m), "{}", v.short);
        }
    }

    #[test]
    fn cores_must_be_a_power_of_two() {
        let mut c = valid();
        for cores in [1, 2, 4, 8, 16] {
            (c.cores, c.vcpus) = (cores, cores * 2);
            assert!(c.validate().is_ok(), "{cores}");
        }
        for cores in [3, 6, 12] {
            (c.cores, c.vcpus) = (cores, cores * 2);
            assert!(
                matches!(c.validate(), Err(e) if e.to_string().contains("Apple logo")),
                "{cores}"
            );
        }
    }

    #[test]
    fn wide_resolutions() {
        let wide: Vec<_> = RESOLUTIONS.iter().filter(|r| wide_resolution(r)).collect();
        assert_eq!(wide, [&"2304x1296", &"2560x1440", &"2560x1600"]);
        assert!(!wide_resolution("junk"));
    }

    #[test]
    fn version_names() {
        assert_eq!(version_display("big-sur"), "Big Sur");
        assert_eq!(version_display("lion"), "lion");
        assert!(
            Config {
                graphics: GRAPHICS_REIMS.into(),
                version: "tahoe".into(),
                ..Config::defaults()
            }
            .reims_kext()
        );
    }

    #[test]
    fn smbios_model_per_version() {
        assert_eq!(smbios_model("high-sierra"), "iMac18,3");
        assert_eq!(smbios_model("mojave"), "iMac19,1");
        assert_eq!(smbios_model("tahoe"), "iMac20,1");
    }

    #[test]
    fn reims_dir_needs_an_absolute_workspace() {
        assert_eq!(reims_dir("/vms/").ok(), Some(PathBuf::from("/vms/.reims")));
        assert!(reims_dir("vms").is_err());
        if dirs::home_dir().is_some() {
            assert!(reims_dir("~/macOS-VMs").is_ok_and(|p| p.ends_with("macOS-VMs/.reims")));
        }
    }
}
