# Tasks

## 1. Settings and cache

- [x] 1.1 `settings.yaml` (embedded commented default created on first read, `KnownFields`, one document, `Setting()` precedence env > settings.yaml > `.env`, `ResetSettings`) and `LoadExtraKexts()` item validation (`https://`, `sha256`, `load` names, `load: []` = none); tests `TestLoadExtraKexts`, `TestSettingsYAML`, `TestExtraKextsEdges`.
- [x] 1.2 `httpsGetMax` that fails past the limit (`TestHTTPSGetMax`), and the size checks in `unpackKext` (`TestUnpackKextSizeMismatch`); root-patch tests still pass.
- [x] 1.3 `checkKextArchive`: count, total size, per-file size, absolute, `..` and symlink checks over the whole archive, kext layout with regular `Info.plist` and executable, several kexts require `load`; `TestCheckKextArchive`, including a valid kext beside an outside symlink.
- [x] 1.4 `DownloadExtraKexts(ctx, ks)`: context-aware flock on the cache folder, re-check after locking, full-digest folder, temp folder then rename, `entry` identity file compared on hits, cached kexts need regular `Info.plist` and executable; `TestDownloadExtraKexts` (hash mismatch caches nothing, forged folder re-downloaded, two concurrent calls make one request) and the cancellation case in `TestExtraKextsEdges`.

## 2. Boot

- [x] 2.1 `kextEntry` shared by root-patch and extra kexts; `TestRootPatchEntryUnchanged` compares the root-patch entry byte for byte.
- [x] 2.2 `Config.Extras` snapshot used by `PrepareBoot`: copy, entries after root-patch kexts, case-insensitive clash check, "not downloaded" error; `Workspace.Prepare` passes none. `TestExtraKextsBoot`: order Lilu, VirtualSMC, WhateverGreen, root-patch, extras; an empty list gives the same config; VM creation with an uncached item works offline; a removed item is gone.
- [x] 2.3 Template switches `@@ENABLED_VirtualSMC@@`/`@@ENABLED_WhateverGreen@@` and `disable`; `TestKextSwitches` (WhateverGreen disabled, Lilu and unknown names refused, an override template without switches).

## 3. GPU and callers

- [x] 3.1 Eligibility of `amdNeedsPatch` IDs from settings.yaml, and from the start snapshot in `GPUReady`; `TestPatchKextGPU`.
- [x] 3.2 GUI `startWith`: download with the start snapshot before `vm.Start`, keep the VM chosen before the download, Preferences open/reset buttons; check by hand on the test machine. — 2026-10-17: with NVMeFix 1.1.2 (not cached) in settings.yaml, Start in the GUI logged "Download the kexts listed in …", then "ok", then started the same VM; the guest loaded NVMeFix 1.1.2.
- [x] 3.3 CLI `vm start` downloads before starting, `settings show`/`settings reset --yes`; `TestSettingsAndKextsBeforeStart`.
- [x] 3.4 MCP `vm_start` downloads in-process with the request context and passes its snapshot to `vm start --kexts-snapshot`; `TestVMStartDownloadsKextsFirst` (an item that cannot be downloaded fails before any VM process is spawned).

## 4. Docs and checks

- [x] 4.1 `docs/architecture.md` (settings.yaml, extra kexts), `docs/gpu-passthrough.md` (patch-dependent AMD GPUs), `docs/troubleshooting.md`, `docs/cli.md`, `docs/mcp.md`, `docs/getting-started.md`.
- [x] 4.2 `go vet ./... && go test ./...` pass; `openspec validate add-extra-kexts --strict` passes.
- [ ] 4.3 On the test machine: boot the Nilton VM with an extra kext and `disable: [WhateverGreen]`, then with the RX 6700 XT passed through and a kext the user supplies; the config shows the kext after the built-in ones and the guest boots. — 2026-10-17, first part done: Nilton (Sequoia 15.8.1, Reims vGPU) with NVMeFix 1.1.3 and `disable: [WhateverGreen]` booted; config.plist order Lilu, VirtualSMC, WhateverGreen (Enabled false), NVMeFix; kextstat shows Lilu, VirtualSMC and NVMeFix, no WhateverGreen; Metal on the paravirtual GPU. A kext named like a built-in or root-patch one (RestrictEvents) is refused before the start. Passthrough part: not run.
