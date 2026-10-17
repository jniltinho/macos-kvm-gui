# rg-assets API

Port of the Go app's `internal/assets`, `boot.go`, `extrakexts.go`, `rootpatch.go` and
`reimskext.go`. Everything is blocking; async callers use `spawn_blocking`. Errors are
`rg_assets::Error` (`Io`, `Zip`, `Json`, `Http`, `Invalid`, `Cancelled`, `Context`), with the
Go app's messages.

Unlike the Go code, nothing reads global settings: the caller passes the `MKG_ASSETS_DIR`
setting, the Workspace folder and the host keyboard (all from `rg-core`).

## Embedded files

The files come from `reims-gui/assets`, a committed copy of `internal/assets` made by
`scripts/sync-assets.sh` (the build container mounts only `reims-gui/`).

| Item | Go |
| --- | --- |
| `BootAssets::open(dir: Option<&Path>) -> Result<BootAssets>`: `Embedded`, or `Dir(path)` once it has every embedded file | `bootAssets()` |
| `BootAssets::read(path) -> Cow<'static, [u8]>`, `BootAssets::files(prefix)`, `BootAssets::versions()` | `fs.ReadFile`, `AssetVersions` |
| `asset_versions(dir: Option<&Path>) -> String` | `AssetVersions()` |
| `LOGO`, `icon("start")`, `version_icon("sonoma")` (SVG bytes) | `assets.Logo`, `Icons`, `VersionIcons` |

## Boot

| Item | Go |
| --- | --- |
| `Identity { serial, mlb, uuid, rom }`, `Identity::generate()`, `Identity::load_or_create(ws_dir)` (`smbios.json`) | `identity`, `newIdentity`, `loadIdentity` |
| `KextCache::new(workspace_dir)`: `<Workspace>/.kexts`, with `root_patch_dir()`, `reims_kext_dir()`, `extra_dir()` | `RootPatchKextsDir`, `ReimsKextDir`, `ExtraKextsDir` |
| `BootOptions { with_media, verbose_boot, extras: Option<&ExtraKexts>, keyboard }` | `withMedia`, `Config.VerboseBoot`, `Config.Extras`, `RecoveryKeyboard()` |
| `BootState { root_patch_was_on, reims_kext, extra_kexts }` (what `prepare_boot` found) | `Config`'s private fields |
| `opencore_config(&assets, &config, &identity, &opts, &state) -> Result<String>` | `OpenCoreConfig` |
| `prepare_boot(ws_dir, &config, &assets, &cache, &opts) -> Result<()>` | `Workspace.PrepareBoot` |

`prepare_boot` extracts the EFI tree, copies OVMF code, creates the NVRAM once, loads or creates
the identity, copies the root-patch kexts (`config.root_patch`), ReimsVgpu.kext (Tahoe on Reims
vGPU, only with `opts.extras` set, i.e. at a start) and the settings.yaml kexts, writes
`config.plist`, the root-patch marker and the `.contentVisibility` files.

## Extra kexts (settings.yaml)

| Item | Go |
| --- | --- |
| `extra_kexts_ready(&ks, &cache) -> bool` | `ExtraKexts.Ready` |
| `download_extra_kexts(&ks, &cache, cancel: &AtomicBool)` (flock on `.kexts/extra/.lock`, polled every 200 ms until `cancel`) | `DownloadExtraKexts` |
| `kext_switches(tmpl, disabled) -> Result<String>` | `kextSwitches` |

`ExtraKext`/`ExtraKexts` (checked when settings.yaml is loaded), the workspace layout constants
(`EFI_DIR`, `ROOT_PATCH_MARKER`...), `smbios_model` and `root_patch_version` are rg-core's;
`is_kepler` is rg-host's.

## Root-patch mode and ReimsVgpu.kext

| Item | Go |
| --- | --- |
| `root_patch_kexts_ready(&cache)`, `download_root_patch_kexts(&cache, cancel)` (OCLP 2.5.1, pinned SHA-256) | `RootPatchKextsReady`, `DownloadRootPatchKexts` |
| `clear_root_patch_marker(ws_dir, &config, ran: Duration)` | `Workspace.ClearRootPatchMarker` |
| `REIMS_UTIL_COMMIT`, `reims_kext_ready(&cache)`, `download_reims_kext(&cache, cancel)` (pinned SHA-256 per file) | `reimsUtilCommit`, `ReimsKextReady`, `DownloadReimsKext` |

## Downloads

rg-core's `http::Client` (ureq 3 with rustls, webpki roots), https only (redirects too, at most 5), 60 s per request.
A body over the limit fails instead of being truncated (archives 32 MiB, OCLP zips 16 MiB,
ReimsVgpu files 4 MiB; 16 MiB per unpacked file, 128 MiB and 4096 entries per archive).
`cancel` is checked before each request and between reads.
