# Spec Delta

## Purpose

Give macOS VMs accelerated graphics without GPU passthrough by running them on Reims vGPU, a paravirtual GPU the user installs, through a per-VM graphics option.

## ADDED Requirements

### Requirement: Graphics option
Each VM SHALL have a Graphics setting, `vmware-svga` or `reims-vgpu`, saved with the VM, exported and imported with it, and defaulting to `vmware-svga` for new and existing VMs. It SHALL be available for every macOS version; for versions other than macOS 26 the app SHALL say that upstream has not measured them.

#### Scenario: Existing VM
- **WHEN** a VM saved before this change is loaded
- **THEN** its Graphics is `vmware-svga` and its QEMU command is unchanged

#### Scenario: Older macOS
- **WHEN** the user picks `reims-vgpu` for a Sequoia VM
- **THEN** the setting is saved and the Settings dialog notes that upstream measured macOS 26 guests only

### Requirement: Exclusive with GPU passthrough
A VM SHALL NOT have both `reims-vgpu` and a passed-through GPU.

#### Scenario: Both chosen
- **WHEN** the user saves a VM with `reims-vgpu` and a GPU address
- **THEN** saving fails with an error saying to choose one of them

### Requirement: Reims QEMU build
The app SHALL NOT embed or ship Reims vGPU; it installs it only when the user asks (Package install, Docker build). A VM with `reims-vgpu` SHALL run the `qemu-system-x86_64` of the user's Reims install: `reims_qemu` in `settings.yaml` when set, otherwise the app's build `<Workspace>/.reims/qemu/current/bin/qemu-system-x86_64`, otherwise an upstream install at `$XDG_DATA_HOME/reims-ctl/qemu/current/bin/qemu-system-x86_64`. The binary SHALL list `apple-gfx-pci` among its devices. Other VMs SHALL keep using the system QEMU.

#### Scenario: Not installed
- **WHEN** a VM with `reims-vgpu` starts and no Reims QEMU build is found
- **THEN** it does not start, and the error names the path searched and points to reims-vgpu's installer

#### Scenario: Build without the device
- **WHEN** the binary found does not list `apple-gfx-pci`
- **THEN** the VM does not start and the error names the binary

