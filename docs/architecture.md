# Architecture

[Documentation index](README.md)

The app owns one QEMU child process per instance; it is not a libvirt frontend and cannot attach to an existing QEMU process. Host support is Linux. There is no bridge/tap networking or physical USB passthrough UI.

[![Architecture](diagrams/architecture.png)](diagrams/architecture.svg)

## Package layout

| Location | Responsibility |
| --- | --- |
| `main.go`, `cmd/` | `main.go` calls `cmd.Execute`; `cmd/` holds one Cobra command per file. Without a command the GUI opens. |
| `internal/mcp/` | The MCP server (`macos-kvm-gui mcp`, official Go SDK, stdio): VM, screen, input, installer and host tools on top of `internal/core`. It finds a running VM through the QEMU process that has its disk open, drives it through that run's VNC socket and its second QMP socket (`qmp-control.sock`), and starts VMs as a detached `vm start` so they outlive the agent. |
| `internal/core/keys.go`, `keyboard.go` | Key codes (X11 keycodes to XT scancodes), key chords by name, text tables for U.S. and ABNT2, the layout-independent install line, and the host layout behind NVRAM `prev-lang:kbd`. |
| `internal/gui/` | VirtualBox-style window: menus, toolbar, VM list and Details (`layout.go`), toolbar buttons, compact key/value sections and sliders (`vbox.go`), the Settings and Preferences dialogs (`settings.go`) with the Network page (`network.go`), the Install stepper and the cards shared with Host and GPU (`install.go`), the New VM wizard (`wizard.go`), VM management dialogs (`vms.go`), the remaining tabs (`gui.go`), the VNC screen (`screen.go`) and the theme (`theme.go`: dark, light or system, square corners). Icons are original SVGs in `internal/assets/icons`. Long operations run off the UI thread; callbacks use `fyne.Do`. |
| `internal/core/config.go`, `settings.go` | Validated VM configuration, version-derived hardware profiles and runtime environment settings. |
| `internal/core/installer.go` | The full installer: Apple's Software Update catalog (a small XML plist reader), version and board-ID selection, resumable download, the CNKL method-2 integrity check, the shared cache and the installer disc: a small ISO 9660 writer (the package in pieces under 4 GiB) with the recovery script. |
| `internal/core/network.go` | Network adapters: models and the macOS driver matrix, MAC validation and generation, QEMU arguments, bridge discovery and the `bridge.conf` ACL check, the SSH port check, the qcow2 virtual size. |
| `internal/core/vms.go`, `bundle.go`, `running.go` | The VM list and preferences in SQLite, the Workspace, move, export/import bundles and detecting a running QEMU (also one started outside the app or by the MCP server) through `/proc`, with its sockets and attached disc; `CheckRunDir` accepts only sockets of this user in a private 0700 run directory. |
| `internal/core/host.go`, `gpu.go` | Host checks, sysfs PCI/DRM/IOMMU discovery, GPU guards and constant privileged scripts. |
| `internal/core/rootpatch.go` | Root-patch mode for NVIDIA Kepler: OCLP 2.5.1's Kepler ID list, the four kexts downloaded from OCLP's pinned commit (SHA-256), and the config.plist parts (SIP 0x803, kexts, boot-args, `revblock`); a marker in the VM folder makes the first boot with the mode off delete the lowered NVRAM values. |
| `internal/core/reims.go`, `reimshost.go`, `reimspkg.go`, `reims/` | [Reims vGPU](reims-vgpu.md): finding its QEMU, the host checks (`apple-gfx-pci`, Vulkan, `vm.max_map_count`, `/dev/udmabuf`), the QEMU arguments, and installing a prebuilt package or building one in Docker from the embedded `reims/Dockerfile` and `reims/manifest.json`. |
| `internal/core/workspace.go` | Workspace lock, staging, disk creation and media removal. |
| `internal/core/recovery.go`, `chunklist.go` | Apple recovery protocol and signed chunklist/image verification. |
| `internal/core/boot.go` | Asset extraction, SMBIOS identity and OpenCore template rendering. |
| `internal/core/vm.go`, `qmp.go` | QEMU argument construction, process lifecycle and QMP control/events. |
| `internal/assets/` | Embedded boot tree, OpenCore template and original app logo. |
| `scripts/` | Fetch boot assets, fetch the test validator, package release artifacts. |

