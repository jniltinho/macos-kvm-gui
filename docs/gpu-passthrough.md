# GPU passthrough

[Documentation index](README.md)

**Untested on real hardware.** The reference host has no supported AMD card. Unit tests cover detection, refusals, privileged command construction and QEMU arguments, not a working passthrough installation. Do not buy hardware solely on the strength of this app's allow-list. Keep a working host display and console recovery route.

Passthrough gives macOS a physical AMD GPU for acceleration/Metal. This is different from improving QEMU's unaccelerated framebuffer. It is optional; for initial installation use **None (emulated display)**.

[![GPU passthrough](diagrams/gpu-passthrough.png)](diagrams/gpu-passthrough.svg)

## Compatibility: what this app accepts

Compatibility is a **vendor/device-ID allow-list**, not a model-name match or a guarantee for every board. All supported IDs below have vendor `1002` (AMD). Consult the [Dortania AMD GPU guide](https://dortania.github.io/GPU-Buyers-Guide/modern-gpus/amd-gpu.html) for board and macOS-version caveats.

| Family / commonly associated models | Allowed device IDs | Additional minimum checked by the app |
| --- | --- | --- |
| Polaris: RX 460/470/480/550/560/570/580/590 variants | `67df`, `67ef`, `67ff` | No extra version-family guard |
| Vega 56/64 | `687f` | No extra version-family guard |
| Radeon VII | `66af` | Mojave (native driver needs 10.14.5+) |
| Navi 10/14: RX 5500/5600/5700 | `731f`, `7340` | Catalina (10.15.1+) |
| Navi 21: RX 6800/6800 XT/6900 XT | `73bf` | Big Sur (11.4+) |
| Navi 23: RX 6600/6600 XT | `73ff` | Monterey (12.1+) |

The guards compare **release families only**; they do not inspect the installed guest's point version. The selected version in Settings must match your guest.

Refused by the app:

- Every NVIDIA (`10de`) and Intel (`8086`) GPU, regardless of historical macOS support.
- Navi 24 `743f` (RX 6400/6500) and RDNA3 `744c`, `7448`, `747e`, `7480` (RX 7000).
- Patch-dependent IDs: `699f` (Lexa), `6fdf` (RX 580 2048SP), `73af`/`73a5` (Navi 21 XTXH/RX 6950 XT), `73df` (RX 6700/6750), `73ef` (RX 6650 XT). The app does not provide device-ID spoofing or NootRX.
- Other AMD IDs are **unknown**, hence ineligible, including Ryzen iGPUs. NootedRed is not bundled or configured; it is not a supported VM workaround here.

## Eligibility rules

The GPU tab reads sysfs; `lspci` is optional for friendly names. A card must meet all these conditions:

1. IOMMU groups exist (enable AMD-Vi or VT-d in BIOS/UEFI).
2. Its ID is supported; it is not the firmware boot display (`boot_vga=1`). Set another GPU/iGPU as primary in BIOS.
3. None of its DRM connectors reports a connected monitor while the host drives it. Move the host monitor to another GPU before reservation.
4. Another GPU in a different slot has a bound driver other than `vfio-pci`, so a host display remains.
5. Every function in its slot is a display or audio function. USB-C/UCSI controllers and other extra functions are refused.
6. Each function's IOMMU group contains only that slot's functions and PCI bridges. A network controller or other endpoint from another slot disqualifies it; use a different slot instead of bypassing isolation checks.
7. No device outside the slot shares any of its function IDs. Reservation is by ID and would claim the duplicate too, including identical audio functions.

## Reserve, reboot, start, release

Stop the VM, select an eligible card in **GPU**, then click **Reserve for VFIO** and confirm. The selection is saved in `config.json`; reservation changes the host globally and persists even if you later select None.

One `pkexec` operation writes:

`/etc/modprobe.d/macos-kvm-gui-vfio.conf`:

```text
options vfio-pci ids=<comma-separated unique GPU/audio IDs> disable_vga=1
softdep amdgpu pre: vfio-pci
softdep radeon pre: vfio-pci
softdep snd_hda_intel pre: vfio-pci
```

`/etc/udev/rules.d/99-macos-kvm-gui-vfio.rules`:

```text
SUBSYSTEM=="vfio", GROUP="kvm", MODE="0660"
```

The rule applies to VFIO devices generally, not only this card. The script holds `/run/lock/macos-kvm-gui-host.lock` and runs `update-initramfs -u -k all`. It **does not blacklist** `amdgpu`, `radeon` or any other driver, hot-unbind a live GPU, edit GRUB, or set up `vendor-reset`. IDs are validated and passed as arguments, not interpolated into shell source.

If the initramfs rebuild fails, the reservation files are removed and a second rebuild is attempted. Fix the cause (for example a full `/boot`) and ensure rebuilding succeeds before rebooting. This is not a backup/restore mechanism for an earlier reservation; the app owns one pair of host files.

**Reboot the host.** Check that the GPU/audio functions now use `vfio-pci`. Use a monitor connected to the passed-through card for macOS output; a host DRM-connected monitor prevents eligibility before reservation, whereas a VFIO-bound card is not driven by host DRM.

On **Start**, the app rechecks eligibility and:

- GPU presence and minimum selected macOS family;
- `vfio-pci` binding for **every** slot function;
- read/write access to each `/dev/vfio/<group>` as the current user;
- soft `RLIMIT_MEMLOCK` at least equal to guest RAM (or unlimited).

QEMU stays unprivileged. It receives `vfio-pci,host=<GPU>,multifunction=on,x-no-kvm-intx=on`, then one `vfio-pci,host=<function>` for each remaining slot function. `vmware-svga` remains for boot screens and keyboard/mouse in the QEMU window. Once the AMD driver loads, macOS drives the physical card's monitor. Set that display as main in **System Settings › Displays** if necessary. OpenCore automatically adds `agdpmod=pikera`; manual edits to the extracted config are overwritten.

To undo, stop the VM and choose **Release reservation**. It removes both files and rebuilds all initramfs images; **reboot again** to restore normal driver binding. Select **None (emulated display)** if you no longer want passthrough. Releasing does not clear the saved GPU selection, and selecting None does not release a reservation.

## Recover a broken reservation

If the host no longer reaches its desktop:

1. At GRUB, press **e** and append `modprobe.blacklist=vfio_pci` to the kernel line for this boot only. Boot using GRUB's displayed shortcut.
2. In the app, use **Release reservation**, then reboot normally. Or, from a working console/recovery environment on the installed host:

   ```sh
   sudo rm -f /etc/modprobe.d/macos-kvm-gui-vfio.conf \
     /etc/udev/rules.d/99-macos-kvm-gui-vfio.rules
   sudo update-initramfs -u -k all
   ```

3. Confirm the rebuild succeeds before rebooting. If it fails, fix `/boot` space or the reported initramfs error and retry. This removes only the app's reservation, not unrelated VFIO configuration.

The reservation scripts assume Debian/Ubuntu's `update-initramfs`, `flock` and a functioning polkit authentication agent. Other initramfs tooling is not implemented.

## Known issues

- Polaris, Vega and RX 5000 cards can encounter the AMD reset bug after guest shutdown/restart; a host reboot may be needed. `vendor-reset` is an external, manual option, not managed by this app.
- Navi black screens can relate to board policy; `agdpmod=pikera` is already generated when a GPU is selected, but it does not guarantee a working board or driver.
- The physical AMD display and emulated console are separate displays. The QEMU window is not a mirror of the GPU output.
- No ROM-file configuration, device-ID spoofing, physical USB-controller passthrough, ACS override or NVIDIA enablement is provided.
- Unit-test success does not validate PCI isolation, GPU reset, firmware, monitor routing or Metal on real hardware. **Full passthrough remains untested.**
