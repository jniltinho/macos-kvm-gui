# Design

## Context

Facts gathered on the reference host (Ubuntu 24.04, Ryzen 9 9900X, kernel 6.8): AMD-Vi is active by default with 36 IOMMU groups, so no kernel command line change is needed. The x16 slot holds the RTX 4070 (`boot_vga=1`, the current host display) in a clean group with only its GPU and audio functions. The V100 slot shares a group with five PCI bridges and a network controller (`0000:08:00.0`, class `0x0280`). The Ryzen iGPU (`1002:13c0`) also uses `amdgpu` and can become the host display if the BIOS makes it primary. Neither `amdgpu` nor `vfio-pci` is in the initramfs. `/dev/vfio/vfio` is 0666, but per-group nodes are created as root-only unless a udev rule says otherwise. The user's `RLIMIT_MEMLOCK` is about 23 GiB (systemd's default of 1/8 of RAM).

Upstream (`boot-passthrough.sh`, `notes.md`) uses `-vga none`, `vfio-pci,host=…,multifunction=on,x-no-kvm-intx=on` for the GPU, a second `vfio-pci` for its audio function and `-display none`, and tells the user to blacklist `amdgpu`.

## Goals / Non-Goals

**Goals:**
- Never leave the host without a display: no driver blacklist, and the boot display GPU is never reservable.
- Keep everything the user can break reversible from the app (release action).

**Non-Goals:**
- Editing OpenCore (`agdpmod=pikera`, device-id spoofing), VBIOS ROM files, `vendor-reset`, USB controller passthrough.
- Changing the kernel command line or GRUB.

## Decisions

1. **Detection from sysfs.** Every `/sys/bus/pci/devices/*` whose `class` starts with `0x03` is a GPU. Vendor and device come from the `vendor` and `device` files, the group from the `iommu_group` symlink, the driver from the `driver` symlink and the boot display flag from `boot_vga`. The name comes from `lspci -s <addr>` when it is available, so no PCI ID database is bundled.
2. **Compatibility table by device ID** (vendor `1002`, per the Dortania GPU buyers guide):
   - **Supported:** Polaris `67df` `67ef` `67ff`; Vega `687f` `66af`; Navi 10/14 `731f` `7340`; Navi 21 `73bf` (RX 6800/6800 XT/6900 XT); Navi 23 `73ff` (RX 6600/6600 XT).
   - **Unsupported:** Navi 24 `743f` (RX 6400/6500); RDNA3 `744c` `7448` `747e` `7480` (RX 7000); every NVIDIA (`10de`) and Intel (`8086`) GPU. The cards that need a device-id spoof or the NootRX kext are also unsupported, with their own reason, because this app does not edit OpenCore: Lexa `699f`, RX 580 2048SP `6fdf`, Navi 21 XTXH/RX 6950 XT `73af` `73a5`, Navi 22 `73df` (RX 6700/6750), RX 6650 XT `73ef`.
   - **Unknown:** any other AMD ID, which includes every APU iGPU (for example the Ryzen 9000 iGPU `13c0`). It is **not eligible**, because an allow-list cannot reserve the host's own iGPU by mistake; the reason points to the guide. Dortania lists the Zen 4 and Zen 5 iGPUs as unsupported, and the NootedRed kext covers only Vega APUs and is still work in progress.
