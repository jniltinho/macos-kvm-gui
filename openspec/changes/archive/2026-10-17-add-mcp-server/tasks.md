# Tasks

## 1. Core
- [x] 1.1 Find runs from QEMU's command line (`FindRunning`): control socket and attached disc; `CheckRunDir` for the sockets' directory
- [x] 1.2 A second QMP socket per run (`qmp-control.sock`)
- [x] 1.3 QMP `Query` that returns data
- [x] 1.4 VNC snapshot after a complete frame; `ParseChord`; text tables for U.S. and ABNT2 (`TypeKeys`)
- [x] 1.5 Installer: a cancelled check keeps the package
- [x] 1.6 Installer: a cancelled disc write keeps the package and removes the partial disc; one download per build at a time (flock)
- [x] 1.7 Detached owner for VMs started by the MCP server (`vm start` in its own session)

## 2. MCP server
- [x] 2.1 `cmd/mcp.go` with the Go SDK v1.8.0 on stdio; stdout for MCP only
- [x] 2.2 Tools: `vm_list`, `vm_status`, `vm_start`, `vm_shutdown`, `vm_force_stop`
- [x] 2.3 Tools: `screen_capture`, `send_keys`, `type_text`, `pointer`; input serialized per screen
- [x] 2.4 Tools: `installer_list`, `installer_download` (monotonic progress), `installer_remove`, `install_offline`, `host_check`
- [x] 2.5 Consent flags checked before side effects; destructive annotations; refusal messages

## 3. GUI
- [x] 3.1 Show and control a VM started by another process (shared VNC; the list and screen follow outside runs every 3 s)

## 4. Tests and docs
- [x] 4.1 Unit: tool list and annotations, consent refusals before any action, progress mapping, key chords and text tables
- [x] 4.2 QEMU 8.2: two QMP clients (the owner and the MCP control socket) and two shared VNC clients (the app and the MCP) at once; closing the MCP process and restarting the app leave QEMU running (same PID)
- [x] 4.3 End-to-end: Big Sur 11.7.11 installed offline and unattended through the MCP server only (vm_start, screen_capture, install_offline, vm_force_stop), to Setup Assistant
- [x] 4.4 `docs/mcp.md` (setup for Claude Code and Codex, tools, what the consent flags do and do not mean), README and CLI links
