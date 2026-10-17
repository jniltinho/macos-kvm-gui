# Proposal

## Why

The app is Go/Fyne, while Reims vGPU (the GPU path the app is moving to) is Rust, and its window lives outside the app. A Rust app can reuse reims-ctl's VM, QEMU, QMP and host code directly, share one toolchain and Docker build with the Reims QEMU packages, and own the guest display inside its main window. The user asked for a 100% Rust version, `reims-gui`, built in Docker and focused on Ubuntu 24.04 and 26.04.

## What Changes

- New Cargo workspace `reims-gui/` in this repository: one desktop app, `reims-gui`, with the same CLI and MCP server in the same binary. It replaces the Go app feature for feature (the Go code stays until parity is reached, then is removed in a later change).
- The GUI keeps the Go app's layout and behavior (VirtualBox-style: menus, toolbar, VM list, Details/Screen/Install/Host/GPU/Log tabs, Settings dialog, New VM wizard, Preferences, light/dark themes, rounded modals), written idiomatically in Rust with egui/eframe on wgpu, following rustXterm and PhotoCraft's patterns (both MIT/Apache, copied and trimmed, not depended on).
- It is focused on Reims vGPU: new VMs default to it when the host can run it, the Host tab installs or builds it, and **the Reims vGPU display is shown inside the app's Screen tab**:
  - Phase 1 (this change): QEMU's Reims window is embedded into the Screen tab by X11 reparenting (QEMU's SDL forced to X11; the app runs on X11 or XWayland, which both Ubuntu releases have), with GPU rendering unchanged and input through the library's own window.
  - Phase 2 (follow-up change): a remote frame sink in Reims vGPU's window crate that exports frames as dma-bufs to the app, for native Wayland.
- VMware SVGA VMs keep the in-app VNC screen (rustXterm's VNC client and desktop widget).
- Everything the Go app does is ported: VM registry and settings (SQLite and `settings.yaml`), workspace and disks, OpenCore EFI and OVMF assets (embedded), recovery download and verification, full installers and offline install, network adapters, GPU passthrough with VFIO reservation, root-patch mode, extra kexts, Reims packages/build/kext, host checks and setup, export/import, CLI and MCP.
- The license of `reims-gui` is **GPL-2.0-or-later OR LGPL-3.0-or-later**, Reims vGPU's, because it starts from reims-ctl's code. The Go app stays BSD-3-Clause.
- Builds run only in Docker (Ubuntu 24.04 and 26.04 images); outputs are a binary, a `.tar.gz` and a `.deb` per release; CI and a release workflow build them.

## Capabilities

### New Capabilities
- `reims-gui`: the Rust desktop app, its embedded Reims display, its CLI and MCP server, its Docker build and packages, and parity with the Go app's behavior.

### Modified Capabilities

## Impact

- New `reims-gui/` (Cargo workspace), `reims-gui/docker/` (build images), `.github/workflows/reims-gui-ci.yml` and `reims-gui-release.yml`.
- Reads the same data as the Go app (`~/.config/macos-kvm-gui/macos-kvm-gui.db`, `settings.yaml`, the Workspace), so a user can switch between them during the transition.
- Reuses `internal/assets` (OpenCore, kexts, OVMF, `config.plist.tmpl`) and `internal/core/reims/Dockerfile`.
- `base/` holds the three reference projects (git-ignored); code copied from them keeps their license notices in `reims-gui/THIRD_PARTY_NOTICES.md`.
