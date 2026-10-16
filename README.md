<p align="center">
  <img src="internal/assets/logo/logo.svg" width="112" alt="macos-kvm-gui logo">
</p>

<h1 align="center">macOS KVM</h1>

<p align="center">
  <b>Run macOS on Linux with KVM, from a single desktop app.</b><br>
  Download, install and boot macOS in a few clicks: OpenCore and OVMF are built in, nothing else to clone or script.
</p>

<p align="center">
  <a href="https://github.com/jniltinho/macos-kvm-gui/releases/latest"><img src="https://img.shields.io/github/v/release/jniltinho/macos-kvm-gui?color=blue" alt="Latest release"></a>
  <a href="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/ci.yml"><img src="https://github.com/jniltinho/macos-kvm-gui/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/jniltinho/macos-kvm-gui?color=blue" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-Linux%20x86__64-blue" alt="Linux x86_64">
</p>

<p align="center">
  <img src="docs/screenshots/details.png" width="860" alt="macOS KVM with a VM selected">
</p>

> **For home lab use.** macOS is Apple's software, and its license only allows it on Apple-branded hardware. Make sure your use complies with Apple's macOS license terms. This project is not affiliated with Apple.

## Why macOS KVM?

Running macOS under KVM usually means cloning repositories, running Python downloaders, converting images, editing shell scripts and fixing OpenCore by hand. macOS KVM turns all of that into a guided desktop app:

- **One binary, no setup.** OpenCore 1.0.8, Lilu, VirtualSMC, WhateverGreen and OVMF are embedded, built from their official BSD-licensed releases. You don't need `git`, `python3` or an OSX-KVM clone.
- **Safe downloads.** The recovery comes straight from Apple, and the app checks Apple's signature and the SHA-256 of every chunk before using it.
- **macOS inside the app.** The VM's screen shows in a VirtualBox-style window with keyboard and mouse, in its own window or full screen (Ctrl+Alt+F is the host key), over a private VNC socket with no network port.
- **Several VMs, VirtualBox style.** A New VM wizard (name and version, hardware, storage), a VM list with status, and each VM in its own folder inside the Workspace (`~/macOS-VMs/<name>`). VMs can be moved, exported as a `.tar` with everything they need, and imported on another host. The list lives in a small SQLite database.
- **Light, dark or system theme**, chosen in Settings and applied right away.
- **Scriptable.** The same binary is a command line tool (`check`, `setup`, `download`, `disk create`, `vm start`...), so installs can be automated. See [docs/cli.md](docs/cli.md).
- **Boots like a Mac.** It generates OpenCore's configuration for you at 1920x1080 and, once macOS is installed, boots without a menu. Each VM gets its own Mac identity.
- **Explains what's wrong.** The Host tab checks KVM, QEMU, `ignore_msrs` (one-click fix) and conflicting VirtualBox/VMware VMs. QEMU errors become plain messages.
- **Careful with your data.** It never overwrites a disk, the NVRAM or existing media. A lock keeps two operations or two app instances from touching the same workspace, and SSH is only reachable from `127.0.0.1`.
- **GPU passthrough** *(experimental)*. It can hand a macOS-supported AMD GPU to the VM for real Metal acceleration, and it refuses setups that would leave your host without a display.

## Quick start

