# Spec Delta

## Purpose

Lets every step of preparing and running a macOS VM be scripted from a terminal, with the same safety rules as the desktop app.

## ADDED Requirements

### Requirement: GUI by default
The system SHALL open the desktop app when started without a command, and SHALL offer the same through `gui`.

#### Scenario: Menu entry
- **WHEN** the binary runs without arguments
- **THEN** the desktop window opens

### Requirement: Scriptable host checks
The system SHALL print the host checks with `check` and SHALL exit with a non-zero status when any check fails.

#### Scenario: Failing host
- **WHEN** `/dev/kvm` cannot be opened
- **THEN** `check` lists it as Failure and exits with an error that points to `setup`

### Requirement: Settings from the command line
The system SHALL show the settings as JSON with `config show`, change them with `config set key=value...` using the same validation as the app, and apply the host recommendation with `config recommend`. A global `--config` flag SHALL select another settings file.

#### Scenario: Invalid value
- **WHEN** the user runs `config set cores=8 vcpus=6`
- **THEN** nothing is saved and the error names the vcpus field

### Requirement: Install steps
The system SHALL create the workspace (`workspace create`), download and verify the recovery (`download [--version]`), create the disk (`disk create --size`) and remove the install media only with `media remove --yes`.

#### Scenario: Media removal without confirmation
- **WHEN** the user runs `media remove` without `--yes`
- **THEN** nothing is deleted and the error asks for `--yes`

### Requirement: Foreground VM
The system SHALL start the VM with `vm start` and wait until QEMU exits. A first interrupt SHALL ask the guest to shut down through ACPI and a second SHALL force the stop. With the in-app display, the command SHALL print the private VNC socket.

#### Scenario: Ctrl+C
- **WHEN** the VM runs under `vm start` and the user presses Ctrl+C once
- **THEN** macOS receives an ACPI shutdown request and the command returns when QEMU exits
