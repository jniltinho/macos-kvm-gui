//! The VM start pipeline: Go `VM.Start` (vm.go) with the GUI's and CLI's `startWith` (the kext
//! downloads before it).

use std::sync::atomic::AtomicBool;

use rg_assets::{
    BootOptions, clear_root_patch_marker, download_extra_kexts, download_reims_kext,
    download_root_patch_kexts, extra_kexts_ready, prepare_boot, reims_kext_ready,
    root_patch_kexts_ready,
};
use rg_core::{Config, Workspace};
use rg_qemu::{StartSpec, Started, Vm, VmEvent, VmState};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::{Ops, Result, msg};

/// A started VM: the controller, its sockets and pid, and its events.
pub struct Running {
    /// Shut down with `vm.powerdown().await`, force stop with `vm.kill()`.
    pub vm: Vm,
    /// pid (the GUI finds QEMU's window by `_NET_WM_PID`), QMP and VNC sockets.
    pub started: Started,
    /// State changes and log lines until the final `State(Stopped, message)`.
    // ponytail: unbounded; QEMU's output rate bounds it, and a dropped Stopped would be worse.
    pub events: UnboundedReceiver<VmEvent>,
    /// The line to type in the recovery's Terminal, when the offline installer disc is attached.
    pub installer_command: Option<&'static str>,
}

impl Running {
    /// Blocks until QEMU exits (the CLI's foreground `vm start`; not inside a tokio runtime).
    /// `each` sees every event; returns the exit message ("" for a clean exit).
    pub fn wait_blocking(&mut self, mut each: impl FnMut(&VmEvent)) -> String {
        while let Some(ev) = self.events.blocking_recv() {
            each(&ev);
            if let VmEvent::State(VmState::Stopped, m) = ev {
                return m;
            }
        }
        String::new()
    }
}

/// Where the VM's screen goes, given the app embeds windows over X11 (design D3).
fn display(c: &Config, x11_embed: bool) -> (bool, bool) {
    // (SDL_VIDEO_DRIVER=x11 for the Reims window to embed; QEMU's own GTK window for VMware
    // SVGA in the app. Not SDL: its display withdraws the window without text consoles.)
    (
        x11_embed && c.reims(),
        x11_embed && !c.reims() && c.in_app(),
    )
}

