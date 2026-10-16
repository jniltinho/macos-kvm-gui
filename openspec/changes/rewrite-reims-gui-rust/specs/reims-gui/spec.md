# Spec Delta

## Purpose

`reims-gui` is the Rust desktop app that manages macOS VMs on QEMU/KVM with Reims vGPU first: same behavior as the Go app, the Reims display inside its window, one binary with GUI, CLI and MCP, built in Docker for Ubuntu 24.04 and 26.04.

## ADDED Requirements

### Requirement: Docker-only build
`reims-gui` SHALL build only inside Docker images based on `ubuntu:24.04` and `ubuntu:26.04`, with a pinned Rust toolchain, from one script (`reims-gui/scripts/build.sh <release>`), producing for each release a binary, a `.tar.gz` and a `.deb`, each with a `.sha256`. The host needs only Docker.

#### Scenario: Build for 24.04
- **WHEN** a developer runs `reims-gui/scripts/build.sh 24.04`
- **THEN** `reims-gui/dist/` holds `reims-gui_<version>_ubuntu24.04_amd64.deb`, `reims-gui-<version>-ubuntu24.04-x86_64.tar.gz`, the binary and their `.sha256` files, and nothing was installed on the host

#### Scenario: Package installs on its release
- **WHEN** the 26.04 `.deb` is installed in a clean `ubuntu:26.04` container with apt
- **THEN** its dependencies resolve and `reims-gui --self-test` opens the GUI under Xvfb, renders a frame and exits 0

### Requirement: Single binary
One executable SHALL provide the GUI (no arguments, or `gui`), the CLI subcommands of the Go app with the same names, flags and output meaning, and the MCP server (`reims-gui mcp`) with the same tools.

#### Scenario: CLI parity
- **WHEN** `reims-gui vm list`, `config show`, `gpu list`, `reims status` or `settings show` runs
- **THEN** it prints the same information as the Go app's command of the same name

#### Scenario: MCP parity
- **WHEN** an MCP client lists the tools of `reims-gui mcp`
- **THEN** it gets the Go server's tools (vm_list, vm_status, vm_start, vm_shutdown, vm_force_stop, screen_capture, send_keys, type_text, pointer, installer_list, installer_download, installer_remove, install_offline, host_check) with the same arguments and consent rules

### Requirement: Shared data
`reims-gui` SHALL read and write the Go app's data unchanged: the SQLite database (schema up to 5) in `~/.config/macos-kvm-gui/`, `settings.yaml` and the Workspace layout (`<Workspace>/<vm>`, `.kexts`, `.installers`, `.reims`), and SHALL migrate older schemas the same way.

#### Scenario: Switching apps
- **WHEN** a VM created by the Go app is opened in `reims-gui`
- **THEN** it lists, boots and keeps its NVRAM, identity, disk and settings

#### Scenario: Both apps at once
- **WHEN** the Go app runs a VM and `reims-gui` is opened
- **THEN** `reims-gui` shows it running, can shut it down over its control socket, and refuses to start it a second time

### Requirement: Same GUI
The GUI SHALL keep the Go app's structure: File/Machine/Help menus, the toolbar (New, Import, Settings, Remove, Start, Shut down, Force stop, Window, Full screen), the VM list with state, the Details, Screen, Install, Host, GPU and Log tabs, the Settings dialog pages, the New VM wizard, Preferences, Offline installers, light/dark/system themes and modal dialogs with depth (rounded corners and shadow).

#### Scenario: Details tab
- **WHEN** a VM is selected
- **THEN** Details shows General, System, Display, Storage, Audio, Network, USB and GPU sections with the Preview, as the Go app does

### Requirement: Reims display inside the window
For a Reims vGPU VM started by the app, the guest display SHALL appear inside the Screen tab, rendered by Reims vGPU on the GPU (no VNC readback), resized with the tab, with keyboard and pointer reaching the guest, and the tab SHALL get it back after the VM restarts. In phase 1 this uses X11 window embedding: QEMU's Reims window is created on X11 and reparented into the tab.

#### Scenario: Start a Reims VM
- **WHEN** the user starts a Reims VM from the app
- **THEN** no separate Reims window stays on the desktop and the Screen tab shows the guest at its frame rate

#### Scenario: Wayland session
- **WHEN** the desktop session is Wayland with XWayland available
- **THEN** the app runs through XWayland and the embedding still works

#### Scenario: Dialog over the screen
- **WHEN** a dialog or menu opens over the Screen tab of a running embedded VM
- **THEN** the embedded window is hidden until the dialog closes, and clicks on the dialog never reach the guest

#### Scenario: Guest changes mode
- **WHEN** the guest changes its display mode
- **THEN** the embedded display stays the size of the tab

#### Scenario: Embedding unavailable
- **WHEN** no X server is reachable
- **THEN** the app runs on Wayland, the Reims window opens on its own with SDL's native backend and the Screen tab says so

### Requirement: Screen for VMware SVGA
VMs on VMware SVGA SHALL show their screen in the Screen tab: with X11, QEMU's own window embedded like Reims (native scancodes for any layout); without X11, QEMU's VNC socket drawn by the app. The VNC socket SHALL always exist for the Preview and MCP.

#### Scenario: SVGA VM
- **WHEN** a VMware SVGA VM starts with display in the app on an X11 or XWayland session
- **THEN** the Screen tab shows QEMU's window, and an ABNT2 key, keypad Enter and keypad Divide reach macOS as their own keys

### Requirement: Reims-first defaults
New VMs SHALL default to Reims vGPU for macOS Sequoia and Tahoe when its QEMU is installed and the host checks pass, and the Host tab SHALL install the prebuilt Reims package or build it in Docker, as the Go app does.

#### Scenario: New Sequoia VM
- **WHEN** the wizard creates a Sequoia VM on a host with Reims installed
- **THEN** its graphics is Reims vGPU

### Requirement: Feature parity
`reims-gui` SHALL implement every behavior of the Go app's specs and code: recovery download and signature/chunk verification, full installers and offline install, disks, network adapters (NAT/bridged, models, MACs), GPU passthrough (eligibility, VFIO reservation and release), root-patch mode, extra kexts and `settings.yaml`, Reims packages, build, kext and resolutions, host checks and setup, export/import and move, running-VM detection, QMP shutdown/force stop, keyboard layouts.

#### Scenario: Parity checklist
- **WHEN** the parity checklist in `reims-gui/PARITY.md` is reviewed
- **THEN** every Go feature has a Rust counterpart with a test or a recorded manual check

### Requirement: License
`reims-gui` source SHALL be GPL-2.0-or-later OR LGPL-3.0-or-later; its binaries SHALL be distributed under GPL-3.0-or-later with every dependency's license text, the notices of code copied from reims-vgpu, rustXterm and PhotoCraft, and the corresponding source attached to each release.

#### Scenario: Release contents
- **WHEN** a release is published
- **THEN** it has the packages, a source archive with vendored crates and the build Dockerfile, and the packages contain the license texts and notices
