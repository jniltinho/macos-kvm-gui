# boot-assets Specification

## Purpose
Ships every file needed to boot macOS inside the app, built from BSD-licensed upstreams, so that no OSX-KVM clone, `git` or `python3` is needed and the files can be redistributed legally.

## Requirements

### Requirement: Embedded boot files
The system SHALL embed OpenCore, the Lilu, VirtualSMC and WhateverGreen kexts and the OVMF firmware, taken from their official upstream releases at pinned versions whose SHA-256 is verified when the files are rebuilt.

#### Scenario: Fresh host
- **WHEN** the user creates a workspace on a host without OSX-KVM, `git` or `python3`
- **THEN** the workspace receives the OpenCore EFI tree and the OVMF firmware from the app itself

#### Scenario: Tampered download while rebuilding
- **WHEN** a download's SHA-256 does not match while the files are rebuilt
- **THEN** the rebuild stops and the previously built files are left untouched

### Requirement: Configurable asset sources
The system SHALL read asset versions, URLs and checksums from a file in the repository, and SHALL let an environment variable or a local, untracked `.env` file override each value, with the environment taking precedence over `.env` and `.env` over the repository file, without executing those files and accepting only the asset keys.

#### Scenario: Override from the environment
- **WHEN** `OPENCORE_URL` and `OPENCORE_SHA256` are set in the environment
- **THEN** the rebuild downloads OpenCore from that URL and verifies it against that checksum

### Requirement: Runtime asset directory
The system SHALL use, instead of the embedded files, a directory named by `MKG_ASSETS_DIR` (from the environment or from `~/.config/macos-kvm-gui/.env`) with the same layout as the embedded files, and SHALL refuse it when any file of the embedded set is missing.

#### Scenario: Newer OpenCore without rebuilding
- **WHEN** `MKG_ASSETS_DIR` points to a complete directory
- **THEN** the VM boots with those files and the app shows their versions with the directory as the source

#### Scenario: Incomplete directory
- **WHEN** `MKG_ASSETS_DIR` points to a directory without `files/EFI/OC/OpenCore.efi`
- **THEN** the VM does not start and the app names the missing file

### Requirement: Generated OpenCore configuration
The system SHALL generate OpenCore's `config.plist` at every VM start from the settings: 1920x1080 output; with install media attached, the picker is shown and every file system is scanned; without it, the picker is hidden and only APFS and HFS+ volumes are scanned; APFS drivers of every macOS version are allowed; a passed-through GPU adds `agdpmod=pikera` to the boot arguments.

#### Scenario: Installed system
- **WHEN** the VM starts without `BaseSystem.img`
- **THEN** OpenCore boots straight into macOS without showing a menu

#### Scenario: First install
- **WHEN** the VM starts with `BaseSystem.img`
- **THEN** OpenCore shows its menu so the user can pick the installer

#### Scenario: Valid configuration
- **WHEN** the configuration is generated for install, installed or GPU passthrough
- **THEN** OpenCore's `ocvalidate` reports no issues

### Requirement: SMBIOS identity per workspace
The system SHALL generate a random serial number, MLB, system UUID and ROM once per workspace, keep them across starts, and pick a Mac model that supports the chosen macOS version: iMac18,3 for High Sierra, iMac19,1 for Mojave and iMac20,1 from Catalina to Tahoe.

#### Scenario: Stable identity
- **WHEN** the VM is started twice in the same workspace
- **THEN** macOS reports the same serial number both times

### Requirement: Recovery download without Python
The system SHALL download the macOS recovery directly from Apple's recovery server, without `python3`, and SHALL keep verifying the chunklist signature and hashes before using the image.

#### Scenario: Sonoma recovery
- **WHEN** the user downloads the Sonoma recovery on a host without `python3`
- **THEN** the chunklist and the DMG are downloaded, verified and converted as before

### Requirement: Home lab notice
The system SHALL state, in the app and in the README, that the project is meant for home lab use and that Apple's macOS license only allows macOS on Apple-branded hardware.

#### Scenario: Host tab
- **WHEN** the user opens the app
- **THEN** the Host tab shows the home lab notice
