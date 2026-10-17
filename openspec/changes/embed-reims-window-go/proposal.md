# Proposal

## Why

In the Go app (`macos-kvm-gui`, Fyne), a Reims vGPU VM's screen is a separate SDL window on the desktop, while the Screen tab only says where it is. reims-gui (Rust) already embeds that window in its Screen tab, using X11 code in `reims-gui/crates/rg-x11` that live GNOME and KDE runs have hardened: waiting for the window manager to release the window, the save-set, focus under Mutter, coalesced configure events, and release on exit. The Go app should give the same single-window experience, and should call that same code through a binding rather than keep a second copy.

## What Changes

- **New C ABI binding for `rg-x11`.** A small crate, `reims-gui/crates/rg-x11-ffi`, exposes `probe`, `Embedder::{new, find_window, embed}` and `Embedded::{set_rect, pump, hide, show, focus, unfocus, is_alive, release}` as `extern "C"` functions over opaque handles, with a generated header. It builds in Docker as a static library, like the rest of reims-gui.
- **New Go package `internal/embed`.** It wraps the binding with cgo under the `reims_embed` build tag. A stub with the same API, used without the tag, reports "embedding unavailable", so a plain `go build` still works and keeps today's behavior.
- **The Go app embeds the Reims vGPU window in the Screen tab.** This applies to a Reims VM the app itself starts while its window is on X11 (Fyne's GLFW window is X11, through XWayland on Wayland desktops). The app:
  - starts QEMU with `SDL_VIDEO_DRIVER=x11`;
  - finds the window by QEMU's PID off the UI thread;
  - reparents it into a container over the Screen tab, which follows the tab's position, size and HiDPI scale;
  - hides it under Fyne overlays (dialogs, menus) and on other tabs, and hands the keyboard to the guest on click or pointer entry, with Ctrl+Alt+Super releasing it;
  - puts it back on the desktop when the app gets SIGTERM, SIGINT or SIGHUP (QEMU still stops with the app, and the app still refuses a normal close while a VM runs).
  - While embedding, it draws its own File, Machine and Help menu buttons, whose menus are overlays it can detect, instead of Fyne's native menu bar.
- **A fix in `rg-x11`, shared with reims-gui.** FocusIn hands the keyboard to the guest only while the app is the active application.
- **Fallback is today's behavior.** VMs started by the CLI or the MCP server, embedding failures, no X11, and builds without the tag keep the standalone Reims vGPU window and the Preview-only VNC.
- **Makefile and CI.**
  - `make build-go` and `make package-go` build the static library in Docker (reusing `reims-gui/scripts/cargo-docker.sh`) and link it with `-tags reims_embed`.
  - `go test` without the tag stays Docker-free.
  - The Go CI and release workflows stay manual-only, as they are now.
- **Out of scope.** VMware SVGA VMs keep the Go app's existing in-app VNC screen: no QEMU GTK window and no `qemu-system-gui` dependency for the Go app. reims-gui (Rust) is not changed, apart from the new binding crate.

## Capabilities

### New Capabilities
- `screen-embedding`: The Go app hosts a QEMU display window (Reims vGPU's SDL3 window) inside its Screen tab through X11 reparenting, using the `rg-x11` C ABI binding. It covers when embedding applies, geometry tracking, overlays, keyboard focus, fallback and release on exit.

### Modified Capabilities
- `reims-vgpu`: The **Display** requirement changes. A Reims vGPU VM started by the Go app on X11 is shown inside the Screen tab instead of only in its own window. The standalone window, with the Preview-only VNC, remains for VMs started by other processes and when embedding is unavailable.

## Impact

- **New code:**
  - `reims-gui/crates/rg-x11-ffi`: the C ABI over `rg-x11` (`staticlib`, `cbindgen` header, `unsafe` confined to this crate);
  - `internal/embed`: a cgo wrapper and a stub, chosen by build tag.
- **Changed code:**
  - `internal/gui/screen.go` and `layout.go`: the Screen tab host widget, the overlay and tab watch, and the hand-off from Preview-only;
  - `internal/core/reimshost.go`: `reimsEnv` adds `SDL_VIDEO_DRIVER=x11` when embedding;
  - `internal/core/vm.go`: whether this start embeds;
  - `main.go`: signal handling that releases the window before exit;
  - `internal/gui/layout.go`: the app-owned menu bar;
  - `reims-gui/crates/rg-x11/src/lib.rs`: the `app_active()` guard on FocusIn;
  - `Makefile`, `scripts/package.sh`, `.github/workflows/ci.yml` and `release.yml` (manual-only).
- **Dependencies:**
  - no new Go modules;
  - the Go build gains a static library built from the existing Rust workspace (`x11rb`, pure Rust, no libX11);
  - the static library is built in the Ubuntu 24.04 image so the Go binary's glibc floor does not rise.
- **Docs:** `docs/reims-vgpu.md` (Display), `docs/development.md` (the build tag), `docs/architecture.md`, and screenshots.
