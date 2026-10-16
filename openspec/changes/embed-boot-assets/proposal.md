# Proposal

## Why

The app depends on a runtime clone of OSX-KVM for OpenCore, OVMF and `fetch-macOS-v2.py`, plus `git` and `python3`. OSX-KVM declares no license, so its files cannot be redistributed safely, and its OpenCore image needs manual edits (1080p resolution, hidden picker, APFS-only scan) to work well. Building the boot assets from their BSD-licensed upstreams and embedding them makes the app a single self-contained binary that is legal to redistribute.

## What Changes

- Embed OpenCore (`OpenCore.efi`, `BOOTx64.efi`, `OpenRuntime.efi`, `OpenHfsPlus.efi`, the `SSDT-EC` sample) and the Lilu, VirtualSMC and WhateverGreen kexts, all from acidanthera's official releases (BSD-3-Clause), at pinned versions with SHA-256 checks.
- Embed OVMF (`OVMF_CODE_4M.fd`, `OVMF_VARS_4M.fd`) from the Debian `ovmf` package (BSD-2-Clause-Patent).
- Generate OpenCore's `config.plist` in Go from the app's settings: 1920x1080 output, picker shown only while install media is attached, APFS-only scanning afterwards, SMBIOS per macOS version, plus a self-written `AppleMCEReporterDisabler` kext.
- Boot OpenCore from a directory exposed by QEMU's `vvfat` driver instead of a qcow2 image.
- Replace `fetch-macOS-v2.py` with a Go port of the recovery protocol, so `python3` and `git` are no longer needed.
- **BREAKING**: "Prepare workspace" extracts the embedded assets instead of cloning OSX-KVM. Existing OSX-KVM workspaces keep working: their disk and media are reused, but they boot with the app's OpenCore and SMBIOS, so macOS sees a different Mac model and serial.
- The README and the app say the project is for **home lab use** and point to Apple's macOS license terms.
- Out of scope: OpenCanopy (its icons live in OcBinaryData, which has no license), audio (AppleALC), NootRX, USB maps and the TRIM patch.

## Capabilities

### New Capabilities
- `boot-assets`: embedded, pinned and verified boot assets, generated OpenCore configuration and boot without external downloads other than Apple's recovery.

### Modified Capabilities
<!-- The earlier capabilities live in the unarchived add-desktop-gui change; their changed behavior (no git/python, extraction instead of clone) is captured in boot-assets. -->

## Impact

- New `assets/` tree (about 9 MB) committed to the repository and embedded with `go:embed`, plus `scripts/fetch-assets.sh` to rebuild it reproducibly. The binary grows accordingly.
- `internal/core`: asset extraction, `config.plist` generation, SMBIOS identity per workspace, QEMU arguments (`vvfat`, OVMF paths), Go recovery download. The Host tab no longer checks `git` and `python3`.
- New `LICENSE` (BSD-3-Clause) and `THIRD_PARTY_NOTICES.md` with the upstream licenses.
- Needs a new end-to-end boot validation of Sonoma, because OpenCore and its configuration change.
