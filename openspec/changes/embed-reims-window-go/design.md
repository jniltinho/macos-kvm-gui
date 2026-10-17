# Design

## Context

- **The Go app (`internal/gui`, Fyne 2.8.2 on GLFW).**
  - Its window is always an X11 window, through XWayland on Wayland desktops. Fyne exposes its id with `driver.NativeWindow.RunNative`, which hands over a `driver.X11WindowContext{WindowHandle}`.
  - The binary already needs CGO, for OpenGL and GLFW.
  - A Reims vGPU VM runs on the Reims QEMU build. `reimsEnv` strips `REIMS_VGPU_WINDOW`, so the library opens its SDL3 window. The GUI connects to the VNC socket in Preview-only mode (`previewOnly` in `layout.go`), and the Screen tab shows a notice.
  - VMware SVGA VMs already have an interactive in-app VNC screen (`screen.go`).
- **reims-gui (Rust).** `crates/rg-x11` (570 lines, `x11rb`, no unsafe) embeds that same window. Its contract is in `crates/rg-x11/API.md`:
  - `Embedder::new(display, parent)`, `find_window(pid, kind, timeout)` and `embed(child)`;
  - `Embedded::{set_rect, pump, hide, show, focus, unfocus, app_active, is_alive, release}`.

  It carries fixes found in live GNOME and KDE runs: waiting for the window manager to release a framed window, the save-set, focus on FocusIn and EnterNotify while the app is active, coalesced ConfigureNotify events, and `_REIMS_GUI_CONTAINER` marking so another instance's window is never stolen. Its tests run under Xvfb with a real SDL3 window.
- **Embedding needs X11 for QEMU's window too.** The Reims QEMU must be started with `SDL_VIDEO_DRIVER=x11`, as reims-gui does.

## Goals / Non-Goals

**Goals:**
- One X11 embedding implementation for both apps. The Go app calls `rg-x11` through a C ABI instead of porting it.
- A plain `go build` and `go test` keep working without Rust or Docker.
- Behavior parity with reims-gui for Reims vGPU VMs: geometry, overlays, focus, release on signals, and fallback.

**Non-Goals:**
- Embedding QEMU's GTK window for VMware SVGA in the Go app. Its VNC screen stays, and no `qemu-system-gui` dependency is added.
- Native Wayland embedding. That is impossible by design; the Go app is on X11 or XWayland anyway.
- Changing reims-gui's own behavior.

## Decisions

### D1. C ABI binding to `rg-x11`, not a pure-Go port
- **The choice.** A new crate, `reims-gui/crates/rg-x11-ffi` (`crate-type = ["staticlib"]`), wraps `rg-x11` in `extern "C"` functions over opaque handles:
  - `rgx_embedder_new(display, parent) -> *mut Embedder`
  - `rgx_find_window(e, pid, kind, timeout_ms, *mut u32) -> i32`
  - `rgx_embed(e, child) -> *mut Embedded`
  - `rgx_set_rect`, `rgx_pump`, `rgx_hide`, `rgx_show`, `rgx_focus`, `rgx_unfocus`, `rgx_is_alive`
  - `rgx_release(*mut Embedded)` and `rgx_embedder_free`
  - Every fallible function returns `0` or a negative code, and copies its error message into a caller-provided buffer (`char *err, size_t errlen`, truncated and NUL-terminated). There is no thread-local "last error": a goroutine can move between OS threads between two cgo calls, so the error has to come back from the same call.
- **Its rules.**
  - The header `internal/embed/rg_x11.h` is written by hand (about fifteen declarations). A Rust test compares the exported symbol list with the header's, so they cannot drift; this is simpler than adding `cbindgen` to the build.
  - The crate is the only reims-gui crate allowed `unsafe` (`#![allow(unsafe_code)]` with a module comment); the workspace lint stays `forbid` everywhere else.
  - Every entry point checks for null pointers and catches panics (`catch_unwind`), returning an error code; no panic ever unwinds into Go.
- **Alternatives.**
  - (a) A pure-Go port with `github.com/jezek/xgb`: a new Go module, about 600 lines duplicated, and every live-found fix done twice. Rejected.
  - (b) Shelling out to a helper process (`reims-gui embed-helper`): it cannot own the parent's focus and save-set semantics as cleanly, and adds IPC. Rejected.
  - (c) cgo against libX11 directly: a third implementation. Rejected.