3. **Eligibility** is computed from sysfs, so it is unit-testable with a fake tree. A GPU is refused when it is the boot display (`boot_vga`), when one of its DRM connectors (`/sys/class/drm/card*-*/status`) is `connected`, or when no other GPU with a bound driver remains for the host. `boot_vga` alone is not enough, because it reflects the firmware's default VGA device, not the current desktop. The slot may only hold display (`0x03`) and audio (`0x0403`) functions: AMD cards can expose USB-C/UCSI functions, and passing those is out of scope. The IOMMU group of **every** slot function must hold only that slot's functions and PCI bridges (`0x0604`). No device outside the slot may share an ID with the slot's functions, because `vfio-pci ids=` matches globally.
4. **Reservation through `vfio-pci ids=`** plus `softdep` lines that load `vfio-pci` before `amdgpu`, `radeon` and `snd_hda_intel` (the HDMI audio driver), in `/etc/modprobe.d/macos-kvm-gui-vfio.conf`, with `disable_vga=1` as in the upstream notes. A udev rule `SUBSYSTEM=="vfio", GROUP="kvm", MODE="0660"` lets the unprivileged QEMU open `/dev/vfio/<group>`; upstream's rule has no `MODE`. The script then runs `update-initramfs -u -k all`, because the plain `-u` only updates the newest kernel.
5. **Privileged script stays constant and is fail-safe.** The IDs go to `pkexec /bin/sh -c '<script>' sh <ids>` as `$1`, never interpolated. Go validates each ID with `^[0-9a-f]{4}:[0-9a-f]{4}$`, and the script re-checks the character set with a `case` pattern, which a test runs in the real shell with hostile input. A `flock` on `/run/lock/macos-kvm-gui-host.lock` serializes reserve and release. If the initramfs rebuild fails, the script deletes its files, rebuilds again and exits non-zero. The host runner appends the last stderr line to the error so the UI shows it. Exit code 126 of pkexec means cancelled. Afterwards the app re-reads the files.
6. **QEMU arguments with a GPU:** the GPU function gets `vfio-pci,host=<addr>,multifunction=on,x-no-kvm-intx=on`, and every other slot function gets `vfio-pci,host=<addr>`. `vmware-svga` **stays**: QEMU's GTK window only delivers keyboard and mouse input through a graphical console, so dropping every emulated display (upstream's `-vga none -display none`) would leave the VM without input unless evdev or USB passthrough is configured. It also shows the OpenCore and boot screens. macOS drives the AMD card's outputs once its driver loads.
7. **Start-time guards:** eligibility is re-checked; the macOS version must reach the GPU's minimum (Navi 10/14: Catalina; Navi 21: Big Sur; Navi 23: Monterey; Radeon VII: Mojave); every slot function must be bound to `vfio-pci`; each `/dev/vfio/<group>` node must open read-write as the launching user; and the soft `RLIMIT_MEMLOCK` must be at least the VM's RAM (or unlimited). The app never falls back to running QEMU as root.
8. **Config:** a new optional `gpu` field holds the PCI address of the GPU function (`0000:03:00.0`), validated with `^[0-9a-f]{4}:[0-9a-f]{2}:[0-9a-f]{2}\.[0-7]$`. Empty means no passthrough.

## Risks / Trade-offs

- [Never validated with a real AMD card] → Unit tests cover detection, eligibility, the reservation command and the argument building. The README states the limitation.
- [Navi cards may show a black screen without `agdpmod=pikera`] → Documented in the README as a manual OpenCore tweak; editing OpenCore is out of scope.
- [AMD reset bug on Polaris, Vega and Navi 10: the GPU fails after the VM restarts] → Documented; the user needs to reboot the host or install `vendor-reset`.
- [The firmware picks the dGPU as boot display once an AMD card sits in the x16 slot] → That card is refused as the boot display, and the reason tells the user to set the iGPU as primary in the BIOS.
- [Two displays in macOS (the vmware-svga console and the AMD card)] → macOS uses the AMD card once its driver loads; if the desktop stays on the QEMU window, the user sets the AMD monitor as the main display in System Settings › Displays. The README documents this.
- [A broken reservation leaves the host unable to boot to a desktop] → Recovery is documented: add `modprobe.blacklist=vfio_pci` to the kernel line in GRUB, or delete the two files from a console and run `update-initramfs -u -k all`.

## Migration Plan

Not applicable: the feature is opt-in. Hosts that never reserve a GPU see no change. The release action undoes the reservation.
