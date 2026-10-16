# Proposal

## Why

macOS under QEMU has no driver for any emulated display device, so the screen is drawn by the CPU and there is no Metal. The only way to get graphics acceleration is to hand a macOS-supported AMD GPU to the VM through VFIO. Doing it by hand means finding PCI IDs, checking IOMMU groups, reserving the card at boot, fixing `/dev/vfio` permissions and editing QEMU arguments. The upstream notes also say `blacklist amdgpu`, which leaves the host without a display when it uses another AMD GPU, such as a Ryzen iGPU.

## What Changes

- New **GPU** tab listing the host's display adapters, each with macOS compatibility, IOMMU group, driver in use and whether it can be passed to the VM.
- Action to **reserve** the chosen card for `vfio-pci` by PCI ID (no driver blacklisting), which needs a host reboot, and an action to **release** it.
- Configuration option to start the VM with the chosen GPU (and its HDMI audio function) instead of the emulated display.
- The app refuses combinations that would leave the host without a display or the VM unable to start.
- Out of scope: editing OpenCore's `config.plist` (boot-args such as `agdpmod=pikera`), USB or other device passthrough, VBIOS dumps and `vendor-reset`.

## Capabilities

### New Capabilities
- `gpu-passthrough`: GPU detection and eligibility, VFIO reserve and release, and starting the VM with the passed-through GPU.

### Modified Capabilities
<!-- None: starting the VM without a GPU behaves as before. -->

## Impact

- `internal/core`: reads `/sys/bus/pci` and `/sys/kernel/iommu_groups`, adds a constant privileged script (through `pkexec`) and chooses between the passed-through GPU and the emulated display when building QEMU arguments.
- Host files: `/etc/modprobe.d/macos-kvm-gui-vfio.conf` and `/etc/udev/rules.d/99-macos-kvm-gui-vfio.rules` (created or removed by the app), plus an initramfs rebuild.
- **Not validated on hardware**: the reference host has no compatible AMD GPU. Detection and refusals are tested on the real host; the full passthrough only by unit tests.
