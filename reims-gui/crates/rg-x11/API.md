# rg-x11 API

Embeds QEMU's display window into the Screen tab by X11 reparenting (design D3): Reims vGPU's
SDL3 window (`SDL_VIDEO_DRIVER=x11`) or QEMU's own SDL window (`-display sdl`,
`SDL_VIDEO_DRIVER=x11`) for VMware SVGA. Pure Rust (`x11rb` `RustConnection`), no unsafe.

```rust
pub fn probe(display: Option<&str>) -> bool;               // None = $DISPLAY
pub fn to_physical(x: f32, y: f32, w: f32, h: f32, pixels_per_point: f32) -> (i32, i32, u32, u32);

pub enum WindowKind { Reims, QemuSdl }                       // .matches(title)

impl Embedder {
    pub fn new(display: Option<&str>, parent: u32) -> Result<Self>;
    pub fn find_window(&self, pid: u32, kind: WindowKind, timeout: Duration) -> Result<u32>;
    pub fn embed(&self, child: u32) -> Result<Embedded>;
}

impl Embedded {                                              // Drop = release()
    pub fn set_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()>;
    pub fn pump(&mut self) -> Result<()>;
    pub fn hide(&self) -> Result<()>;
    pub fn show(&self) -> Result<()>;
    pub fn focus(&self) -> Result<()>;
    pub fn unfocus(&self) -> Result<()>;
    pub fn is_alive(&self) -> bool;
    pub fn last_latency(&self) -> Option<Duration>;
    pub fn child(&self) -> u32;
    pub fn release(self) -> Result<()>;
}
```

## Use from the UI

1. Startup: `probe(None)`; if true run eframe on X11 and take the top-level window id from
   raw-window-handle (`Xlib`/`Xcb` handle), then `Embedder::new(None, id)`.
2. After the VM starts: `find_window(qemu_pid, kind, 30 s)` **blocks** (polls every 50 ms) — run
   it off the UI thread (`spawn_blocking`). It matches a *viewable* window with
   `_NET_WM_PID == pid` anywhere in the tree (window-manager frames are fine) and a title from
   `_NET_WM_NAME`/`WM_NAME`: `Reims` = starts with `Reims vGPU`; `QemuSdl` = starts with `QEMU`
   (QEMU's SDL UI sets `QEMU`, `QEMU (<-name>)`, plus ` [Stopped]` or a grab hint).
   Errors: `Error::NotFound` on timeout.
3. `embed(win)`: creates a container (child of the app window, background None so the server
   never clears it), puts QEMU's window in the save-set (if the app dies it goes back to the
   root instead of being destroyed), removes its `_NET_WM_STATE`, unmaps, reparents at (0,0),
   maps. `Embedded` owns its own X connection, so its events reach only it.
4. Every frame: `set_rect(to_physical(rect, ctx.pixels_per_point()))` then `pump()`.
   `set_rect` sends one ConfigureWindow for the container and one for the child, and nothing
   when the rect is unchanged. `pump` never blocks: it drains the queue, coalesces the child's
   ConfigureNotify storm to the last one and, if the child moved/resized itself (the library
   calls `SDL_SetWindowSize` on a guest mode change), configures it back once. DestroyNotify
   (QEMU exited) makes `is_alive()` false; call `release()` (or drop) and re-embed on restart.
5. Overlay over the tab, tab switch, minimize: `hide()` (focus reverts to the app); `show()`
   after. The pointer entering QEMU's window gives it the keyboard (EnterNotify, handled by
   `pump`: SDL's ButtonPress selection is exclusive, so clicks on it cannot be watched);
   `focus()` does the same explicitly (`SetInputFocus`; errors if the child is not viewable) and
   `unfocus()` gives the keyboard back to the app window (a click elsewhere in the app).
6. `last_latency()`: time from the last child configure to its ConfigureNotify (diagnostics).
7. `release()`: a live child goes back to the root, mapped (standalone fallback); the
   container is destroyed.

## Tests

`cargo test -p rg-x11` in the Docker image: Xvfb, a real SDL3 window (`examples/sdl_window.rs`,
SDL 3.4 built from source by the `sdl3` dev-dependency, as Ubuntu 24.04 has no libsdl3), titled
like Reims vGPU's window and like QEMU's `-display sdl` window.

## Limits

- Xvfb has no window manager: WM frames, `_NET_WM_STATE` fights, focus stealing, compositor
  behaviour and XWayland are only covered by the manual release gates (D3 risks).
- No XEmbed protocol: SDL does not speak it for foreign parents; focus is given explicitly.
