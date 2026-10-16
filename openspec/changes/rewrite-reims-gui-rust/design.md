# Design

## Context

The Go app (about 18k lines: `internal/core`, `internal/gui`, `internal/mcp`, `cmd`) is the behavior reference; its OpenSpec changes and docs are the spec. Three projects are in `base/` (git-ignored), studied read-only:

- **reims-vgpu** (GPL-2.0-or-later OR LGPL-3.0-or-later, Rust 1.98.1, edition 2024): `crates/reims-ctl` is a binary-only CLI/TUI whose non-TUI modules (`vm.rs`, `qemu.rs`, `qmp.rs`, `runtime.rs`, `host.rs`, `gpu.rs`, `install.rs`, `paths.rs`, `view/keys.rs`) depend on nothing UI-related. The display library owns its window (SDL3 + ash, `crates/reims-vgpu-window`) inside QEMU's process; there is no API to render into another process's window or export frames, and every path through QEMU's console surface (VNC, screendump, D-Bus 2D) pays a slow GPU readback (0.42 s per 1080p frame measured here).
- **rustXterm** (MIT, Rust 1.99, egui/eframe 0.36.2 on wgpu, Wayland and X11): a VirtualBox-like shell (menu/toolbar/sidebar/tabs), a theme token system, dialogs, an RFB client and a per-frame texture widget (`crates/vnc`, `crates/ui/src/desktop.rs`), an MCP server on `rmcp` 3.5 and clap CLI, Docker-only builds on an Ubuntu 24.04 toolchain image, `egui_kittest` UI tests.
- **PhotoCraft** (MIT OR Apache-2.0, egui 0.36/wgpu 30): theme tokens with shadows/radii and contrast tests, Lucide icons, widgets (cards, dropdowns, toggles, dialog buttons), "dialogs as data", a wgpu canvas with dirty-rect uploads.

## Goals / Non-Goals

**Goals:**
- Feature parity with the Go app, same data, same CLI/MCP surface, same GUI structure.
- The Reims display inside the Screen tab.
- Docker-only builds and `.deb`/`.tar.gz` packages for Ubuntu 24.04 and 26.04; CI and releases on GitHub.
- Work split into crates that parallel agents can implement independently.

**Non-Goals:**
- Other distributions, macOS or Windows hosts, arm64.
- Phase 2 (Wayland-native dma-buf frame sink in the Reims library): a later change.
- Changing the Go app beyond keeping it working until removal.

## Decisions

### D1. Workspace and crates
`reims-gui/` is a Cargo workspace (resolver 3, edition 2024, Rust 1.99.0 pinned in `rust-toolchain.toml`, exact dependency pins, `unsafe_code = "forbid"` except in `reims-gui-x11` where needed, clippy denies `unwrap`/`expect` outside tests). Layers (lower never depends on higher):

| Layer | Crate | Owns | Ported from |
| --- | --- | --- | --- |
| 0 | `rg-core` | types (Config, VmEntry, Graphics, NIC...), errors, paths, settings.yaml, SQLite registry (rusqlite, bundled) with schema 1–5 migrations, workspace, keyboard maps, export/import | `config.go`, `vms.go`, `settings.go`, `workspace.go`, `bundle.go`, `keyboard.go`, `keys.go`, `network.go` |
| 1 | `rg-assets` | embedded OpenCore EFI, kexts, OVMF, `config.plist.tmpl` (from `internal/assets`, via `include_dir`), EFI extraction, config rendering, extra kexts, root-patch kexts, ReimsVgpu.kext | `boot.go`, `assets/`, `extrakexts.go`, `rootpatch.go`, `reimskext.go` |
| 1 | `rg-host` | host checks and setup (pkexec), GPU listing/eligibility/VFIO reservation, Reims host checks | `host.go`, `gpu.go`, `reimshost.go`, reims-ctl `host.rs`/`gpu.rs` |
| 1 | `rg-macos` | recovery download, chunklist and RSA signature check, DMG conversion, full installer catalog/download/cache, offline disc | `recovery.go`, `chunklist.go`, `installer.go` |
| 2 | `rg-qemu` | QEMU argument building (SVGA and Reims), process runtime and logs, running-VM detection, QMP client, RFB client, input key mapping | `vm.go`, `running.go`, `qmp.go`, `vnc.go`, reims-ctl `qemu.rs`/`qmp.rs`/`runtime.rs`/`view/keys.rs` |
| 2 | `rg-reims` | Reims packages (manifest, install, Docker build, activation, pruning) | `reimspkg.go`, `reims/` |
| 2 | `rg-x11` | X11 embedding of QEMU's Reims window (x11rb) | new |
| 3 | `rg-ui` | the egui app: shell, theme, tabs, dialogs, wizard, Screen (VNC texture or X11 embed) | `internal/gui`, rustXterm `crates/ui`, PhotoCraft `ui-egui` (copied, trimmed) |
| 3 | `rg-agent` | clap CLI and `rmcp` MCP server | `cmd/`, `internal/mcp` |
| 4 | `apps/reims-gui` | the binary | `main.go` |

Each crate exposes a small public API; the scaffold commits stub signatures first so crates can be built in parallel.

### D2. GUI toolkit: egui/eframe 0.36.2 on wgpu
Both GUI bases use it; it does per-frame textures well (Screen), runs on X11 and Wayland, and has `egui_kittest` for screenshot tests. Immediate mode means the wizard and dialogs are explicit state machines ("dialogs as data", PhotoCraft). Alternatives: GTK4 (gtk4-rs; good dma-buf story for phase 2, but no reusable base here), iced, slint — rejected for lack of a base and the user's ask to reuse these projects.

