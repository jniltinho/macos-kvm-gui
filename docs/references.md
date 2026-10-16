# References

[Documentation index](README.md)

These links distinguish bundled components from background references and unsupported alternatives. Inclusion is not a compatibility promise; see [third-party notices](../THIRD_PARTY_NOTICES.md) for licenses of redistributed files.

## Boot and macOS-on-KVM projects

- [kholia/OSX-KVM](https://github.com/kholia/OSX-KVM) — Original inspiration for the KVM hardware/boot flow and compatible disk/media filenames; no clone or bundled upstream files are required now.
- [acidanthera/OpenCorePkg](https://github.com/acidanthera/OpenCorePkg) — Source of the embedded OpenCore bootloader, UEFI drivers, configuration sample and test validator.
- [OpenCore macrecovery](https://github.com/acidanthera/OpenCorePkg/tree/master/Utilities/macrecovery) — Apple recovery protocol utility ported to Go in `internal/core/recovery.go`.
- [acidanthera/Lilu](https://github.com/acidanthera/Lilu) — Embedded macOS kernel/process patching framework used by companion kexts.
- [acidanthera/VirtualSMC](https://github.com/acidanthera/VirtualSMC) — Embedded SMC emulation kext.
- [acidanthera/WhateverGreen](https://github.com/acidanthera/WhateverGreen) — Embedded graphics compatibility/patching kext; it does not supply acceleration for QEMU's emulated framebuffer.
- [ChefKissInc/NootRX](https://github.com/ChefKissInc/NootRX) — RDNA 2 discrete-GPU support patches, cited for cards this app refuses; not bundled or configured.
- [ChefKissInc/NootedRed](https://github.com/ChefKissInc/NootedRed) — Community AMD graphics support/patching kext, cited for APU discussion; not bundled or validated in this app's VMs.
- [Dortania OpenCore Install Guide](https://dortania.github.io/OpenCore-Install-Guide/) — Community reference for OpenCore configuration and macOS installation concepts.
- [Dortania GPU Buyers Guide](https://dortania.github.io/GPU-Buyers-Guide/) — Hardware compatibility reference; its [AMD GPU chapter](https://dortania.github.io/GPU-Buyers-Guide/modern-gpus/amd-gpu.html) informs the app's device-ID allow-list.
- [Coopydood/ultimate-macOS-KVM](https://github.com/Coopydood/ultimate-macOS-KVM) — Related macOS-on-KVM automation project, a reference rather than an app dependency.

## Firmware, virtualization and host integration

- [TianoCore EDK II](https://github.com/tianocore/edk2) — Upstream UEFI firmware project containing OVMF.
- [Debian ovmf package](https://packages.debian.org/trixie/ovmf) — Distribution providing the embedded 4M OVMF CODE/VARS pair; exact download/hash pins live in `assets.env`.
- [QEMU](https://www.qemu.org/) — External system emulator providing the KVM guest process and `qemu-img` disk/media tools.
- [QEMU virtual FAT disks (vvfat)](https://www.qemu.org/docs/master/system/images.html#virtual-fat-disk-images) — Documents presenting a host directory as a FAT disk, used read-only for the extracted EFI tree.
- [QEMU Machine Protocol specification](https://www.qemu.org/docs/master/interop/qmp-spec.html) — JSON greeting, negotiation, commands and events used by the app's QMP client.
- [QEMU QMP reference](https://www.qemu.org/docs/master/interop/qemu-qmp-ref.html) — Command/event definitions, including ACPI powerdown and block-I/O notifications.
- [QEMU host device assignment](https://gitlab.com/qemu-project/qemu/-/blob/master/docs/qdev-device-use.txt) — Upstream device-option documentation, including `vfio-pci,host=ADDR` under Host Device Assignment.
- [Linux VFIO documentation](https://docs.kernel.org/driver-api/vfio.html) — Kernel reference for IOMMU groups and userspace device access/isolation.
- [vendor-reset](https://github.com/gnif/vendor-reset) — External reset-workaround module cited for some AMD GPUs; not installed or managed by the app.

## Reims vGPU

- [steelbrain/reims-vgpu](https://github.com/steelbrain/reims-vgpu) — The paravirtual GPU library, its QEMU fork with `apple-gfx-pci`, and the measured x86-64 command line [Reims vGPU](reims-vgpu.md) follows; built at a pinned commit, not bundled in the app.
- [reims-util](https://github.com/steelbrain/reims-vgpu/tree/main/crates/reims-util) — Upstream's tool for its own OpenCore images, and `reims-ctl`, its VM runner; neither is used or installed by this app.
- [Apple ParavirtualizedGraphics](https://developer.apple.com/documentation/paravirtualizedgraphics) — Apple's framework for paravirtual GPUs, whose guest driver macOS ships and Reims vGPU implements the host side of.
- [Vulkan](https://www.vulkan.org/) — The host graphics API Reims vGPU renders through.

## Application, development and licensing

- [Fyne](https://fyne.io/) — Go desktop GUI toolkit used for all tabs/widgets and the custom theme.
- [Go](https://go.dev/) — Application language, compiler and test toolchain; the repository pins its required version in `go.mod`.
- [wgpu](https://github.com/gfx-rs/wgpu) — Graphics library cited when discussing Metal-dependent guest apps; not an app dependency or an emulated GPU driver.
- [Fission-AI/OpenSpec](https://github.com/Fission-AI/OpenSpec) — Spec-driven change workflow used in `openspec/` and validated in CI.
- [Apple software license agreements](https://www.apple.com/legal/sla/) — Authoritative place to obtain the license for the macOS version being used, including hardware and virtualization restrictions.

This is a **home lab** project, not an Apple-supported installation method or an exemption from Apple's license.
