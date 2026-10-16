# Design

## Context

`PrepareBoot` rebuilds `EFI/` from the embedded assets and renders `config.plist.tmpl`. It runs on every start (`vm.go`) and also when a VM or workspace is created (`Workspace.Prepare`, called by the GUI wizard, the installer flow, `cmd/vms.go` and `cmd/workspace.go`). Root-patch mode (`rootpatch.go`) downloads kexts with `httpsGet`, checks SHA-256, unpacks with `unpackKext` into `<Workspace>/.kexts/...`, copies with `copyTree` and adds `Kernel > Add` entries through `@@ROOTPATCHKEXTS@@`. `httpsGet` and `unpackKext` truncate at their limits instead of failing. The MCP server starts VMs by spawning `vm start` and waits 30 s for QEMU's VNC socket. `Setting()` re-reads `.env` on every call. `Workspace.Lock` is a per-VM-folder flock.

## Goals / Non-Goals

**Goals:**
- Reuse root-patch mode's pieces where they are safe; harden the shared ones.
- One user settings file, `settings.yaml`, for every user setting (theme, boot files, extra kexts), applied to every VM.

**Non-Goals:**
- Per-VM kext lists, a form-based GUI editor (Preferences opens the file and resets it), ACPI/SSDT files, `DeviceProperties`, boot-args per kext.
- Knowing or validating what a kext does, or whether it suits the GPU.
- Reading `Info.plist` to order kexts by dependency.
- Cleaning stale cache folders.

## Decisions

- **settings.yaml replaces the runtime `.env`.** `gopkg.in/yaml.v3` was already in the module graph. The file is created on first read from an embedded default whose keys are all commented out (so it does not override the theme chosen in Preferences); unknown keys fail (`KnownFields`). `Setting()` reads environment, then `settings.yaml`, then the old `.env` for compatibility. Alternative: env variables with a URL-fragment syntax (`#sha256=...&kexts=...`), rejected as hard to read and to edit.
- **Settings snapshot.** `LoadExtraKexts()` reads the file once into an `ExtraKexts` value (entries, disabled names). Callers that start a VM put it in `Config.Extras` (never saved); `DownloadExtraKexts(ctx, ks)`, `GPUReady` and `PrepareBoot` use that value. `vm.Start` loads it only when the caller did not. `Workspace.Prepare` passes nil, so creation needs no network and no cache; the EFI is rebuilt on every start anyway. Alternative: re-read the file in each phase, rejected (codex #11).
- **Item identity.** Cache folder `<Workspace>/.kexts/extra/<full sha256 of url, sha256 and load>/` holding the kexts plus an `entry` file with that identity and the kext names, inside a folder published by rename. A hit requires `entry` to equal the requested text, so a key collision or a half-written folder never counts.
- **Cross-process lock.** `DownloadExtraKexts` holds a blocking flock on `<Workspace>/.kexts/extra/.lock` for the whole download, re-checks readiness after taking it, extracts into a temporary sibling folder and renames it into place. Stale temporary folders (`.tmp-*`) are removed only while holding the lock. Alternative: tolerate concurrent renames, rejected as harder to reason about.
- **Downloads that fail instead of truncating.** New `httpsGetMax(ctx, url, max)` reads `max+1` bytes and fails when it gets more; `httpsGet` keeps its current behavior for the installer catalog. Archive limit 32 MiB.
- **Archive prevalidation.** `checkKextArchive` walks the whole central directory before extracting: entry count ≤ 4096, sum of uncompressed sizes ≤ 128 MiB, no absolute paths, no `..`, no symlinks, anywhere in the archive. Extraction then copies with a per-file `LimitReader(size+1)` and fails if more bytes come than the header declared, so a lying header cannot truncate silently. `unpackKext` gets the same overflow check, which also fixes root-patch mode; its OCLP archives stay within the limits (existing tests keep passing).
- **Kext layout and names.** A kext is a root-level `<Name>.kext/` with regular `Contents/Info.plist` and `Contents/MacOS/<Name>`. Names match `[A-Za-z0-9._+-]{1,64}`: valid on FAT and needing no XML escaping. Collisions are checked case-insensitively against built-in, root-patch and other extra kexts, since the EFI is a FAT disk.
- **Order inside an archive.** One kext needs nothing. Several require `load: [A, B]`, which picks and orders them. Alternative: dependency order from `OSBundleLibraries`, rejected: needs a plist parser and still guesses.
- **Plist entries.** One `kextEntry(name, minKernel, comment)` helper, used by `rootPatchPlist` (output byte-identical) and the extras, appended after root-patch kexts through the same placeholder.
- **Disabling built-ins with template switches.** The template gets `@@ENABLED_VirtualSMC@@` and `@@ENABLED_WhateverGreen@@` in place of `<true/>` under each entry's `Enabled` key; they render `<true/>` by default and `<false/>` when disabled. A name with no switch in the template in use (Lilu, an unknown name, or an `MKG_ASSETS_DIR` template without the switch) is an error. Alternative: matching the Comment/Enabled text, rejected (breaks on reformatted override templates, codex #10). Override templates without switches keep working when nothing is disabled.
- **Reset.** `ResetSettings()` writes the embedded default; the CLI `settings reset --yes` and a confirmed Preferences button call it. `settings show` prints the path and the settings in effect.
- **Callers.** GUI `startWith`, CLI `vm start` (before `vm.Start`) and the MCP `vm_start` tool (in-process, before spawning the CLI, with the request context so the client can cancel) call `DownloadExtraKexts`. `vm.Start` itself never downloads; with an uncached entry its `PrepareBoot` fails with the "start from the GUI, `vm start` or the MCP server" error. The MCP 30 s timeout then covers only QEMU.
- **Eligibility.** `eligibility()` makes `amdNeedsPatch` IDs eligible when `LoadExtraKexts()` succeeds with at least one entry; a file error becomes the reason. `GPUReady` re-checks the start snapshot (`Config.Extras`) for a patch-dependent GPU, so removing the setting after choosing the GPU fails clearly. `gpuMinVersion` stays unchanged for these IDs.

## Risks / Trade-offs

- [A kext panics the guest or conflicts with a built-in one] → opt-in and global; removing the item restores the previous boot; `disable` handles known conflicts. Docs say to try it with a disposable VM.
- [Settings affect every VM] → documented; per-VM lists are a later change.
- [A broken `settings.yaml`] → `Setting()` ignores it (theme and boot files fall back to `.env`/defaults), while starts and the GPU tab show the error with the file's path.
- [A malicious archive] → HTTPS only, size and count limits, prevalidation, optional SHA-256. The kext still runs in the guest kernel by the user's choice.
- [A blocking flock while a slow download holds it] → the second start waits; the GUI shows its usual progress dialog, and the MCP request can be cancelled.
- [Stricter `unpackKext`] → affects root-patch mode only when a file is larger than its header says, which a valid archive never is.

## Migration Plan

None: the default file sets nothing, and an existing `.env` keeps working below `settings.yaml`. Rollback is `settings reset --yes`. The template change is internal; override trees made from an older template keep working unless they disable a kext.
