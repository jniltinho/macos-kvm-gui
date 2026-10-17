# gpu-passthrough Specification

## Purpose
Lets the user give the macOS VM a real, macOS-supported AMD GPU through VFIO, the only way to get Metal acceleration, without leaving the host without a display.

## Requirements

### Requirement: GPU listing
The system SHALL list every PCI display adapter of the host with its PCI address, vendor and device ID, name, IOMMU group, bound driver and macOS compatibility (supported, unknown or unsupported).

#### Scenario: Reference host
- **WHEN** the host has an NVIDIA RTX 4070, an NVIDIA Tesla V100 and an AMD Ryzen iGPU
- **THEN** all three are listed and all are marked unsupported by macOS

#### Scenario: Supported AMD card
- **WHEN** the host has an AMD RX 6600 (`1002:73ff`)
- **THEN** it is listed as supported by macOS

### Requirement: Passthrough eligibility
The system SHALL mark a GPU eligible only when the IOMMU is active, its device ID is supported, it is neither the boot display nor driving a connected monitor, another GPU with a driver remains for the host, its slot holds only display and audio functions, no device outside the slot shares their IDs, and each function's IOMMU group holds only that slot's functions and PCI bridges. Each refusal SHALL state its reason.

#### Scenario: Boot display
- **WHEN** the GPU is the one the firmware used to show the boot screen
- **THEN** it is not eligible and the reason tells the user to make the integrated GPU the primary display in the BIOS

#### Scenario: Shared IOMMU group
- **WHEN** the IOMMU group of any function of the GPU's slot also contains a device from another slot that is not a PCI bridge
- **THEN** it is not eligible and the reason names that device

#### Scenario: GPU driving a monitor
- **WHEN** a DRM connector of the GPU reports a connected monitor
- **THEN** it is not eligible and the reason tells the user to move the host's monitor to another GPU

#### Scenario: Extra function in the slot
- **WHEN** the GPU's slot also exposes a USB controller function
- **THEN** it is not eligible and the reason names that function

#### Scenario: Identical second card
- **WHEN** another device outside the slot has the same vendor and device ID as one of the slot's functions
- **THEN** it is not eligible, because `vfio-pci ids=` would claim that device too

#### Scenario: Last GPU of the host
- **WHEN** no other GPU with a bound driver exists outside the slot
- **THEN** it is not eligible, because the host would have no display left

#### Scenario: AMD GPU outside the supported list
- **WHEN** the GPU is an AMD device whose ID is in neither list, such as the Ryzen iGPU `1002:13c0`
- **THEN** it is marked unknown, it is not eligible, and the reason points to the Dortania GPU buyers guide

#### Scenario: IOMMU disabled
- **WHEN** the host has no IOMMU groups
- **THEN** no GPU is eligible and the reason tells the user to enable IOMMU (AMD-Vi or VT-d) in the BIOS

### Requirement: Reserve a GPU for VFIO
The system SHALL offer, for an eligible GPU, an action that uses privilege elevation to make `vfio-pci` claim the slot's functions by ID at boot, ahead of `amdgpu`, `radeon` and `snd_hda_intel`, give the `kvm` group access to VFIO group devices and rebuild the initramfs of every kernel, without blacklisting any driver, holding a host-wide lock and rolling back if the rebuild fails.

#### Scenario: Reservation applied
- **WHEN** the user reserves an RX 6600 with functions `1002:73ff` and `1002:ab28`
- **THEN** `/etc/modprobe.d/macos-kvm-gui-vfio.conf` contains `options vfio-pci ids=1002:73ff,1002:ab28`, no `blacklist` line exists in it, and the app says a host reboot is required

#### Scenario: Only valid IDs reach the privileged script
- **WHEN** a device ID read from the system does not match four hexadecimal digits, a colon and four hexadecimal digits
- **THEN** the reservation is refused before any privilege elevation

#### Scenario: Elevation cancelled
- **WHEN** the user cancels the privilege prompt
- **THEN** nothing is changed and the app reports that the reservation was cancelled

#### Scenario: Initramfs rebuild fails
- **WHEN** `update-initramfs` fails, for example because `/boot` is full
- **THEN** the reservation files are removed again, the initramfs is rebuilt without them and the app shows the error

### Requirement: Release a GPU
The system SHALL offer an action that removes the files the reservation created and rebuilds the initramfs, so that the GPU returns to its normal driver after a reboot.

#### Scenario: Release applied
- **WHEN** the user releases the reservation
- **THEN** `/etc/modprobe.d/macos-kvm-gui-vfio.conf` and `/etc/udev/rules.d/99-macos-kvm-gui-vfio.rules` no longer exist and the app says a host reboot is required

### Requirement: Start the VM with a passed-through GPU
The system SHALL let the user choose a GPU in the configuration and, when one is chosen, SHALL re-check its eligibility and start QEMU with every function of its slot as a VFIO device, keeping the emulated display so the QEMU window still delivers keyboard and mouse input.

#### Scenario: GPU ready
- **WHEN** the chosen GPU's functions are all bound to `vfio-pci`
- **THEN** QEMU starts with one `vfio-pci` device per function, plus `vmware-svga` as the input console

#### Scenario: GPU not yet reserved
- **WHEN** a function of the chosen GPU is bound to another driver
- **THEN** the VM does not start and the app says to reserve the GPU and reboot the host

#### Scenario: Not enough locked memory
- **WHEN** the user's soft locked-memory limit is lower than the VM's RAM
- **THEN** the VM does not start and the app shows the current limit and the RAM required

#### Scenario: macOS too old for the GPU
- **WHEN** the GPU is an RX 6600 and the chosen macOS version is Big Sur
- **THEN** the VM does not start and the app says the GPU needs Monterey or newer

#### Scenario: VFIO device not accessible
- **WHEN** the user cannot open the GPU's `/dev/vfio/<group>` node for reading and writing
- **THEN** the VM does not start and the app points to the udev rule and the `kvm` group, never falling back to running QEMU as root

#### Scenario: No GPU chosen
- **WHEN** no GPU is chosen in the configuration
- **THEN** the VM starts with the emulated display, as before
