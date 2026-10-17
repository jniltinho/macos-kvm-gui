# rg-agent API

The command line and the MCP server, ported from the Go app's `cmd/` and `internal/mcp`.
Same commands, flags, output and exit status (0, or 1 with `Error: <reason>` on stderr; clap's
parse errors also exit 1), with `reims-gui` where the Go app says `macos-kvm-gui`.

## Entry points

| Item | Notes |
| --- | --- |
| `Ctx { version, app: AppDir, ops: Arc<dyn Ops> }` | What every command acts on. `Ctx::user(version, ops)` uses `~/.config/macos-kvm-gui`. Tests use `AppDir::at(tmp)`. |
| `main(args_os, &ctx, gui: &dyn Fn(Option<&Path>) -> anyhow::Result<()>) -> ExitCode` | The binary's `main`: no command or `gui` calls `gui(--config)`; `--version`; hidden `--self-test` (MCP initialize + tools/list in-process); otherwise the command. |
| `execute(args, &ctx, out, err) -> anyhow::Result<()>` | Parses `args` (no program name) and runs the command into `out`/`err` (tests). |
| `Cli` | The clap derive tree (`Cmd`, `ConfigCmd`, `VmCmd`...). |
| `serve_stdio(ctx)` | `reims-gui mcp`: rmcp 3.5.1 over stdin/stdout until the client leaves. |
| `Server::new(ctx)` | The `rmcp::ServerHandler`. |
| `exchange(ctx, messages)`, `handshake(client)`, `self_test(ctx)` | In-memory JSON-RPC session (tests, `--self-test`). |

The binary (`apps/reims-gui`) passes `AgentOps` (its `agent_ops.rs`: `Ops` on `rg_ops::Ops`,
kept there so rg-ops never depends on rg-agent) and a `run_gui` that opens `rg_ui::run` on
`RealBackend`. It handles `--demo` and `--self-test` itself: the self-test runs the MCP
initialize + tools/list and then the GUI for a few frames, both on a temporary config folder and
Workspace.

## `Ops`: what rg-ops provides

```rust
pub trait Ops: Send + Sync {
    fn prepare_workspace(&self, c: &Config) -> anyhow::Result<()>;          // Go Workspace.Prepare
    fn download_media(&self, c: &Config, version: &str, cancel: &AtomicBool,
        progress: &mut dyn FnMut(rg_macos::Progress), log: &mut dyn FnMut(&str)) -> anyhow::Result<()>; // Go Workspace.DownloadMedia
    fn start_vm(&self, c: Config, on_event: Box<dyn Fn(VmEvent) + Send + Sync>)
        -> anyhow::Result<Box<dyn RunningVm>>;                              // Go VM.Start
}
pub trait RunningVm: Send {
    fn vnc_socket(&self) -> Option<PathBuf>;
    fn installer_command(&self) -> Option<String>;   // Go VM.InstallerCommand
    fn powerdown(&self) -> anyhow::Result<()>;
    fn kill(&self) -> anyhow::Result<()>;
}
```

- `prepare_workspace`: under the VM folder's lock, create it and write the boot files
  (`workspace create`, `vm create`).
- `download_media`: lock, free-space check, recovery download, verification, conversion
  (`download` without `--full`).
- `start_vm`: everything Go's `VM.Start` does. `c.extras` is the start's settings.yaml
  snapshot (already downloaded, as is ReimsVgpu.kext for Tahoe on Reims) and `c.verbose_boot`
  the one-off flag. It returns once QEMU runs; `VmEvent::State(VmState::Stopped, msg)` ends
  `vm start` (a non-empty `msg` before any Ctrl+C is the command's error).

Everything else is implemented here on rg-core, rg-host, rg-macos, rg-qemu, rg-reims and
rg-assets, including the running checks of `vm delete --purge`, `vm move` and
`vm export --full` (`find_running`, Go's messages), and the kext downloads before a start.

## CLI

`check`, `mcp`, `setup`, `config show|set|recommend` (keys `workspace version display gpu
graphics resolution ram cores vcpus ssh-port root-patch nicN.enabled|mode|bridge|model|mac`,
`--root-patch-reverted`), `workspace show|set|create`, `download [--version] [--full]`,
`installer list|remove`, `disk create [--size]`, `media remove --yes`, `vm list|status|create|
select|move|export|import|delete|start` (`--display`, `-v/--verbose`, `--verbose-boot`, hidden
`--kexts-snapshot`), `gpu list|reserve|release`, `settings show|reset --yes`,
`reims install|build|status`, `version`, `gui`; global `--config <file.json>` and `--vm <name>`.
Tables are laid out like Go's tabwriter (padding 2).

`vm start`: Ctrl+C (or SIGTERM) presses the power button, a second one kills QEMU.

## MCP

Tools, descriptions, input/output schemas (Go jsonschema shape: required = fields without
`omitempty`, `additionalProperties: false`, slices `["null","array"]`), annotations
(`readOnlyHint` on vm_list, vm_status, screen_capture, installer_list, host_check;
`destructiveHint` on vm_force_stop, installer_remove, install_offline) and consent checks
(`confirm`, `accept_license`, `erase_disk`) are the Go server's. Typed results
(vm_list, vm_status, installer_list, host_check) carry `structuredContent` plus its JSON as
text; failures are tool errors (`isError`) with the Go messages.

- `vm_start` downloads settings.yaml's kexts (and ReimsVgpu.kext) first, then spawns
  `reims-gui vm start --vm <id> --display app --kexts-snapshot <json>` in its own process group,
  output in `<folder>/macos-kvm-gui/mcp-start.log`, and waits up to 30 s for the VNC socket.
- Screen and input go over the VM's control socket (QMP), one lock per VM:
  `screen_capture` = `screendump` (PNG into the private run directory, scaled to `max_width`,
  default 1280); `send_keys`/`type_text`/`install_offline` = `input-send-event` with scancodes
  (`number` keys; chords down in order, up in reverse; 80 ms between chords, 30 ms while
  typing); `pointer` = absolute tablet position plus `btn` events (the screen size comes from a
  screendump).
- `installer_download` sends `notifications/progress` (0–100, only growing, at most once a
  second) when the request has a progress token; cancelling the request stops the download.

## Deviations from Go

- Input uses QMP instead of a VNC connection, so `screen_available` means "has a control
  socket" and a VM in QEMU's own window works too.
- `vm_start` uses a new process group, not `setsid` (no `unsafe`).
- `setup` takes the user name from `$USER` (or `$LOGNAME`).
