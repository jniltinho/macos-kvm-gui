# Proposal

## Why

Installing and checking macOS in a VM means watching its screen, typing in the recovery's Terminal and clicking through the installer. Today an AI agent (Claude Code, Codex) helping a user can only do that from outside the app, with `xdotool` and window coordinates: clicks land on the wrong tab when the window is maximized, `xdotool type` produces wrong characters on non-US host layouts (ABNT2 `\`, `[`), and the app already holds the VM's only QMP connection. An MCP server built into the app gives agents the VM itself: its screen as an image, keys and pointer in guest coordinates, and the app's own actions (start, stop, offline install), so a user can ask an agent to install or check a VM and watch what it does.

## What Changes

- **`macos-kvm-gui mcp`:** a Model Context Protocol server over stdio only (no network listener), built on the official Go SDK (`github.com/modelcontextprotocol/go-sdk`, pinned to v1.8.0) and registered in an agent with one line, for example `claude mcp add macos-kvm -- macos-kvm-gui mcp`. Standard output carries only MCP messages; logs go to standard error.
- **Tools:**
  - `vm_list`, `vm_status`: the VMs and, for a running one, `qemu_state` (from QEMU), `guest_phase` (always `unknown`: the server does not interpret the guest screen), whether the offline disc was attached at startup and the keyboard layout the run assumes.
  - `vm_start` (optional `verbose_boot`), `vm_shutdown` (ACPI power button), `vm_force_stop` (needs `confirm: true`).
  - `screen_capture`: the guest screen as PNG image content (optionally scaled down) plus its size and scale; screen interpretation is the agent's job.
  - `send_keys`: key chords such as `ctrl+f2`, `return`, `kp_divide`, with pauses between events; allowed at any time, including in the boot menu.
  - `type_text`: text as physical keys under the run's keyboard layout, without pressing Return (that is a separate `send_keys`); characters the layout's table does not cover are refused before any key is sent.
  - `pointer`: move, click (left, right, double) and scroll, in guest framebuffer coordinates.
  - `installer_list`, `installer_download` (MCP progress when the request carries a progress token), `installer_remove` (needs `confirm: true`).
  - `install_offline`: does what **Type it in the VM** does. With `unattended: true` it needs both `accept_license: true` (it accepts Apple's macOS license for the user) and `erase_disk: true` (it erases the VM's empty disk, or installs over the existing "Macintosh HD"); every flag and prerequisite (offline disc attached, keyboard layout typeable) is checked before any key is sent.
  - `host_check`: the Host tab's checks.
- **Consent is the agent's and the user's responsibility:** the flags and MCP's `destructiveHint` are advisory and model-supplied; they do not prove a human approved anything, and raw `send_keys`/`pointer` can do what the guarded tools refuse. The docs say so, and say to run the server only for a user who wants an agent to control their VMs.
- **Running VMs shared with the app, owned by one process:** the process that started a VM owns it; others attach to it. Screen and input go through the run's VNC socket (shared clients), power through a second QMP socket (`qmp-mcp.sock`), so a run supports its owner plus one attached controller. A VM the MCP server starts runs under a detached owner (`macos-kvm-gui vm start`, in its own session) so that closing the agent never stops it; detaching an observer never kills QEMU, releases the owner's locks or deletes its runtime directory. The app learns to attach to and show a VM it did not start.
- **Run discovery:** a VM's run is the QEMU process that has its disk open, found in `/proc` as the app already does (`FindRunning`); its command line gives the screen and control sockets and whether the offline disc is attached. Sockets are used only inside this user's private 0700 run directories. This protects against stale or misattributed runs and other users, not against a malicious process of the same user.
- **Screen and input need the in-app display:** VMs with **Display: In a window** have no VNC socket; for them only power and status tools work, and the tools say so.
- **Out of scope:** an HTTP transport, remote hosts, recognising guest screens, automating Setup Assistant, reading the guest's file system, creating VMs or disks (the CLI does that).

## Capabilities

### New Capabilities
- `mcp-server`: VM control, screen capture and input for AI agents over MCP.

## Impact

- New `cmd/mcp.go` and `internal/mcp/`; dependency `github.com/modelcontextprotocol/go-sdk` v1.8.0.
- `internal/core/vm.go` and `running.go`: a second `-qmp` socket; `RunInfo` gains the control socket and the disc; `CheckRunDir`.
- `internal/core/qmp.go`: commands that return data (`Query`).
- `internal/core/vnc.go` and `keys.go`: a snapshot after a complete frame; key chords by name; text tables for U.S. and ABNT2.
- `internal/core/installer.go`: cancelling keeps resumable data (fixed for the check; disc writing becomes cancellable).
- `internal/gui`: attaching to and showing a VM started elsewhere.
- Docs: `docs/mcp.md`, linked from the README and the CLI page.
