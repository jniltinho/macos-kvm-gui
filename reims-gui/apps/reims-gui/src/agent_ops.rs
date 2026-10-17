//! rg-agent's [`rg_agent::Ops`] on rg-ops. It lives in the binary: rg-ops (layer 3) must not
//! depend on rg-agent (layer 4), and rg-agent stays testable without rg-ops.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use rg_core::{AppDir, Config, Registry, VmEntry};
use rg_qemu::VmEvent;
use tokio::runtime::{Handle, Runtime};

/// rg-ops for the config folder `app`, with the Workspace read from its registry at each call
/// (a `workspace set` earlier in the same process applies).
pub struct AgentOps {
    app: AppDir,
    /// For the VMs `vm start` runs; made on first use.
    rt: OnceLock<Runtime>,
}

impl AgentOps {
    pub fn new(app: AppDir) -> Self {
        Self {
            app,
            rt: OnceLock::new(),
        }
    }

    fn ops(&self) -> Result<rg_ops::Ops> {
        let reg = Registry::open(&self.app)?;
        Ok(rg_ops::Ops::new(self.app.clone(), &reg.workspace_root())?)
    }

    fn handle(&self) -> Result<Handle> {
        if let Some(rt) = self.rt.get() {
            return Ok(rt.handle().clone());
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        Ok(self.rt.get_or_init(|| rt).handle().clone())
    }
}

impl rg_agent::Ops for AgentOps {
    fn prepare_workspace(&self, c: &Config) -> Result<()> {
        Ok(self.ops()?.prepare_workspace(c)?)
    }

    fn create_vm(
        &self,
        reg: &Registry,
        c: Config,
        disk_gib: Option<u64>,
    ) -> Result<(VmEntry, Option<String>)> {
        let opts = rg_ops::CreateOptions {
            disk_gib,
            download: false,
        };
        let no = AtomicBool::new(false);
        Ok(self
            .ops()?
            .create_vm(reg, c, opts, &no, &mut |_| {}, &mut |_| {})?)
    }

    fn download_media(
        &self,
        c: &Config,
        version: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(rg_macos::Progress),
        log: &mut dyn FnMut(&str),
    ) -> Result<()> {
        Ok(self
            .ops()?
            .download_media(c, version, cancel, progress, log)?)
    }

    fn start_vm(
        &self,
        c: Config,
        on_event: Box<dyn Fn(VmEvent) + Send + Sync>,
    ) -> Result<Box<dyn rg_agent::RunningVm>> {
        let rt = self.handle()?;
        let cancel = AtomicBool::new(false);
        let run = self
            .ops()?
            .start_vm(rt.clone(), c, false, &cancel, &mut |l| {
                on_event(VmEvent::Log(l.into()));
            })?;
        let rg_ops::Running {
            vm,
            started,
            mut events,
            installer_command,
        } = run;
        rt.spawn(async move {
            while let Some(ev) = events.recv().await {
                on_event(ev);
            }
        });
        Ok(Box::new(Running {
            vm,
            vnc: started.vnc_socket,
            installer_command,
            rt,
        }))
    }
}

struct Running {
    vm: rg_qemu::Vm,
    vnc: Option<PathBuf>,
    installer_command: Option<&'static str>,
    rt: Handle,
}

impl rg_agent::RunningVm for Running {
    fn vnc_socket(&self) -> Option<PathBuf> {
        self.vnc.clone()
    }

    fn installer_command(&self) -> Option<String> {
        self.installer_command.map(str::to_owned)
    }

    fn powerdown(&self) -> Result<()> {
        Ok(self.rt.block_on(self.vm.powerdown())?)
    }

    fn kill(&self) -> Result<()> {
        Ok(self.vm.kill()?)
    }
}
