//! The egui desktop app (port of the Go app's `internal/gui`): menu bar, toolbar, VM list,
//! Details/Screen/Install/Host/GPU/Log tabs, the Settings dialog, the New VM wizard,
//! Preferences, light and dark themes, and the Screen area the embedding layer fills (design D3).
//!
//! The UI talks to the core crates only through [`Backend`]; [`FakeBackend`] has sample data for
//! tests and `--demo`; [`RealBackend`] drives the real crates.

mod app;
pub mod backend;
mod dialogs;
mod fake;
mod files;
mod icons;
mod real;
mod run;
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
pub use real::RealBackend;
pub use run::{RunOptions, run};
pub use screen::backend_choice::{native_options, x11_available};

/// `reims-gui --demo`: the window on [`FakeBackend`].
pub fn run_demo() -> anyhow::Result<()> {
    run(Box::new(FakeBackend::default()), RunOptions::default())
}
