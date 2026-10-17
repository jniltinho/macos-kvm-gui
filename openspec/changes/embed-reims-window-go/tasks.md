# Tasks

## 1. C ABI binding (Rust)

- [x] 1.1 Add the `reims-gui/crates/rg-x11-ffi` crate: — done: 4 tests (header vs exports, null handles, errors in the caller's buffer, a real embed through the C ABI under Xvfb); clippy clean.
  - `staticlib` with the `extern "C"` functions of design D1 over opaque handles;
  - `catch_unwind` on every entry point; errors copied into the caller's buffer (no thread-local state);
  - the hand-written header `internal/embed/rg_x11.h`, with a Rust test that its declarations match the exported `rgx_*` symbols;
  - `unsafe` allowed only in this crate.

  Verify: `cargo-docker.sh 24.04 cargo clippy -p rg-x11-ffi -- -D warnings` is clean, and `cargo test -p rg-x11-ffi` covers null handles, an invalid display, and a real embed under Xvfb with the `sdl_window` example.
- [x] 1.2 Build `librg_x11_ffi.a` from the Makefile (`make embed-lib`), in the Ubuntu 24.04 image, with `CARGO_TARGET_DIR=target/rg-x11-ffi`, so it lands at `reims-gui/target/rg-x11-ffi/release/`, the path cgo links. — done: `make embed-lib`; `nm -g` lists all 14 `rgx_*` symbols.

  Verify: `nm -g` lists every `rgx_*` symbol, and `objdump -T` of a test binary linking it requires no glibc symbol newer than the Go binary already does.

- [x] 1.3 In `rg-x11`, guard the FocusIn hand-off with `app_active()` as well (shared with reims-gui). — done: the queued-FocusIn case fails without the guard and passes with it; the 7 rg-x11 tests pass.

  Verify: an Xvfb test in `crates/rg-x11/tests/embed.rs` queues a FocusIn on the parent after another top-level became `_NET_ACTIVE_WINDOW`, and `pump()` leaves the focus there; the existing focus tests still pass.

## 2. Go wrapper

- [x] 2.1 Add `internal/embed`: — done: `go test ./internal/embed` (stub) and the tagged build pass.
  - `embed_cgo.go` (`reims_embed` tag) wraps the C API with Go types and errors;
  - `embed_stub.go` (`!reims_embed`) has the same API, with `Available() == false` and `ErrUnavailable`.

  Verify: `go vet ./...` and `go test ./...` pass without the tag, and `go test -tags reims_embed ./internal/embed` passes after `make embed-lib`.
- [x] 2.2 Add a tagged integration test: it starts Xvfb, the `sdl_window` example titled `Reims vGPU`, and a parent X11 window, then finds, embeds, sets the rect, hides, shows and releases. — done: `make test-embed` (TestEmbedReimsWindow: embedded under the app's window, released back to the root).

  Verify: `make test-embed` passes and asserts the child's parent before and after release.

## 3. Starting QEMU for embedding

- [x] 3.1 Add `core.VM.EmbedX11`: with it, `reimsEnv` adds `SDL_VIDEO_DRIVER=x11`; the CLI and MCP never set it. — done: TestReimsEnvAndExit covers both; no caller outside internal/gui sets EmbedX11.

  Verify: `TestReimsEnvAndExit` is extended (with and without the flag), and a `cmd` test checks that `vm start` leaves it unset.

## 4. Screen tab integration (Go GUI)

- [x] 4.1 Add the `screenHost` widget and the 60 Hz ticker of design D3: — done: TestPhysRect (scale 1, 1.25, 2, edge rounding) and TestStaleLookup; visibility = Screen tab selected and no overlay.
  - physical rect from `AbsolutePositionForObject` × `Canvas().Scale()`;
  - `set_rect` and `pump` on the UI goroutine;
  - hidden on other tabs, under `Overlays().Top()` or an open menu, and while minimized.

  Verify: a unit test of the rect math (scale 1, 1.25, 2) and of the visibility rule (tab, overlay, minimized).
- [x] 4.2 On a Reims start from the GUI with `embed.Available()` and an `X11WindowContext`: — done on this host (KDE X11, RTX 4070), Nilton VM: embedded in the Screen tab, typing, resize; confirmed by the user.
  - set `EmbedX11`;
  - run `find_window(pid, Reims, 30 s)` in a goroutine, then `embed` on the UI goroutine;
  - show the host instead of the notice, while the Preview keeps following the VNC socket;
  - on failure, log "embedding skipped: …" and keep today's notice;
  - keep the run generation and PID check of design D3: drop a stale lookup result, and release and clear the handle on `is_alive() == false`, a pump error, or Stopped.

  Verify with a unit test of the generation handoff (a stale result is dropped, a current one is applied), using a fake embedder behind an interface.

  Verify, by hand on this host (KDE X11, RTX 4070), with the Nilton VM:
  - the guest shows in the tab, a click and typing reach macOS, and Ctrl+Alt+Super releases;
  - a dialog hides it, resize and maximize follow, and a restart re-embeds;
  - a screenshot is saved.
- [x] 4.3 Keyboard hand-back (design D3): `rg-x11` `Embedded::return_focus_on_app_click()` with its C ABI and Go method, called every tick. — done: `a_click_on_the_app_returns_the_keyboard` in rg-x11 (XTEST) and by hand; confirmed by the user.

  Verify: an Xvfb test in `rg-x11` (guest focused, a button held over the app's window outside the guest, focus moves to the app; with the pointer over the guest, it stays), and by hand: click the guest and type, then click the VM list, and the arrow keys move in the list.
- [x] 4.6 In GUI mode with embedding built in, `$DISPLAY` set and `FYNE_PLATFORM` unset, set `FYNE_PLATFORM=x11` before the app starts (design D4). — 2026-10-17, GNOME Shell 46 headless (rg-live-img:24.04): without FYNE_PLATFORM the window is on XWayland (`xwininfo -root -tree` lists it); TestForceX11 covers the decision. With FYNE_PLATFORM=wayland the Go app crashes in glfwPollEvents on the first pointer motion, also without the embed tag (a GLFW/Fyne Wayland issue in that container, not this change).

  Verify: a unit test of the decision function (each combination of the three inputs), and on KDE Plasma Wayland (`echo $XDG_SESSION_TYPE` = wayland) `xprop` finds the app's window.
- [x] 4.4 App-owned menu bar (design D6): while embedding is available, File, Machine and Help become buttons that open `widget.PopUpMenu` overlays with the same items and Alt shortcuts; otherwise the native `SetMainMenu` stays. — done: File menu over the embedded guest fully visible (guest hidden), dialogs too; confirmed by the user.

  Verify: a test that the menu items match the old main menu, and by hand that the File menu over an embedded guest is fully visible and clickable.
- [ ] 4.5 Leave VMs started by the CLI or MCP, and VMs found running at startup, as they are.

  Verify: with `macos-kvm-gui vm start` while the GUI is open, the window stays on the desktop and the tab shows the notice (screenshot).

## 5. Release on exit

- [ ] 5.1 In GUI mode, handle SIGTERM, SIGINT and SIGHUP: stop the ticker, release the embedding on the UI goroutine, then quit, with a 2 s fallback `os.Exit(143)`. The close-while-running refusal stays, and QEMU still stops with the app (`Pdeathsig`).

  Verify: `kill -TERM` of the GUI with a guest embedded makes `xwininfo -tree` show QEMU's window under the root before QEMU exits, and the X server and the desktop stay up.

## 6. Build, docs and checks

- [x] 6.1 Makefile: `build-go`, `build-prod` and `package-go` depend on `embed-lib` and pass `-tags reims_embed`; `make test` stays tag-free; `scripts/package.sh` is unchanged. — done: `go version -m dist/macos-kvm-gui` shows `-tags=reims_embed`; `go build ./...` works without it.

  Verify: `make build-go` produces a binary whose `go version -m` shows `-tags=reims_embed`, and `go build .` alone still works.
- [x] 6.2 Docs: — done: docs/reims-vgpu.md (The screen), docs/development.md (Embedding Reims vGPU's window), docs/architecture.md, docs/screenshots/reims-embedded.png.
  - `docs/reims-vgpu.md` (Display: embedded in the Screen tab, and the fallback);
  - `docs/development.md` (the tag, `make embed-lib`, `make test-embed`);
  - `docs/architecture.md`;
  - a new screenshot in `docs/screenshots`.

  Verify: links resolve and `openspec validate embed-reims-window-go --strict` passes.
- [x] 6.3 Live check on GNOME (the `rg-live-img:24.04` container with GNOME Shell and XWayland): the Go app embeds the Reims window and Super stays in the guest once `xwayland-allow-grabs` and the app's WM_CLASS rule are set. — embedding passes under GNOME Shell 46: Reims window → container → app window, the login in the Screen tab, hidden on Details and shown on Screen, pointer entry gives the guest the keyboard, Mutter logs "XWayland window … has a grab granted" for the app's window. The Go app's WM_CLASS is now fixed to io.github.jniltinho.macos-kvm-gui (GLFW used the title). NOT met: Super still opened the overview, in reims-gui 0.1.4 as well: when the X focus moves to QEMU's child, Mutter's focus window becomes NULL and the granted grab stops blocking shortcuts. Follow-up outside this change (rg-x11 focus hand-off on Mutter).

  Verify: Mutter logs "has a grab granted" for the app's window.