### Requirement: Prebuilt packages
This repository SHALL publish, on a release tagged `reims-<commit>`, one package per supported Ubuntu release (24.04, 26.04) named `reims-qemu-<commit>-ubuntu<release>-x86_64.tar.gz` with a `.sha256` file, built from upstream at that commit in an `ubuntu:<release>` container and stripped, and the corresponding source `reims-vgpu-src-<commit>.tar.gz` (upstream, its `vendor/qemu` with the meson subprojects its build fetches, and the vendored Rust crates, SDL3's source among them) with the license texts, in the same release.

#### Scenario: Release
- **WHEN** a `reims-<commit>` tag is pushed
- **THEN** the release holds both packages, their SHA-256 files and the source archive

### Requirement: Package install
The app SHALL embed a manifest naming the reims release tag and each package's SHA-256, and install the package for the host's Ubuntu release from the Host tab's **Install Reims vGPU** button and `reims install`: download over HTTPS, verify the SHA-256, unpack into `<Workspace>/.reims/qemu/<build>` (no path may leave it, no links outside it), then activate it under the same rules as a build. A host release without a package SHALL get an error offering the Docker build.

#### Scenario: Install on Ubuntu 24.04
- **WHEN** the user installs on an Ubuntu 24.04 host
- **THEN** the 24.04 package is downloaded and verified, and `<Workspace>/.reims/qemu/current/bin/qemu-system-x86_64` lists `apple-gfx-pci`

#### Scenario: Tampered download
- **WHEN** the downloaded package does not match the manifest's SHA-256
- **THEN** nothing is unpacked and `current` is unchanged

#### Scenario: Unsupported release
- **WHEN** the host is Debian 13
- **THEN** the install stops naming the supported Ubuntu releases and the Docker build

### Requirement: Docker build
The app SHALL embed a build recipe and build upstream reims-vgpu at a pinned commit with upstream's `scripts/install-ctl.sh` inside an `ubuntu:<release>` container, from the Host tab's **Build Reims vGPU** button and `reims build [--release 24.04|26.04]`, showing progress. The release SHALL default to the host's when it is 24.04 or 26.04; otherwise the build stops naming the supported releases. Docker missing or unusable SHALL stop the build with the fix.

#### Scenario: Build on Ubuntu 24.04
- **WHEN** the user builds on an Ubuntu 24.04 host with Docker
- **THEN** `<Workspace>/.reims/qemu/current/bin/qemu-system-x86_64` exists, runs on the host, and lists `apple-gfx-pci`

#### Scenario: Unsupported host release
- **WHEN** the host is Ubuntu 22.04 and no release is given
- **THEN** the build stops before starting Docker and names 24.04 and 26.04

### Requirement: Build activation
Installed packages and builds SHALL live in `<Workspace>/.reims/` (`qemu/<build>`, `current`, and the build cache), and nothing SHALL be installed elsewhere on the host. A build SHALL be activated (`current` pointing to it) only when, on the host, every library of its `qemu-system-x86_64` resolves and its device list has `apple-gfx-pci`; otherwise `current` SHALL stay as it was and the error SHALL list the missing libraries. A build for another release SHALL stay staged.

#### Scenario: Missing host library
- **WHEN** the staged QEMU needs a library the host lacks
- **THEN** `current` is unchanged and the error names the library and its Ubuntu package

### Requirement: Build concurrency and retention
Builds SHALL be serialized by a host-wide lock, and a build folder a running VM uses SHALL never be removed.

#### Scenario: Two builds at once
- **WHEN** a second build starts while one runs
- **THEN** it waits for the first or stops with a busy error, and neither corrupts the other

#### Scenario: Rebuild while a VM runs
- **WHEN** a Reims VM runs and a new build is activated
- **THEN** the running VM's build folder stays

### Requirement: QEMU command line
For a VM with `reims-vgpu` the QEMU command SHALL: replace `vmware-svga` with `apple-gfx-pci` at a PCI address no other device of the command uses, back guest RAM with a shared memfd (`memory-backend-memfd,share=on`), disable ACPI S3, keep keyboard and tablet on xHCI, add a USB audio device on xHCI bound to the same audio backend as the HDA codec, and keep the VM's other devices (disks, network, OpenCore). A VM with `vmware-svga` SHALL get exactly today's command.

#### Scenario: Command
- **WHEN** the arguments of a `reims-vgpu` VM are built
- **THEN** they contain `apple-gfx-pci`, `memory-backend-memfd` with `share=on`, `ICH9-LPC.disable_s3=1` and `usb-audio`, and no `vmware-svga`

#### Scenario: QEMU accepts the command
- **WHEN** the Reims QEMU realizes that command paused (`-S`, no disks needed) with the `none` audio backend
- **THEN** it starts without a PCI address or audio error

### Requirement: Display
A `reims-vgpu` VM's screen SHALL be Reims vGPU's own window (QEMU with `-display none`, `REIMS_VGPU_WINDOW` not set to 0), whatever the VM's Display setting. The app's VNC socket SHALL still be served: the Preview and the MCP server's screen capture and input SHALL keep working, while the Screen tab SHALL NOT attach as the interactive screen and SHALL say that the screen is in Reims vGPU's window. This SHALL hold for VMs started by the app and by another process (CLI, MCP).

#### Scenario: Start from the app
- **WHEN** a `reims-vgpu` VM starts from the app
- **THEN** a Reims vGPU window shows the guest, the Preview updates, and the Screen tab says the screen is in that window

#### Scenario: Started by the MCP server
- **WHEN** an MCP client starts a `reims-vgpu` VM and calls `screen_capture`
- **THEN** the capture shows the guest, and the app shows the VM running with its Preview, without attaching the Screen tab

### Requirement: Host checks
Before a `reims-vgpu` VM starts, and in the Host tab, the app SHALL check the Reims QEMU build (above), a Vulkan driver (an ICD manifest under `/usr/share/vulkan/icd.d` or `/etc/vulkan/icd.d`), and `vm.max_map_count` of at least 64 per MiB of the VM's RAM plus 65,536. A failed check SHALL stop the start with an error naming the fix (the `sysctl` value).

#### Scenario: Low map count
- **WHEN** a VM with 16384 MiB starts and `vm.max_map_count` is 65530
- **THEN** it does not start, and the error gives `sudo sysctl -w vm.max_map_count=<value>` and a file in `/etc/sysctl.d` to keep it

### Requirement: udmabuf warning
When the host has an AMD GPU and the user cannot open `/dev/udmabuf` read-write, the app SHALL warn, not block (QEMU may render on another GPU), naming the group to join.

#### Scenario: AMD without udmabuf
- **WHEN** the host has an AMD GPU and the user cannot open `/dev/udmabuf` read-write
- **THEN** the VM starts, and the log and Host tab warn with the device's group to join; a QEMU exit mentioning udmabuf is explained the same way

### Requirement: Guest guidance
The app SHALL tell the user, when Graphics is set to `reims-vgpu` and in the docs, that macOS needs `METAL_ERROR_MODE` 5 set once, with the commands for a working desktop (`sudo defaults write …`) and for a new install (from Recovery's Terminal), and that displays wider than 2048 pixels need a kext the app does not ship.

#### Scenario: Turning it on
- **WHEN** the user selects `reims-vgpu` in Settings
- **THEN** the dialog shows the `METAL_ERROR_MODE` commands

### Requirement: CLI
`config set graphics=vmware-svga|reims-vgpu` SHALL change the setting with the same validation as the app, and `config show` SHALL print it. `reims status` SHALL print the build in use, its commit and the host checks.

#### Scenario: Invalid value
- **WHEN** the user runs `config set graphics=virtio`
- **THEN** nothing is saved and the error lists the valid values
