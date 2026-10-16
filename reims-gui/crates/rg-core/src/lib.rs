//! Shared types, settings.yaml, the SQLite VM registry, the Workspace and keyboard maps (port of
//! the Go app's internal/core config/vms/settings/workspace/bundle/keyboard/network).
//!
//! The types here are the contract between crates: they serialize exactly like the Go app's JSON
//! (`internal/core/config.go`), so both apps share one database and one export format. The public
//! API is summarized in `API.md`.

pub mod bundle;
pub mod config;
pub mod keyboard;
pub mod network;
pub mod registry;
pub mod settings;
pub mod workspace;

pub use config::{Config, FieldError, Profile, VERSIONS, Version};
pub use network::Nic;
pub use registry::{Registry, VmEntry};
pub use settings::{AppDir, ExtraKext, ExtraKexts};
pub use workspace::{Workspace, WorkspaceLock};

/// Errors of this crate. Messages match the Go app's.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An invalid setting, naming the field for the UI to highlight.
    #[error(transparent)]
    Field(#[from] FieldError),
    /// The Workspace is locked by another operation or app instance (Go's `ErrBusy`).
    #[error("workspace in use by another operation or another app instance")]
    Busy,
    /// The SSH forward's port is taken (Go's `ErrPortInUse`).
    #[error(
        "SSH port {0} on 127.0.0.1 is in use (another VM?): choose another port in Settings › Network: port in use"
    )]
    PortInUse(u16),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Any other failure, already worded for the user.
    #[error("{0}")]
    Msg(String),
}

/// This crate's result.
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn msg(s: impl Into<String>) -> Error {
    Error::Msg(s.into())
}
