# Tasks

## 1. Core
- [x] 1.1 Kepler device-ID list and eligibility rules per macOS version (native on Big Sur; root-patch mode on Monterey–Sequoia; none on Tahoe)
- [x] 1.2 `Config.RootPatch`, validation (only Monterey–Sequoia), database column and migration to version 3
- [x] 1.3 OpenCore profile when on: csr-active-config 0x803, AMFIPass.kext, RestrictEvents (revblock=media), KDKlessWorkaround, RSRHelper, ipc_control_port_options=0 and -nokcmismatchpanic; NVRAM Delete while on and on the first boot after, so turning off restores full SIP; ocvalidate passes (off, on, clear)
- [x] 1.5 Pin the extra kexts (OCLP 2.5.1 commit, versions, SHA-256) and download them when the mode is turned on, not embedded (see design); THIRD_PARTY_NOTICES
- [x] 1.4 Tests: never-enabled config unchanged; mode on emits exactly the profile; on→off clears the NVRAM values; Kepler allowlist with negative cases (GT 730 GF108 0F02, GM107 1380-1382)
- [x] 1.6 Guest acceptance with a real Kepler card: root patch, Metal acceleration, update → repatch, revert → mode off, recovery on the emulated display. Done without a card on Sequoia 15.8.1: with the mode on, csrutil shows the custom configuration, csr-active-config 03080000, revblock media, the boot-args and the four kexts loaded; after turning it off, SIP enabled, csr-active-config 0, no revblock, no kexts — the real Kepler card part is waived by the user on 2026-10-17 (no such card); everything testable without one is done as recorded.

## 2. GUI and CLI
- [x] 2.1 GPU tab: Kepler shown as eligible with the condition; root-patch switch with the security warning and the steps inside macOS
- [x] 2.2 Turning the mode off asks to confirm the root patch was reverted in OCLP
- [x] 2.3 CLI `config set root-patch=true|false` with the same validation

## 3. Docs
- [x] 3.1 GPU passthrough guide: Kepler, OCLP steps, risks, updates, revert; troubleshooting
