//! The command-line interface and the MCP server (port of the Go app's `cmd/` and
//! `internal/mcp`). See `API.md`.
//!
//! Everything works on a [`Ctx`]: the app's config folder (database, settings.yaml) and the
//! [`Ops`] that start VMs and prepare their folders (rg-ops, wired by the binary).

mod cli;
mod mcp;
#[cfg(test)]
mod testutil;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use rg_core::{AppDir, Config, Registry};
use rg_qemu::VmEvent;

pub use cli::{Cli, execute, main};
pub use mcp::{Server, exchange, handshake, self_test, serve_stdio};

/// What the CLI and the MCP server act on.
#[derive(Clone)]
pub struct Ctx {
    /// The app's version, for `version` and the MCP server's info.
    pub version: String,
    /// `~/.config/macos-kvm-gui` (tests use a temporary folder).
    pub app: AppDir,
    /// The operations that span crates (rg-ops).
    pub ops: Arc<dyn Ops>,
}

/// The operations rg-agent needs from `rg-ops`: each spans several crates (boot files, locks,
/// host checks, the QEMU process), so they live there and the binary wires them in.
pub trait Ops: Send + Sync {
    /// Go `Workspace.Prepare`: under the VM folder's lock, creates the folder and writes the
    /// boot files (OpenCore, OVMF, NVRAM, identity) for `c`.
    fn prepare_workspace(&self, c: &Config) -> Result<()>;

    /// Go `Workspace.DownloadMedia`: under the VM folder's lock, downloads, verifies and
    /// converts the recovery of `version` into `<folder>/BaseSystem.img`.
    fn download_media(
        &self,
        c: &Config,
        version: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(rg_macos::Progress),
        log: &mut dyn FnMut(&str),
    ) -> Result<()>;

    /// Go `VM.Start`: the whole start (validation, Workspace lock, boot files with
    /// `c.extras`, host and GPU checks, installer disc, QEMU). Returns once QEMU runs; state
    /// changes and output lines go to `on_event` (`VmEvent::State(Stopped, msg)` ends the run,
    /// `msg` empty for a clean stop).
    fn start_vm(
        &self,
        c: Config,
        on_event: Box<dyn Fn(VmEvent) + Send + Sync>,
    ) -> Result<Box<dyn RunningVm>>;
}

/// A VM [`Ops::start_vm`] started.
pub trait RunningVm: Send {
    /// The private VNC socket, when the screen is not QEMU's own window.
    fn vnc_socket(&self) -> Option<PathBuf>;
    /// Go `VM.InstallerCommand`: the line to type in the recovery's Terminal when the offline
    /// installer disc is attached.
    fn installer_command(&self) -> Option<String>;
    /// The ACPI power button.
    fn powerdown(&self) -> Result<()>;
    /// Stops QEMU at once.
    fn kill(&self) -> Result<()>;
}

impl Ctx {
    /// The user's config folder and `ops`.
    pub fn user(version: impl Into<String>, ops: Arc<dyn Ops>) -> Result<Self> {
        let app =
            AppDir::user().ok_or_else(|| anyhow::anyhow!("no config folder ($HOME is not set)"))?;
        Ok(Self {
            version: version.into(),
            app,
            ops,
        })
    }

    fn registry(&self) -> Result<Registry> {
        Ok(Registry::open(&self.app)?)
    }

    /// The Workspace folder, `~` expanded.
    fn workspace_root(&self) -> Result<PathBuf> {
        let root = self.registry()?.workspace_root();
        Ok(Config {
            workspace: root,
            ..Config::default()
        }
        .workspace_dir()?)
    }

    fn kext_cache(&self) -> Result<rg_assets::KextCache> {
        Ok(rg_assets::KextCache::new(&self.workspace_root()?))
    }

    fn reims(&self) -> Result<rg_reims::Reims> {
        Ok(rg_reims::Reims::for_app(
            self.workspace_root()?.join(".reims"),
            Some(&self.app),
        ))
    }

    /// The host with the Reims QEMU lookup and settings.yaml's kexts wired in.
    fn host(&self) -> rg_host::Host {
        let mut h = rg_host::Host::system();
        let ctx = self.clone();
        h.reims_qemu = Arc::new(move || {
            ctx.reims()
                .map_err(|e| e.to_string())
                .and_then(|r| r.qemu().map_err(|e| e.to_string()))
        });
        let app = self.app.clone();
        h.extra_kexts = Arc::new(move || {
            app.load_extra_kexts()
                .map(|k| k.entries.len())
                .map_err(|e| e.to_string())
        });
        h.settings_file = self.app.settings_file().display().to_string();
        h
    }
}

/// settings.yaml's kexts as rg-assets takes them.
// ponytail: copy until rg-assets uses rg-core's ExtraKexts (its compat.rs goes at integration).
/// The kexts snapshot `vm start --kexts-snapshot` takes from the MCP server, which already
/// downloaded them.
#[derive(serde::Serialize, serde::Deserialize)]
struct KextsSnapshot {
    disabled: Vec<String>,
    entries: Vec<(String, String, Vec<String>)>,
}

impl KextsSnapshot {
    fn json(ks: &rg_core::ExtraKexts) -> Result<String> {
        let s = KextsSnapshot {
            disabled: ks.disabled.clone(),
            entries: ks
                .entries
                .iter()
                .map(|e| (e.url.clone(), e.sha256.clone(), e.kexts.clone()))
                .collect(),
        };
        Ok(serde_json::to_string(&s)?)
    }

    fn parse(s: &str) -> Result<rg_core::ExtraKexts> {
        let s: KextsSnapshot = serde_json::from_str(s)?;
        Ok(rg_core::ExtraKexts {
            disabled: s.disabled,
            entries: s
                .entries
                .into_iter()
                .map(|(url, sha256, kexts)| rg_core::ExtraKext { url, sha256, kexts })
                .collect(),
        })
    }
}

/// Go's `time.Duration.String` for whole seconds: `1h2m3s`, `4m0s`, `5s`.
fn go_duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    match (h, m) {
        (0, 0) => format!("{s}s"),
        (0, _) => format!("{m}m{s}s"),
        _ => format!("{h}h{m}m{s}s"),
    }
}

/// Whether a path names a file in a folder that exists (Go's `os.Create` needs the folder).
fn parent_exists(p: &Path) -> bool {
    p.parent().is_some_and(Path::is_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_print_like_go() {
        let d = std::time::Duration::from_secs;
        assert_eq!(go_duration(d(5)), "5s");
        assert_eq!(go_duration(d(240)), "4m0s");
        assert_eq!(go_duration(d(3723)), "1h2m3s");
    }

    #[test]
    fn kexts_snapshot_round_trips() -> Result<()> {
        let ks = rg_core::ExtraKexts {
            disabled: vec!["Lilu.kext".into()],
            entries: vec![rg_core::ExtraKext {
                url: "https://x/y.zip".into(),
                sha256: String::new(),
                kexts: vec![],
            }],
        };
        assert_eq!(KextsSnapshot::parse(&KextsSnapshot::json(&ks)?)?, ks);
        Ok(())
    }
}