`internal/core` is **one Go package**, independent of Fyne, not a separate package per source file. Host filesystem roots and command execution are injectable for tests.


## Command line and display (added later)

- `main.go` only calls `cmd.Execute()`. `cmd/` holds one Cobra command per file (`check`, `setup`, `config`, `workspace`, `download`, `installer`, `disk`, `media`, `vm`, `gpu`, `reims`, `mcp`, `version`, `gui`); without a command the binary opens the GUI from `internal/gui`. Both front ends use the same `internal/core`, so the CLI and the app follow the same rules (locks, validation, verification).
- With **Display: In the app**, QEMU runs with `-display none -vnc unix:<private dir>/vnc.sock,lock-key-sync=off` (no `-k`: with it QEMU drops the physical scancode of extended key events), next to the QMP socket in the same 0700 runtime directory. `internal/core/vnc.go` is a minimal RFB 3.8 client (None security, 32-bit RGB, Raw, CopyRect, DesktopSize and QEMU extended key events, so keys travel as scancodes). `internal/gui/screen.go` draws the frames at up to 60 fps, hides the host pointer, maps keys and the mouse through the same fit-to-window math as the image, and reserves Ctrl+Alt+F as the host key for full screen.

## From Start to macOS

[![Boot flow](diagrams/boot-flow.png)](diagrams/boot-flow.svg)

