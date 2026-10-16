# macos-kvm-gui

Desktop app (Go + [Fyne](https://fyne.io)) to install and run macOS on KVM with [OSX-KVM](https://github.com/kholia/OSX-KVM), without editing scripts.

The app:

- checks the host (VT-x/AMD-V, `/dev/kvm`, QEMU ≥ 8.2.2, `ignore_msrs`, running VirtualBox/VMware VMs) and fixes `ignore_msrs` through `pkexec`;
- clones OSX-KVM (shallow clone) into the workspace;
- downloads the macOS recovery, **verifies Apple's signature and the SHA-256 of every chunk**, converts it to a raw image and deletes the DMG;
- creates the thin qcow2 disk;
- starts the VM, shuts it down through ACPI (QMP) or force-stops it, keeping the UEFI NVRAM between runs;
- removes the install media after installation to free space;
- suggests CPU and RAM for the host and explains how to make video smoother;
- reserves a macOS-supported AMD GPU for VFIO and passes it to the VM (see [GPU passthrough](#gpu-passthrough)).

Partitioning the disk and running the installer **inside** macOS stay manual; the Install tab lists the steps.

## Requirements

Runtime (Ubuntu/Debian):

```sh
sudo apt install qemu-system-x86 qemu-utils git python3 pkexec
```

Build (Fyne needs CGO, OpenGL, X11 and Wayland headers):

```sh
sudo apt install gcc pkg-config libgl1-mesa-dev xorg-dev libwayland-dev libxkbcommon-dev wayland-protocols
```

Go 1.27 or newer (see `go.mod`).

## Build and use

```sh
make build   # produces ./macos-kvm-gui
make run
```

Flow: **Host** (all green) → **Settings** (workspace, version, RAM, CPUs, SSH port) → **Install** (Prepare workspace → Download recovery → Create disk) → **VM** (Start).

An existing OSX-KVM workspace (for example `~/OSX-KVM` with `BaseSystem.img` and `mac_hdd_ng.img`) is used as is.

The guest's SSH is forwarded to `127.0.0.1:<port>` only (default 2222): `ssh -p 2222 user@127.0.0.1`, after turning on Remote Login in macOS.

## Video performance

The VM has no graphics acceleration. macOS has no driver for any display device QEMU emulates on Linux (VMware SVGA, VGA, QXL, virtio-gpu), so the CPU draws the screen and there is no Metal. QEMU's `apple-gfx` paravirtual display only works on macOS hosts.

- **Recommended for this host** (Settings tab) uses half of the host threads (up to 16 vCPUs) and a quarter of the RAM (up to 16 GiB).
- In macOS, turn on *Reduce motion* and *Reduce transparency* (Accessibility › Display) and turn off the Dock animations.
- The default Sonoma wallpaper is animated and stays **blank** without Metal. Pick a static one, such as `/System/Library/Desktop Pictures/Sonoma.heic`.
- Raising the `vmware-svga` VRAM (`vgamem_mb`) does not help: macOS only uses the boot framebuffer, and the default 16 MB covers up to 2560x1440.

OSX-KVM's `OpenCore.qcow2` leaves `Resolution` empty, so macOS may start at 1280x800. Setting `UEFI › Output › Resolution` to `1920x1080@32` in its `config.plist` gives 1080p. The app does not edit OpenCore yet.

## GPU passthrough

Passing a real AMD GPU to the VM through VFIO is the only way to get Metal acceleration. The **GPU** tab lists every display adapter, says whether it can be passed through and why not, and lets you choose one for the VM.

**Supported** (per the [Dortania GPU buyers guide](https://dortania.github.io/GPU-Buyers-Guide/modern-gpus/amd-gpu.html)): RX 460/470/480/550/560/570/580/590, Vega 56/64, Radeon VII, RX 5500/5600/5700, RX 6600/6600 XT, RX 6800/6800 XT/6900 XT. **The RX 6600 is the safest choice**: it is the card OSX-KVM tested, and it has no reset bug.

**Not supported:**

- RX 7000 series and RX 6400/6500: no macOS driver.
- NVIDIA (no driver since Mojave) and Intel GPUs.
- AMD Ryzen iGPUs (APUs). Zen 4 and Zen 5 iGPUs, such as the Ryzen 7000/9000 "Radeon Graphics", have no macOS support. The community [NootedRed](https://github.com/ChefKissInc/NootedRed) kext covers only Vega APUs (Raven Ridge to Barcelo), is a work in progress, and is not known to work inside a VM.
- Cards that need a device-id spoof or the NootRX kext (RX 6650 XT, RX 6700/6750, RX 6950 XT, XTXH variants, RX 580 2048SP, Lexa). They can work, but this app does not edit OpenCore.

**Steps:**

1. The host must show its screen on another GPU. The app refuses a card that is the boot display (set the integrated GPU as primary in the BIOS), a card with a monitor connected, and the host's last GPU.
2. The card must sit in a slot whose IOMMU groups hold only that card (its GPU and audio functions, plus PCI bridges). The app also refuses cards with extra functions, such as a USB-C controller, and cards with an identical twin in the host.
3. **Reserve for VFIO** (one password prompt) writes the following files, then runs `update-initramfs -u -k all`. If the rebuild fails, the change is rolled back:
   - `/etc/modprobe.d/macos-kvm-gui-vfio.conf`: `options vfio-pci ids=<gpu>,<audio> disable_vga=1`, plus `softdep` lines so `vfio-pci` loads before `amdgpu`, `radeon` and `snd_hda_intel`;
   - `/etc/udev/rules.d/99-macos-kvm-gui-vfio.rules`: makes `/dev/vfio/<group>` owned by group `kvm` with mode 0660, so QEMU can run without root.

   No driver is blacklisted. The upstream notes say `blacklist amdgpu`, which would leave the host without a display if it uses an AMD iGPU.
4. Reboot the host, choose the GPU in the GPU tab and start the VM. QEMU gets one `vfio-pci` device per function of the card. The emulated `vmware-svga` display stays, because QEMU's window needs it to deliver keyboard and mouse input; it also shows the OpenCore and boot screens. Once the AMD driver loads, macOS shows its desktop on the monitor plugged into the card. If the desktop stays in the QEMU window, set the AMD monitor as the main display in *System Settings › Displays*.
5. To undo, click **Release reservation**: it removes both files and rebuilds the initramfs. Then reboot the host again.

**Before starting the VM, the app checks** that the card is still eligible, that the macOS version is new enough for it (RX 6600: Monterey 12.1+; RX 6800/6900: Big Sur 11.4+; RX 5000: Catalina 10.15.1+), that every function is bound to `vfio-pci`, that you can open `/dev/vfio/<group>` (log in again after joining the `kvm` group), and that the soft locked-memory limit (`ulimit -l`) is at least the VM's RAM, because VFIO pins all guest memory. QEMU never runs as root.

**If the host no longer reaches a desktop after a reservation**, add `modprobe.blacklist=vfio_pci` to the kernel line in GRUB (press `e` at the boot menu) to boot once, then click Release reservation. From a console you can also run `sudo rm /etc/modprobe.d/macos-kvm-gui-vfio.conf /etc/udev/rules.d/99-macos-kvm-gui-vfio.rules && sudo update-initramfs -u -k all`.

**Known issues:**

- Polaris, Vega and RX 5000 cards suffer from the AMD reset bug: after the VM restarts, the card may need a host reboot (or the `vendor-reset` module).
- Navi cards may show a black screen without `agdpmod=pikera` in OpenCore's boot-args.
- **Untested on real hardware**: the reference host has no supported AMD card. Detection, refusals, the reservation command and the QEMU arguments are covered by unit tests only.

## Tests

```sh
make test               # unit tests, with -race
make test-integration   # boots a real QEMU in a temporary workspace (needs KVM and ~/OSX-KVM)
```

## Files

| Where | What |
|---|---|
| `~/.config/macos-kvm-gui/config.json` | app settings |
| `<workspace>/macos-kvm-gui-vars.fd` | the VM's UEFI NVRAM (created on first start, never overwritten) |
| `<workspace>/.macos-kvm-gui-media` | version of the `BaseSystem.img` downloaded by the app |
| `$XDG_RUNTIME_DIR/macos-kvm-gui-*.lock` | workspace lock (one operation or VM at a time, across app instances) |
| `/etc/modprobe.d/macos-kvm-gui.conf` | persistent `ignore_msrs` (created by the fix in the Host tab) |
| `/etc/modprobe.d/macos-kvm-gui-vfio.conf`, `/etc/udev/rules.d/99-macos-kvm-gui-vfio.rules` | GPU reservation (GPU tab) |

## Limitations

- Only **Sonoma** has been validated end to end. Other versions are marked *experimental*. The CPU model and network card follow the version: Penryn up to Monterey, Skylake-Client from Ventura on, and vmxnet3 on High Sierra.
- `ignore_msrs` persistence works when `kvm` is loaded as a module. If it lives in the initramfs, run `sudo update-initramfs -u`.
- VirtualBox and VMware VMs cannot run at the same time as KVM.
- Closing the app while the VM runs is blocked. If the app dies, QEMU gets SIGTERM, which is like a forced stop.
- No USB passthrough, bridge/tap networking or libvirt.

## Development

Proposals and specs live in [`openspec/changes`](openspec/changes) (OpenSpec).

OSX-KVM is GPLv3 and is used at runtime: none of it is embedded in the binary.
