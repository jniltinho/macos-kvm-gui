# rg-qemu API

QEMU for macOS VMs: command lines, the process, running-VM detection, QMP, the VNC (RFB)
client and key mapping. Port of the Go app's `internal/core` `vm.go`, `running.go`, `qmp.go`,
`vnc.go`, `keys.go` and `network.go` `netArgs`, plus reims-ctl's `qmp.rs` and `view/keys.rs`.
Async APIs are tokio; `blocking` wraps them for the CLI. Errors: `rg_qemu::Error` (`Io`,
`QmpClosed`, `Timeout`, `Qemu { cmd, desc }`, `Msg`), messages as the Go app words them.

## Command line — `args`

```rust
let env = ArgsEnv { with_media, installer_disc, gpu_funcs, gtk_window, ..ArgsEnv::host(dir, qmp_sock) };
let args: Vec<String> = build_args(&config, &env);
```

- `ArgsEnv` holds everything that is not the `Config`, including host lookups (`bridge_helper`,
  `audio`) so the output is reproducible. `ArgsEnv::host` fills those from the host.
- Defaults give the Go app's arguments byte for byte (`testdata/*.golden`, checked by
  `tests/golden.rs`): VMware SVGA, and Reims vGPU (`apple-gfx-pci`, memfd RAM, no S3, audio).
- `gtk_window: true` (SVGA only): `-display gtk,show-menubar=off,show-tabs=off` plus the VNC
  socket, for the GUI to embed QEMU's own window.
- `net_args(&config, helper)`, `qemu_path(p)` (comma escaping), `vnc_socket(qmp)`,
  `control_socket(qmp)`, `OSK`.

## Running a VM — `vm`

```rust
let vm = Vm::new(runtime_handle, |ev: VmEvent| { /* State(VmState, msg) | Log(line) */ });
let started: Started = vm.start(StartSpec { config, qemu, x11_embed, on_exit: Some(Box::new(|ran| unlock())), .. })?;
// started.pid, started.qmp_socket, started.vnc_socket
vm.powerdown().await?;   // ACPI power button over QMP
vm.kill()?;              // force stop
vm.state(); vm.pid(); vm.vnc_socket(); vm.qmp(); vm.disc_attached();
```

- `start` is blocking but short (it returns once QEMU runs); call it off the UI thread.
- The caller does the checks and preparation that need other crates (validation, Workspace
  lock, boot files, bridges, SSH port, GPU/Reims host checks, installer disc) and passes
  `on_exit` for the cleanup after QEMU exits. `start` itself refuses a running VM, a missing
  folder or disk, and a disk already used by another QEMU (`find_running`).
- Run directory: a private 0700 `macos-kvm-gui-*` under `$XDG_RUNTIME_DIR` (else the temp
  dir) with `qmp.sock`, `qmp-control.sock` and `vnc.sock`: the Go app's layout, so either app
  attaches to the other's VMs.
- Environment: Reims VMs lose `REIMS_VGPU_WINDOW` (the library opens its window); with
  `x11_embed` they get `SDL_VIDEO_DRIVER=x11` (design D3); otherwise SDL chooses.
- QEMU must not outlive the app. `unsafe` is forbidden, and both `prctl(PR_SET_PDEATHSIG)`
  via rustix and std's `pre_exec` would need it in the child, so QEMU is started through
  `setpriv --pdeathsig TERM --` (util-linux), which execs QEMU (same pid and comm). The spawn
  and the wait run on one supervisor thread that lives until QEMU exits (the death signal
  follows the forking thread). Without setpriv a warning is logged. tokio waits on a pidfd (no
  polling); QEMU has its own process group (Ctrl+C never reaches it).
- Output: stdout and stderr in one pipe, each line a `VmEvent::Log`; the last 20 go to
  `explain_exit(err, lines)`, the Go app's messages (KVM busy, disk locked, SSH port, bridge,
  udmabuf, /dev/kvm, generic with the last non-warning line).
