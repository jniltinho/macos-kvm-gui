//! A [`Ctx`] on a temporary config folder and Workspace, with fake [`Ops`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, bail};
use rg_core::{AppDir, Config, Registry, VmEntry};
use rg_qemu::VmEvent;

use crate::{Ctx, Ops, RunningVm};

/// `prepare_workspace` creates the folder and its boot folder, `create_vm` an empty disk;
/// nothing else is supported.
pub struct FakeOps;

impl Ops for FakeOps {
    fn prepare_workspace(&self, c: &Config) -> Result<()> {
        std::fs::create_dir_all(c.workspace_dir()?.join(rg_core::workspace::BOOT_DIR))?;
        Ok(())
    }

    fn create_vm(
        &self,
        reg: &Registry,
        mut c: Config,
        disk_gib: Option<u64>,
    ) -> Result<(VmEntry, Option<String>)> {
        if c.networks.is_none() {
            c.networks = Some(rg_core::network::new_networks(&c.version));
        }
        let (e, notice) = reg.create_noted(c)?;
        reg.select(&e.id)?;
        self.prepare_workspace(&e.config)?;
        if disk_gib.is_some() {
            let disk = e
                .config
                .workspace_dir()?
                .join(rg_core::workspace::DISK_FILE);
            std::fs::write(disk, "")?;
        }
        Ok((e, notice))
    }

    fn download_media(
        &self,
        _: &Config,
        _: &str,
        _: &AtomicBool,
        _: &mut dyn FnMut(rg_macos::Progress),
        _: &mut dyn FnMut(&str),
    ) -> Result<()> {
        bail!("fake: no download")
    }

    fn start_vm(
        &self,
        _: Config,
        _: Box<dyn Fn(VmEvent) + Send + Sync>,
    ) -> Result<Box<dyn RunningVm>> {
        bail!("fake: no VM is started in tests")
    }
}

pub struct Env {
    pub tmp: tempfile::TempDir,
    pub ctx: Ctx,
}

impl Env {
    /// A config folder and a Workspace (`root`) in a new temporary folder.
    pub fn new() -> Result<Self> {
        let tmp = tempfile::tempdir()?;
        let ctx = Ctx {
            version: "test".into(),
            app: AppDir::at(tmp.path().join("config")),
            ops: Arc::new(FakeOps),
        };
        let env = Self { tmp, ctx };
        Registry::open(&env.ctx.app)?.set_workspace_root(&env.root().display().to_string())?;
        Ok(env)
    }

    pub fn root(&self) -> PathBuf {
        self.tmp.path().join("root")
    }

    pub fn path(&self, p: impl AsRef<Path>) -> PathBuf {
        self.tmp.path().join(p)
    }
}