impl Ops {
    /// Starts `c` (Go `startWith` + `VM.Start`): validates it, takes the settings.yaml kexts
    /// snapshot (`c.extras`, else loaded now), downloads the root-patch kexts, ReimsVgpu.kext
    /// and the extra kexts when missing, checks the folder, disk, running QEMU, bridges, SSH
    /// port, GPU and Reims vGPU (warnings to `log`), takes the Workspace lock, writes the boot
    /// files with the snapshot, attaches the cached installer disc with the recovery, and
    /// starts QEMU on `rt`. When QEMU exits the root-patch marker is cleared (after 60 s), the
    /// disc released and the lock dropped. `x11_embed`: the app embeds QEMU's window (SDL with
    /// `SDL_VIDEO_DRIVER=x11`). Blocking (downloads); call it off the UI thread.
    pub fn start_vm(
        &self,
        rt: Handle,
        mut c: Config,
        x11_embed: bool,
        cancel: &AtomicBool,
        log: &mut dyn FnMut(&str),
    ) -> Result<Running> {
        c.validate()?;
        let ks = match c.extras.take() {
            Some(ks) => ks,
            None => self.app.load_extra_kexts()?,
        };
        let cache = self.kext_cache();
        if c.root_patch && !root_patch_kexts_ready(&cache) {
            log("[VM] downloading OCLP's kexts for root-patch mode");
            download_root_patch_kexts(&cache, cancel)?;
        }
        if c.reims_kext() && !reims_kext_ready(&cache) {
            log("[VM] downloading ReimsVgpu.kext (Reims vGPU on Tahoe)");
            download_reims_kext(&cache, cancel)?;
        }
        if !extra_kexts_ready(&ks, &cache) {
            log(&format!(
                "[VM] downloading the kexts listed in {}",
                self.app.settings_file().display()
            ));
            download_extra_kexts(&ks, &cache, cancel)?;
        }

        let dir = c.workspace_dir()?;
        let w = Workspace::new(&dir);
        if !w.exists() {
            return Err(msg(format!(
                "VM folder {} does not exist: create it in the Install tab",
                dir.display()
            )));
        }
        if !w.has_disk() {
            return Err(msg(format!(
                "{} does not exist: create the disk in the Install tab",
                rg_core::workspace::DISK_FILE
            )));
        }
        if let Some(r) = self.find_running(&c) {
            let who = if r.by_app {
                "this app"
            } else {
                "another program (for example OSX-KVM's OpenCore-Boot.sh)"
            };
            return Err(msg(format!(
                "this VM's disk is already in use by QEMU (pid {}), started by {who}: shut that VM down first",
                r.pid
            )));
        }
        rg_core::network::check_bridges(
            &c,
            &self.net_root,
            &self.bridge_acl,
            rg_core::network::bridge_helper(),
        )?;
        rg_core::network::check_ssh_port(&c)?;
        let gpu_funcs = if c.gpu.is_empty() {
            Vec::new()
        } else {
            self.host
                .gpu_ready(&c, ks.entries.len(), rg_qemu::memlock_limit())?
        };
        let qemu = if c.reims() {
            let r = self.host.reims_ready(&c)?;
            for w in &r.warnings {
                log(&format!("warning: Reims vGPU: {w}"));
            }
            r.qemu
        } else {
            self.qemu.clone()
        };

        let lock = w.lock()?;
        let with_media = w.has_media();
        prepare_boot(
            &dir,
            &c,
            &self.assets()?,
            &cache,
            &BootOptions {
                with_media,
                verbose_boot: c.verbose_boot,
                extras: Some(&ks),
                keyboard: &self.keyboard,
            },
        )
        .map_err(|e| msg(format!("could not prepare the boot files: {e}")))?;
        if c.verbose_boot {
            log(
                "[VM] verbose boot: macOS prints its boot log on screen (boot-args -v debug=0x100)",
            );
        }
        // The disc is held (a shared lock) for the whole run, so nothing deletes it meanwhile.
        let disc = with_media
            .then(|| rg_macos::cached_installer_for(&self.installers_dir(), &c.version))
            .flatten();
        let lease = disc.as_ref().and_then(|ci| {
            if let Err(e) = rg_macos::refresh_installer_disc(ci) {
                log(&format!("[installer] {e}"));
            }
            rg_macos::use_disc(&ci.path).ok()
        });
        let installer_command = lease.is_some().then_some(rg_macos::DISC_COMMAND);

        let (tx, events) = unbounded_channel();
        let vm = Vm::new(rt, move |ev| {
            let _ = tx.send(ev);
        });
        // QEMU's GTK window needs qemu-system-gui; without it SVGA uses the VNC screen.
        let (x11, sdl) = display(&c, x11_embed && (c.reims() || rg_qemu::gtk_display(&qemu)));
        let exit_c = c.clone();
        let started = vm.start(StartSpec {
            qemu,
            with_media,
            installer_disc: disc.map(|d| d.path),
            gpu_funcs,
            x11_embed: x11,
            qemu_window: sdl,
            on_exit: Some(Box::new(move |ran| {
                clear_root_patch_marker(&dir, &exit_c, ran);
                drop(lease);
                drop(lock);
            })),
            config: c,
        })?;
        Ok(Running {
            vm,
            started,
            events,
            installer_command,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use std::os::unix::fs::PermissionsExt;

    fn text<T>(r: Result<T>) -> String {
        r.err().map(|e| e.to_string()).unwrap_or_default()
    }

    /// A VM folder with an (empty) disk, prepared.
    fn ready(
        t: &tempfile::TempDir,
        o: &Ops,
        name: &str,
    ) -> std::result::Result<Config, Box<dyn std::error::Error>> {
        let c = config(t, name);
        o.prepare_workspace(&c)?;
        std::fs::write(c.workspace_dir()?.join(rg_core::workspace::DISK_FILE), "")?;
        Ok(c)
    }

    /// A stand-in for QEMU that records its arguments and environment, then exits; `gtk`:
    /// whether its `-display help` lists GTK (qemu-system-gui installed).
    fn fake_qemu_bin(t: &tempfile::TempDir, gtk: bool) -> std::io::Result<std::path::PathBuf> {
        let p = t
            .path()
            .join(if gtk { "fake-qemu" } else { "fake-qemu-nogtk" });
        let displays = if gtk {
            "none\\ngtk\\ncurses"
        } else {
            "none\\ncurses"
        };
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nif [ \"$1\" = -display ] && [ \"$2\" = help ]; then printf '{displays}\\n'; exit 0; fi\n\
                 printf '%s\\n' \"$@\" > args.txt\necho \"SDL=$SDL_VIDEO_DRIVER\" > env.txt\n"
            ),
        )?;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
        Ok(p)
    }

    #[test]
    fn svga_without_qemu_system_gui_uses_the_vnc_screen() -> TestResult {
        let t = tempfile::tempdir()?;
        let mut o = ops(&t)?;
        let rt = runtime()?;
        let no = AtomicBool::new(false);
        let c = ready(&t, &o, "nogtk")?;
        let dir = c.workspace_dir()?;
        o.qemu = fake_qemu_bin(&t, false)?;
        let mut run = None;
        for _ in 0..5 {
            match o.start_vm(rt.handle().clone(), c.clone(), true, &no, &mut |_| {}) {
                Err(e) if e.to_string().contains("Text file busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                r => {
                    run = Some(r?);
                    break;
                }
            }
        }
        let mut run = run.ok_or("could not exec the fake QEMU")?;
        run.wait_blocking(|_| {});
        let args = std::fs::read_to_string(dir.join("args.txt"))?;
        assert!(
            !args.contains("show-menubar=off") && args.contains("-vnc"),
            "no GTK display: the VNC screen, not QEMU's window: {args}"
        );
        Ok(())
    }

    fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
    }

    #[test]
    fn display_embeds_qemu_gtk_for_svga_and_reims_window_over_x11() {
        let mut c = Config::defaults();
        assert_eq!(display(&c, false), (false, false));
        assert_eq!(
            display(&c, true),
            (false, true),
            "SVGA in the app: QEMU's GTK window"
        );
        c.display = rg_core::config::DISPLAY_WINDOW.into();
        assert_eq!(
            display(&c, true),
            (false, false),
            "its own window stays standalone"
        );
        c.graphics = rg_core::config::GRAPHICS_REIMS.into();
        assert_eq!(display(&c, true), (true, false));
    }

    /// Go `TestStartRefusesWithoutDisk`, and the other checks before the lock.
    #[test]
    fn start_refuses_invalid_missing_and_running() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let rt = runtime()?;
        let no = AtomicBool::new(false);
        let mut log = |_: &str| {};
        let mut c = config(&t, "v");
        c.ram_mib = 1024;
        assert!(text(o.start_vm(rt.handle().clone(), c, false, &no, &mut log)).contains("ram"));
        let c = config(&t, "v");
        assert!(
            text(o.start_vm(rt.handle().clone(), c.clone(), false, &no, &mut log))
                .contains("does not exist")
        );
        o.prepare_workspace(&c)?;
        assert!(
            text(o.start_vm(rt.handle().clone(), c.clone(), false, &no, &mut log))
                .contains("mac_hdd_ng.img")
        );
        std::fs::write(c.workspace_dir()?.join(rg_core::workspace::DISK_FILE), "")?;
        fake_qemu(&o, &c, 77)?;
        assert!(text(o.start_vm(rt.handle().clone(), c, false, &no, &mut log)).contains("pid 77"));
        Ok(())
    }

