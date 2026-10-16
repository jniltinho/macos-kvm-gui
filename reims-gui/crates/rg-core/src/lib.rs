//! Shared types, settings.yaml, the SQLite VM registry, the Workspace and keyboard maps (port of
//! the Go app's internal/core config/vms/settings/workspace/bundle/keyboard/network).
//!
//! The types here are the contract between crates: they serialize exactly like the Go app's JSON
//! (`internal/core/config.go`), so both apps share one database and one export format.

pub mod config;

pub use config::{Config, Nic};
