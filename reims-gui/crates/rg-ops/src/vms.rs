//! Workspace preparation, media download, creating a VM, and the operations that need the VM
//! shut down (Go `Workspace.Prepare`, `DownloadMedia`, `CreateVM` callers, `ExportBundle`,
//! `DeleteVM`, `MoveVM`, MCP `vm_shutdown`/`vm_force_stop`).

use std::path::Path;
use std::sync::atomic::AtomicBool;

use rg_assets::{BootOptions, prepare_boot};
use rg_core::{Config, Registry, VmEntry, Workspace};
use rg_macos::Progress;

use crate::{Ops, Result, msg};

/// What [`Ops::create_vm`] does after adding the VM.
#[derive(Clone, Copy, Debug, Default)]
pub struct CreateOptions {
    /// Create the thin disk (GiB); `None` leaves it for later (CLI `vm create`).
    pub disk_gib: Option<u64>,
    /// Download the recovery for the VM's version.
    pub download: bool,
}

impl Ops {
    /// Go `Workspace.Prepare`: under the Workspace lock, creates the VM folder and writes its
    /// boot files (offline: no extra kext, no ReimsVgpu.kext).
    pub fn prepare_workspace(&self, c: &Config) -> Result<()> {
        let w = Workspace::new(c.workspace_dir()?);
        let _lock = w.lock()?;
        std::fs::create_dir_all(&w.dir)?;
        prepare_boot(
            &w.dir,
            c,
            &self.assets()?,
            &self.kext_cache(),
            &BootOptions {
                with_media: w.has_media(),
                keyboard: &self.keyboard,
                ..BootOptions::default()
            },
        )?;
        Ok(())
    }

    /// Go `Workspace.DownloadMedia`: holding the Workspace lock, downloads, verifies and
    /// converts the recovery for `version` into the VM folder. Any failure keeps the previous
    /// media.
    pub fn download_media(
        &self,
        c: &Config,
        version: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
        log: &mut dyn FnMut(&str),
    ) -> Result<()> {
        let w = Workspace::new(c.workspace_dir()?);
        let _lock = w.lock()?;
        std::fs::create_dir_all(&w.dir)?;
        rg_macos::download_media(&self.http, &w.dir, version, cancel, progress, log)?;
        Ok(())
    }

    /// The wizard's and `vm create`'s flow: default adapters, validation, the VM added and
    /// selected, its folder prepared, then the disk and the recovery when asked. The entry is
    /// returned once added, even if a later step fails (`Err` names that step's error; the VM
    /// stays in the list, as in the Go app).
    pub fn create_vm(
        &self,
        reg: &Registry,
        mut c: Config,
        opts: CreateOptions,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
        log: &mut dyn FnMut(&str),
    ) -> Result<VmEntry> {
        if c.networks.is_none() {
            c.networks = Some(rg_core::network::new_networks(&c.version));
        }
        c.validate()?;
        let e = reg.create(c)?;
        reg.select(&e.id)?;
        self.prepare_workspace(&e.config)?;
        if let Some(gib) = opts.disk_gib {
            let w = Workspace::new(e.config.workspace_dir()?);
            if let Some(warning) = w.create_disk(gib)? {
                log(&format!("[disk] {warning}"));
            }
        }
        if opts.download {
            self.download_media(&e.config, &e.config.version, cancel, progress, log)?;
        }
        Ok(e)
    }

    fn refuse_running(&self, c: &Config, what: String) -> Result<()> {
        match self.find_running(c) {
            Some(_) => Err(msg(what)),
            None => Ok(()),
        }
    }

    /// Go `ExportBundle`: refused while the VM runs; the copy holds the Workspace lock.
    pub fn export_bundle(
        &self,
        e: &VmEntry,
        dst: &Path,
        cancel: &AtomicBool,
        progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        self.refuse_running(
            &e.config,
            format!("shut down {} before exporting it", e.display_name()),
        )?;
        Ok(rg_core::bundle::export_bundle(e, dst, cancel, progress)?)
    }

    /// Go `DeleteVM`: with `purge`, refused while the VM runs. Returns what was deleted.
    pub fn delete_vm(&self, reg: &Registry, e: &VmEntry, purge: bool) -> Result<Vec<String>> {
        if purge {
            self.refuse_running(
                &e.config,
                "shut down the VM before deleting its files".into(),
            )?;
        }
        Ok(reg.delete(e, purge)?)
    }

    /// Go `MoveVM`: refused while the VM runs.
    pub fn move_vm(&self, reg: &Registry, e: &VmEntry, dst: &str) -> Result<VmEntry> {
        self.refuse_running(&e.config, "shut down the VM before moving it".into())?;
        Ok(reg.move_vm(e, dst)?)
    }

    /// Presses the power button of a VM started by any app instance or the CLI, through its
    /// control socket (MCP `vm_shutdown`). For a VM this instance started, use
    /// `rg_qemu::Vm::powerdown`.
    pub fn shutdown(&self, c: &Config) -> Result<()> {
        let sock = self.control_socket(c)?;
        rg_qemu::blocking::qmp(&sock, "system_powerdown", None)?;
        Ok(())
    }

    /// Ends a running VM at once through its control socket (MCP `vm_force_stop`; QEMU exits
    /// before it replies). For a VM this instance started, use `rg_qemu::Vm::kill`.
    pub fn force_stop(&self, c: &Config) -> Result<()> {
        let sock = self.control_socket(c)?;
        let _ = rg_qemu::blocking::qmp(&sock, "quit", None);
        Ok(())
    }

