//! Orchestration that spans crates (design D1, layer 3): workspace preparation, media download,
//! creating a VM, export/delete/move with running checks, the VM start pipeline, shutdown and
//! the offline install. Port of the Go app's `workspace.go` `Prepare`/`DownloadMedia`,
//! `vm.go` `Start`, the GUI's and CLI's `startWith`, and the running checks of `bundle.go` and
//! `vms.go`. See `API.md`.
//!
//! Everything but [`Ops::start_vm`]'s event stream is blocking: async callers use
//! `tokio::task::spawn_blocking`. Long calls take a `cancel` flag.

mod install;
mod start;
mod vms;

use std::path::PathBuf;
use std::sync::Arc;

use rg_assets::{BootAssets, KextCache};
use rg_core::settings::MKG_ASSETS_DIR;
use rg_core::{AppDir, Config, Registry};
use rg_host::Host;
use rg_qemu::RunInfo;
use rg_reims::Reims;

pub use install::install_line;
pub use start::Running;
pub use vms::CreateOptions;

/// Errors of the crates this one drives, plus its own checks. Messages are the Go app's.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Includes `rg_core::Error::Busy` (the Workspace is locked) and `PortInUse`.
    #[error(transparent)]
    Core(#[from] rg_core::Error),
    #[error(transparent)]
    Assets(#[from] rg_assets::Error),
    #[error(transparent)]
    Host(#[from] rg_host::HostError),
    #[error(transparent)]
    Macos(#[from] rg_macos::Error),
    #[error(transparent)]
    Qemu(#[from] rg_qemu::Error),
    #[error(transparent)]
    Reims(#[from] rg_reims::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Msg(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn msg(s: impl Into<String>) -> Error {
    Error::Msg(s.into())
}

/// What the operations need from the user's settings and the host, read once. Tests swap the
/// paths and the [`Host`] for fakes.
#[derive(Clone)]
pub struct Ops {
    pub app: AppDir,
    /// The Workspace folder, absolute: `.kexts`, `.reims` and `.installers` live there.
    pub workspace_root: PathBuf,
    /// `MKG_ASSETS_DIR`; `None` = the embedded files.
    pub assets_dir: Option<PathBuf>,
    /// NVRAM `prev-lang:kbd` (rg-core's `recovery_keyboard()`).
    pub keyboard: String,
    pub http: rg_macos::Http,
    /// The QEMU for VMware SVGA VMs (`qemu-system-x86_64` from PATH).
    pub qemu: PathBuf,
    /// Host checks; `reims_qemu` and `extra_kexts` are wired to `<Workspace>/.reims` and
    /// `app`'s settings.yaml.
    pub host: Host,
    /// `/proc`, for running-VM detection.
    pub proc_root: PathBuf,
    /// `/sys/class/net` and `/etc/qemu/bridge.conf`, for the bridged adapters' check.
    pub net_root: PathBuf,
    pub bridge_acl: PathBuf,
}

impl Ops {
    /// Operations for the config folder `app` and the Workspace root `workspace_root` (the
    /// registry's `workspace` preference, `~` allowed).
    pub fn new(app: AppDir, workspace_root: &str) -> Result<Self> {
        let root = Config {
            workspace: workspace_root.into(),
            ..Config::defaults()
        }
        .workspace_dir()?;
        let mut host = Host::system();
        let (reims_dir, a) = (root.join(".reims"), app.clone());
        host.reims_qemu = Arc::new(move || {
            Reims::for_app(&reims_dir, Some(&a))
                .qemu()
                .map_err(|e| e.to_string())
        });
        let a = app.clone();
        host.extra_kexts = Arc::new(move || {
            a.load_extra_kexts()
                .map(|k| k.entries.len())
                .map_err(|e| e.to_string())
        });
        host.settings_file = app.settings_file().to_string_lossy().into_owned();
        let assets = app.setting(MKG_ASSETS_DIR);
        Ok(Self {
            assets_dir: (!assets.is_empty()).then(|| assets.into()),
            keyboard: rg_core::keyboard::recovery_keyboard(),
            app,
            workspace_root: root,
            http: rg_macos::Http::default(),
            qemu: "qemu-system-x86_64".into(),
            host,
            proc_root: "/proc".into(),
            net_root: "/sys/class/net".into(),
            bridge_acl: "/etc/qemu/bridge.conf".into(),
        })
    }

    /// The user's config folder and the registry's Workspace.
    pub fn user(reg: &Registry) -> Result<Self> {
        let app = AppDir::user().ok_or_else(|| msg("no user config folder ($HOME is not set)"))?;
        Self::new(app, &reg.workspace_root())
    }

    /// `<Workspace>/.kexts`.
    pub fn kext_cache(&self) -> KextCache {
        KextCache::new(&self.workspace_root)
    }

    /// `<Workspace>/.installers`.
    pub fn installers_dir(&self) -> PathBuf {
        rg_macos::installers_dir(&self.workspace_root)
    }

    /// Reims packages in `<Workspace>/.reims`.
    pub fn reims(&self) -> Reims {
        Reims::for_app(self.workspace_root.join(".reims"), Some(&self.app))
    }

    /// The QEMU using `c`'s disk, whoever started it.
    pub fn find_running(&self, c: &Config) -> Option<RunInfo> {
        rg_qemu::running::running_in(&self.proc_root, c)
    }

    pub(crate) fn assets(&self) -> Result<BootAssets> {
        Ok(BootAssets::open(self.assets_dir.as_deref())?)
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    pub(crate) type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    /// Ops on temporary folders: config folder, Workspace root, empty /proc and /sys/class/net.
    pub(crate) fn ops(t: &tempfile::TempDir) -> std::io::Result<Ops> {
        let p = t.path();
        for d in ["app", "root", "proc", "net"] {
            std::fs::create_dir_all(p.join(d))?;
        }
        let mut o = Ops::new(AppDir::at(p.join("app")), &p.join("root").to_string_lossy())
            .map_err(std::io::Error::other)?;
        o.assets_dir = None;
        o.keyboard = "en-US:0".into();
        o.proc_root = p.join("proc");
        o.net_root = p.join("net");
        o.bridge_acl = p.join("bridge.conf");
        Ok(o)
    }

    /// A stopped VM's config with its folder in `t`.
    pub(crate) fn config(t: &tempfile::TempDir, name: &str) -> Config {
        Config {
            name: name.into(),
            workspace: t.path().join(name).to_string_lossy().into_owned(),
            ssh_port: free_port(),
            ..Config::defaults()
        }
    }

    pub(crate) fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_or(2222, |a| a.port())
    }

    /// A fake `/proc/<pid>` for a QEMU that opens `c`'s disk.
    pub(crate) fn fake_qemu(o: &Ops, c: &Config, pid: u32) -> std::io::Result<()> {
        let d = o.proc_root.join(pid.to_string());
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join("comm"), "qemu-system-x86\n")?;
        let disk = c
            .workspace_dir()
            .map_err(std::io::Error::other)?
            .join(rg_core::workspace::DISK_FILE);
        std::fs::write(
            d.join("cmdline"),
            format!(
                "qemu-system-x86_64\0-drive\0id=MacHDD,file={}\0",
                disk.display()
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;

    #[test]
    fn ops_wire_reims_and_kexts_to_the_workspace_and_settings() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        assert_eq!(
            o.kext_cache().extra_dir(),
            t.path().join("root/.kexts/extra")
        );
        assert_eq!(o.reims().dir(), t.path().join("root/.reims"));
        assert_eq!(o.installers_dir(), t.path().join("root/.installers"));
        std::fs::write(
            o.app.settings_file(),
            "kexts:\n  - url: https://example.com/a.zip\n",
        )?;
        assert_eq!((o.host.extra_kexts)()?, 1);
        assert!(super::Ops::new(o.app.clone(), "relative").is_err());
        Ok(())
    }
}
