# Overview

[Documentation index](README.md)

## Features

Running macOS under KVM usually means cloning repositories, running Python downloaders, converting images, editing shell scripts and fixing OpenCore by hand. macOS KVM turns all of that into a guided desktop app:

- **One binary, no setup.** OpenCore 1.0.8, Lilu, VirtualSMC, WhateverGreen and OVMF are embedded, built from their official BSD-licensed releases. You don't need `git`, `python3` or an OSX-KVM clone.
- **Safe downloads.** The recovery comes straight from Apple, and the app checks Apple's signature and the SHA-256 of every chunk before using it.
- **macOS inside the app.** The VM's screen shows in a VirtualBox-style window with keyboard and mouse, in its own window or full screen (Ctrl+Alt+F is the host key), over a private VNC socket with no network port.
- **Several VMs, VirtualBox style.** A manager modelled on VirtualBox's (menus, toolbar, Details with Preview, a Settings window), a New VM wizard, a VM list with status, and each VM in its own folder inside the Workspace (`~/macOS-VMs/<name>`). VMs can be moved, exported as a `.tar` with everything they need, and imported on another host. The list lives in a small SQLite database.
- **Light, dark or system theme**, chosen in File › Preferences and applied right away.
- **Scriptable.** The same binary is a command line tool (`check`, `setup`, `download`, `disk create`, `vm start`...), so installs can be automated. See [docs/cli.md](cli.md).
- **Boots like a Mac.** It generates OpenCore's configuration for you at 1920x1080 and, once macOS is installed, boots without a menu. Each VM gets its own Mac identity.
- **Explains what's wrong.** The Host tab checks KVM, QEMU, `ignore_msrs` (one-click fix) and conflicting VirtualBox/VMware VMs. QEMU errors become plain messages.
- **Careful with your data.** It never overwrites a disk, the NVRAM or existing media. A lock keeps two operations or two app instances from touching the same workspace, and SSH is only reachable from `127.0.0.1`.
- **GPU passthrough** *(experimental)*. It can hand a macOS-supported AMD GPU to the VM for real Metal acceleration, and it refuses setups that would leave your host without a display.

## How it works

[![Architecture](diagrams/architecture.png)](diagrams/architecture.svg)

At every start the app extracts the embedded OpenCore tree into the VM's folder, generates `config.plist` from your settings, and starts QEMU. QEMU sees OpenCore as a read-only FAT disk, and the app controls the VM through QMP. See [architecture](architecture.md).

## macOS support

| macOS | Status |
|---|---|
| Sonoma 14 | ✅ Validated end to end (14.8.9 on Ubuntu 24.04, QEMU 8.2.2) |
| High Sierra 10.13 → Sequoia 15, Tahoe 26 | 🧪 Experimental: downloadable and configured, not validated |

There is no graphics acceleration without a passed-through AMD GPU, because macOS has no driver for QEMU's emulated displays. The app shows how to keep the desktop smooth anyway; see [video performance](troubleshooting.md).

## Compared with

| | macOS KVM | [OSX-KVM](https://github.com/kholia/OSX-KVM) | [ultimate-macOS-kvm](https://github.com/Coopydood/ultimate-macOS-kvm) |
|---|---|---|---|
| Interface | Desktop app (GUI) | Shell scripts | Python menu (`main.py`) |
| OpenCore | Built in, `config.plist` generated at every start | Prebuilt image, edited by hand | Image edited by assistants (NBD, GenSMBIOS) |
| Recovery download | Built in (Go), Apple signature verified | Python script | — |
| Needs `git` / `python3` | No | Yes | Yes |
| GPU passthrough | Assistant with safety checks (experimental) | Manual notes | VFIO and IOMMU assistants |

