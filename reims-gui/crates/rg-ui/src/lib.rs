//! The egui desktop app (port of the Go app's `internal/gui`): menu bar, toolbar, VM list,
//! Details/Screen/Install/Host/GPU/Log tabs, the Settings dialog, the New VM wizard,
//! Preferences, light and dark themes, and the Screen area the embedding layer fills (design D3).
//!
//! The UI talks to the core crates only through [`Backend`]; [`FakeBackend`] has sample data for
//! tests and `--demo`.

mod app;
pub mod backend;
mod dialogs;
mod fake;
mod files;
mod icons;
pub mod screen;
mod tabs;
pub mod theme;
mod widgets;

#[cfg(test)]
mod tests;

pub use app::{App, Tab};
pub use backend::Backend;
pub use dialogs::{Modal, Page};
pub use fake::FakeBackend;
pub use files::FileDialog;
pub use screen::backend_choice::{native_options, x11_available};

/// Opens the main window on `backend` and blocks until it closes: on X11 when an X server
/// answers, so QEMU's windows can be embedded (design D3).
pub fn run(backend: Box<dyn Backend>) -> eframe::Result {
    let options = native_options(x11_available());
    eframe::run_native(
        "reims-gui",
        options,
        Box::new(|_| Ok(Box::new(App::new(backend)))),
    )
}

/// `reims-gui --demo`: the window on [`FakeBackend`].
pub fn run_demo() -> eframe::Result {
    run(Box::new(FakeBackend::default()))
}