    fn control_socket(&self, c: &Config) -> Result<std::path::PathBuf> {
        let r = self
            .find_running(c)
            .ok_or_else(|| msg(format!("{} is not running", c.name)))?;
        r.control.ok_or_else(|| {
            msg(format!(
                "{} was not started by this app (pid {}): shut it down from macOS or where it was started",
                c.name, r.pid
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use rg_core::workspace::{EFI_DIR, IDENTITY_FILE, OVMF_CODE, VARS_FILE};

    /// Go `TestPrepareExtractsBootFiles`.
    #[test]
    fn prepare_creates_the_folder_and_extracts_boot_files() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let c = config(&t, "new");
        let w = Workspace::new(c.workspace_dir()?);
        assert!(!w.exists(), "must not exist yet");
        o.prepare_workspace(&c)?;
        for f in [
            format!("{EFI_DIR}/EFI/BOOT/BOOTx64.efi"),
            format!("{EFI_DIR}/EFI/OC/OpenCore.efi"),
            format!("{EFI_DIR}/EFI/OC/config.plist"),
            format!("{EFI_DIR}/EFI/OC/Kexts/Lilu.kext/Contents/MacOS/Lilu"),
            OVMF_CODE.into(),
            VARS_FILE.into(),
            IDENTITY_FILE.into(),
        ] {
            assert!(w.path(&f).exists(), "{f} was not extracted");
        }
        let id1 = std::fs::read(w.path(IDENTITY_FILE))?;
        std::fs::write(w.path(VARS_FILE), "modified-by-guest")?;
        o.prepare_workspace(&c)?;
        assert_eq!(
            std::fs::read(w.path(IDENTITY_FILE))?,
            id1,
            "identity made once"
        );
        assert_eq!(std::fs::read(w.path(VARS_FILE))?, b"modified-by-guest");
        Ok(())
    }

    #[test]
    fn prepare_shows_the_picker_once_media_is_there() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let c = config(&t, "m");
        let w = Workspace::new(c.workspace_dir()?);
        std::fs::create_dir_all(&w.dir)?;
        std::fs::write(w.path(rg_core::workspace::MEDIA_FILE), "")?;
        o.prepare_workspace(&c)?;
        let plist = std::fs::read_to_string(w.path(EFI_DIR).join("EFI/OC/config.plist"))?;
        assert!(plist.contains("<key>ShowPicker</key>\n\t\t\t<true/>"));
        Ok(())
    }

    #[test]
    fn prepare_and_download_wait_for_no_one_holding_the_lock() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let c = config(&t, "busy");
        let w = Workspace::new(c.workspace_dir()?);
        let _held = w.lock()?;
        let busy = |r: Result<()>| matches!(r, Err(crate::Error::Core(rg_core::Error::Busy)));
        assert!(busy(o.prepare_workspace(&c)));
        let no = AtomicBool::new(false);
        assert!(busy(o.download_media(
            &c,
            "sonoma",
            &no,
            &mut |_| {},
            &mut |_| {}
        )));
        assert!(!w.path(EFI_DIR).exists(), "nothing written while locked");
        Ok(())
    }

    #[test]
    fn create_vm_adds_selects_and_prepares() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let reg = Registry::open(&o.app)?;
        let no = AtomicBool::new(false);
        let e = o.create_vm(
            &reg,
            config(&t, "Lab"),
            CreateOptions::default(),
            &no,
            &mut |_| {},
            &mut |_| {},
        )?;
        assert_eq!(reg.selected(&reg.list()?).map(|s| s.id), Some(e.id.clone()));
        assert_eq!(
            e.config.networks.as_ref().map(Vec::len),
            Some(1),
            "default adapter"
        );
        assert!(
            Workspace::new(e.config.workspace_dir()?)
                .path(VARS_FILE)
                .exists()
        );
        let mut bad = config(&t, "Bad");
        bad.workspace = "relative".into();
        let err = o.create_vm(
            &reg,
            bad,
            CreateOptions::default(),
            &no,
            &mut |_| {},
            &mut |_| {},
        );
        assert!(
            err.is_err() && reg.list()?.len() == 1,
            "invalid configs are not added"
        );
        Ok(())
    }

    #[test]
    fn export_delete_move_refuse_a_running_vm() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let reg = Registry::open(&o.app)?;
        let e = reg.create(config(&t, "Run"))?;
        std::fs::create_dir_all(e.config.workspace_dir()?)?;
        fake_qemu(&o, &e.config, 4242)?;
        let no = AtomicBool::new(false);
        let text = |r: Result<()>| r.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            text(o.export_bundle(&e, &t.path().join("x.tar"), &no, |_, _| {}))
                .contains("before exporting")
        );
        assert!(text(o.delete_vm(&reg, &e, true).map(|_| ())).contains("before deleting"));
        assert!(text(o.move_vm(&reg, &e, "/elsewhere").map(|_| ())).contains("before moving"));
        assert!(text(o.shutdown(&e.config)).contains("not started by this app"));
        o.delete_vm(&reg, &e, false)?; // only the entry: allowed while running
        assert!(reg.list()?.is_empty());
        Ok(())
    }

    #[test]
    fn shutdown_needs_a_running_vm() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let c = config(&t, "Off");
        let err = o
            .force_stop(&c)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("is not running"), "{err}");
        Ok(())
    }
}
