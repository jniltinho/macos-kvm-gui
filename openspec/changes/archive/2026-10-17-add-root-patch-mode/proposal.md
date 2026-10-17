# Proposal

## Why

The only GPUs that give a macOS VM real acceleration today are macOS-supported AMD cards. NVIDIA Kepler cards (GeForce GTX 6xx/7xx, GT 710/730 with the GK208 chip, Quadro K) are cheap and widely available: macOS Big Sur drives them natively, and from Monterey on the community's OpenCore Legacy Patcher (OCLP) restores Metal with a "Post-Install Root Patch" inside macOS. That patch needs OpenCore settings the app never sets (reduced SIP, AMFI handling, Secure Boot model disabled), so a Kepler card passed through to a Monterey–Sequoia VM cannot be accelerated today.

## What Changes

- **Kepler eligibility in the GPU tab:** NVIDIA Kepler cards, matched against a pinned explicit device-ID list (OCLP's `kepler_ids`, vendor `10de`, display class; never ranges or marketing names, since GT 730 also exists as Fermi and GTX 745/750 are Maxwell), become eligible for passthrough: natively on Big Sur, and with root-patch mode on Monterey through Sequoia. Maxwell and newer NVIDIA, Intel Arc/Xe and the AMD Ryzen iGPU stay ineligible, with the reason.
- **Root-patch mode (per VM, off by default, experimental):** a GPU-tab switch that changes only that VM's generated OpenCore configuration to a versioned profile matching OCLP 2.5.1's own settings for Kepler on Monterey–Sequoia: `csr-active-config` = `0x803` (`AwgAAA==`); **AMFIPass.kext** injected (no `amfi=0x80`); `ipc_control_port_options=0` and `-nokcmismatchpanic` in boot-args; RestrictEvents with `revblock=media`; KDKlessWorkaround (`MinKernel` 22.3.0) and RSRHelper. `SecureBootModel` is already `Disabled` in every VM, so it is not a new relaxation. `boot-args`, `csr-active-config` and the RestrictEvents variables are in `NVRAM › Delete`, so turning the mode **off** also clears the reduced SIP that would otherwise persist in NVRAM. FileVault must stay off. Turning it on shows what is weakened; it is stored with the VM. The app's KVM quirks stay; nothing else is copied from OCLP's real-Mac configuration.
- **Guidance, not automation:** the app does not download or run OCLP. The GPU tab lists the steps inside macOS (install OCLP, Post-Install Root Patch, reboot, reapply after every macOS update, keep automatic updates off) and, before the mode can be turned off, warns that the root patch must be reverted in OCLP first or macOS will not boot.
- **Tahoe:** excluded until a stable, validated profile exists (OCLP 3.0.0-rc.3 only has preliminary support).
- **Out of scope:** Maxwell/Pascal non-Metal patches, Intel iGPU (GVT-d) passthrough on Intel hosts, Wi-Fi/Bluetooth patches, running OCLP's patcher from Linux.

## Capabilities

### New Capabilities
- `root-patch-mode`: Kepler passthrough eligibility and the per-VM OpenCore settings for OCLP's post-install root patches.

- **Guest steps:** keep the VM's model and EFI (never use OCLP's "Build and Install OpenCore"); OCLP root-patches on iMac20,1 without "Allow native models" and downloads MetallibSupportPkg itself on Sequoia, so the guest needs internet. Keep backups of the disk, `OVMF_VARS` and the EFI before patching.
- **QEMU:** no `x-vga` and no hypervisor hiding (NVIDIA's error 43 policy is Windows-only); an optional matching `romfile` only for troubleshooting.

## Impact

- `Config` gains `root_patch` (bool); the SQLite `vms` table gains a column (`PRAGMA user_version = 3`).
- `OpenCoreConfig` emits the relaxed security settings only when the mode is on; the default configuration does not change.
- GPU eligibility (`internal/core/gpu.go`) learns the Kepler device-ID list.
