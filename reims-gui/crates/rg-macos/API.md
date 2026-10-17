# rg-macos API

Recovery download and verification, DMG conversion, full installers and the offline install
disc. Port of the Go app's `internal/core/recovery.go`, `chunklist.go`, `installer.go` and the
media part of `workspace.go` (`DownloadMedia`, `MediaVersion`).

Everything is blocking: call it from a worker thread. Long operations take
`cancel: &AtomicBool` (checked between reads and chunks; a cancelled call returns
`Error::Cancelled`) and `progress: &mut dyn FnMut(Progress)`.

## Common types

| Item | Notes |
| --- | --- |
| `Http` | The HTTP clients, built on rg-core's `http::Client` (ureq 3 + rustls/ring). `Http::default()`; share one. API client: HTTPS only (redirects too, at most 5), 60 s. Download client: HTTPS only, 30 s connect, 60 s to the response headers, no overall timeout. Recovery clients: plain HTTP to `osrecovery.apple.com`, User-Agent `InternetRecovery/1.0`. Proxies from the environment. |
| `Http::https_get(url, limit)` | Go's `httpsGet`: refuses non-HTTPS URLs, any status but 200 is an error, reads at most `limit` bytes. |
| `Http::https_get_max(url, max)` | Go's `httpsGetMax`: fails instead of truncating past `max`. |
| `Progress { phase, done, total }` | `total == 0` = indeterminate. Units: bytes (downloads, package check), chunks (recovery check), thousandths (conversion). `pct()` gives 0–100. |
| `Error` | `Cancelled`, `Io(io::Error)`, `Other(String)` with the Go app's messages. `Result<T>`. |

## Recovery and install media

| Item | Notes |
| --- | --- |
| `RECOVERY_PRODUCTS`, `recovery_product(version)`, `RecoveryProduct { board_id, mlb, os_type }` | macrecovery's table by short version (`high-sierra` … `tahoe`). |
| `download_recovery(http, version, dir, cancel, progress) -> (dmg, chunklist)` | Session cookie, `RecoveryImage` POST, then `BaseSystem.chunklist` and `BaseSystem.dmg` into `dir` (fixed names; server names never reach the disk) through exclusive `<name>.<random>.part` files renamed when complete. Fewer bytes than `Content-Length` is an error; on any error nothing is left. Phases `STAGE_CHUNKLIST`, `STAGE_IMAGE`. No resume (as in Go). |
| `verify_image(dmg, chunklist, progress(done, total))` | Chunklist header checks, RSA-2048 PKCS#1 v1.5 SHA-256 signature with Apple's EFI ROM key (signature stored little-endian), per-chunk SHA-256, missing or trailing bytes rejected. |
| `download_media(http, vm_dir, version, cancel, progress, log)` | `Workspace.DownloadMedia` without the lock (the caller holds the VM folder's lock, rg-core): free-space check (4 GiB), staging in `vm_dir/STAGING_DIR` (always removed), download, verify, `qemu-img convert -p -f dmg -O raw` (percentages to the bar, other lines to `log`), old `MEDIA_MARKER` removed, image renamed to `MEDIA_FILE`, marker written. Any failure keeps the previous media and marker. |
| `STAGE_*` | Same texts as Go. `MEDIA_FILE`, `MEDIA_MARKER`, `STAGING_DIR` and `media_version` are rg-core's (`workspace`, `Workspace::media_version`). |

## Full installers (shared cache)

`dir` below is the cache folder, `installers_dir(workspace)` = `<Workspace>/.installers`, for the
resolved absolute Workspace (rg-core's `WorkspaceRoot` / `WorkspaceDir`).

| Item | Notes |
| --- | --- |
| `installer_supported(version)` | Big Sur and later. |
| `find_installers(http, version) -> Vec<InstallerProduct>` | Apple's catalog (`.sucatalog.gz` first, gzip-decoded, then the plain one), products with `InstallAssistant.pkg`, distributions fetched 6 at a time, kept when the major version matches and the distribution lists the VM model's board ID. Newest first. |
| `InstallerProduct` | Serializes like Go's (`product_id`, `version`, `build`, `url`, `size`, `integrity_url`); also the cache's `info.json`. |
| `download_installer(http, dir, version, cancel, progress, log) -> CachedInstaller` | Newest product; returns the cached one if the build matches. Otherwise: per-build folder, exclusive `flock` on `.lock` (one download across the app, CLI and MCP), integrity data (CNKL method 2), free-space check (2× size + 1 GiB), resumable download into `InstallAssistant.pkg.part`, integrity check (a bad package is deleted; a cancelled check keeps it), disc written, `info.json` written atomically. |
| `cached_installers(dir)`, `cached_installer_for(dir, version)` | Verified installers (valid build name, disc larger than the package), newest first / newest of that major version. |
| `remove_installer(dir, build)` | Refused while a VM holds the disc. |
| `use_disc(path) -> DiscLease` | Shared `flock` while a VM runs with the disc; dropping the lease releases it. `disc_in_use(path)` tries an exclusive one. |
| `refresh_installer_disc(&CachedInstaller)` | Rewrites a cached disc's head when the script changed. |
| `installer_name("15.8.1")` | `"Sequoia"`. |
| `DISC_FILE`, `DISC_VOLUME`, `DISC_COMMAND`, `DISC_TYPED` | As Go. The disc script is `src/install.sh`, byte-identical to Go's `discScript`. |

Resume: a `Range: bytes=N-` request; 206 with a matching `Content-Range` appends, 200 restarts
the file, 416 deletes it and starts over, a file longer than the package is deleted. Network
errors are retried 5 times (2 s, 4 s, … between connection attempts). `http://` package URLs are
fetched over HTTPS.

## Not here

- The workspace lock, `WorkspaceRoot`, `Config.WorkspaceDir`, `version_display`, `smbios_model`,
  `write_file_atomic`, `free_bytes`, `valid_bridge_name`: rg-core. Holding the lock around
  `download_media`: rg-ops.
- Attaching the disc to QEMU (`BuildArgs`), `GuestKeys`/`RecoveryTerminal`: rg-qemu.