### D2. `internal/embed` with a build tag and a stub
- **Two builds.** `embed_cgo.go` (`//go:build reims_embed`) links `-L${SRCDIR}/../../reims-gui/target/rg-x11-ffi/release -lrg_x11_ffi -ldl -lpthread -lm`. `embed_stub.go` (`//go:build !reims_embed`) has the same API, returns `ErrUnavailable`, and `Available()` is false.
- **What runs it.** `make embed-lib` runs `cd reims-gui && CARGO_TARGET_DIR=target/rg-x11-ffi scripts/cargo-docker.sh 24.04 cargo build --release --locked -p rg-x11-ffi`, so the archive is always at `reims-gui/target/rg-x11-ffi/release/librg_x11_ffi.a`, the one path cgo links. The Makefile targets that ship (`build-go`, `build-prod`, `package-go`) depend on it and pass `-tags reims_embed`. `make test` and `go test ./...` use the stub; a separate `make test-embed` runs the cgo wrapper's tests with the tag.
- **Why the 24.04 image.** Building the static library there keeps the Go binary's glibc floor where the Go release builds already are.
- **Alternative.** Always cgo, with no stub. That breaks `go build` for anyone without Docker. Rejected.

### D3. Who drives it: a small host in `internal/gui`
- **The host widget.** A `screenHost` widget replaces the notice in the Screen tab for Reims VMs. Its renderer's `Layout` gives the tab's logical rectangle; physical pixels are `fyne.CurrentApp().Driver().AbsolutePositionForObject(host)` and `host.Size()` times `win.Canvas().Scale()` (the same math as `rg_x11::to_physical`).
- **The ticker.** One 60 Hz ticker goroutine calls `fyne.Do` to:
  - apply the rectangle (`set_rect` sends nothing when it is unchanged);
  - `pump()`;
  - check visibility: the Screen tab is selected, `Canvas().Overlays().Top()` is nil, no menu of the app's menu bar is open (D6), and the window is not minimized. It calls `hide()` or `show()` on changes.
- **Embedding on start, and the run generation.**
  - Each start from the GUI bumps a run generation and records the QEMU PID.
  - `rgx_find_window` blocks, polling for up to 30 s, so it runs in a goroutine with its own `Embedder`. Its result is handed to the UI goroutine with the generation it was started for. A result whose generation or PID no longer matches the current run is dropped (the child is not touched) and that `Embedder` is freed.
  - `embed` runs on the UI goroutine. All `Embedded` calls happen there, so the handle is never shared across threads, and the `Embedder` owns its X connection, separate from GLFW's.
  - When `is_alive()` turns false, a pump fails, or the VM's state goes to Stopped, the UI goroutine calls `release()` and clears the handle in the same `fyne.Do`. The ticker checks for a nil handle first, and on app exit it is stopped, and a pending tick drained, before any handle is freed.
- **Keyboard and pointer.**
  - Focus comes from `rg-x11`: EnterNotify, and FocusIn while the pointer is on the guest, both only while `app_active()` (FocusIn gains that check in this change, so a queued event never refocuses the guest after the user switched to another application). A click on the guest calls `focus()`.
  - Ctrl+Alt+Super is Reims vGPU's own capture release. It does not move X focus, so the app adds no key of its own.
  - Giving the keyboard back to the app: Fyne delivers a click only to the widget hit, so no Fyne layer can see every click, and GLFW owns the app window's ButtonPress selection (exclusive in X11). So `rg-x11` gains `Embedded::return_focus_on_app_click()`, called every tick. It asks the server (`GetInputFocus` and `QueryPointer`) and, while the keyboard is in the guest, a pointer button is down and the pointer is in the app's window but not over the guest, gives the keyboard to the app's window. A click on any control (list, entry, button) thus hands the keyboard back, and the control still gets its click. Because EnterNotify only fires on entry, the guest takes the keyboard again only when the pointer re-enters it or it is clicked. Polling at 60 Hz catches any click held for 16 ms or more.

