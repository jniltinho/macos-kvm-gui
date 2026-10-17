<p align="center">
  <img src="internal/assets/logo/logo.svg" width="112" alt="macos-kvm-gui logo">
</p>

<h1 align="center">macOS KVM</h1>

<p align="center">
  <b>Run macOS on Linux with KVM, from a single desktop app.</b><br>
  Download, install and boot macOS in a few clicks: OpenCore and OVMF are built in, nothing else to clone or script.
</p>

<p align="center">
  <a href="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/reims-gui-ci.yml"><img src="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/reims-gui-ci.yml/badge.svg" alt="reims-gui CI"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/reims-gui-release.yml"><img src="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/reims-gui-release.yml/badge.svg" alt="reims-gui release"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/releases/latest"><img src="https://img.shields.io/github/v/release/jniltinho/macos-kvm-gui?color=blue" alt="Latest release"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/releases"><img src="https://img.shields.io/github/downloads/jniltinho/macos-kvm-gui/total?color=blue" alt="Downloads"></a>
  <a href="https://go.dev/"><img src="https://img.shields.io/github/go-mod/go-version/jniltinho/macos-kvm-gui" alt="Go version"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/jniltinho/macos-kvm-gui?color=blue" alt="License"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/stargazers"><img src="https://img.shields.io/github/stars/jniltinho/macos-kvm-gui?style=social" alt="Stars"></a>
</p>

<p align="center">
  <img src="docs/screenshots/details.png" width="860" alt="macOS KVM with a VM selected">
</p>

<p align="center">
  <b><a href="https://github.com/jniltinho/macos-kvm-gui/releases/latest">⬇ Download the latest release</a></b>
</p>

> [!TIP]
> **Use [reims-gui](reims-gui/README.md), the Rust version of this app.** This repository has two apps: `reims-gui` in Rust (recommended, `reims-gui-v*` releases) and `macos-kvm-gui` in Go (this page, `v*` releases, no longer released). Both read the same VMs and settings.

> **For home lab use.** macOS is Apple's software, and its license only allows it on Apple-branded hardware. Make sure your use complies with Apple's macOS license terms. This project is not affiliated with Apple.

## Highlights

- **One binary.** OpenCore, Lilu, VirtualSMC, WhateverGreen and OVMF are embedded; no `git`, `python3` or OSX-KVM clone.
- **Verified downloads.** The recovery comes from Apple, checked against Apple's signature and every chunk's SHA-256.
- **A manager like VirtualBox's.** Menus, toolbar, a VM list, Details with a live Preview, a Settings window and a New VM wizard, in a light theme with VirtualBox's colors or a dark one.
- **Several VMs.** One folder per VM in the Workspace, move, export/import, and macOS on screen inside the app.
- **Guided install.** A step-by-step Install tab, and Host checks with one-click setup.
- **Offline install.** Download Apple's full installer once (Big Sur and later) and every VM of that version installs from a local disc, with no network; **Type it in the VM** does the typing, and **Unattended** goes all the way to Setup Assistant.
- **Metal without a spare GPU** *(experimental)*. [Reims vGPU](docs/reims-vgpu.md), a paravirtual GPU, renders macOS's Metal through the host's own Vulkan driver (NVIDIA, AMD or Intel); one click in the Host tab installs it.
- **Networking.** Up to four adapters per VM, NAT or bridged, with the adapter type and MAC address of your choice.
- **Scriptable.** The same binary is a [command line tool](docs/cli.md) and an [MCP server](docs/mcp.md) that lets AI agents see, type in and install VMs.

Sonoma 14 is validated end to end; Sequoia 15 and Big Sur 11 have installed from the offline installer; the other versions, High Sierra through Tahoe, are experimental. More in the [overview](docs/overview.md).

## Quick start

```sh
sudo apt install ./macos-kvm-gui_*_amd64.deb      # Ubuntu 24.04+, Debian 13+
sudo dnf install ./macos-kvm-gui-*.x86_64.rpm     # Fedora 39+
```

Both add "macOS KVM" to the applications menu, and a `.tar.gz` is there for other distributions. Needs Linux x86_64 with glibc 2.38+, VT-x/AMD-V, `qemu-system-x86`, `qemu-utils` and `pkexec`.

1. Open **macOS KVM** and make the **Host** checks green (**Set up host** fixes the basics).
2. Click **New**: the wizard creates the VM's folder and disk and downloads the macOS recovery from Apple.
3. Click **Start**. Inside the VM, erase the disk as APFS in Disk Utility and run **Reinstall macOS**; the **Install** tab walks you through it. To install offline instead, click **Download full installer** in Install step 2 before starting.

Full walkthrough: [getting started](docs/getting-started.md).

## reims-gui (Rust, preview)

[reims-gui](reims-gui/README.md) is this app rewritten in Rust for Ubuntu 24.04 and 26.04, built around Reims vGPU: macOS's Metal display runs inside the app's window, and VMware SVGA VMs show QEMU's own screen there too. It reads the same Workspace and settings, so both apps can be installed side by side.

```sh
sudo apt install ./reims-gui_0.1.4_amd64.deb   # one package for Ubuntu 24.04 and 26.04
```

Packages are on the [reims-gui releases](https://github.com/jniltinho/macos-kvm-gui/releases?q=reims-gui&expanded=true), built in Docker (`reims-gui/scripts/build.sh`). Guide and known limits: [docs/reims-gui.md](docs/reims-gui.md) and [PARITY.md](reims-gui/PARITY.md).

<p align="center">
  <img src="docs/screenshots/reims-gui-screen-desktop.png" width="860" alt="reims-gui with macOS on the Reims vGPU display">
</p>

## Documentation

[Overview](docs/overview.md) · [Getting started](docs/getting-started.md) · [Command line](docs/cli.md) · [MCP server](docs/mcp.md) · [Architecture](docs/architecture.md) · [GPU passthrough](docs/gpu-passthrough.md) · [Reims vGPU](docs/reims-vgpu.md) · [reims-gui (Rust)](docs/reims-gui.md) · [Troubleshooting](docs/troubleshooting.md) · [Development](docs/development.md) · [Screenshots](docs/screenshots/README.md) · [References](docs/references.md)

## Build

```sh
make build   # dist/macos-kvm-gui (Go 1.27+, Fyne build deps: see docs/development.md)
make test
```

The Go app's CI and release workflows no longer run on push or tags (run them by hand from the Actions tab); new builds and releases are those of reims-gui.

## License

BSD-3-Clause ([LICENSE](LICENSE)). The embedded boot files keep their own licenses, and Reims vGPU's separately published QEMU packages theirs ([THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)). Built on the work of [OSX-KVM](https://github.com/kholia/OSX-KVM), [acidanthera](https://github.com/acidanthera), [TianoCore](https://github.com/tianocore/edk2), [Dortania](https://dortania.github.io/) and [Fyne](https://fyne.io); see [references](docs/references.md).