    #[test]
    fn start_refuses_a_bad_settings_yaml_and_a_missing_bridge() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let rt = runtime()?;
        let no = AtomicBool::new(false);
        let c = ready(&t, &o, "b")?;
        std::fs::write(o.app.settings_file(), "kexts:\n  - url: http://x/a.zip\n")?;
        let err = text(o.start_vm(rt.handle().clone(), c.clone(), false, &no, &mut |_| {}));
        assert!(err.contains("https://"), "{err}");
        std::fs::write(o.app.settings_file(), "")?;
        let mut c = c;
        c.networks = Some(vec![rg_core::Nic {
            enabled: true,
            mode: rg_core::network::NET_BRIDGED.into(),
            bridge: "br9".into(),
            model: "virtio-net-pci".into(),
            mac: "52:54:00:12:34:56".into(),
        }]);
        let err = text(o.start_vm(rt.handle().clone(), c, false, &no, &mut |_| {}));
        assert!(err.contains("br9"), "{err}");
        Ok(())
    }

    #[test]
    fn start_refuses_the_ssh_port_in_use_and_a_locked_workspace() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let rt = runtime()?;
        let no = AtomicBool::new(false);
        let mut c = ready(&t, &o, "p")?;
        let taken = std::net::TcpListener::bind("127.0.0.1:0")?;
        c.ssh_port = taken.local_addr()?.port();
        let err = o.start_vm(rt.handle().clone(), c.clone(), false, &no, &mut |_| {});
        assert!(matches!(
            err,
            Err(crate::Error::Core(rg_core::Error::PortInUse(_)))
        ));
        drop(taken);
        let _held = Workspace::new(c.workspace_dir()?).lock()?;
        let err = o.start_vm(rt.handle().clone(), c, false, &no, &mut |_| {});
        assert!(matches!(err, Err(crate::Error::Core(rg_core::Error::Busy))));
        Ok(())
    }

    /// The whole pipeline with a fake QEMU: boot files with the start's snapshot, arguments
    /// and environment, events, and the lock released on exit.
    #[test]
    fn start_runs_qemu_with_the_snapshot_and_releases_the_lock() -> TestResult {
        let t = tempfile::tempdir()?;
        let mut o = ops(&t)?;
        let rt = runtime()?;
        let no = AtomicBool::new(false);
        let mut c = ready(&t, &o, "run")?;
        let dir = c.workspace_dir()?;
        std::fs::write(dir.join(rg_core::workspace::MEDIA_FILE), "")?;
        c.verbose_boot = true;
        c.extras = Some(rg_core::ExtraKexts {
            disabled: vec!["WhateverGreen".into()],
            ..Default::default()
        });
        o.qemu = fake_qemu_bin(&t, true)?;
        let mut lines = Vec::new();
        // Exec of a just-written script can race another test's fork (ETXTBSY): retry.
        let mut run = None;
        for _ in 0..5 {
            match o.start_vm(rt.handle().clone(), c.clone(), true, &no, &mut |l| {
                lines.push(l.to_owned());
            }) {
                Err(e) if e.to_string().contains("Text file busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                r => {
                    run = Some(r?);
                    break;
                }
            }
        }
        let mut run = run.ok_or("could not exec the fake QEMU")?;
        assert!(run.started.pid > 0 && run.started.vnc_socket.is_some());
        assert!(run.installer_command.is_none(), "no cached installer");
        let mut seen = Vec::new();
        let exit = run.wait_blocking(|e| seen.push(e.clone()));
        assert_eq!(exit, "", "a clean exit has no message");
        assert!(seen.contains(&VmEvent::State(VmState::Running, String::new())));
        assert!(lines.iter().any(|l| l.contains("verbose boot")));
        let args = std::fs::read_to_string(dir.join("args.txt"))?;
        assert!(
            args.contains("BaseSystem.img") && args.contains("show-menubar=off"),
            "VMware SVGA in the app gets QEMU's GTK window to embed: {args}"
        );
        assert_eq!(std::fs::read_to_string(dir.join("env.txt"))?, "SDL=\n");
        let plist = std::fs::read_to_string(
            dir.join(rg_core::workspace::EFI_DIR)
                .join("EFI/OC/config.plist"),
        )?;
        assert!(plist.contains("-v debug=0x100"), "the start's boot options");
        let weg = plist
            .find("<string>WhateverGreen.kext</string>\n\t\t\t\t<key>Comment</key>")
            .and_then(|i| plist.get(i..i + 200))
            .unwrap_or_default();
        assert!(
            weg.contains("<false/>"),
            "the snapshot's disable list: {weg}"
        );
        // The supervisor drops the lock right after the Stopped event is sent.
        for _ in 0..100 {
            if Workspace::new(&dir).lock().is_ok() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Err("the Workspace lock was not released".into())
    }
}
