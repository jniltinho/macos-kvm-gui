# vm-lifecycle Specification

## Purpose
Lets the user configure the macOS VM and control it (start, shut down, force stop) without editing scripts, while following what QEMU reports.

## Requirements

### Requirement: Persistent VM configuration
The system SHALL allow configuring the workspace, macOS version, RAM (MiB), cores, total vCPUs and host SSH port, and SHALL keep these values across app runs.

#### Scenario: Values kept
- **WHEN** the user saves 8192 MiB of RAM and reopens the app
- **THEN** the RAM field shows 8192

#### Scenario: First run
- **WHEN** there is no saved configuration
- **THEN** the app uses the defaults: workspace `~/OSX-KVM`, Sonoma, 8192 MiB, 4 cores, 8 vCPUs, port 2222

### Requirement: Configuration validation
The system SHALL refuse to save RAM below 4096 MiB, cores or vCPUs below 1, vCPUs that are not a multiple of the cores, a port outside 1024–65535 and a workspace that is not an absolute path after expanding `~`.

#### Scenario: Impossible topology
- **WHEN** the user enters 4 cores and 6 vCPUs
- **THEN** the system refuses and points to the vCPUs field

#### Scenario: Invalid port
- **WHEN** the user enters port 80
- **THEN** the system refuses and states the allowed range

### Requirement: Per-version hardware profile
The system SHALL choose the CPU model and network card from the macOS version: Penryn up to Monterey, Skylake-Client from Ventura on, and vmxnet3 for High Sierra. Only validated versions SHALL appear without the "experimental" marker.

#### Scenario: Sonoma
- **WHEN** the version is Sonoma
- **THEN** the VM uses the Skylake-Client CPU and virtio networking

#### Scenario: High Sierra
- **WHEN** the version is High Sierra
- **THEN** the VM uses the Penryn CPU and vmxnet3 networking and the version is shown as experimental

### Requirement: Start the VM
The system SHALL start QEMU with KVM, q35 machine, OpenCore in snapshot mode, OVMF firmware with persistent UEFI variables, the macOS disk, NAT networking and SSH forwarded only from `127.0.0.1:<port>` to guest port 22.

#### Scenario: Start with install media
- **WHEN** the user clicks "Start" and `BaseSystem.img` exists
- **THEN** QEMU starts with the install media attached and the VM window opens

#### Scenario: Start without install media
- **WHEN** the user clicks "Start" and `BaseSystem.img` does not exist
- **THEN** QEMU starts with only the macOS disk

#### Scenario: Missing disk
- **WHEN** the user tries to start and `mac_hdd_ng.img` does not exist
- **THEN** the system does not start QEMU and advises creating the disk

#### Scenario: SSH restricted to the host
- **WHEN** the VM is running with port 2222
- **THEN** port 2222 accepts connections only on `127.0.0.1`

### Requirement: UEFI variable persistence
The system SHALL create, on first start, its own copy of the UEFI variables file from `OVMF_VARS-1920x1080.fd` and SHALL reuse it on later starts, never overwriting it.

#### Scenario: Second start
- **WHEN** the VM has been started once and is started again
- **THEN** QEMU uses the same UEFI variables copy, with changes made by the guest preserved

### Requirement: Stop the VM
The system SHALL offer graceful shutdown ("Shut down (ACPI)", an ACPI power request to the guest), which is only considered complete when QEMU exits, and a forced process stop ("Force stop"), which asks for confirmation and warns about data loss.

#### Scenario: Graceful shutdown
- **WHEN** the user clicks "Shut down (ACPI)" with the VM running
- **THEN** the guest receives the request and the state stays "Running" until QEMU exits, without escalating to a force stop on its own

#### Scenario: Force stop
- **WHEN** the user confirms "Force stop"
- **THEN** the QEMU process is terminated and the state becomes "Stopped"

### Requirement: QEMU monitoring
The system SHALL display the VM state ("Stopped", "Running" or "Paused (error)") and the QEMU output, and SHALL translate known failures into clear messages.

#### Scenario: KVM busy
- **WHEN** QEMU exits with `failed to initialize kvm: Device or resource busy`
- **THEN** the state returns to "Stopped" and the system reports that another hypervisor is using virtualization

#### Scenario: Disk error
- **WHEN** QEMU pauses the guest due to a disk I/O error, for example lack of space on the host
- **THEN** the state becomes "Paused (error)" with the error message, even with the process alive

#### Scenario: Normal exit
- **WHEN** the guest shuts down and QEMU exits with code 0
- **THEN** the state returns to "Stopped" with no error message

#### Scenario: Closing the app with the VM running
- **WHEN** the user closes the window with the VM running
- **THEN** the app does not close and asks to shut down the VM first

### Requirement: Recommended resources for the host
The system SHALL offer an action ("Recommended for this host") that fills cores, vCPUs and RAM with values recommended for the host: vCPUs equal to half the host threads (even, between 2 and 16), cores equal to half the vCPUs (minimum 1) and RAM equal to a quarter of total memory, rounded down to a multiple of 1024 MiB, between 4096 and 16384 MiB. Without a supported GPU, video is rendered by the CPU.

#### Scenario: Reference host
- **WHEN** the host has 24 threads and 186 GiB of RAM
- **THEN** the fields become 12 vCPUs, 6 cores and 16384 MiB, without saving until the user confirms

#### Scenario: Small host
- **WHEN** the host has 4 threads and 8 GiB of RAM
- **THEN** the fields become 2 vCPUs, 1 core and 4096 MiB

### Requirement: Video performance guidance
The system SHALL state that the VM has no graphics acceleration (macOS has no driver for the virtual video device or for NVIDIA GPUs) and SHALL list the macOS settings that reduce rendering cost.

#### Scenario: VM tab
- **WHEN** the user opens the VM tab
- **THEN** they see the "Reduce motion", "Reduce transparency" and Dock animation settings, and the note that only a supported AMD GPU in passthrough provides acceleration
