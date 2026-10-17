# Tasks

## 1. Build and packages

- [x] 1.1 `.github/workflows/reims-release.yml`: on `reims-*` tags, matrix 24.04/26.04 with the embedded Dockerfile, strip, `tar.gz`, `.sha256`, source archive with `vendor/qemu` and `cargo vendor`, notices; check by pushing a tag on a fork or `act`, and that both packages unpack and list `apple-gfx-pci` in their `ubuntu:<release>` containers. — 2026-10-17: the workflow published reims-c099dd5 (24.04 and 26.04 packages, .sha256, source archive); it checks `apple-gfx-pci` in each `ubuntu:<release>` container before publishing.
- [x] 1.2 `internal/core/reims/manifest.json` and `InstallReims(ctx, log)`: host release detection, download, SHA-256, safe unpack, promotion; tests with a local TLS server for a good package, a tampered one, an archive with `..` or an outside link, and an unsupported release.
- [x] 1.3 Embedded `internal/core/reims/Dockerfile` and `BuildReims(ctx, release, log)`: release and Docker checks, host-wide lock, build, strip, chown, staging, host validation (`ldd`, `apple-gfx-pci`), atomic `current` switch, pruning that spares builds in use; unit tests for release detection, the `ldd` parser and the package hints, promotion and pruning with fake folders; on this Ubuntu 24.04 host a real build activates `current`.
- [x] 1.4 Build for 26.04 on this host: it stays staged (not activated), and its QEMU runs `-device help` inside an `ubuntu:26.04` container with the runtime packages.

## 2. Core

- [x] 2.1 `Config.Graphics` with validation (values, exclusive with `GPU`), schema 4 migration, export/import; tests for an old database, a round trip and the exclusivity error.
- [x] 2.2 `reims.go`: `ReimsQEMU()` (`reims_qemu` in settings.yaml, `MKG_REIMS_QEMU`, `<Workspace>/.reims`, upstream's path), `apple-gfx-pci` probe, Vulkan ICD, `vm.max_map_count` and udmabuf checks over `Host` roots; table tests with a fake sysfs/procfs.
- [x] 2.3 `BuildArgs` for Reims VMs (memfd, S3 off, `-vga none`, `apple-gfx-pci` without a fixed address, xHCI interrupt mapping, one `snd0` audiodev bound to HDA and `usb-audio`, `-display none` plus VNC) and unchanged arguments for `vmware-svga`; golden tests for both, and a paused realization with the installed Reims QEMU (skipped when absent).
- [x] 2.4 `vm.Start` runs the Reims QEMU with `REIMS_VGPU_WINDOW` removed and runs the checks first; test the error messages for a missing build and a low map count.

## 3. Interfaces

- [x] 3.1 Settings › Display: Graphics choice, the macOS-26 note, `METAL_ERROR_MODE` commands, disabled GPU choice while Reims is selected (and the reverse); Details pane. Check by hand.
- [x] 3.2 GUI VNC split: preview-only connection for Reims VMs (started by the app and by CLI/MCP), no Screen attachment, Screen tab text; check by hand with a VM started from the app and one from `vm start`. — 2026-10-17 on this host (RTX 4070): the Nilton VM (Reims vGPU) started by `vm start` shows "Running (outside the app)", the Preview follows it, Details says "VNC on a private socket (Preview only)" and the Screen tab says the screen is in the Reims vGPU window; started from the app, the same Preview-only connection and status "In the Reims vGPU window; the Preview follows it".
- [x] 3.3 Host tab: Reims checks, installed build and commit, **Install Reims vGPU** (package) and **Build from source** (Docker) buttons with progress in the log. — 2026-10-17: `reimsSection` in internal/gui/gui.go (checks, build/commit, Install and Build from source with progress in the Log tab).
- [x] 3.4 CLI `config set graphics=…`, `config show`, `reims install`, `reims build [--release]`, `reims status`; command tests for an invalid value, the exclusivity error and an unsupported release. — 2026-10-17: `reims install|build|status` and `config set graphics=` exist; TestReimsCommands and TestConfigCommands cover the unsupported release, the invalid value and the exclusivity error; `reims status` on this host shows build 20261016-220959-c099dd5a-ubuntu24.04 with apple-gfx-pci.

## 5. Resolution and ReimsVgpu.kext

- [x] 5.1 `Config.Resolution` with the allowed list, schema 5 migration, export/import, `resolution=` in `apple-gfx-pci`; tests for validation, an old database, a repeated migration, a round trip and the arguments.
- [x] 5.2 Settings › Display Resolution choice (enabled with Reims, warning above 2048 px unless Tahoe), Details shows it, the wizard keeps the default; Preview pacing from the frame size.
- [x] 5.3 CLI `config set resolution=…` (warning above 2048 px unless Tahoe), `config show`; command tests for an invalid value and the warning.
- [x] 5.4 ReimsVgpu.kext: pinned commit and SHA-256s, download to `<Workspace>/.kexts/reims-util-b302389`, injection for Tahoe + Reims at a start only (MinKernel 25.0.0, after Lilu and the built-ins, before extra kexts), `ReimsVgpu` reserved for extra kexts, downloads in GUI start, `vm start` and MCP `vm_start`; tests with a local TLS server (hash mismatch caches nothing, injection only for Tahoe + Reims, entry order, the clash).
- [x] 5.5 Docs (`docs/reims-vgpu.md`, `docs/cli.md`, `docs/architecture.md`, `docs/troubleshooting.md`) and `THIRD_PARTY_NOTICES.md`.

## 4. Docs and validation

- [x] 4.1 `docs/reims-vgpu.md` (install, build from source, host setup, guest setup, limits), `THIRD_PARTY_NOTICES.md`, `docs/architecture.md`, `docs/troubleshooting.md`, `docs/cli.md`, `docs/references.md`.
- [x] 4.2 `go vet ./... && go test ./...`; `openspec validate add-reims-vgpu --strict`; Codex review of the change. — 2026-10-17: `go vet` and `go test -race ./...` pass, TestReimsRealizesPaused passes with the installed Reims QEMU, `openspec validate --all --strict` passes; Codex reviewed the change during implementation.
- [x] 4.3 On this host: the Nilton VM (Sequoia) with `reims-vgpu` boots on the RTX 4070 with `METAL_ERROR_MODE` set; `system_profiler SPDisplaysDataType` over SSH shows the paravirtual GPU with Metal; switching back to `vmware-svga` boots as before.
