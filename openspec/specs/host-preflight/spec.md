# host-preflight Specification

## Purpose
Ensures the Linux host has what it needs to run macOS under KVM and, before the VM starts, points out each problem with its fix.

## Requirements

### Requirement: Prerequisite checks
The system SHALL check and display, each item with status OK, Warning, Failure or Inconclusive and a fix hint: VT-x/AMD-V on the CPU, read and write access to `/dev/kvm`, `qemu-system-x86_64` version 8.2.2 or later, `qemu-img`, `git`, `python3` and the kvm module's `ignore_msrs` value.

#### Scenario: Host ready
- **WHEN** all prerequisites are present and `ignore_msrs` is `Y`
- **THEN** all checks show as OK

#### Scenario: User without KVM access
- **WHEN** `/dev/kvm` exists but the user cannot open it for reading and writing
- **THEN** the KVM check shows as Failure, with the hint to add the user to the `kvm` group

#### Scenario: Missing tool
- **WHEN** `qemu-img` is not found in `PATH`
- **THEN** the corresponding check shows as Failure and names the package to install

#### Scenario: Old QEMU
- **WHEN** `qemu-system-x86_64 --version` reports a version lower than 8.2.2
- **THEN** the QEMU check shows as Failure and states the minimum version

### Requirement: CPU compatibility with the macOS version
The system SHALL warn when the chosen macOS version is Ventura or later and the host CPU lacks the SSE4.1 and AVX2 instructions.

#### Scenario: CPU without AVX2
- **WHEN** the chosen version is Sonoma and the CPU flags do not include `avx2`
- **THEN** the compatibility check shows as Failure and suggests a version up to Monterey

#### Scenario: Older version
- **WHEN** the chosen version is Monterey or earlier
- **THEN** the compatibility check does not require AVX2

### Requirement: Conflicting hypervisor indication
The system SHALL look for VirtualBox or VMware VM processes and, if it finds any, SHALL show a potential conflict warning, without killing processes or unloading modules.

#### Scenario: VirtualBox VM running
- **WHEN** a `VBoxHeadless` or `VirtualBoxVM` process is running
- **THEN** the conflict check shows as Warning and advises shutting down that VM before starting macOS

#### Scenario: Module loaded without a running VM
- **WHEN** the `vboxdrv` module is loaded but no VM process is running
- **THEN** the conflict check shows as OK

#### Scenario: Processes not inspectable
- **WHEN** the process list cannot be read
- **THEN** the conflict check shows as Inconclusive

### Requirement: Assisted ignore_msrs fix
The system SHALL offer an action ("Fix ignore_msrs") that, with privilege elevation requested from the system, enables `ignore_msrs` in the running kernel and writes its own file in `/etc/modprobe.d/`, without changing existing files and never storing the password.

#### Scenario: Fix accepted
- **WHEN** the user triggers the fix and authorizes the elevation
- **THEN** `/sys/module/kvm/parameters/ignore_msrs` becomes `Y`, `/etc/modprobe.d/macos-kvm-gui.conf` contains `options kvm ignore_msrs=1` and the checks are rerun

#### Scenario: Elevation cancelled
- **WHEN** the user cancels the elevation prompt
- **THEN** nothing is changed and the system reports that the fix was cancelled

#### Scenario: Partial failure
- **WHEN** the elevation is authorized but only one of the two changes was applied
- **THEN** the system reports which change failed, checking the actual state of each one after the run

#### Scenario: Elevation error
- **WHEN** the elevation mechanism returns an error that is not a cancellation
- **THEN** the system shows the returned error and nothing is reported as applied without verification

### Requirement: Assisted host setup
The system SHALL offer a "Set up host" action that, with one privilege prompt and a constant script taking the user name, KVM module and package list as validated arguments, installs missing QEMU packages, loads the KVM module, adds the user to the kvm group and enables ignore_msrs, and SHALL say when a new login is needed.

#### Scenario: Fresh host
- **WHEN** qemu-utils is missing and the user is not in the kvm group
- **THEN** the package is installed, the user is added to the kvm group, ignore_msrs is enabled and the app says to log out and back in

#### Scenario: Hostile arguments
- **WHEN** the user name, module or package list contains characters outside their allowed sets
- **THEN** the script exits before changing anything

#### Scenario: Groups not needed
- **WHEN** the user runs the setup
- **THEN** the user is not added to the libvirt or input groups
