# Tasks

## 1. Go (macos-kvm-gui) core

- [ ] 1.1 `core.NextSSHPort(exceptID string, probe func(int) bool) (int, error)` and `core.SSHPortUsers(port int, exceptID string) ([]string, error)`, both over the VM list; `DefaultConfig` uses port 0 (automatic).

  Verify: table tests with fake VMs (2222 taken → 2223; 2222 and 2223 taken → 2224; a probe that refuses 2223 → 2224; none free → an error).
- [ ] 1.2 `CreateVM` allocates inside a `BEGIN IMMEDIATE` transaction (design D1): port 0 → automatic; an explicit port is kept unless another VM has it (next free, with a notice); exhaustion fails and leaves nothing behind. Imports (`bundle.go`) and the default VM go through it.

  Verify: a test creates "A" (auto → 2222), then "B" (2222 requested → 2223 with a notice), then imports "C" with 2222 (→ 2224), "D" with 2300 (kept), exhaustion (error, no row, no unpacked folder).

## 2. Go interfaces

- [ ] 2.1 Wizard pre-fills `NextSSHPort`; `vm create --ssh-port N` (new flag); `vm create` and `vm import` print the notice; `core.CreateVMWithDefaults` holds `vm create`'s workflow (recommended resources, fresh MACs, Prepare, select) for `cmd` and the MCP.

  Verify: `cmd` tests: no flag (automatic), an unused explicit port, another VM's port (notice), 80 (refused, no VM).
- [ ] 2.2 Settings › Network inline warning, `config set ssh-port=` warning on stderr (exit 0), a Details warning row, `vm status` and MCP `vm_status` warning field.

  Verify: `cmd` tests for `config set` and `vm status`, and a screenshot of Settings and Details with two VMs sharing 2222.

## 3. Rust (reims-gui)

- [ ] 3.1 `Registry::next_ssh_port`, `Registry::ssh_port_users` (both returning `Result`), `Config::default` port 0, and allocation in `Registry::create` inside an immediate transaction (design D1); `import_settings` and `import_bundle` go through it and clean up on failure.

  Verify: rg-core tests mirroring 1.1 and 1.2.
- [ ] 3.2 The wizard pre-fill, `vm create --ssh-port`, CLI and MCP notices, the Settings › Network warning, the Details warning, and `vm status` / MCP warning.

  Verify: rg-agent CLI tests (the same four cases as 2.1), rg-ui kittest (the warning label under the port field) and snapshots.

## 4. Interface button and MCP tools (both apps)

- [ ] 4.1 "Use a free port" button next to the SSH port field in Settings › Network (Go `internal/gui/network.go`, Rust `rg-ui/src/dialogs.rs`).

  Verify: an rg-ui kittest clicks it with a fake VM list (2222 taken → 2223), and the same check by hand in the Go app with a screenshot.
- [ ] 4.2 Go MCP `vm_config_get`, `vm_config_set` (allowed keys only; Workspace lock + immediate transaction, design D5) and `vm_create` (via `CreateVMWithDefaults`), over a `core` key table shared with `cmd/config.go`.

  Verify: `internal/mcp` tests: get; set `ssh-port` and `ram`; `root-patch` refused; refusal while running and while the Workspace lock is held; invalid value; create with a taken 2222 → 2223, then `vm_start` preconditions pass (folder and disk exist).
- [ ] 4.3 Rust MCP `vm_config_get`, `vm_config_set` (same rules) and `vm_create` (via `rg-ops::Ops::create_vm`) over `rg-agent`'s `config set` logic.

  Verify: rg-agent MCP tests mirroring 4.2, and both apps' `tools/list` include the three tools with input schemas.

- [ ] 4.4 Go MCP `vm_ssh` (design D6): the system `ssh` with the app's known_hosts; a password through the `ssh-askpass` helper mode and a per-call 0700 Unix socket checked with SO_PEERCRED; output caps; timeout with a process-group kill; `confirm` required; the port from the running QEMU's `hostfwd`.

  Verify: unit tests for argv (no password in it), the socket answering only its ssh's child and only once, and the guards (stopped VM, no confirm); the real `ssh → askpass` chain against a local test sshd in a container; live against the Nilton VM: `sw_vers -productVersion` returns 15.8.1 with the password.
- [ ] 4.5 Rust MCP `vm_ssh`, mirroring 4.4.

  Verify: rg-agent tests mirroring 4.4, and the same live check through reims-gui's MCP.

## 5. Both apps together and docs

- [ ] 5.1 Cross-app: with one shared config folder, `macos-kvm-gui vm create A`, then `reims-gui vm create B`, then `macos-kvm-gui vm import <C with 2222>` give 2222, 2223 and 2224.

  Verify: a script run in a temp `XDG_CONFIG_HOME`, output recorded in the task.
- [ ] 5.2 Docs: `docs/cli.md`, `docs/mcp.md` (the four new tools, vm_ssh's consent and authentication), `docs/getting-started.md` (Network), `docs/reims-gui.md`.

  Verify: `openspec validate unique-ssh-ports --strict`, `make test`, and the reims-gui workspace tests in Docker pass.
