# Proposal

## Why

Some AMD GPUs (Navi 22 RX 6700/6750, Navi 23 RX 6650 XT, Navi 21 XTXH, Lexa, RX 580 2048SP) only work in macOS with a third-party patch kext, so the app refuses to pass them through. Users who already have such a kext have no way to give it to OpenCore: the EFI is rebuilt from the embedded assets on every start.

## What Changes

- New user settings file `~/.config/macos-kvm-gui/settings.yaml`, embedded as a commented default and created on first use: `theme`, `assets_dir` (the runtime `.env` keys, which it replaces; environment variables still win and an existing `.env` still works below it), `kexts` and `disable`. Reset to default from Preferences or `settings reset --yes`; `settings show` prints it.
- `kexts` lists the user's extra kext archives: HTTPS zip `url`, optional `sha256`, optional `load` order. The user chooses and supplies every URL; the app never names, pins, bundles or fetches any specific third-party kext.
- The GUI, the CLI `vm start` command and the MCP server download the listed archives before starting a VM, when they are not cached yet, into a locked cache shared by the Workspace's VMs; archives are size-limited, validated as a whole and checked against the optional SHA-256 before anything is extracted. Creating a VM never needs them.
- Shared download and unzip helpers fail at their limits instead of truncating (this also hardens root-patch mode).
- Every start copies the cached kexts into OpenCore's `Kexts` folder and loads them after the built-in kexts (and after root-patch mode's kexts).
- `disable` names built-in kexts (for example `WhateverGreen`) whose OpenCore entry is disabled, for patch kexts that conflict with them.
- GPU passthrough: the patch-dependent AMD IDs become eligible while `kexts` lists an item; otherwise the reason points to `settings.yaml`.
- Docs: the settings section, the GPU passthrough guide and troubleshooting.

## Capabilities

### New Capabilities
- `extra-kexts`: the user settings file and user-supplied kexts loaded by OpenCore (download, verification, cache, boot injection, disabling built-in kexts) and the GPU eligibility that depends on them.

### Modified Capabilities

## Impact

- `internal/core`: new `extrakexts.go`, `settings.default.yaml`; `settings.go` (settings.yaml, Setting precedence, reset); `boot.go` (PrepareBoot signature, OpenCoreConfig); `workspace.go` (Prepare passes no extras); `vm.go` (start snapshot, GPU re-check); `rootpatch.go` (shared kext entry, `unpackKext` overflow check); `installer.go` (`httpsGetMax`); `gpu.go` (eligibility of `amdNeedsPatch` IDs).
- `internal/assets/config.plist.tmpl`: `@@ENABLED_VirtualSMC@@` and `@@ENABLED_WhateverGreen@@` switches.
- `internal/gui/layout.go`, `cmd/vm.go`, `internal/mcp/server.go`: download before start. `internal/gui/settings.go`: Preferences open/reset. New `cmd/settings.go`.
- `gopkg.in/yaml.v3` becomes a direct dependency (it was already indirect).
- Docs: `docs/architecture.md`, `docs/gpu-passthrough.md`, `docs/troubleshooting.md`.
- Network access only when `kexts` lists items that are not cached.
