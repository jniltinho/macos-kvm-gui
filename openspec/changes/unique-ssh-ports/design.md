# Design

## Context

- **One entry point per app.** Every way of adding a VM ends in a single registry function:
  - Go `core.CreateVM`, called by the wizard (`internal/gui/wizard.go`), `vm create` (`cmd/vms.go`), the default VM (`cmd/root.go`), imports (`internal/core/bundle.go`) and, through the CLI, MCP `vm_create`;
  - Rust `Registry::create`, called by `rg-ops::Ops::create_vm`, the GUI backend (`rg-ui/src/real.rs`), the CLI and MCP (`rg-agent`) and `import_settings` / `import_bundle`.
- **Shared data.** Both apps share the SQLite VM list (`vms.ssh_port`).
- **Existing check.** `checkSSHPort` (Go) and its Rust port already fail a start whose port is taken on 127.0.0.1.

## Goals / Non-Goals

**Goals:**
- No new or imported VM ever duplicates another VM's port.
- A shared port is visible wherever the port is edited or shown.

**Non-Goals:**
- Renumbering existing VMs.
- A UNIQUE constraint in SQLite: it would break existing lists with duplicates and forbid deliberate sharing.
- Picking a port at start time: the port is part of the VM's identity for SSH users.

## Decisions

### D1. Allocation inside the registry's create
- **Where.** `CreateVM` and `Registry::create`, inside the same immediate transaction as the insert:
  - **Automatic** when `SSHPort == 0`: the defaults (`DefaultConfig` / `Config::default`) and every creation path use 0 unless a port was asked for. The VM gets `NextSSHPort`, probing the host.
  - **Explicit** when the port is non-zero (`--ssh-port`, the wizard's field, an imported VM's stored port). It is kept if no other VM has it, otherwise replaced with `NextSSHPort`, with a notice ("SSH port 2222 belongs to Nilton; using 2223"). Range validation (1024–65535) runs on explicit ports first.
  - The entry is returned with the port actually stored, plus the notice; callers print or log it.
- **Why there.** Every path (wizard, CLI, MCP, import, the default VM) is covered by one rule in each app, and the transaction stops two concurrent creates from picking the same port.
  - Go: `BEGIN IMMEDIATE` on the modernc sqlite connection.
  - Rust: `rusqlite` `TransactionBehavior::Immediate`.
- **Alternative.** Allocating in each caller: five call sites per app, easy to miss one. Rejected.

### D2. `NextSSHPort`
- **The search.** It starts at 2222, skips every port another VM in the list uses, and skips ports where `net.Listen("tcp", "127.0.0.1:p")` (Go) / `TcpListener::bind` (Rust) fails. It returns the first that passes, up to 65535.
- **No free port** is an error ("no free SSH port from 2222 to 65535"). Creation and import abort: the transaction rolls back, and an import removes the folder it unpacked (as it does today on a CreateVM error). The helpers return errors, so database failures propagate too.
- **Bind failures only affect allocation.** A port held by another process is skipped when allocating, but a port the user explicitly chose is never rejected for that reason. The start check covers it.
- **The wizard** calls the same helper to pre-fill its Network section, so the user sees the port before creating.

### D3. Warnings: `SSHPortUsers(port, exceptID)`
- **The lookup.** A query returning the names of other VMs with that port. It is used by:
  - Settings › Network: an inline warning under the port field;
  - `config set ssh-port=`: printed on stderr, exit 0;
  - Details › Network › Port Forwarding: a warning row;
  - `vm status` / MCP `vm_status`: a `warning` field.
- **The same wording in both apps:** "Also used by <name>: they cannot run at the same time".

### D4. Rust and Go stay in step
- The same algorithm, wording and tests on both sides. A Go test and a Rust test each create two VMs and import a third with a taken port, and check 2222, 2223 and 2224.
- A cross-app check: a VM created by one app, then one created by the other, get different ports (shared database).

