# Tasks

## 1. Go (macos-kvm-gui) core

- [x] 1.1 `core.NextSSHPort(exceptID string, probe func(int) bool) (int, error)` and `core.SSHPortUsers(port int, exceptID string) ([]string, error)`, both over the VM list; `DefaultConfig` uses port 0 (automatic). — done (Go agent): TestNextSSHPort, TestSSHPortUsers.

  Verify: table tests with fake VMs (2222 taken → 2223; 2222 and 2223 taken → 2224; a probe that refuses 2223 → 2224; none free → an error).
- [x] 1.2 `CreateVM` allocates inside a `BEGIN IMMEDIATE` transaction (design D1): port 0 → automatic; an explicit port is kept unless another VM has it (next free, with a notice); exhaustion fails and leaves nothing behind. Imports (`bundle.go`) and the default VM go through it. — done: TestCreateAllocatesSSHPort, TestImportBundleNoFreePort, TestConcurrentCreatesGetDistinctPorts (8 parallel, -race).

  Verify: a test creates "A" (auto → 2222), then "B" (2222 requested → 2223 with a notice), then imports "C" with 2222 (→ 2224), "D" with 2300 (kept), exhaustion (error, no row, no unpacked folder).

## 2. Go interfaces

- [x] 2.1 Wizard pre-fills `NextSSHPort`; `vm create --ssh-port N` (new flag); `vm create` and `vm import` print the notice; `core.CreateVMWithDefaults` holds `vm create`'s workflow (recommended resources, fresh MACs, Prepare, select) for `cmd` and the MCP. — done: TestSSHPortCommands (automatic, explicit kept, another VM's → notice, 80 refused); wizard pre-fill.

  Verify: `cmd` tests: no flag (automatic), an unused explicit port, another VM's port (notice), 80 (refused, no VM).
- [x] 2.2 Settings › Network inline warning, `config set ssh-port=` warning on stderr (exit 0), a Details warning row, `vm status` and MCP `vm_status` warning field. — done: TestSSHPortCommands (config set warning, vm status), TestVMConfigGetSet (vm_status warning); Details capture shots/details-shared.png.

  Verify: `cmd` tests for `config set` and `vm status`, and a screenshot of Settings and Details with two VMs sharing 2222.

## 3. Rust (reims-gui)

- [x] 3.1 `Registry::next_ssh_port`, `Registry::ssh_port_users` (both returning `Result`), `Config::default` port 0, and allocation in `Registry::create` inside an immediate transaction (design D1); `import_settings` and `import_bundle` go through it and clean up on failure. — done (Rust agent): next_free_port_skips_used_and_busy_ports, create_and_import_get_unique_ports, concurrent_creates_get_different_ports, bundle_import_without_a_free_port_leaves_nothing.

  Verify: rg-core tests mirroring 1.1 and 1.2.
- [x] 3.2 The wizard pre-fill, `vm create --ssh-port`, CLI and MCP notices, the Settings › Network warning, the Details warning, and `vm status` / MCP warning. — done: cli vm_create_and_import_pick_free_ssh_ports; kittest settings_warns_about_a_shared_port_and_saves_anyway, wizard_prefills_the_next_free_port, details_warns_for_both_vms_sharing_a_port; snapshots.

  Verify: rg-agent CLI tests (the same four cases as 2.1), rg-ui kittest (the warning label under the port field) and snapshots.

## 4. Interface button and MCP tools (both apps)

- [x] 4.1 "Use a free port" button next to the SSH port field in Settings › Network (Go `internal/gui/network.go`, Rust `rg-ui/src/dialogs.rs`). — done: Go TestUseAFreePort (Fyne test driver) + captures settings-network-shared/free.png; Rust kittest use_a_free_port_fills_the_first_free_one.

  Verify: an rg-ui kittest clicks it with a fake VM list (2222 taken → 2223), and the same check by hand in the Go app with a screenshot.
- [x] 4.2 Go MCP `vm_config_get`, `vm_config_set` (allowed keys only; Workspace lock + immediate transaction, design D5) and `vm_create` (via `CreateVMWithDefaults`), over a `core` key table shared with `cmd/config.go`. — done: TestVMConfigGetSet, TestVMConfigSetReloads, TestVMCreate, TestTools.

  Verify: `internal/mcp` tests: get; set `ssh-port` and `ram`; `root-patch` refused; refusal while running and while the Workspace lock is held; invalid value; create with a taken 2222 → 2223, then `vm_start` preconditions pass (folder and disk exist).
- [x] 4.3 Rust MCP `vm_config_get`, `vm_config_set` (same rules) and `vm_create` (via `rg-ops::Ops::create_vm`) over `rg-agent`'s `config set` logic. — done: rg-agent mcp tests tools (18), consent_first, vm_config_get_and_set, vm_config_set_and_vm_ssh_refuse_a_running_or_busy_vm, vm_create_picks_a_free_port_and_is_ready_to_start.

  Verify: rg-agent MCP tests mirroring 4.2, and both apps' `tools/list` include the three tools with input schemas.

- [x] 4.4 Go MCP `vm_ssh` (design D6): the system `ssh` with the app's known_hosts; a password through the `ssh-askpass` helper mode and a per-call 0700 Unix socket checked with SO_PEERCRED; output caps; timeout with a process-group kill; `confirm` required; the port from the running QEMU's `hostfwd`. — done: unit tests (argv, askpass socket, guards, cap, timeout); real chain vs an alpine sshd (MKG_TEST_SSHD=1); live 2026-10-17 through `macos-kvm-gui mcp` on the Nilton VM: vm_config_set moved it to 2223 (the "Also used by sonoma-test" warning went away), vm_start, vm_ssh without a password → 255 at once, with the password → stdout "15.8.1\nnilton", exit 0.

  Verify: unit tests for argv (no password in it), the socket answering only its ssh's child and only once, and the guards (stopped VM, no confirm); the real `ssh → askpass` chain against a local test sshd in a container; live against the Nilton VM: `sw_vers -productVersion` returns 15.8.1 with the password.
- [x] 4.5 Rust MCP `vm_ssh`, mirroring 4.4. — done: unit tests and the ssh_e2e test vs an ubuntu sshd; live through `reims-gui mcp` on the Nilton VM: vm_ssh → "15.8.1\nnilton", exit 0; vm_config_set refused while running; vm_ssh on the stopped sonoma-test → "is not running".

  Verify: rg-agent tests mirroring 4.4, and the same live check through reims-gui's MCP.

## 5. Both apps together and docs

- [x] 5.1 Cross-app: with one shared config folder, `macos-kvm-gui vm create A`, then `reims-gui vm create B`, then `macos-kvm-gui vm import <C with 2222>` give 2222, 2223 and 2224. — done 2026-10-17, temp HOME/XDG dirs, one VM list: Go `vm create A` → 2222; reims-gui `vm create B` → 2223; Go `vm import` of A's export as C → "SSH port 2222 belongs to A; using 2224"; Go `vm create D --ssh-port 2300` → 2300; `vm status` in both apps agrees.

  Verify: a script run in a temp `XDG_CONFIG_HOME`, output recorded in the task.
- [x] 5.2 Docs: `docs/cli.md`, `docs/mcp.md` (the four new tools, vm_ssh's consent and authentication), `docs/getting-started.md` (Network), `docs/reims-gui.md`. — done: docs/cli.md, docs/mcp.md (vm_create, vm_config_get/set, vm_ssh), docs/getting-started.md, docs/reims-gui.md.

  Verify: `openspec validate unique-ssh-ports --strict`, `make test`, and the reims-gui workspace tests in Docker pass.