1. Download the [latest release](https://github.com/jniltinho/macos-kvm-gui/releases/latest) and install it:

   ```sh
   sudo apt install ./macos-kvm-gui_*_amd64.deb   # adds "macOS KVM" to your applications menu
   ```

   Or use the standalone binary from the `.tar.gz`. Both need Linux x86_64 with glibc 2.38+ (Ubuntu 24.04+, Debian 13+), a CPU with VT-x/AMD-V, and `qemu-system-x86`, `qemu-utils` and `pkexec`.

2. Open **macOS KVM** and follow the tabs:
   - **Host:** all checks green.
   - **New:** the wizard asks for a name and macOS version, suggests CPU and RAM for this host, creates the VM's folder and disk, and downloads the recovery from Apple.
   - **Start**, and macOS shows in the **Screen** tab.

3. Inside the VM, erase the disk as APFS in Disk Utility and run **Reinstall macOS**. When macOS boots from the disk, remove the install media from the Install tab.

The full walkthrough is in [docs/getting-started.md](docs/getting-started.md).

## Screenshots

| New VM wizard | Install |
|---|---|
| [![New VM](docs/screenshots/new-vm.png)](docs/screenshots/new-vm.png) | [![Install](docs/screenshots/install.png)](docs/screenshots/install.png) |
| **Host checks** | |
| [![Host](docs/screenshots/host.png)](docs/screenshots/host.png) | |
| **Settings** | **GPU passthrough** |
| [![Settings](docs/screenshots/settings.png)](docs/screenshots/settings.png) | [![GPU](docs/screenshots/gpu.png)](docs/screenshots/gpu.png) |

## How it works

[![Architecture](docs/diagrams/architecture.png)](docs/diagrams/architecture.svg)

At every start the app extracts the embedded OpenCore tree into the workspace, generates `config.plist` from your settings, and starts QEMU. QEMU sees OpenCore as a read-only FAT disk, and the app controls the VM through QMP. See [architecture](docs/architecture.md).

## macOS support

| macOS | Status |
|---|---|
| Sonoma 14 | ✅ Validated end to end (14.8.9 on Ubuntu 24.04, QEMU 8.2.2) |
| High Sierra 10.13 → Sequoia 15, Tahoe 26 | 🧪 Experimental: downloadable and configured, not validated |

There is no graphics acceleration without a passed-through AMD GPU, because macOS has no driver for QEMU's emulated displays. The app shows how to keep the desktop smooth anyway; see [video performance](docs/troubleshooting.md).

## Compared with

| | macOS KVM | [OSX-KVM](https://github.com/kholia/OSX-KVM) | [ultimate-macOS-kvm](https://github.com/Coopydood/ultimate-macOS-kvm) |
|---|---|---|---|
| Interface | Desktop app (GUI) | Shell scripts | Python menu (`main.py`) |
| OpenCore | Built in, `config.plist` generated at every start | Prebuilt image, edited by hand | Image edited by assistants (NBD, GenSMBIOS) |
| Recovery download | Built in (Go), Apple signature verified | Python script | — |
| Needs `git` / `python3` | No | Yes | Yes |
| GPU passthrough | Assistant with safety checks (experimental) | Manual notes | VFIO and IOMMU assistants |

## Documentation

- [Getting started](docs/getting-started.md)
- [Architecture](docs/architecture.md)
- [GPU passthrough](docs/gpu-passthrough.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Command line](docs/cli.md): automate installs from a terminal
- [Development](docs/development.md): build, tests, boot assets, releases
- [References](docs/references.md): the projects this builds on

## Building from source

```sh
sudo apt install gcc pkg-config libgl1-mesa-dev xorg-dev libwayland-dev libxkbcommon-dev wayland-protocols qemu-utils
make build     # dist/macos-kvm-gui
make test      # gofmt, go vet, race tests (downloads OpenCore's ocvalidate once)
make package   # dist/: .tar.gz, .deb and SHA-256 files
```

Requires Go 1.27+. Build output goes to `dist/`, which is never committed.

## Credits and license

macOS KVM started as a GUI for [OSX-KVM](https://github.com/kholia/OSX-KVM) by Dhiru Kholia, and it builds on the work of [acidanthera](https://github.com/acidanthera) (OpenCore, Lilu, VirtualSMC, WhateverGreen, macrecovery), [TianoCore](https://github.com/tianocore/edk2) (OVMF), [Dortania](https://dortania.github.io/) and [Fyne](https://fyne.io). The full list is in [docs/references.md](docs/references.md).

The app is BSD-3-Clause licensed ([LICENSE](LICENSE)). The embedded boot files keep their own licenses ([THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)). No file from OSX-KVM is included.
