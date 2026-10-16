# GPU passthrough

[Documentation index](README.md)

**Untested on real hardware.** The reference host has no supported AMD card. Unit tests cover detection, refusals, privileged command construction and QEMU arguments, not a working passthrough installation. Do not buy hardware solely on the strength of this app's allow-list. Keep a working host display and console recovery route.

Passthrough gives macOS a physical AMD GPU, or an NVIDIA Kepler GPU (see [NVIDIA Kepler and root-patch mode](#nvidia-kepler-and-root-patch-mode)), for acceleration/Metal. This is different from improving QEMU's unaccelerated framebuffer. It is optional; for initial installation use **None (emulated display)**.

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

NVIDIA Kepler GPUs (vendor `10de`) are accepted by an explicit device-ID list, OCLP 2.5.1's `kepler_ids` (160 IDs: GK104, GK106, GK107, GK110/GK110B, GK180, GK208/GK208B, GK210), never by marketing name: a GT 730 can be Fermi (`0f02`) and a GTX 745/750/750 Ti is Maxwell (`1380`–`1382`). They work natively up to Big Sur and need root-patch mode from Monterey to Sequoia; the VM refuses to start otherwise.

Accepted only with a patch kext you supply: `699f` (Lexa), `6fdf` (RX 580 2048SP), `73af`/`73a5` (Navi 21 XTXH/RX 6950 XT), `73df` (RX 6700/6750), `73ef` (RX 6650 XT). macOS has no driver for these IDs. They become eligible once `kexts` in `settings.yaml` lists at least one item (see [patch-dependent AMD GPUs](#patch-dependent-amd-gpus)); the app neither ships such a kext nor checks which one you list.

Refused by the app:

- NVIDIA GPUs other than Kepler (Fermi, Maxwell, Pascal and newer: GTX 750, 900, 10, 16, RTX) and every Intel (`8086`) GPU.
- Navi 24 `743f` (RX 6400/6500) and RDNA3 `744c`, `7448`, `747e`, `7480` (RX 7000).
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

Stop the VM, select an eligible card in **GPU**, then click **Reserve for VFIO** and confirm. The selection is saved with the VM; reservation changes the host globally and persists even if you later select None.

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

## Patch-dependent AMD GPUs

For an RX 6700 XT, for example (`1002:73df`):

1. Pick a kext that supports the card, read its license and documentation, and put its release zip's URL (and SHA-256) in `~/.config/macos-kvm-gui/settings.yaml` (File › Preferences › Open settings.yaml):

   ```yaml
   kexts:
     - url: https://example.com/TheKext-1.0.zip
       sha256: <64 hex digits>
   disable: [WhateverGreen]   # only if the kext's documentation says it replaces WhateverGreen
   ```

2. The GPU tab now lists the card as eligible (if every other rule below passes). Reserve it, reboot the host, choose it in the VM's settings and connect a monitor to it.
3. Start the VM. The app downloads the zip first, then OpenCore loads the kext after the built-in ones ([extra kexts](architecture.md#extra-kexts)).

Try it with a disposable VM or a backup first: a kext that does not fit the card or the macOS version can stop the guest from booting. Removing the item (or **Reset to default**) restores the earlier boot. A VM that has such a GPU refuses to start while `kexts` is empty.

## NVIDIA Kepler and root-patch mode

Apple dropped the Kepler drivers after Big Sur. OpenCore Legacy Patcher (OCLP) puts them back inside macOS ("Post-Install Root Patch"), which needs OpenCore settings the app does not use by default. **Root-patch mode** (GPU tab, per VM, off by default, experimental; CLI `config set root-patch=true`) turns them on, for **Monterey 12 to Sequoia 15** only (Big Sur needs nothing; Tahoe is left out until OCLP supports it in a stable release). It matches OCLP 2.5.1's own profile for Kepler:

| Setting | Value |
| --- | --- |
| `csr-active-config` | `0x803` (`AwgAAA==`): unsigned kexts, writable system volume, unauthenticated root |
| Kexts (from OCLP 2.5.1, after Lilu) | AMFIPass 1.4.1 (instead of disabling AMFI), RSRHelper 1.0.2, RestrictEvents 1.1.7, KDKlessWorkaround 1.0.0 (MinKernel 22.3.0) |
| boot-args | `ipc_control_port_options=0 -nokcmismatchpanic` |
| NVRAM `revblock` | `media` (RestrictEvents blocks `mediaanalysisd`, which hangs these GPUs) |

The four kexts are **downloaded when the mode is turned on**, from OCLP's repository at the 2.5.1 commit, each archive checked against its SHA-256, into `<Workspace>/.kexts/oclp-2.5.1/`; they are not part of the app's release. `SecureBootModel` is already `Disabled` in every VM, and the app's KVM quirks and Mac model stay.

Inside macOS, with the card passed through:

1. Turn **FileVault off** and keep a backup of the VM's folder (disk, `OVMF_VARS` and the `macos-kvm-gui` folder).
2. Install OCLP and open it. **Never use "Build and Install OpenCore"**: the app's OpenCore and Mac model must stay.
3. **Post-Install Root Patch › Start Root Patching**, then restart. On Sequoia OCLP downloads MetallibSupportPkg, so the VM needs internet.
4. **Every macOS update removes the patch**: turn automatic downloads off, update on purpose, then root-patch again.
5. To stop: **Revert Root Patches** in OCLP, restart, then turn the mode off. The app asks to confirm that (the CLI needs `--root-patch-reverted`), because remaining root patches can prevent macOS from booting once its protections are restored.

Turning the mode off removes the kexts and boot arguments, and the next start deletes the lowered `csr-active-config` and `revblock` from NVRAM (OpenCore's NVRAM Add never replaces an existing value), so SIP is fully on again. A VM that never had the mode keeps exactly its earlier configuration, including any SIP value set with `csrutil`.

**Validated without a Kepler card** (the reference host has none): on Sequoia 15.8.1 with the mode on, `csrutil status` reports the custom configuration (Kext Signing and Filesystem Protections off), `csr-active-config` is `03080000`, `revblock` is `media`, the boot-args are set and AMFIPass, RSRHelper, RestrictEvents and KDKlessWorkaround are loaded; after turning it off, SIP is enabled, `csr-active-config` is `00000000`, `revblock` is gone and none of the kexts load. The root patch itself and Metal on a real Kepler card are untested.

## Known issues

- Polaris, Vega and RX 5000 cards can encounter the AMD reset bug after guest shutdown/restart; a host reboot may be needed. `vendor-reset` is an external, manual option, not managed by this app.
- Navi black screens can relate to board policy; `agdpmod=pikera` is already generated when a GPU is selected, but it does not guarantee a working board or driver.
- The physical AMD display and emulated console are separate displays. The QEMU window is not a mirror of the GPU output.
- No ROM-file configuration, built-in patch kext, device-ID spoofing, physical USB-controller passthrough or ACS override is provided. NVIDIA support is limited to Kepler with root-patch mode; the app never runs OCLP.
- Unit-test success does not validate PCI isolation, GPU reset, firmware, monitor routing or Metal on real hardware. **Full passthrough remains untested.**