- QMP events: `BLOCK_IO_ERROR` (action=stop) and `STOP` → `Paused` with the cause, `RESUME` →
  `Running`; `SHUTDOWN`/`POWERDOWN`/`RESET` are logged.
- `memlock_limit()`: soft RLIMIT_MEMLOCK (rustix), `u64::MAX` when unlimited.

## Running-VM detection — `running`

`find_running(&config) -> Option<RunInfo>` scans `/proc/<pid>/{comm,cmdline,cwd}` for a
`qemu-system*` that opens the VM's disk (absolute, `,,`-escaped, or relative to its cwd as
OSX-KVM's script does). `RunInfo { pid, started, by_app, vnc_socket, control, disc, reims }`,
`uptime()`. `check_run_dir(sock)` accepts only sockets of this user in a 0700 run directory.

## QMP — `qmp`

```rust
let (q, mut events) = Qmp::dial(&sock).await?;      // retries 5 s while QEMU creates it
q.execute("query-status", None).await?;              // 5 s timeout per command
q.system_powerdown().await?; q.quit().await?; q.status().await?;
q.screendump(path, None).await?; q.input_send_event(&qcode::tap(Mods::CMD, "q")).await?;
while let Some(QmpEvent { name, data }) = events.recv().await {}  // None = disconnected
```

Commands are serialized; a late reply of a timed-out command is dropped.

## VNC — `vnc`

```rust
let (v, reader) = Vnc::dial(&vnc_sock).await?;
let reader = reader.with_pace(|w, h| interval_for(w, h));   // optional back-pressure
tokio::spawn(reader.run(|u: &Update| { /* u.dirty: Vec<Rect>, u.resized */ ctx.request_repaint(); }));
let mut gen = v.updates();            // watch: changes after every update (latest frame only)
v.with_frame(|f: &Frame| upload(f, &dirty));   // or v.frame() / v.frame_into(&mut buf)
v.key(true, k).await?; v.chord(&keys).await?; v.pointer(BUTTON_LEFT, x, y).await?;
let frame = Vnc::capture(&vnc_sock, Duration::from_secs(10)).await?;  // a complete frame
```

- RFB 3.8, security None only (private socket), Raw, CopyRect, DesktopSize, QEMU extended key
  events (scancodes, so the guest's layout applies; plain keysyms when the server lacks them).
- `Frame { width, height, pixels }` is RGBA with alpha 255. The wire format is BGRX (QEMU's
  own surface format, so the server only copies); `bgrx_to_rgba` converts a 4K frame in well
  under 20 ms in release (a test checks it). Raw rectangles go through one reused buffer.
- `Update::dirty` lists the changed rectangles for `TextureHandle::set_partial`; after a resize
  the frame is blank and the next request is a full one.
- `snapshot(&mut updates, &mut run_future, timeout)` for a caller that already runs a reader.

## Keys — `keys`, `qcode`

- `KeyCode { keysym, scancode }`; `key_for("A" | "Return" | "LeftSuper" | ...)`,
  `key_for_x11(keycode)`, `key_for_evdev(code)` (layout independent, ISO/ABNT2 keys included).
- `guest_keys(line)`, `recovery_terminal()`, `parse_chord("shift+cmd+t")`,
  `type_keys(text, layout)` (`LAYOUT_US`, `LAYOUT_ABNT2`; the host layout id comes from
  rg-core's keyboard code), `fit_rect`, `to_frame` (letterbox mapping for the Screen tab).
- `qcode`: QMP `input-send-event` events (`Ev`, `Mods`, `tap`, `char_key`, `abs`), from
  reims-ctl.

## Blocking — `blocking`

`qmp(socket, cmd, args)`, `capture(vnc_socket, timeout)`, `send_chords(vnc_socket, chords,
pause)`: each checks the run directory and runs on its own current-thread runtime. Not for use
inside a tokio runtime.

## Integration notes

`compat.rs` holds what belongs to crates written in parallel (workspace file names, version
CPU profiles, `Config` adapters and workspace dir, bridge helper, audio backend, udmabuf hint);
move them to rg-core/rg-host and call them from here.
