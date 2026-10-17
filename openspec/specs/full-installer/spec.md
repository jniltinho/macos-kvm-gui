# full-installer Specification

## Purpose
Lets the app download Apple's full macOS installer once, verify it and hand it to VMs as a local disc, instead of each VM downloading 13–15 GB through the recovery.

## Requirements

### Requirement: Full installer download
The system SHALL find, in Apple's public Software Update catalog over HTTPS, the newest `InstallAssistant.pkg` of the VM's macOS major version for Big Sur 11 and later whose distribution supports the VM's Mac board ID, SHALL download it with progress and resume into a shared cache, and SHALL refuse High Sierra to Catalina with a message that the recovery must be used.

#### Scenario: Sequoia
- **WHEN** the user downloads the full installer for a Sequoia VM
- **THEN** the newest macOS 15 InstallAssistant.pkg is downloaded into `<Workspace>/.installers/<build>/`

### Requirement: Verification
The system SHALL check the downloaded package's size and every chunk's SHA-256 against Apple's integrity data, fetched over HTTPS and checked against its own checksum, before offering it, and SHALL delete a package that fails.

#### Scenario: Corrupted download
- **WHEN** one chunk of the package does not match its hash
- **THEN** the package is deleted and the error says the package integrity check failed

### Requirement: Shared cache
The system SHALL keep one installer per macOS build for all VMs, SHALL show which builds are cached and their size, and SHALL let the user delete a cached installer.

#### Scenario: Second VM
- **WHEN** a second Sequoia VM is created and the installer for that build is cached
- **THEN** no download is needed and Install shows the cached installer

### Requirement: Offline delivery
The system SHALL keep a verified installer as an ISO 9660 disc holding the package in contiguous pieces of under 4 GiB and an install script, SHALL attach it to a VM that starts with install media with the guest's writes discarded, SHALL keep an attached installer from being deleted, and SHALL show the recovery Terminal line that installs from it with no network.

#### Scenario: Install without network
- **WHEN** a Sequoia VM with a cached Sequoia installer starts from its recovery and the user types `sh /Volumes/OFFLINE/INSTALL.SH` after erasing the disk
- **THEN** the installer app opens with the package as its SharedSupport.dmg, without any network access

#### Scenario: Unattended
- **WHEN** the user ticks Unattended and clicks Type it in the VM while the recovery shows its window, with the VM's disk empty
- **THEN** the app opens the recovery's Terminal and types the line with -u, the script erases the only empty internal disk, runs startosinstall with --agreetolicense and restarts, and macOS installs to Setup Assistant with no further input

#### Scenario: Files over 4 GiB
- **WHEN** the package is larger than 4 GiB
- **THEN** it is stored as pieces PKG.000, PKG.001, … that join back into the verified package
