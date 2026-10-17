# root-patch-mode Specification

## Purpose
Lets a NVIDIA Kepler card passed through to a macOS VM be accelerated: natively on Big Sur, and through OpenCore Legacy Patcher's post-install root patches on Monterey to Sequoia, with OpenCore settings the user turns on knowingly.

## Requirements

### Requirement: Kepler eligibility
The system SHALL treat NVIDIA Kepler GPUs, identified by PCI device ID, as eligible for passthrough on Big Sur without changes and on Monterey through Sequoia only with root-patch mode on, and SHALL keep Maxwell and newer NVIDIA GPUs and Tahoe ineligible with the reason.

#### Scenario: GT 710 on Sequoia
- **WHEN** a GT 710 (GK208) is listed for a Sequoia VM
- **THEN** it shows as eligible with root-patch mode, and as not eligible while the mode is off

### Requirement: Root-patch mode settings
The system SHALL, only for a VM with root-patch mode on, generate OpenCore settings with `csr-active-config` 0x803, AMFIPass, RestrictEvents `revblock=media`, KDKlessWorkaround, RSRHelper and the boot arguments OCLP adds, with the kexts downloaded from OCLP's pinned commit and checked against their SHA-256; a VM that never had the mode on SHALL get the same configuration as before.

#### Scenario: Default VM
- **WHEN** root-patch mode was never turned on
- **THEN** the generated config.plist equals the one produced before this change, and a SIP value set with csrutil is kept

### Requirement: Restoring SIP
The system SHALL list `csr-active-config` and `revblock` in `NVRAM › Delete` while root-patch mode is on and on the first boot after it is turned off, so that full SIP is restored.

#### Scenario: Turned off again
- **WHEN** root-patch mode is turned off after being on
- **THEN** the next boot clears the reduced `csr-active-config` from NVRAM

### Requirement: Informed consent
The system SHALL keep root-patch mode off by default, SHALL explain what it weakens and the steps inside macOS when it is turned on, and SHALL ask the user to confirm the root patch was reverted in OCLP before turning it off.

#### Scenario: Turning it off
- **WHEN** the user turns root-patch mode off
- **THEN** a confirmation says remaining root patches can prevent macOS from booting once protections are restored, so they must be reverted in OCLP first
