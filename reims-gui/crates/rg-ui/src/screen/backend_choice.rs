//! X11 or Wayland for the app's own window (design D3), chosen once at startup without
//! touching the environment: X11 (XWayland on Wayland desktops) when an X server answers, so
//! the app has an X11 window to embed QEMU's into; otherwise winit's choice (native Wayland),
//! and QEMU's windows stay standalone (SVGA VMs use the VNC screen).
//!
//! `run()` calls `native_options(x11)` with `x11 = x11_available()` and passes the same choice
//! to `Backend::attach`, so VMs start for embedding exactly when the window is on X11.

/// Whether an X server is reachable on `$DISPLAY`.
pub fn x11_available() -> bool {
    std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty()) && rg_x11::probe(None)
}

/// The main window's options; `x11` opens it through X11 even where winit would pick Wayland
/// (PhotoCraft's `event_loop_builder` pattern).
pub fn native_options(x11: bool) -> eframe::NativeOptions {
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("reims-gui")
            .with_app_id("io.github.jniltinho.reims-gui")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    if x11 {
        use winit::platform::x11::EventLoopBuilderExtX11 as _;
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_x11();
        }));
    }
    options
}

/// The app window's X11 id, when it is on X11.
pub fn x11_window(frame: &eframe::Frame) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}
