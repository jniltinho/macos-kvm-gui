# Spec Delta

## Purpose

Lets one host keep several macOS VMs, each in its own folder, created through a wizard and movable between folders and hosts.

## ADDED Requirements

### Requirement: VM list
The system SHALL keep the VMs and app preferences in a local SQLite database, SHALL import an older `config.json` once, and the desktop app SHALL run at most one VM at a time.

#### Scenario: Upgrade from a single VM
- **WHEN** the app starts with a `config.json` and no database
- **THEN** that VM appears in the list with the same settings and `config.json` is renamed to `config.json.migrated`

### Requirement: Workspace with one folder per VM
The system SHALL keep an app-wide Workspace (default `~/macOS-VMs`) and SHALL give each new or imported VM the folder `<Workspace>/<name>` unless another folder is chosen. Changing the Workspace SHALL NOT move existing VMs.

#### Scenario: New VM folder
- **WHEN** the Workspace is `~/macOS-VMs` and the user creates "Tahoe lab"
- **THEN** its folder is `~/macOS-VMs/tahoe-lab`

### Requirement: Move a VM
The system SHALL move only the files the app created for a stopped VM (disk, install media, NVRAM, identity, boot files) into a new or empty, readable folder on the same filesystem, holding the lock of both folders, SHALL point the VM there, and SHALL put everything back on any error.

#### Scenario: OSX-KVM folder
- **WHEN** a VM in an OSX-KVM checkout is moved
- **THEN** its disk and boot files move and the checkout's own files stay

#### Scenario: Running VM
- **WHEN** QEMU uses the VM's disk
- **THEN** the move is refused

### Requirement: New VM wizard
The system SHALL create a VM through one wizard with folding sections, like VirtualBox's expert mode: name, folder and macOS version; hardware (memory and CPU sliders prefilled with the host recommendation, display); and hard disk (size slider, download now). On Finish it SHALL validate every section, open the first section with a problem, and otherwise make the folder and disk and download the recovery when asked.

#### Scenario: Duplicate name
- **WHEN** the name is already used by another VM and the user clicks Finish
- **THEN** the wizard opens the name section, says so and creates nothing

### Requirement: Export and import
The system SHALL export a stopped VM (refusing one that any QEMU is running) as a `.tar` with its settings, disk, NVRAM, identity and install media, or its settings as `.json`. Importing a `.tar` SHALL accept only the known file names, SHALL require a new or empty folder, SHALL drop the GPU address and SHALL leave nothing behind on error. Importing a `.json` SHALL add the settings as they are, using the folder they name.

#### Scenario: Crafted bundle
- **WHEN** a bundle contains `../../escape.txt`
- **THEN** the import fails and no file or VM is added

### Requirement: Running detection
The system SHALL detect a QEMU process using a VM's disk, including one started outside the app, and SHALL refuse to start that VM while it runs.

#### Scenario: OSX-KVM script running
- **WHEN** `OpenCore-Boot.sh` runs the same disk with a relative path
- **THEN** the VM shows as running outside the app and Start is refused

### Requirement: Theme preference
The system SHALL offer Dark, Light and System themes, SHALL apply the choice at once to every window including the toolbar, and SHALL let `MKG_THEME` override it.

#### Scenario: Switch to Light
- **WHEN** the user picks Light in File › Preferences
- **THEN** the window, toolbar band and separators turn light without a restart
