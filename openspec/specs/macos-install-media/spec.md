# macos-install-media Specification

## Purpose
Prepares everything the VM needs on disk (OSX-KVM workspace, verified recovery media and virtual disk) using minimal space and without destroying existing data.

## Requirements

### Requirement: OSX-KVM workspace preparation
The system SHALL use a configurable workspace directory and SHALL classify it as missing or empty, ready, incomplete or unrelated. Only a missing or empty directory receives a shallow clone (with submodules) of `https://github.com/kholia/OSX-KVM`.

#### Scenario: Empty workspace
- **WHEN** the user triggers "Prepare workspace (clone OSX-KVM)" with a missing or empty directory
- **THEN** the repository is cloned with depth 1 and the workspace becomes ready

#### Scenario: Ready workspace
- **WHEN** the directory contains `OpenCore/OpenCore.qcow2`, `OVMF_CODE_4M.fd`, `OVMF_VARS-1920x1080.fd` and `fetch-macOS-v2.py`
- **THEN** nothing is downloaded and the workspace shows as ready

#### Scenario: Unrelated or incomplete directory
- **WHEN** the directory is not empty and any of those files is missing
- **THEN** the system neither clones nor changes anything and reports which files are missing

### Requirement: Verified recovery download
The system SHALL download the recovery for the chosen version and SHALL verify Apple's signature on the chunklist and the SHA-256 of each DMG chunk before using it, regardless of the downloader's exit code.

#### Scenario: Intact download
- **WHEN** the user chooses Sonoma, triggers "Download macOS recovery" and verification passes
- **THEN** the DMG proceeds to conversion

#### Scenario: Tampered or truncated image
- **WHEN** any chunk does not match the chunklist or the signature is invalid
- **THEN** the system reports a verification failure, discards the downloaded files and converts nothing

#### Scenario: Network failure
- **WHEN** the download is interrupted
- **THEN** the system shows the error, discards the partial files and allows retrying

### Requirement: Per-stage progress
The system SHALL show the current stage (chunklist download, image download, verification, conversion) and, when the total size is known, the percentage. If it is not known, progress SHALL appear as indeterminate.

#### Scenario: Known size
- **WHEN** the downloader reports the downloaded percentage
- **THEN** the progress bar reflects that percentage during the image download

#### Scenario: Unknown size
- **WHEN** the downloader does not report a percentage
- **THEN** the stage shows indeterminate progress

### Requirement: Safe media publishing
The system SHALL convert the verified DMG to a raw image in a temporary area on the same filesystem and SHALL only then replace `BaseSystem.img`, record the media version and delete the DMG and the chunklist. Any failure SHALL preserve the previous `BaseSystem.img`.

#### Scenario: Successful conversion
- **WHEN** the conversion finishes without error
- **THEN** `BaseSystem.img` is the new media, the recorded version is the chosen one and the DMG and chunklist have been removed

#### Scenario: Conversion failure
- **WHEN** the conversion fails, for example due to lack of space
- **THEN** the previous `BaseSystem.img` stays intact, temporary files are removed and the error is shown

#### Scenario: Low free space
- **WHEN** free space in the workspace is less than 4 GiB before the download
- **THEN** the system refuses to start and reports the required space

### Requirement: Virtual disk creation
The system SHALL create the macOS disk as thin qcow2, with a virtual size in GiB (minimum 40), SHALL refuse to overwrite an existing disk and SHALL warn that the space is not reserved.

#### Scenario: New disk
- **WHEN** the user triggers "Create disk" with 80 GiB and no disk exists
- **THEN** `mac_hdd_ng.img` exists as qcow2 with an 80 GiB virtual size and less than 10 MiB allocated on the host

#### Scenario: Disk already exists
- **WHEN** the user tries to create a disk and `mac_hdd_ng.img` already exists
- **THEN** the system refuses and does not change the file

#### Scenario: Size too small
- **WHEN** the user enters less than 40 GiB
- **THEN** the system refuses and reports the minimum

#### Scenario: Free space smaller than the disk
- **WHEN** free space on the host is smaller than the chosen virtual size
- **THEN** the disk is created and the system warns that the VM may stop due to lack of space

### Requirement: Install media removal
The system SHALL allow removing `BaseSystem.img` ("Remove install media") only with the VM stopped and after the user confirms that macOS already boots from the disk, and SHALL show the bytes freed (allocated size of the removed file).

#### Scenario: Removal confirmed
- **WHEN** the VM is stopped and the user confirms
- **THEN** `BaseSystem.img` is deleted and the bytes freed are shown

#### Scenario: Workspace in use
- **WHEN** the VM or a media operation is running, in this or another app instance
- **THEN** media removal is refused

### Requirement: Workspace mutual exclusion
The system SHALL prevent two operations that change the workspace (preparation, download, disk or media creation or removal, running VM) from running at the same time on the same workspace, including across app instances.

#### Scenario: Second instance
- **WHEN** one instance has the VM running and another tries to download media into the same workspace
- **THEN** the second instance refuses and reports that the workspace is in use

### Requirement: Automation boundary
The system SHALL make clear in the interface that partitioning the disk (Disk Utility, APFS) and installing macOS inside the guest remain manual steps.

#### Scenario: First installation
- **WHEN** the workspace has media and a disk, but the user has not yet confirmed that macOS boots from the disk
- **THEN** the interface shows the manual installation steps inside the VM
