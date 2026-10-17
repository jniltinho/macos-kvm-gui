# Proposal

## Why

Installing macOS on KVM with [OSX-KVM](https://github.com/kholia/OSX-KVM) takes many manual terminal steps: cloning the repository, downloading the recovery with `fetch-macOS-v2.py`, converting the DMG, creating the qcow2 disk, setting `ignore_msrs`, editing RAM/CPU directly in `OpenCore-Boot.sh`, and diagnosing conflicts on your own, such as a VirtualBox VM holding AMD-V (`KVM_CREATE_VM: Device or resource busy`). A desktop app that walks through these steps and shows the host state reduces errors and setup time.

## What Changes

- New desktop app in Go (single binary) that:
  - checks host prerequisites (CPU with VT-x/AMD-V, access to `/dev/kvm`, `qemu-system-x86_64`, `qemu-img`, `git`, `python3`, `ignore_msrs`, conflicting hypervisors) and offers to fix `ignore_msrs`;
  - prepares the OSX-KVM workspace (shallow clone) and reuses its OpenCore, OVMF and `fetch-macOS-v2.py`;
  - downloads the recovery for the chosen macOS version (High Sierra → Tahoe; only Sonoma is validated, the others are experimental), verifies Apple's signature, converts it to a raw image and deletes the DMG to save space;
  - creates the thin qcow2 virtual disk with a configurable size;
  - configures RAM, cores, vCPUs and SSH port and saves them to a user config file; the CPU model and network card are derived from the macOS version;
  - starts, shuts down (ACPI) and force-stops the VM, keeping the UEFI NVRAM across runs and showing the QEMU log and errors;
  - removes the install media after installation to free space.
- The `OpenCore-Boot.sh` logic is ported to Go: QEMU arguments are generated from the configuration, with no script editing. The forwarded SSH port is bound to `127.0.0.1` only.
- Partitioning the disk and running the installer inside the guest remain manual; the app only shows the steps.

## Capabilities

### New Capabilities
- `host-preflight`: checks host prerequisites for KVM/macOS and assisted fix of `ignore_msrs`.
- `macos-install-media`: OSX-KVM workspace preparation, macOS recovery download and conversion, disk creation and cleanup.
- `vm-lifecycle`: persistent VM configuration and start, shut down and force stop controls, with log monitoring.

### Modified Capabilities
<!-- None: new project. -->

## Impact

- New repository `macos-kvm-gui` (Go, Fyne toolkit).
- Runtime dependencies: `qemu-system-x86_64`, `qemu-img`, `git`, `python3`, `pkexec` (only to fix `ignore_msrs`).
- Build dependencies: C compiler and OpenGL/X11 headers (required by Fyne).
- The app depends on an OSX-KVM clone (GPLv3) at runtime; none of its code is embedded in the binary.
- Out of scope for this change: GPU/USB passthrough, bridge/tap networking, libvirt, macOS/Windows as host.
