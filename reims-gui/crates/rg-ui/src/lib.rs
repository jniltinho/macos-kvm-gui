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
mod icons;
mod tabs;
pub mod theme;
mod widgets;

#[cfg(test)]
mod tests;

pub use app::{App, Tab};
pub use backend::Backend;
pub use dialogs::{Modal, Page};
pub use fake::FakeBackend;

/// Opens the main window on `backend` and blocks until it closes.
pub fn run(backend: Box<dyn Backend>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("reims-gui")
            .with_app_id("io.github.jniltinho.reims-gui")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
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
