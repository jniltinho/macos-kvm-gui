# Design

## Context

`internal/core` already has most of what the tools need: `VM` (start, stop, QMP events), the VNC client (`VNC.Frame`, `Key`, `Chord`, `Pointer`), `GuestKeys`, the installer cache and `CheckHost`. Missing: reaching a VM another process runs, the MCP protocol, and a few core gaps the review found.

## Decisions

1. **Transport and protocol.** stdio only, through the official Go SDK v1.8.0 (`github.com/modelcontextprotocol/go-sdk/mcp`, v1 API stable), which handles the protocol revisions it supports. stdout is reserved for MCP; the `mcp` command never reuses CLI handlers that print.
2. **Ownership.** The process that runs QEMU owns the run (QEMU has `Pdeathsig` on its thread). The GUI keeps owning what it starts. `vm_start` runs `macos-kvm-gui vm start --vm <id>` detached (`setsid`, output to the run's log), so the MCP process exiting does not stop QEMU. Attaching processes never signal QEMU: `vm_force_stop` sends QMP `quit` on the control socket of the process that has the VM's disk open, so no PID is signalled.
3. **Discovery: the QEMU process itself, no manifest.** `FindRunning` already finds the `qemu-system` process whose command line opens the VM's disk (the disk's write lock makes it unique); its `-vnc`, second `-qmp` and `id=InstallerDisc` arguments give the sockets and whether the disc is attached. Nothing is written that could go stale, and a recycled PID cannot match a VM unless that process opens its disk. Sockets are used only inside a `macos-kvm-gui-*` directory owned by the user with mode 0700 (`CheckRunDir`). The keyboard layout is the host's, as the recovery's `prev-lang:kbd` is set from it at every boot.
4. **Runtime directory.** Discovery does not depend on it (it reads the sockets from QEMU's command line), so the GUI and the MCP server find each other's runs even with different `XDG_RUNTIME_DIR` values; `CheckRunDir` applies the ownership rules.
5. **QMP.** Two monitors per run (`qmp.sock` for the owner, `qmp-mcp.sock` for one attached controller); QEMU broadcasts events to both. Core gains a QMP call that returns data (`Query`); `vm_status` reads `query-status`, and the attached disc comes from QEMU's command line, fixed for the run.
6. **Screen.** `screen_capture` connects as a shared VNC client, requests a full update and returns it only after a complete frame of the current size (not the initial blank frame), as PNG image content with `mimeType: image/png`; `max_width` (default 1280) scales it and the reply gives the scale. Guest coordinates are always used for input.
7. **Input.** All input for a run goes through one serialized queue (per process; one attached controller per run keeps processes from interleaving), so chords and text never interleave; a cancelled call releases what it pressed. `type_text` uses a per-layout character table (U.S. and ABNT2 first, from the layouts `keyboard.go` knows), with keypad keys where they are layout-independent, and never sends Return. `install_offline` keeps `GuestKeys`' restricted line.
8. **Guest readiness.** The server reports `guest_phase: unknown` and never guesses from pixels (verbose boot is text, not the Apple logo). Tool descriptions tell the agent to look at `screen_capture` and to send `install_offline` only when the recovery's window is shown, because keys during boot change how macOS boots.
9. **Consent.** `vm_force_stop` and `installer_remove` need `confirm: true`; unattended `install_offline` needs `accept_license: true` and `erase_disk: true`; all are checked before any side effect. These are guardrails for well-behaved agents, not authorization.
10. **Progress.** Sent only when the request has `_meta.progressToken`. Stages map to one increasing overall value (catalog 0–1 %, download 1–90 %, check 90–99 %, disc 99–100 %), with the stage in `message`; no notifications after the result or a cancel. Cancelling keeps the partial download and, after this change, a cancelled check or disc write too.

## Risks

- A person typing in the VM while an agent does still interleaves keys; the docs say not to.
- Agents misreading screens: the server reports only what it knows (QEMU state, attached disc, layout).