### D5. MCP settings tools (both apps)
- **One implementation per app, shared with the CLI.** `vm_config_set` reuses the CLI's key table:
  - Go `configKeys` in `cmd/config.go` moves to `internal/core/configkeys.go`, so `cmd` and `internal/mcp` share it;
  - Rust: `rg-agent`'s `set_config_value`.
- **Allowed keys.** Only `ssh-port`, `ram`, `cores`, `vcpus`, `graphics`, `resolution`, `display`, `gpu` and `version`. `root-patch` (its revert needs a confirmation and its enable downloads OCLP's kexts) and `workspace` (a folder change, not a move) are refused with "use the app or `config set`".
- **Atomic with respect to the other app and to starts.**
  1. Take the VM folder's Workspace lock, the one a start takes, so neither app can start it meanwhile; refuse if it is held or the VM runs (`find_running` on the stored settings).
  2. In an immediate SQLite transaction: reload the VM, apply the keys, validate, save.
  3. Release the lock and return the settings with the D3 warnings.

  Tests cover a concurrent start, and a concurrent edit from the other app's code path.
- **`vm_config_get`** returns the stored settings as JSON. Paths stay as stored; there are no secrets in VM settings.
- **`vm_create` shares the CLI's whole create workflow.** In Go, `cmd/vms.go`'s create body moves to `core.CreateVMWithDefaults` (recommended resources, fresh adapters and MACs, `Workspace.Prepare` for the folder and disk, select), used by `cmd` and `internal/mcp`; Rust uses `rg-ops::Ops::create_vm`. So a VM created over MCP starts with `vm_start` right away. It downloads nothing; the recovery and installers stay with their tools.
- **Annotations.** The three tools are not marked destructive: they change settings but never delete data, and `vm_config_set` cannot remove disks or folders.

### D6. `vm_ssh` (both apps)
- **The client.** The system's `ssh` (OpenSSH), not an SSH library: no new Go module or Rust crate, and it uses the user's keys, agent and `~/.ssh/config` as they are.
- **The call:**
  ```
  ssh -p <port> -o ConnectTimeout=10 -o BatchMode=<yes|no>
      -o StrictHostKeyChecking=accept-new
      -o UserKnownHostsFile=<config dir>/known_hosts
      <user>@127.0.0.1 -- <command>
  ```
  - The app keeps its own `known_hosts`, so guests reinstalled on the same port do not break the user's file. It is re-learned with `reset_host_key: true` (it removes the `[127.0.0.1]:<port>` entry).
- **Password, when given:**
  - `SSH_ASKPASS` points at the app's own binary in a hidden helper mode (`macos-kvm-gui ssh-askpass` / `reims-gui ssh-askpass`), with `SSH_ASKPASS_REQUIRE=force` and `DISPLAY` unset as needed.
  - OpenSSH closes inherited descriptors above stderr, so a pipe cannot reach the helper. Instead the server creates a per-call Unix socket in a fresh 0700 directory under `$XDG_RUNTIME_DIR` and passes its path in an environment variable. The socket answers once, only to a peer whose parent process is the `ssh` it started (checked with SO_PEERCRED), then is closed and removed.
  - The password is never in argv, the environment or a file, and never logged; no `sshpass`.
- **Limits:**
  - Output is capped at 1 MiB per stream, truncated with a note.
  - `timeout_sec` defaults to 60 (max 600). On timeout the process group is killed.
- **Guards:**
  - The VM must be running (`find_running`), and the port comes from the run's own argv (`hostfwd`), not the stored settings, so a VM whose port changed after it started still works.
  - The tool needs `confirm: true` and is marked destructive.

## Risks / Trade-offs

- **Ports owned by others.** [A port free at creation is taken later by another service] → the start check already reports it; the user changes the port in Settings.
- **Surprise for explicit choices.** [A user explicitly asks for a port another VM has and gets a different one] → the notice says so; Settings can set it back, with the warning.
- **Probing cost.** [Bind probing in tests (ports in use on CI)] → tests inject the probe function (as `checkSSHPort` already allows), so they do not depend on the host.

## Migration Plan

No schema change. Existing duplicates stay and are shown by D3's warnings. Rollback means reverting the code; stored ports remain valid.
