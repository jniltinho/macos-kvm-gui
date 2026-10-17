# Design

## Context

The app starts QEMU itself and already keeps a private 0700 runtime directory per run for the QMP socket. On the reference host, QEMU 8.2.2 offers `-vnc` (including Unix sockets) and the `none`, `gtk`, `sdl`, `egl-headless`, `curses`, `spice-app` and `dbus` display backends. The guest has no GPU driver, so the screen is the plain boot framebuffer that macOS draws on the CPU, at 1920x1080 by default.

## Goals / Non-Goals

**Goals:**
- No new dependencies, and no network exposure.
- Usable for installing and using macOS: smooth enough at 1080p, correct keyboard and pointer.

**Non-Goals:**
- Clipboard, audio over VNC, VNC authentication (the socket is private), Tight/ZRLE encodings.

## Decisions

1. **VNC on a Unix socket over QEMU's D-Bus display or SPICE.** VNC is a small, documented protocol (RFB 3.8, RFC 6143), and a minimal client fits in one file with only the standard library. D-Bus display would need a D-Bus binding and only works on a session bus, and SPICE needs a large client library. The socket lives in the existing private runtime directory (mode 0700), so only the user can connect, without a password and without a TCP port.
2. **Minimal RFB client.** Security type None (1) only; it refuses servers that require authentication. Pixel format 32 bpp true color, little endian, red/green/blue shifts 0/8/16, so the wire bytes are already R,G,B,x and copy straight into an `image.RGBA` with alpha set to 255. Encodings: Raw (0), CopyRect (1) and DesktopSize (-223). Incremental update requests are sent after each update. Colour-map, bell and cut-text messages are read and ignored. The client is unit-tested against a fake server over `net.Pipe`.
3. **Rendering.** The client writes into an `image.RGBA` guarded by a mutex and marks it dirty. The UI refreshes a `canvas.Image` (`ImageFillContain`, fastest scaling) at most 60 times per second from a ticker, through `fyne.Do`, copying into one `image.RGBA` that is updated in place. Swapping two buffers made Fyne recreate textures and corrupted unrelated text textures on the reference host. `fyne.Do` runs on the goroutine that paints, so the copy never races the renderer. A guest resolution change (DesktopSize) replaces the image.
4. **Input widget.** A custom widget implements `fyne.Focusable`, `fyne.Tabbable` (Tab goes to the guest), `desktop.Cursorable` (hides the host pointer, since macOS draws its own), `desktop.Keyable` (KeyDown/KeyUp, which also report modifiers), `desktop.Mouseable`, `desktop.Hoverable` and `fyne.Scrollable`. Fyne key names (US physical positions) map to an X11 keysym and an XT scancode. The client advertises QEMU's extended key event pseudo-encoding (-258); once QEMU acknowledges it, keys travel as scancodes and the guest's layout applies (the host's layout, set through `prev-lang:kbd`). QEMU runs without `-k`, which would drop the scancode of extended key events. Otherwise plain keysyms are sent. `lock-key-sync=off` stops QEMU from injecting Caps Lock presses inferred from keysym case. Super maps to `Super_L`, which macOS sees as Command, and Alt maps to `Alt_L` (Option). The wheel maps to buttons 4 and 5. Pointer positions are converted from widget coordinates with the same contain-fit math the image uses; clicks in the letterbox bars are ignored. Pressed keys are tracked, and all are released on `FocusLost`.
5. **Connecting.** The VM exposes the VNC socket path while it runs. The UI dials with the same short retry as QMP (50 × 100 ms), and the viewer closes when the VM stops.
6. **Display setting.** A new `display` config field: `app` (default) or `window`. With `window` the arguments stay exactly as before. With `app`, `-display none` replaces the GTK window. `vmware-svga` stays in both modes, because it is the console VNC reads.
7. **Separate window.** "Open in window" moves the same screen widget into a new Fyne window and puts a placeholder in the tab; closing the window moves it back. A widget can only have one parent, so it moves instead of being duplicated.

8. **Full screen and host key.** Full screen opens the separate window in full screen. Ctrl+Alt+F is intercepted before it reaches the guest; held Ctrl/Alt are released in the guest, then it leaves full screen or, in a normal separate window, closes it. Real-hardware check: entering and leaving full screen and returning to the tab worked on the reference host with an installed Sonoma.

## Risks / Trade-offs

- [Non-US keyboards: a key such as ç or a dead key may not map through QEMU's `en-us` keymap] → Documented. Most keys work because keysyms are sent with US semantics; layout support can come later.
- [CPU cost of uploading a 1080p texture up to 30 times per second] → Refreshes only when dirty, and macOS usually redraws small regions.
- [GPU passthrough: the AMD card's outputs are not shown in the app] → Documented. The viewer shows the emulated console, which is what macOS uses until the AMD driver takes over.

## Migration Plan

Existing configurations have no `display` field, so they get the default `app`. Users who prefer the old QEMU window choose **QEMU window** in Settings.
