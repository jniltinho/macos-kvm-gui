# rg-reims API

Reims vGPU's QEMU packages: the embedded manifest and Dockerfile, install of a prebuilt package,
Docker build, activation and pruning. Port of the Go app's `internal/core/reims.go` (`ReimsDir`,
`ReimsQEMU`) and `internal/core/reimspkg.go`, with `internal/core/reims/{Dockerfile,manifest.json}`
copied to `reims/` and embedded.

The API is blocking: run it on a worker thread (`tokio::task::spawn_blocking`). Long calls take a
`cancel: &AtomicBool` (Go's `context`) and a `log: &(dyn Fn(&str) + Sync)` that gets each progress
line, possibly from another thread.

## Items

| Rust | Go |
| --- | --- |
| `RELEASES: [&str; 2]` (`24.04`, `26.04`) | `ReimsReleases` |
| `DOCKERFILE: &str` | `reimsDockerfile` |
| `manifest() -> Result<Manifest>` (`tag`, `commit`, `packages[release] = Package { file, sha256 }`) | `loadReimsManifest` |
| `rg_core::config::reims_dir(workspace_root) -> Result<PathBuf>` | `ReimsDir` |
| `Reims::new(reims_dir)` | (globals) |
| `Reims::dir()` | |
| `Reims::host_ubuntu_release() -> Result<String>` | `HostUbuntuRelease` |
| `Reims::qemu() -> Result<PathBuf>` | `ReimsQEMU` |
| `Reims::installed() -> Result<Info>` (`qemu`, `build`, `commit`, `apple_gfx: Result<()>`) | `ReimsInstalled` / `ReimsInfo` |
| `Reims::install(cancel, log) -> Result<()>` | `InstallReims` |
| `Reims::build(release: Option<&str>, cancel, log) -> Result<()>` | `BuildReims` (`None`/`""` = the host's release) |
| `reims_commit(build_name) -> &str` | `reimsCommit` |
| `Error { Msg(String), Io(io::Error), Cancelled }`, `Result<T>` | `error` |

```rust
let dir = rg_core::config::reims_dir(&workspace_root)?;
let reims = rg_reims::Reims::new(dir);
reims.install(&cancel, &|line| println!("{line}"))?;
let info = reims.installed()?;
```

## Behavior (same as the Go app)

- `qemu()`: the `reims_qemu` setting (`MKG_REIMS_QEMU` relative to the current directory,
  settings.yaml's or `.env`'s relative to their folder, made absolute, must exist), then
  `<Workspace>/.reims/qemu/current/bin/qemu-system-x86_64`, then
  `$XDG_DATA_HOME/reims-ctl/qemu/current/bin/qemu-system-x86_64` (regular files only).
- `install()`: host release from `/etc/os-release`, package from the manifest, host-wide flock on
  `<ReimsDir>/.lock` (waits until `cancel`), HTTPS-only download (redirects stay on HTTPS, at most
  5; 30 s connect, 60 s to the response head, no overall timeout) capped at 512 MiB, SHA-256 check,
  unpack into `qemu/.tmp-*` then rename to `qemu/<tag>-ubuntu<release>`.
- Unpack refuses: absolute names, `..` components, `.` as anything but a folder, links that are
  absolute or contain `..`, anything under a link, existing files (`O_EXCL`), hard links, devices,
  FIFOs, GNU sparse entries, more than 20000 entries; every decompressed byte and the declared file
  sizes (cumulative) count against 2 GiB.
- `build()`: release check, `docker info`, lock, `docker build -` with the embedded Dockerfile
  (`DOCKER_BUILDKIT=1`, image `macos-kvm-gui-reims:<release>`), `docker run` with
  `HOST_UID`/`HOST_GID` and `<ReimsDir>/build-<release>` at `/build`, then the build is moved to
  `qemu/<date>-<commit>-ubuntu<release>`. Cancel sends SIGINT (docker passes it on), SIGKILL 30 s
  later.
- Activation: only for the host's release (others stay staged); the build must have
  `share/qemu/ReimsVgpu.rom`, resolve every library with `ldd` (missing ones are named with the
  Ubuntu package to `apt install`), and list `"apple-gfx-pci"` in `-device help` (30 s timeout).
  `current` is switched atomically (temp symlink + rename).
- Pruning (after every activation, even a failed one): builds of the same release other than the
  new one and `current`'s are removed, unless a `/proc/<pid>/exe` (canonicalized) is inside the
  build (canonicalized too), a process whose exe cannot be read has a `qemu-system*` comm, or the
  build path does not resolve.

## Deviations

- Globals that Go tests swapped (`osReleaseFile`, `lddCommand`, `reimsReleaseURL`,
  `maxReimsExpanded`, the HTTP transport) are private fields of `Reims`, set by the crate's tests.
- `ReimsDir`, settings.yaml's `reims_qemu` and the lookup order are rg-core's (`reims_dir`,
  `AppDir::reims_qemu_setting`, `find_reims_qemu`); downloads use rg-core's `http::Client`.
  The Workspace root itself (the `workspace` preference in the SQLite DB) is the caller's to pass.
- Messages name `reims-gui reims install|build` instead of `macos-kvm-gui ...`.
- Cancelling a download is seen between reads (Go's context aborts a blocked read).
- Spawning a binary retries on `ETXTBSY` (a QEMU this process just unpacked).