### D4. Only the app's own starts; X11 for both windows
- **The app's window on X11.** Fyne 2.8.2 picks native Wayland when the compositor offers server-side decorations (`internal/driver/glfw/wayland_csd_linux.go`). In GUI mode, when `embed.Available()`, `$DISPLAY` is set and `FYNE_PLATFORM` is not, `main.go` sets `FYNE_PLATFORM=x11` before the app starts, so the window is on X11 (XWayland) and has an id to embed into. An explicit `FYNE_PLATFORM` wins: with `wayland`, `RunNative` yields no `X11WindowContext` and the app falls back.
- **The start option.** `core.VM` gains `EmbedX11 bool`, set only by the GUI when `embed.Available()` and the window's `RunNative` yields `X11WindowContext`. With it, `reimsEnv` adds `SDL_VIDEO_DRIVER=x11`. The CLI and MCP never set it, so their VMs keep the default SDL backend and their own window, as the spec requires.
- **Finding the window.** `find_window` uses QEMU's PID from `VM.Start`, so a VM found running at startup (an outside VM) is never embedded.

### D5. Release on signals, not VM survival
- **Normal quit.** The Go app keeps refusing to close while a VM runs (`SetCloseIntercept` in `gui.go`), so a normal quit never has a guest embedded.
- **Signals.** In GUI mode, `main.go` runs `signal.Notify(SIGTERM, SIGINT, SIGHUP)`. The handler does `fyne.Do(release all)`, then quits, with a 2-second fallback `os.Exit(143)`, mirroring reims-gui's `on_term_signal`.
  - The point is to keep the X server safe: Xwayland 24.1 crashes when a client dies with a foreign window in its save-set inside a Mutter frame.
  - Keeping QEMU alive is not the point. QEMU still has `Pdeathsig: SIGTERM` (`vm.go`) and stops with the app, as today and as in reims-gui.
- **Not covered.** SIGKILL is not handled: the save-set returns the window to the root, and the known Xwayland crash is documented as in reims-gui.

### D6. App-owned menu bar
- **The problem.** Fyne's GLFW main menu (`SetMainMenu`) draws its dropdowns as private renderer children, not in the overlay stack, so they cannot be detected and would sit under the native guest window.
- **The choice.** While the embedding is in use, the app replaces `SetMainMenu` with its own row of flat buttons (File, Machine, Help), in the same place and with the same items. Each opens a `widget.PopUpMenu`, which lives in `Canvas().Overlays()`, so D3's overlay check covers it. Keyboard access (Alt+F and the like) is kept with `desktop.CustomShortcut` on the canvas.
- **Alternative.** Patching Fyne's private menu bar. That ties the app to Fyne internals. Rejected.

## Risks / Trade-offs

- **Stale handles.** [GLFW recreates the window on fullscreen toggles, which would orphan the container] → Fyne keeps one GLFW window per `fyne.Window` and the fullscreen toggle does not recreate it; a test checks the parent id after `SetFullScreen`. If the id changes, the ticker re-embeds.
- **Build complexity.** [The Go release build now needs Docker and the Rust image] → only with the tag. The stub keeps a no-Docker path, and the Go workflows are manual-only.
- **Overlay detection.** [Fyne has no global "an overlay opened" event, and its main menu is not an overlay] → poll `Overlays().Top()` each tick (60 Hz, well under 1 ms) and use the app-owned menu bar (D6), whose menus are overlays.
- **Focus fights.** [The guest refocuses itself after the user gave the keyboard back to the app] → focus is taken only on EnterNotify, a click, or FocusIn with the pointer on the guest, all guarded by `app_active()`. `unfocus()` is called on Fyne-side clicks.
- **Unsafe code in the workspace.** [It weakens `forbid(unsafe_code)`] → it is confined to `rg-x11-ffi`, the rest of the workspace keeps `forbid`, and the FFI surface is small and fuzz-free (plain integers and pointers). It gets a unit test per entry point, null cases included.
- **Thread safety.** [Calling `Embedded` from several goroutines] → the Go wrapper is not goroutine-safe by contract; only the UI goroutine owns it (D3), `find_window` uses its own `Embedder`, and errors come back from the same cgo call (D1).
- **Stale lookups.** [A lookup for a stopped run embeds into the next one] → the run generation and PID check of D3.

## Migration Plan

Additive. Builds without the tag behave as today. Rollback means building without `-tags reims_embed`.

## Open Questions

- Whether to also expose `last_latency` in the Go Log tab for diagnostics. This is deferrable and does not change the API shape.
