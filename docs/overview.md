# Overview

[Documentation index](README.md)

## Features

Running macOS under KVM usually means cloning repositories, running Python downloaders, converting images, editing shell scripts and fixing OpenCore by hand. macOS KVM turns all of that into a guided desktop app:

- **One binary, no setup.** OpenCore 1.0.8, Lilu, VirtualSMC, WhateverGreen and OVMF are embedded, built from their official BSD-licensed releases. You don't need `git`, `python3` or an OSX-KVM clone.
- **Safe downloads.** The recovery comes straight from Apple, and the app checks Apple's signature and the SHA-256 of every chunk before using it.
- **macOS inside the app.** The VM's screen shows in a VirtualBox-style window with keyboard and mouse, in its own window or full screen (Ctrl+Alt+F is the host key), over a private VNC socket with no network port.
- **A manager like VirtualBox's.** File / Machine / Help menus, a toolbar with large icons, the VM list with each VM's state, Details in sections with a live Preview, a Settings window with categories (General, System, Display, Storage, Audio, Network, USB) and a New VM wizard in VirtualBox's expert style. The icons are original and every corner is square.
- **Several VMs.** Each VM has its own folder inside the Workspace (`~/macOS-VMs/<name>`). VMs can be moved, exported as a `.tar` with everything they need, and imported on another host. The list lives in a small SQLite database that upgrades itself.
- **Guided install.** The Install tab is a stepper (VM folder, recovery, disk, install, clean up) that shows what is done and what comes next. The Host and GPU tabs use the same cards.
- **Offline install.** **Download full installer** fetches Apple's `InstallAssistant.pkg` once (Big Sur and later, 12–18 GB), checks every chunk against Apple's integrity data and keeps it as a local disc shared by every VM of that version (**File › Offline installers** lists them). A VM started with its install media gets the disc and installs with no network: **Type it in the VM** opens the recovery's Terminal and types the line, and **Unattended** also erases the empty disk and runs Apple's installer with no questions, up to Setup Assistant. The Install tab says which mode a VM uses (**Install mode: Offline** or **Online**).
- **Your keyboard.** Keys reach macOS by physical position, and the recovery and installer use the host's layout (for example Brazilian ABNT2), in English.
- **Networking.** Up to four adapters per VM, like VirtualBox's Adapter 1–4: NAT or bridged to a host bridge, an adapter type macOS has a driver for (`virtio-net-pci`, `vmxnet3`, Intel 82545EM) and a MAC address of your choice. Existing VMs keep their adapter and MAC.
- **Light, dark or system theme**, chosen in File › Preferences and applied right away. The light theme uses VirtualBox's colors.
- **Scriptable.** The same binary is a command line tool (`check`, `setup`, `download`, `download --full`, `installer`, `disk create`, `vm start`...), so installs can be automated. See [docs/cli.md](cli.md).
- **For AI agents.** `macos-kvm-gui mcp` is an MCP server (stdio only): Claude Code, Codex or another agent can see a VM's screen, type and click in it, start and stop it and install macOS offline, while the app shows the same VM. See [docs/mcp.md](mcp.md).
- **Boots like a Mac.** It generates OpenCore's configuration for you at 1920x1080 and, once macOS is installed, boots without a menu. Each VM gets its own Mac identity.
- **Explains what's wrong.** The Host tab checks KVM, QEMU, `ignore_msrs` (one-click fix) and conflicting VirtualBox/VMware VMs. QEMU errors become plain messages.
- **Careful with your data.** It never overwrites a disk, the NVRAM or existing media. A lock keeps two operations or two app instances from touching the same VM folder, it refuses to start a disk another QEMU already uses, and the SSH forward is only reachable from `127.0.0.1`.
- **Packages for Ubuntu, Debian and Fedora.** Each release has a `.deb`, an `.rpm` (Fedora 39+) and a `.tar.gz`, with SHA-256 files.
- **GPU passthrough** *(experimental)*. It can hand a macOS-supported AMD GPU to the VM for real Metal acceleration, and it refuses setups that would leave your host without a display. NVIDIA Kepler cards work natively up to Big Sur and, with **root-patch mode** and OpenCore Legacy Patcher inside macOS, on Monterey to Sequoia.

## How it works

[![Architecture](diagrams/architecture.png)](diagrams/architecture.svg)

At every start the app extracts the embedded OpenCore tree into the VM's folder, generates `config.plist` from your settings, and starts QEMU. QEMU sees OpenCore as a read-only FAT disk, and the app controls the VM through QMP. See [architecture](architecture.md).

## macOS support

| macOS | Status |
|---|---|
| Sonoma 14 | ✅ Validated end to end (14.8.9 on Ubuntu 24.04, QEMU 8.2.2) |
| Sequoia 15 | 🧪 Experimental: 15.8.1 installed from the offline installer, guided and unattended, to the desktop (4 cores × 2 threads, 16 GB); not used day to day yet |
| Big Sur 11 | 🧪 Experimental: 11.7.11 installed from the offline installer, unattended through the MCP server, to Setup Assistant |
| High Sierra 10.13 → Catalina 10.15, Monterey 12, Ventura 13, Tahoe 26 | 🧪 Experimental: downloadable and configured, not validated |

Every version needs a core count of 1, 2, 4, 8 or 16 (2, 4, 8, 16 or 32 vCPUs with 2 threads per core); see the [table of valid processor counts](troubleshooting.md#valid-processor-counts). There is no graphics acceleration without a passed-through AMD GPU, because macOS has no driver for QEMU's emulated displays. The app shows how to keep the desktop smooth anyway; see [video performance](troubleshooting.md).

## Compared with

| | macOS KVM | [OSX-KVM](https://github.com/kholia/OSX-KVM) | [ultimate-macOS-kvm](https://github.com/Coopydood/ultimate-macOS-kvm) |
|---|---|---|---|
| Interface | Desktop app (GUI) | Shell scripts | Python menu (`main.py`) |
| OpenCore | Built in, `config.plist` generated at every start | Prebuilt image, edited by hand | Image edited by assistants (NBD, GenSMBIOS) |
| Recovery download | Built in (Go), Apple signature verified | Python script | — |
| Needs `git` / `python3` | No | Yes | Yes |
| GPU passthrough | Assistant with safety checks (experimental) | Manual notes | VFIO and IOMMU assistants |