### D3. Reims display inside the window (phase 1: X11 embedding)
- QEMU for a Reims VM runs with `SDL_VIDEO_DRIVER=x11` (and `DISPLAY` from the app), so its Reims window is an X11 window even on Wayland desktops (through XWayland).
- The app itself runs on X11 when embedding is enabled: before eframe starts it drops `WAYLAND_DISPLAY` from its own environment if `DISPLAY` is set, so winit picks X11/XWayland.
- `rg-x11` (x11rb): finds the Reims window by `_NET_WM_PID` = the QEMU pid and title "Reims vGPU" (polling until it maps, up to 30 s), creates a child container window of the app's X11 window (from eframe's `raw-window-handle`), `ReparentWindow`s the Reims window into it, and every frame moves/resizes the container to the Screen tab's rectangle in physical pixels (hidden when the tab is not visible). A click in the area gives the Reims window input focus (`SetInputFocus`); the library's own capture (Ctrl+Alt+Super to release) keeps working. On VM stop the window disappears with QEMU.
- If no X server is reachable, or embedding fails, the Reims window stays standalone (today's behavior) and the tab says so.
- Alternatives: VNC (readback too slow), `-display dbus` (same readback; apple-gfx has no GL scanout), Wayland embedding (no cross-process embedding in Wayland), in-process library (impossible). Phase 2 adds a dma-buf "remote sink" in our fork of `reims-vgpu-window`, already built in our Docker packages.

### D4. VNC for VMware SVGA
Port the Go RFB client (Raw, DesktopSize, Extended Key Event, pacing) to `rg-qemu` rather than rustXterm's vendored vnc-rs: it is small, known to work with QEMU's server, and keeps scancodes for any keyboard layout. The Screen widget follows rustXterm's `desktop.rs` (texture `set_partial` on dirty rects, letterbox fit, keyboard grab).

### D5. Data compatibility
Same paths and formats: `os.UserConfigDir()/macos-kvm-gui/` (DB, `settings.yaml`, `.env` fallback), Workspace layout, JSON export and `.tar` bundles. The DB code ports the Go migrations verbatim; tests open a database written by the Go app (fixtures generated by the Go tests).

### D6. Async and processes
tokio multi-thread runtime owned by a `Backend` (rustXterm pattern) between UI and core; the UI never blocks and gets events over bounded channels, requesting repaints. Long operations (downloads, builds, VM runs) are tasks with progress events shown in the Log tab, like Go's `u.run`.

### D7. CLI and MCP
clap with the same command tree and flags as `cmd/`; MCP on `rmcp` 3.5 over stdio with the same tools and JSON schemas as `internal/mcp`, starting VMs through `reims-gui vm start` as the Go server does.

### D8. Build, packaging, CI
- `reims-gui/docker/Dockerfile`: `ubuntu:${UBUNTU}` (24.04 or 26.04) with build deps and the pinned Rust toolchain; `reims-gui/scripts/cargo-docker.sh <release> <cmd>` runs cargo as the user with the source mounted and caches in `reims-gui/target/` (rustXterm pattern).
- `reims-gui/scripts/build.sh <release>`: release build in that release's image (so the binary links that release's glibc and libraries), strip, `.tar.gz` (binary, license, notices, README), `.deb` built with `dpkg-deb` (Depends computed with `dpkg-shlibdeps` in the container; `qemu-system-x86`, `qemu-utils` recommended), `.sha256` files.
- CI (`reims-gui-ci.yml`): fmt, clippy, tests in the 24.04 image; build both releases. Release (`reims-gui-release.yml`): on `reims-gui-v*` tags, both releases' packages to a GitHub Release.

### D9. License
`reims-gui` takes reims-ctl code, so it is `GPL-2.0-or-later OR LGPL-3.0-or-later`; rustXterm (MIT) and PhotoCraft (MIT OR Apache-2.0) code keep their notices. PhotoCraft's brand mark and names are not copied; Lucide icons (ISC), Inter/JetBrains Mono (OFL) keep their licenses.

### D10. Parallel work
After the scaffold (workspace, crate stubs with the APIs from D1, Docker image, CI skeleton), one agent per crate works in its own worktree on its own crate only, building and testing in Docker with its own `CARGO_TARGET_DIR`. Integration (wiring the UI to real crates, live tests on the Nilton VM, docs, screenshots) follows.

## Risks / Trade-offs

- [X11 reparenting is fragile: focus, keyboard grab, window manager fights, HiDPI] → phase 1 tests on GNOME (24.04/26.04, Wayland session via XWayland) and on this X11 KDE host; a standalone-window fallback stays; phase 2 removes the dependency.
- [Parity of an 18k-line app] → `PARITY.md` checklist generated from the Go code and specs, checked per item; the Go app stays until it is all checked.
- [GPL obligations] → source is public in this repository; notices shipped in packages.
- [Very new toolchain and pinned dependencies] → Docker image pins them; upgrades are deliberate.
- [egui immediate mode CPU use] → continuous repaint only while the Screen tab shows a VNC VM; the X11-embedded Reims window repaints itself.

## Migration Plan

Both apps read the same data; users can try `reims-gui` and go back. When `PARITY.md` is complete, a later change removes the Go code and makes `reims-gui` the release.

## Open Questions

- Phase 2's frame sink protocol (dma-buf with modifiers, sync fds) is designed in its own change.
