<p align="center">
  <img src="internal/assets/logo/logo.svg" width="112" alt="macos-kvm-gui logo">
</p>

<h1 align="center">macOS KVM · reims-gui</h1>

<p align="center">
  <b>Run macOS on Linux with KVM, from a single desktop app.</b><br>
  Download, install and boot macOS in a few clicks: OpenCore and OVMF are built in, nothing else to clone or script.
</p>

<p align="center">
  <a href="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/reims-gui-ci.yml"><img src="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/reims-gui-ci.yml/badge.svg" alt="reims-gui CI"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/reims-gui-release.yml"><img src="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/reims-gui-release.yml/badge.svg" alt="reims-gui release"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/releases/latest"><img src="https://img.shields.io/github/v/release/jniltinho/macos-kvm-gui?color=blue" alt="Latest release"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/releases"><img src="https://img.shields.io/github/downloads/jniltinho/macos-kvm-gui/total?color=blue" alt="Downloads"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.99-blue" alt="Rust 1.99"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/jniltinho/macos-kvm-gui?color=blue" alt="License"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/stargazers"><img src="https://img.shields.io/github/stars/jniltinho/macos-kvm-gui?style=social" alt="Stars"></a>
</p>

<p align="center">
  <img src="docs/screenshots/reims-gui-screen-desktop.png" width="860" alt="reims-gui with macOS Sequoia on Reims vGPU inside the Screen tab">
</p>

<p align="center">
  <b><a href="https://github.com/jniltinho/macos-kvm-gui/releases/latest">⬇ Download reims-gui (latest release)</a></b>
</p>

> **For home lab use.** macOS is Apple's software, and its license only allows it on Apple-branded hardware. Make sure your use complies with Apple's macOS license terms. This project is not affiliated with Apple.

This repository has two apps that read the same VMs and settings:

| App | Language | Releases | Status |
|---|---|---|---|
| **[reims-gui](reims-gui/README.md)** | Rust (egui) | [`reims-gui-v*`](https://github.com/jniltinho/macos-kvm-gui/releases?q=reims-gui&expanded=true) | **Recommended.** Reims vGPU's Metal display inside the window; `.deb` for Ubuntu 24.04, 26.04 and Debian 13, `.tar.gz` also for Rocky Linux 10 |
| macos-kvm-gui | Go (Fyne) | [`v*`](https://github.com/jniltinho/macos-kvm-gui/releases?q=macos-kvm-gui&expanded=true) | The first app; also embeds Reims vGPU's window since v0.11.0; `.deb`, `.rpm` (Fedora 39+), `.tar.gz` |

## Download and install reims-gui

From the [latest release](https://github.com/jniltinho/macos-kvm-gui/releases/latest), take the `.deb` (one package for Ubuntu 24.04, 26.04 and Debian 13) and check it:

```sh
sha256sum -c reims-gui_0.1.4_amd64.deb.sha256
sudo apt install ./reims-gui_0.1.4_amd64.deb
```

It adds **Reims GUI** to the applications menu and pulls in QEMU (`qemu-system-x86`, `qemu-system-gui`, `qemu-utils`). Or unpack the `.tar.gz` and run `reims-gui` from it. Needs an x86_64 CPU with VT-x/AMD-V and KVM.

1. Open **Reims GUI** and, in the **Host** tab, click **Set up host** (one password prompt), then **Install Reims vGPU**.
2. Click **New**: the wizard creates the VM's folder and disk and downloads the macOS recovery from Apple.
3. Click **Start**: macOS shows in the **Screen** tab. Inside the VM, erase the disk as APFS in Disk Utility and run **Reinstall macOS**; the **Install** tab walks you through it.

Guide, limits and troubleshooting: [docs/reims-gui.md](docs/reims-gui.md).

**Debian 13 and Rocky Linux 10.** The `.deb` installs and runs on Debian 13, whose QEMU has what macOS needs. The `.tar.gz` runs on Rocky Linux 10.2 (the window opens on Wayland and XWayland), but Rocky's own `qemu-kvm` has no `isa-applesmc` or `vmware-svga`, so macOS VMs there need Reims vGPU's QEMU. On both, Reims vGPU's QEMU needs its own build ([Debian](docs/reims-vgpu.md#debian-13), [Rocky](docs/reims-vgpu.md#rocky-linux-rhel-family)); **Install Reims vGPU** in the Host tab installs the Ubuntu packages only.

## Highlights

- **Metal without a spare GPU.** [Reims vGPU](docs/reims-vgpu.md), a paravirtual GPU, renders macOS's Metal through the host's own Vulkan driver (NVIDIA, AMD or Intel), and the app shows it inside its window; one click in the Host tab installs it.
- **One binary.** GUI, [command line](docs/cli.md) and [MCP server](docs/mcp.md) in one file; OpenCore, Lilu, VirtualSMC, WhateverGreen and OVMF are embedded, with no `git`, `python3` or OSX-KVM clone.
- **Verified downloads.** The recovery comes from Apple, checked against Apple's signature and every chunk's SHA-256.
- **A manager like VirtualBox's.** Menus, toolbar, a VM list, Details with a live Preview, a Settings window and a New VM wizard, in light and dark themes.
- **Several VMs.** One folder per VM in the Workspace, move, export/import.
- **Guided install.** A step-by-step Install tab, and Host checks with one-click setup.
- **Offline install.** Download Apple's full installer once (Big Sur and later) and install every VM of that version from a local disc.
- **Networking.** Up to four adapters per VM, NAT or bridged, with the adapter type and MAC address of your choice.
- **GPU passthrough** for supported AMD and NVIDIA Kepler cards, with root-patch mode for Kepler.

Sonoma 14 is validated end to end; Sequoia 15 runs on Reims vGPU with Metal; Sequoia 15 and Big Sur 11 have installed from the offline installer; the other versions, High Sierra through Tahoe, are experimental. More in the [overview](docs/overview.md).

<p align="center">
  <img src="docs/screenshots/reims-gui-details.png" width="860" alt="reims-gui: Details of a Reims vGPU VM">
</p>

More in the [screenshots](docs/screenshots/README.md).

## macos-kvm-gui (Go, previous app)

The Go app is still in this repository, with [its own releases](https://github.com/jniltinho/macos-kvm-gui/releases?q=macos-kvm-gui&expanded=true) (`.deb` for Ubuntu 24.04+ and Debian 13+, `.rpm` for Fedora 39+, `.tar.gz`). Since v0.11.0 it also shows Reims vGPU's window inside its Screen tab. Its guide: [getting started](docs/getting-started.md).

## Documentation

[Overview](docs/overview.md) · [Getting started](docs/getting-started.md) · [Command line](docs/cli.md) · [MCP server](docs/mcp.md) · [Architecture](docs/architecture.md) · [GPU passthrough](docs/gpu-passthrough.md) · [Reims vGPU](docs/reims-vgpu.md) · [reims-gui (Rust)](docs/reims-gui.md) · [Troubleshooting](docs/troubleshooting.md) · [Development](docs/development.md) · [Screenshots](docs/screenshots/README.md) · [References](docs/references.md)

## Build

```sh
make build     # dist/reims-gui (Rust, built in Docker) and dist/macos-kvm-gui (Go 1.27+, Fyne build deps)
make package  # both apps' packages in dist/
make test     # the Go app's tests; reims-gui's run in Docker (reims-gui/README.md)
```

The Go app's CI and release workflows no longer run on push or tags (run them by hand from the Actions tab); new builds and releases are those of reims-gui.

## License

BSD-3-Clause ([LICENSE](LICENSE)); reims-gui's source is GPL-2.0-or-later OR LGPL-3.0-or-later and its binaries GPL-3.0-or-later ([reims-gui/README.md](reims-gui/README.md#license)). The embedded boot files keep their own licenses, and Reims vGPU's separately published QEMU packages theirs ([THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)). Built on the work of [OSX-KVM](https://github.com/kholia/OSX-KVM), [acidanthera](https://github.com/acidanthera), [TianoCore](https://github.com/tianocore/edk2), [Dortania](https://dortania.github.io/) and [Fyne](https://fyne.io); see [references](docs/references.md).
