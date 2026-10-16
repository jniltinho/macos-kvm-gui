//! A VM's settings, serialized like the Go app's `core.Config`.

use serde::{Deserialize, Serialize};

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
/// The display sizes offered for Reims vGPU.
pub const RESOLUTIONS: [&str; 7] =
    ["1920x1080", "1920x1200", "2048x1152", "2304x1296", "2560x1440", "2560x1600", "3840x2160"];
/// The least RAM a VM may have.
pub const MIN_RAM_MIB: u32 = 4096;

/// One network adapter (Adapter 1–4).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nic {
    pub enabled: bool,
    /// `nat` or `bridged`.
    pub mode: String,
    /// Host bridge for `bridged`, e.g. `br0`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bridge: String,
    pub model: String,
    pub mac: String,
}

/// A VM's settings. Field names and omission rules match the Go JSON exactly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Shown in the VM list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub workspace: String,
    pub version: String,
    pub ram_mib: u32,
    pub cores: u32,
    pub vcpus: u32,
    pub ssh_port: u16,
    /// PCI address of a passed-through GPU; empty = emulated display.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gpu: String,
    /// `app` (default) or `window`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display: String,
    /// `None` (JSON null or absent, VMs saved before adapters existed) means one NAT adapter
    /// with the legacy MAC; an empty list means no adapter. Never omitted when serializing.
    pub networks: Option<Vec<Nic>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub root_patch: bool,
    /// Empty or `vmware-svga` (the default), or `reims-vgpu`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub graphics: String,
    /// Reims vGPU display size (one of [`RESOLUTIONS`]); empty = [`DEFAULT_RESOLUTION`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resolution: String,
}

impl Config {
    /// The Go app's `Defaults()`.
    pub fn defaults() -> Self {
        Self {
            workspace: "~/OSX-KVM".into(),
            version: "sonoma".into(),
            ram_mib: 8192,
            cores: 4,
            vcpus: 8,
            ssh_port: 2222,
            ..Self::default()
        }
    }

    /// Whether the VM runs on Reims vGPU.
    pub fn reims(&self) -> bool {
        self.graphics == GRAPHICS_REIMS
    }

    /// Whether the screen is shown inside the app.
    pub fn in_app(&self) -> bool {
        self.display != DISPLAY_WINDOW
    }

    /// The Reims vGPU display size.
    pub fn reims_resolution(&self) -> &str {
        if self.resolution.is_empty() { DEFAULT_RESOLUTION } else { &self.resolution }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
