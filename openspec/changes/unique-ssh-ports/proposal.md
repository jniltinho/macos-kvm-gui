# Proposal

## Why

Every new VM gets SSH port 2222 (`DefaultConfig` in Go, `Config::default` in Rust), whether it comes from the New VM wizard, `vm create`, the MCP server's `vm_create` or an import. So the second VM always collides with the first. Two VMs cannot run together until someone changes one of them by hand, and SSH-based tooling (install scripts, agents) may reach the wrong guest. It happened today: "Nilton" and "sonoma-test" both use 2222. The port can already be changed (Settings › Network, `config set ssh-port=`), but nothing helps the user pick a free one or tells them two VMs share a port, and agents using the MCP server cannot create VMs or change their settings at all.

## What Changes

- **A free port for every new VM.** Creating or importing a VM gives it the lowest port from 2222 up that no other VM in the VM list uses and that is free on 127.0.0.1. This applies to the New VM wizard, `vm create`, MCP `vm_create`, `vm import` and the GUI's Import, in both apps.
  - The wizard pre-fills that port, and the user can still change it.
  - If the requested port belongs to another VM, the new VM gets the next free one and the app says so (log line, CLI and MCP output).
- **Warnings on edits.** Settings › Network and `config set ssh-port=` warn when another VM already uses the chosen port, naming it. Saving is still allowed: the two may never run at the same time.
- **Details shows the warning too** while two VMs share a port, so existing duplicates surface without renumbering anything.
- **"Use a free port" button.** Settings › Network gets the button next to the SSH port field in both apps; it fills in `NextSSHPort`.
- **MCP settings tools**, in both apps. Today an agent can start, stop and drive VMs but cannot create one or change its settings.
  - `vm_config_get` reads a VM's settings.
  - `vm_config_set` changes them with the CLI's `config set` keys and validation, `ssh-port` included; it refuses while the VM runs and returns warnings.
  - `vm_create` creates a VM with a free SSH port.
  - `vm_ssh` runs one command in a running VM's macOS over its SSH forward and returns stdout, stderr and the exit code.
    - Authentication is by the user's SSH keys or agent, or a password passed in the call; the password is never logged, stored or put on a command line.
    - It needs `confirm: true` and is marked destructive.
- **Unchanged:**
  - Existing VMs keep their ports; nothing is renumbered automatically.
  - Starting a VM whose port is taken still fails with today's clear "SSH port … is in use" message.
  - The port range (1024–65535) and the SSH forward itself.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `networking`: new requirements for a unique SSH port per VM at creation and import, and warnings when ports are shared. The existing SSH forward requirement is unchanged.
- `mcp-server`: new `vm_config_get`, `vm_config_set`, `vm_create` and `vm_ssh` tools.

## Impact

- **Go (macos-kvm-gui):**
  - `internal/core/vms.go`: `CreateVM`, `insertVM`, a new `NextSSHPort` helper and a `SSHPortUsers` lookup;
  - `internal/gui/wizard.go` and `settings.go` / `network.go`: the pre-fill and the warning;
  - `internal/gui/layout.go`: the Details warning;
  - `cmd/vms.go` and `cmd/config.go`: the output and the warning;
  - `internal/core/bundle.go`: imports;
  - `internal/mcp`: `vm_create` reports the port.
- **Rust (reims-gui):**
  - `crates/rg-core/src/registry.rs`: `create`, `next_ssh_port`, `ssh_port_users`;
  - `crates/rg-core/src/bundle.rs`: imports;
  - `crates/rg-ui/src/dialogs.rs` and `tabs.rs`: the wizard, Settings and Details;
  - `crates/rg-agent/src/cli.rs` and `mcp.rs`;
  - `crates/rg-ops/src/vms.rs`.
- **Shared data:** the SQLite VM list is shared by both apps; no schema change.
- **Docs:** `docs/cli.md`, `docs/mcp.md`, `docs/getting-started.md`, `docs/reims-gui.md`.
