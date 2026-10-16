# Tasks

## 1. Scaffold (before parallel work)

- [ ] 1.1 `reims-gui/` workspace: `Cargo.toml` (resolver 3, edition 2024, lints, exact pins), `rust-toolchain.toml` (1.99.0), crates from design D1 with stub public APIs, `apps/reims-gui` printing `--version`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `PARITY.md` (checklist generated from the Go code); verify `cargo check --workspace` in Docker.
- [ ] 1.2 `reims-gui/docker/Dockerfile` (ubuntu 24.04/26.04 build images, pinned toolchain) and `scripts/cargo-docker.sh`; verify both images build and run `cargo --version`.

## 2. Core crates (parallel, one agent each)

- [ ] 2.1 `rg-core`: types, settings.yaml, SQLite registry with schema 1–5 migrations, workspace, keyboard maps, network, export/import; tests including a database written by the Go app.
- [ ] 2.2 `rg-assets`: embedded assets, EFI extraction, config.plist rendering (byte-compared with the Go output), extra kexts, root-patch kexts, ReimsVgpu.kext; tests ported from Go.
- [ ] 2.3 `rg-host`: host checks and setup, GPU listing/eligibility/VFIO reservation, Reims host checks; fake-sysfs tests ported from Go.
- [ ] 2.4 `rg-macos`: recovery download and verification, DMG conversion, full installers and offline disc; tests ported from Go (signature, chunklist, catalog).
- [ ] 2.5 `rg-qemu`: QEMU args (golden tests against the Go args for SVGA and Reims), runtime, running detection, QMP, RFB client, key mapping.
- [ ] 2.6 `rg-reims`: Reims packages install/build/activate/prune with the embedded manifest and Dockerfile; tests ported from Go.
- [ ] 2.7 `rg-x11`: find, reparent and track QEMU's Reims window; test under Xvfb in Docker with a dummy X client.

## 3. Interfaces (parallel after the core APIs exist)

- [ ] 3.1 `rg-ui` shell: theme (light/dark/system, Go colors), menus, toolbar, VM list, tabs, status bar, modals with depth; kittest snapshots.
- [ ] 3.2 `rg-ui` dialogs: Settings pages, New VM wizard, Preferences, Offline installers, confirmations.
- [ ] 3.3 `rg-ui` tabs: Details (with Preview), Install stepper, Host (with Reims section), GPU, Log.
- [ ] 3.4 `rg-ui` Screen: VNC texture widget for SVGA VMs; X11-embedded Reims window for Reims VMs with fallback.
- [ ] 3.5 `rg-agent`: CLI command tree and MCP tools with tests matching the Go tests.

## 4. Build, packages, CI

- [ ] 4.1 `scripts/build.sh <release>`: release builds, `.tar.gz`, `.deb` (dpkg-shlibdeps), `.sha256`; verify each `.deb` installs in a clean `ubuntu:<release>` container.
- [ ] 4.2 `.github/workflows/reims-gui-ci.yml` and `reims-gui-release.yml`; lint with actionlint.

## 5. Integration and release

- [ ] 5.1 Wire the UI to the real crates; `PARITY.md` fully checked.
- [ ] 5.2 Live tests on this host: the Nilton VM on Reims inside the Screen tab, on SVGA over VNC; CLI and MCP start/stop; screenshots.
- [ ] 5.3 Docs (`reims-gui/README.md`, `docs/reims-gui.md`), Codex review, fixes.
- [ ] 5.4 Release `reims-gui-v0.1.0` with packages for 24.04 and 26.04.