1. `VM.Start` refuses an already-running VM, validates a copy of the configuration, requires the VM folder and `mac_hdd_ng.img`, and refuses a disk another QEMU already uses. A selected GPU is rechecked with `GPUReady` before taking the workspace lock. Host-tab checks are advisory, not a complete start-time gate.
2. The lock is held for the whole QEMU run. `PrepareBoot` replaces the extracted EFI tree, refreshes OVMF CODE, preserves existing VARS/identity, and renders `EFI/OC/config.plist`.
3. A private temporary directory (0700) holds `qmp.sock`. `BuildArgs` derives QEMU arguments from configuration, media presence and GPU slot functions. Commas in paths are escaped as `,,`.
4. QEMU runs as the launching user: KVM, q35, OVMF pflash, OpenCore via read-only `vvfat` on virtio-blk, and SATA-attached qcow2 disk/optional raw recovery. `vmware-svga`, virtual USB keyboard/tablet and audio devices are retained. A VM with `graphics=reims-vgpu` runs the [Reims vGPU](reims-vgpu.md) QEMU instead of `qemu-system-x86_64` from `PATH`, after its host checks (`ReimsReady`), with `apple-gfx-pci` in place of `vmware-svga`, memfd-backed RAM, S3 off, USB audio and `-display none` plus the VNC socket; see [what changes in QEMU's command line](reims-vgpu.md#what-changes-in-qemus-command-line). Running-VM detection still works: its `/proc/<pid>/comm` is `qemu-system-x86` too. Networking has one `-netdev`/`-device` pair per enabled adapter (up to four): user-mode NAT or `bridge` through `qemu-bridge-helper`, with the adapter's model and MAC. Adapter 1 keeps its place among the devices, adapters 2–4 sit on fixed PCI slots (`bus=pcie.0,addr=0x16`–`0x18`), and with none enabled QEMU gets `-nic none`. Guest port 22 is forwarded only from `127.0.0.1:<ssh_port>`, on the first NAT adapter; further NAT adapters get their own subnet. A VM without an adapter list keeps the single NAT adapter and MAC `52:54:00:c9:18:27` it always had. Before QEMU starts, `VM.Start` checks bridged adapters (bridge, helper, `bridge.conf` ACL) and that the SSH port is free.
5. OVMF loads OpenCore. With `BaseSystem.img`, the text picker is shown (five seconds, `ScanPolicy=0`, which the Sonoma recovery needs: with the file-system lock it stops at the prohibited sign); a `.contentVisibility` file containing `Disabled` at the root of OpenCore's FAT volume and in `EFI/BOOT` hides that volume ("QEMU VVFAT"), so **macOS Base System** is the first, default entry. Without it, the picker is hidden (zero timeout) and `ScanPolicy=0x301` permits APFS **and HFS+**, hiding the boot FAT volume. Output is requested at `1920x1080@32`. A selected GPU adds `agdpmod=pikera` to `keepsyms=1`, and **Start with verbose boot** (`vm start --verbose-boot`) adds `-v debug=0x100` for that start only. NVRAM `prev-lang:kbd` is set (and refreshed through `NVRAM › Delete`) to English with the host's keyboard layout (`internal/core/keyboard.go`: `en-US:0` U.S., `en-US:128` Brazilian ABNT2); the option is a field of the configuration that is never saved (`json:"-"`). `Validate` refuses a core count that is not a power of two (1, 2, 4, 8, 16): with 6 cores × 2 threads the guest kernel hangs right after `EXITBS:START`, and `Recommended` only proposes such counts (24 host threads give 4 cores × 2).
6. The app connects to QMP in the background (up to 50 attempts, 100 ms apart), negotiates `qmp_capabilities` and sends commands with five-second deadlines. `system_powerdown` requests ACPI shutdown; only process exit means Stopped. `BLOCK_IO_ERROR` with `action=stop` and `STOP` produce Paused (error); `RESUME` restores Running. There is no UI pause/resume control.
7. On exit, output is interpreted, QMP and its temporary directory are cleaned up, and the lock is released. `Pdeathsig=SIGTERM` terminates QEMU if the creating app thread dies; Start/Wait pin that OS thread. An app crash is therefore a forced-stop risk, not a saved VM state.

Profiles use Penryn through Monterey and Skylake-Client from Ventura onward. The NIC is vmxnet3 on High Sierra and virtio-net-pci otherwise. Only Sonoma is validated end to end.

## Workspace and host files

Each VM has its own folder, by default `<Workspace>/<name>` with the Workspace at `~/macOS-VMs`:

```text
<Workspace>/<name>/
  mac_hdd_ng.img                    # qcow2 guest disk
  BaseSystem.img                   # optional raw recovery media
  .macos-kvm-gui-media              # downloaded media's version
  .macos-kvm-gui-staging/           # temporary download/conversion area
  macos-kvm-gui/
    efi/EFI/BOOT/BOOTx64.efi
    efi/EFI/OC/                    # OpenCore, drivers, kexts, generated config.plist
    OVMF_CODE_4M.fd                 # refreshed firmware code
    OVMF_VARS_4M.fd                 # persistent guest NVRAM
    smbios.json                    # persistent serial, MLB, UUID and ROM
```

The Workspace also holds data shared by every VM: `.installers/` (cached full installers), `.kexts/` (downloaded kexts) and `.reims/` for [Reims vGPU](reims-vgpu.md):

```text
<Workspace>/.reims/
  qemu/<build>/                    # an installed package or build: bin/qemu-system-x86_64, share/qemu/ReimsVgpu.rom
  qemu/current -> <build>          # switched only after the build runs on this host
  build-<release>/                 # Docker build cache (reims build)
  .lock                            # serializes installs and builds
```

The normal user config directory is `~/.config/macos-kvm-gui/` (`os.UserConfigDir`, respecting `XDG_CONFIG_HOME`). It contains `macos-kvm-gui.db` and the optional runtime `.env`. The database (SQLite through the pure-Go `modernc.org/sqlite`, WAL, `PRAGMA user_version` for migrations) has a `vms` table (one row per VM, with its network adapters as JSON in `networks`) and a `settings` table (selected VM, Workspace, theme, VM list width). Version 1 created the tables; version 2 added `networks` (an empty value keeps an older VM on its single NAT adapter with the legacy MAC); version 3 added `root_patch`; version 4 added `graphics` (`''` or `vmware-svga`, the default, or `reims-vgpu`), so existing VMs keep VMware SVGA. Each migration step is idempotent and the version is written last, so an interrupted upgrade runs again on the next start. On first use it imports an older `config.json` and renames it to `config.json.migrated`. Saving a VM also writes a read-only snapshot to `<folder>/macos-kvm-gui/vm.json`.

Workspace `flock` files live outside the workspace in `$XDG_RUNTIME_DIR`, falling back to `os.TempDir()` when unset. Names contain the first eight bytes of SHA-256 of the cleaned workspace path. The same lock covers preparation, media work, disk creation/removal and a running VM across app instances. The kernel releases it on process death; an existing lock file alone does not mean it is held. External QEMU processes do not participate, so QEMU's own disk lock is still important.

GPU reserve/release additionally use `/run/lock/macos-kvm-gui-host.lock`. Privileged scripts handle host setup and reservation, never QEMU itself. **Set up host** installs missing QEMU packages with `apt-get`, loads `kvm_amd`/`kvm_intel` if needed, creates/adds the current user to the `kvm` group, enables/persists `ignore_msrs`, and raises `vm.max_map_count` to 2097152 in `/etc/sysctl.d/90-macos-kvm-gui.conf` when it is lower (for Reims vGPU). It uses validated arguments to a constant script and does not add `libvirt`/`input` groups, upgrade an existing QEMU or rebuild initramfs. See [GPU passthrough](gpu-passthrough.md) and [troubleshooting](troubleshooting.md#ignore_msrs).

### Persistent NVRAM and identity

VARS is copied through a temporary file/rename only when absent, never overwritten on normal preparation/start. `smbios.json` is generated once with a 12-character serial, 17-character MLB, UUID v4 and six-byte ROM. Back up both with the guest disk while stopped. Deleting the identity changes the guest's apparent Mac; it is not a general troubleshooting step.

The generated model is iMac18,3 for High Sierra, iMac19,1 for Mojave, and iMac20,1 for Catalina–Tahoe. The iMac profile avoids AppleMCEReporter loading on AMD hosts; no extra MCE-disabler kext is embedded. Existing OSX-KVM disks/media are reused, but its boot scripts, OpenCore and NVRAM files are not used or modified.

## Boot assets pipeline

[![Boot assets pipeline](diagrams/assets-pipeline.png)](diagrams/assets-pipeline.svg)

`assets.env` pins versions, download URLs and SHA-256 values. `scripts/fetch-assets.sh` verifies all five downloads before deleting/rebuilding `internal/assets/files`; extraction itself is not transactional. It writes `files/VERSIONS` and obtains OpenCore's test-only `ocvalidate`.

`internal/assets/assets.go` embeds `all:files`, `config.plist.tmpl` and the logo. The binary needs no boot-asset download at runtime. `PrepareBoot` extracts the EFI directory into `<workspace>/macos-kvm-gui/efi`, which QEMU exposes as a **read-only FAT disk** using `file=fat:<path>`. Guest writes cannot edit that host EFI tree; local edits there are replaced on the next start.

Pinned versions are OpenCore 1.0.8, Lilu 1.7.2, VirtualSMC 1.3.8, WhateverGreen 1.7.1 and Debian OVMF 2025.02-8+deb13u1. The authoritative pins are [`assets.env`](../assets.env); licenses are in [third-party notices](../THIRD_PARTY_NOTICES.md).

## Recovery download and verification

The Go macrecovery port obtains a session cookie from `http://osrecovery.apple.com/`, POSTs a board/MLB product identity to `/InstallationPayload/RecoveryImage`, then downloads chunklist and DMG URLs using the returned AssetTokens. Server-provided filenames are not used locally. API requests have a 60-second timeout; large image downloads have header/TLS timeouts but no overall client timeout.

Downloads use exclusive `.part` temporary files and fixed final names in staging. The app validates the CNKL header, an RSA-2048 PKCS#1 v1.5/SHA-256 signature with Apple's EFI public key, every DMG chunk's SHA-256, and rejects missing or trailing image bytes. Do not confuse HTTP transport with signature verification: the recovery API is HTTP, and verification is required before conversion.

`qemu-img convert -p -f dmg -O raw` writes a staged image on the workspace filesystem. Only successful conversion is renamed to `BaseSystem.img`; earlier failures preserve the previous image. The old version marker is removed before publication; a later marker-write failure leaves the version unknown rather than stale. Staging is removed on normal success/failure return. The app records the requested release family, not a verified exact macOS point version.

## Settings and overrides

| Source | Meaning and precedence |
| --- | --- |
| `macos-kvm-gui.db` | One row per VM: name, folder, version, `ram_mib`, cores, vCPUs, `ssh_port`, display, optional GPU PCI address, network adapters, root-patch mode, graphics adapter; plus the Workspace and theme preferences. Save validates first. `--config <file.json>` uses a standalone JSON file instead. |
| `settings.yaml` | The user's settings file, `~/.config/macos-kvm-gui/settings.yaml`: `theme`, `assets_dir`, `kexts`, `disable` and `reims_qemu` (below). Created on first use from a commented default that sets nothing; **File › Preferences › Reset to default** or `settings reset --yes` writes that default again. |
| `MKG_ASSETS_DIR` / `assets_dir` | Runtime boot tree instead of embedded assets. **Environment > `settings.yaml` > the older `.env`**; empty uses built-ins. |
| `MKG_THEME` / `theme` | `dark`, `light` or `system`; it wins over the theme chosen in File › Preferences. Same precedence. |
| `MKG_REIMS_QEMU` / `reims_qemu` | The `qemu-system-x86_64` a VM on Reims vGPU runs, instead of `<Workspace>/.reims/qemu/current` or upstream's `~/.local/share/reims-ctl/qemu/current`; see [which QEMU runs](reims-vgpu.md#which-qemu-runs). Same precedence. |
| `kexts`, `disable` | Extra kexts OpenCore loads in every VM, and built-in kexts to disable: see [extra kexts](#extra-kexts). |
| Repository `.env` and `assets.env` | Asset-fetch scripts only: **environment > repository `.env` > `assets.env`**. Not the runtime settings file. |

An example `settings.yaml`:

```yaml
theme: light
assets_dir: /home/your-user/src/macos-kvm-gui/internal/assets
kexts:
  - url: https://example.com/MyKext-1.0.zip
    sha256: 0123…(64 hex digits)
disable: [WhateverGreen]
```

The file is one YAML document; an unknown key is an error, reported with the file's path when a VM starts and in the GPU tab (the theme and boot files then fall back to the environment, `.env` and defaults). `settings show` prints the path and the settings in effect.

The older runtime `.env` still works below `settings.yaml`. It accepts plain `KEY=value` lines, trims surrounding whitespace/quotes and skips comment lines; it does not execute shell code or expand `$HOME`/`~`. Use a literal absolute directory in it. For example:

```text
MKG_THEME=light
MKG_ASSETS_DIR=/home/your-user/src/macos-kvm-gui/internal/assets
```

An override tree needs `config.plist.tmpl` and the same `files/` contents as `internal/assets`. Every embedded file except `files/VERSIONS` is checked for existence before preparation; missing versions merely display as unknown. This checks completeness, not trust or binary compatibility: use trusted boot assets and a matching OpenCore template. See [development](development.md#updating-boot-assets).

### Extra kexts

`kexts` in `settings.yaml` lists kexts the user chooses, for example a patch kext a GPU needs. The app ships, names and fetches none on its own: every URL comes from the user. Each item has:

| Key | Meaning |
| --- | --- |
| `url` | `https://` URL of a zip with `<Name>.kext` at its root (`Contents/Info.plist` and `Contents/MacOS/<Name>`). |
| `sha256` | Optional, recommended: the archive's SHA-256; a mismatch stops the start and caches nothing. |
| `load` | Only for a zip with several kexts: which to load, in load order (`load: [Provider, Plugin]`). |

Before a start, the app (GUI, `vm start` and the MCP server's `vm_start`) downloads the items that are not cached yet into `<Workspace>/.kexts/extra/<sha256 of the item>/`, under a host-wide lock so two starts download an item once. Archives are refused, before anything is extracted, when larger than 32 MiB, expanding past 128 MiB, holding more than 4096 entries or a file over 16 MiB, or with an absolute path, `..` or a symbolic link anywhere. Kext names must match `[A-Za-z0-9._+-]{1,64}`.

Every start copies the cached kexts into the EFI and loads them after Lilu, VirtualSMC, WhateverGreen and root-patch mode's kexts, in list order. A name clashing with another kext, ignoring case (OpenCore's disk is FAT), stops the start. A start reads `settings.yaml` once and uses that snapshot throughout; creating a VM needs neither the kexts nor the network. Removing an item removes its kext from the next boot; the cache stays until deleted by hand.

`disable` turns off built-in kexts, `VirtualSMC` or `WhateverGreen`, for a kext that replaces one; Lilu cannot be disabled. An `assets_dir` template without the `@@ENABLED_<Name>@@` switches works, but cannot disable a kext.
