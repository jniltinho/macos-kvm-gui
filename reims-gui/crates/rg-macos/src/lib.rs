//! macOS recovery download and verification, DMG conversion, full installers and the offline
//! disc (port of the Go app's `recovery.go`, `chunklist.go`, `installer.go` and the media part of
//! `workspace.go`). See `API.md`.
//!
//! Everything here is blocking: run it on a worker thread. Long operations take a `cancel` flag
//! (checked between reads and chunks) and a progress callback.

mod chunklist;
mod compat;
mod http;
mod installer;
mod recovery;
#[cfg(test)]
mod testserver;

use std::io;

pub use chunklist::{Chunk, verify_image};
pub use http::Http;
pub use installer::{
    CachedInstaller, DISC_COMMAND, DISC_FILE, DISC_TYPED, DISC_VOLUME, DiscLease, InstallerProduct,
    cached_installer_for, cached_installers, disc_in_use, download_installer, find_installers,
    installer_name, installer_supported, installers_dir, refresh_installer_disc, remove_installer,
    use_disc,
};
pub use recovery::{
    MEDIA_FILE, MEDIA_MARKER, RECOVERY_PRODUCTS, RecoveryProduct, STAGE_CHUNKLIST, STAGE_CONVERT,
    STAGE_IMAGE, STAGE_VERIFY, STAGING_DIR, download_media, download_recovery, media_version,
    recovery_product,
};

/// Progress of a long operation, for the UI and the CLI.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// What is happening, as text for the user (e.g. [`STAGE_IMAGE`]).
    pub phase: String,
    /// Units done: bytes for downloads and package checks, chunks while verifying the recovery,
    /// thousandths while converting.
    pub done: u64,
    /// Units in all; 0 when unknown (an indeterminate bar).
    pub total: u64,
}

impl Progress {
    /// A progress report; `total` 0 means indeterminate.
    pub fn new(phase: impl Into<String>, done: u64, total: u64) -> Self {
        Self {
            phase: phase.into(),
            done,
            total,
        }
    }

    /// Percentage in 0..=100, or `None` when indeterminate.
    pub fn pct(&self) -> Option<f64> {
        (self.total > 0).then(|| self.done as f64 * 100.0 / self.total as f64)
    }
}

/// Errors of this crate. Messages match the Go app's.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The operation was stopped through its `cancel` flag.
    #[error("cancelled")]
    Cancelled,
    /// A file system error.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Anything else, with the Go app's message.
    #[error("{0}")]
    Other(String),
}

/// Result of this crate.
pub type Result<T> = std::result::Result<T, Error>;

fn other<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Other(msg.into()))
}

fn check_cancel(cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
