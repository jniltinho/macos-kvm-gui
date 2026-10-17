# Design

## Context

OSX-KVM declares no license, so its `OpenCore.qcow2`, `config.plist` and scripts cannot be redistributed safely. Its components come from licensed upstreams: OpenCore and the kexts (acidanthera, BSD-3-Clause), OVMF (Debian `ovmf`, BSD-2-Clause-Patent) and `fetch-macOS-v2.py`, which is acidanthera's `macrecovery` (BSD-3-Clause). On the reference host, QEMU 8.2.2 supports `vvfat`. OSX-KVM's configuration was read only to learn which functional settings KVM needs (booter, kernel and UEFI quirks); none of its files are copied.

## Goals / Non-Goals

**Goals:**
- One binary that boots macOS with no external files except Apple's recovery and QEMU.
- Asset updates by editing data, not code: versions and URLs live in `assets.env`.

**Non-Goals:**
- OpenCanopy graphics (its icons live in OcBinaryData, which has no license), AppleALC audio, USB port maps, the TRIM patch and NootRX.
- Migrating an OSX-KVM SMBIOS identity into the app's identity.

## Decisions

1. **Assets committed and embedded.** `scripts/fetch-assets.sh` downloads the pinned releases, checks SHA-256 and copies only what is needed into `internal/assets/files`, about 6 MB: `BOOTx64.efi`, `OpenCore.efi`, `OpenRuntime.efi`, `OpenHfsPlus.efi`, the three kexts, and `OVMF_CODE_4M.fd`/`OVMF_VARS_4M.fd` with Debian's copyright file. It also writes a `VERSIONS` file. The tree is committed and `go:embed` includes it, so builds need no network. The repository is the only source the app uses.
2. **`assets.env` with overrides.** The script reads `KEY=value` lines from `.env`, then from `assets.env`. A key that is already set wins, so the precedence is environment > `.env` > `assets.env`. The files are parsed line by line, never `source`d, and only `<COMPONENT>_VERSION|URL|SHA256` keys are accepted and never exported, so a `.env` cannot set `LD_PRELOAD` or `PATH` for `curl`. All downloads are verified before the old tree is deleted.
3. **Runtime override `MKG_ASSETS_DIR`.** It is read from the environment or `~/.config/macos-kvm-gui/.env`. It points to a directory with the same layout as `internal/assets` (`config.plist.tmpl`, `files/…`), so a checkout where the script ran with newer versions can be used without rebuilding. Every file of the embedded set must exist in it (kexts, drivers, `BOOTx64.efi`, firmware) before anything in the workspace is replaced.
4. **`config.plist` from a template derived once from OpenCore's `Sample.plist`** (BSD-3-Clause; attribution in the file). The template has every key the 1.0.8 schema expects; Go only replaces placeholders (picker, timeout, scan policy, boot-args, SMBIOS fields, resolution). The functional settings are the KVM-relevant ones: booter `AvoidRuntimeDefrag` and `EnableWriteUnprotector`; kernel `ProvideCurrentCpuInfo`, `DisableLinkeditJettison`, `ForceSecureBootScheme`, `DummyPowerManagement`; UEFI `RequestBootVarRouting`, `EnableVectorAcceleration`, `ProvideConsoleGop`; `SecureBootModel` set to `Disabled`; builtin text picker. A test runs OpenCore's `ocvalidate` on the generated files.
5. **Picker and scan policy by media.** With `BaseSystem.img` the picker is shown (5 s timeout) and `ScanPolicy` is 0. Without it, `ShowPicker` is false and `ScanPolicy` is `0x301` (file-system lock plus APFS and HFS+), which hides OpenCore's own FAT disk, so the installed system is the default entry, and HFS+ High Sierra installs stay visible. `UEFI › APFS` `MinDate`/`MinVersion` are `-1`, because 0 would restrict the APFS driver to recent releases and break High Sierra to Catalina. `bless` cannot set the default on Sonoma with SIP on, so this avoids depending on NVRAM.
6. **iMac SMBIOS.** iMac18,3, iMac19,1 and iMac20,1 cover High Sierra to Tahoe and do not load AppleMCEReporter, which panics on AMD hosts, so no MCE disabler kext is needed. The identity is random and stored in `macos-kvm-gui/smbios.json`.
7. **OpenCore on `vvfat`, read-only, on virtio-blk.** `-drive …,readonly=on,file=fat:<efi dir>` with `-device virtio-blk-pci,…,bootindex=0`. The `ide-hd` device refuses read-only drives, and `fat:rw:` with `snapshot=on` is rejected by QEMU. This needs no FAT image builder and never writes to the host directory. The EFI tree is re-extracted and the config regenerated on every start.
8. **Fresh OVMF pair and NVRAM file.** Debian's `OVMF_VARS_4M.fd` (528 KiB) pairs with its `OVMF_CODE_4M.fd`. It is copied once, through a temporary file and a rename so a failed copy is retried, to `macos-kvm-gui/OVMF_VARS_4M.fd`, a new name, so an older 128 KiB VARS from OSX-KVM is never mixed with the new code.
9. **Recovery download in Go.** The port of `macrecovery` (`recovery.go`) speaks the same HTTP protocol (session cookie, `RecoveryImage` POST, `AssetToken` downloads). Local file names are fixed (`BaseSystem.chunklist`, `BaseSystem.dmg`) so names from the server never reach the file system; it writes exclusive temporary files and renames them on success, and it feeds the existing chunklist verification.

## Risks / Trade-offs

- [The new OpenCore and SMBIOS make an existing install look like a different Mac (model iMac20,1, new serial)] → It boots the same disk. iCloud or Messages may ask the user to sign in again. Documented.
- [Upstream releases move or are removed] → The repository keeps the built files; `assets.env` or `.env` can point to a mirror.
- [No OpenCanopy: plain text picker] → Accepted. The picker only shows while install media is attached.
- [vvfat is less common than a FAT image] → Validated end to end: Sonoma 14.8.9 booted with the embedded OpenCore on the reference host, as iMac20,1 with the generated serial, at 1920x1080, with the three kexts loaded.

## Migration Plan

Existing OSX-KVM workspaces keep their `mac_hdd_ng.img` and `BaseSystem.img`. On the next start, the app writes its boot files to `macos-kvm-gui/` inside the workspace and ignores the OSX-KVM files. To return to OSX-KVM, keep using its `OpenCore-Boot.sh`, which the app does not change.
