# rg-ops API

Layer 3 (design D1): the orchestration that spans crates, so lower crates never call upward.
Port of the Go app's `Workspace.Prepare`/`DownloadMedia` (workspace.go), `VM.Start` (vm.go)
with the GUI's and CLI's `startWith`, the running checks of `bundle.go`/`vms.go`, the MCP
server's shutdown/force stop and `install_offline`, and the wizard / `vm create` flow.

Blocking API (downloads, file copies): async callers use `tokio::task::spawn_blocking`. The
async part is a started VM's event stream (`tokio` mpsc). Long calls take `cancel: &AtomicBool`.
Errors: `rg_ops::Error` wraps each crate's error (`Core` — including `rg_core::Error::Busy` and
`PortInUse` —, `Assets`, `Host`, `Macos`, `Qemu`, `Reims`, `Io`) plus `Msg`, with the Go app's
messages.

## Ops

```rust
let reg = Registry::open(&AppDir::user().unwrap())?;
let ops = Ops::user(&reg)?;            // or Ops::new(app_dir, &reg.workspace_root())
```

`Ops` (Clone) holds what the operations read from settings and the host, once:
`app: AppDir`, `workspace_root` (absolute), `assets_dir` (`MKG_ASSETS_DIR`), `keyboard`
(`recovery_keyboard()`), `http: rg_macos::Http`, `qemu` (SVGA QEMU, `qemu-system-x86_64`),
`host: rg_host::Host` with `reims_qemu` wired to `rg_reims::Reims::new(<Workspace>/.reims).qemu()`
and `extra_kexts` to `app`'s settings.yaml, and `proc_root`, `net_root`, `bridge_acl` (tests use
fake trees). Helpers: `kext_cache()`, `installers_dir()`, `reims()`, `find_running(&c)`.

## Workspace and VMs

| Item | Go |
| --- | --- |
| `prepare_workspace(&c)` — Workspace lock, folder created, boot files (offline: no extra kexts) | `Workspace.Prepare` |
| `download_media(&c, version, cancel, progress, log)` — holds the lock around `rg_macos::download_media` | `Workspace.DownloadMedia` |
| `create_vm(&reg, c, CreateOptions { disk_gib, download }, cancel, progress, log) -> VmEntry` — default adapters, validate, add, select, prepare, then disk and recovery when asked | wizard `createVM`, CLI `vm create` |
| `export_bundle(&e, dst, cancel, progress)` — refused while running ("shut down X before exporting it") | `ExportBundle` |
| `delete_vm(&reg, &e, purge)` — purge refused while running | `DeleteVM` |
| `move_vm(&reg, &e, dst)` — refused while running | `MoveVM` |
| `shutdown(&c)`, `force_stop(&c)` — `system_powerdown` / `quit` over the running VM's control socket (any instance) | MCP `vm_shutdown`, `vm_force_stop` |

Importing (`Registry::import_bundle`, `import_settings`) has no running check in Go and is
called on rg-core directly.

## Starting a VM

```rust
let mut c = entry.config.clone();
c.verbose_boot = verbose;          // optional
c.extras = Some(snapshot);         // optional: the MCP server's snapshot; else loaded now
let mut run = ops.start_vm(rt.handle().clone(), c, x11_embed, &cancel, &mut |l| log(l))?;
run.started.pid; run.started.qmp_socket; run.started.vnc_socket;
run.installer_command;             // Some(DISC_COMMAND) with the offline disc attached
while let Some(ev) = run.events.recv().await { /* VmEvent::State(..) | VmEvent::Log(..) */ }
run.vm.powerdown().await?; run.vm.kill()?;   // this instance's VM
let msg = run.wait_blocking(|ev| print(ev)); // CLI foreground
```

`start_vm` (blocking; downloads):
1. `c.validate()`; the settings.yaml kexts snapshot (`c.extras` or loaded now).
2. Downloads when missing: OCLP root-patch kexts (`c.root_patch`), ReimsVgpu.kext
   (`c.reims_kext()`), the snapshot's extra kexts.
3. VM folder exists, disk exists, no QEMU uses the disk (`find_running`), bridges
   (`check_bridges`), SSH port free (`check_ssh_port`), GPU (`Host::gpu_ready` with the
   snapshot's kext count and RLIMIT_MEMLOCK), Reims (`Host::reims_ready`; warnings to `log` as
   `warning: Reims vGPU: ...`, its QEMU used).
4. Workspace lock; boot files with the snapshot, verbose boot and the media flag
   ("could not prepare the boot files: ...").
5. With the recovery present, the cached installer disc for the version is refreshed, leased
   (shared lock) and attached.
6. `rg_qemu::Vm::start` on `rt`. `x11_embed` = the app embeds QEMU's window over X11 (design
   D3): Reims VMs get `SDL_VIDEO_DRIVER=x11`; in-app VMware SVGA VMs get `-display sdl` with
   `SDL_VIDEO_DRIVER=x11` (the VNC socket stays for the Preview and MCP); `display=window`
   VMs keep their own window.
7. On QEMU's exit: the root-patch marker is cleared (after 60 s with the mode off), the disc
   lease and the Workspace lock are dropped. A failed start drops them too.

## Offline install

- `install_line(unattended) -> String`: what the user types in the recovery's Terminal.
- `ops.install_offline(&c, unattended, accept_license, erase_disk)`: refuses the unattended
  install without both consents, a layout that moves letters (Dvorak, Russian), a VM not
  running or without the disc; then opens Utilities › Terminal (`recovery_terminal()` chords,
  120 ms apart), waits 2 s and types `DISC_TYPED` (plus ` -u`) over the VM's VNC socket.
  Blocking; not inside a tokio runtime.
