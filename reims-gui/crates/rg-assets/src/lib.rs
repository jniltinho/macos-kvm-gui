//! Embedded OpenCore/OVMF assets, EFI extraction, config.plist rendering, extra kexts,
//! root-patch kexts and ReimsVgpu.kext (port of the Go app's `boot.go`, `extrakexts.go`,
//! `rootpatch.go`, `reimskext.go` and `internal/assets`). See `API.md`.
//!
//! Everything here is blocking (file copies, downloads with `ureq`); callers on the async
//! runtime use `spawn_blocking`. Long downloads stop when their cancel flag is set.

mod assets;
mod boot;
pub mod compat;
mod extrakexts;
mod http;
mod reimskext;
mod rootpatch;
mod util;

pub use assets::{BootAssets, LOGO, asset_versions, icon, version_icon};
pub use boot::{
    BOOT_DIR, BootOptions, BootState, EFI_DIR, Identity, KextCache, OVMF_CODE, VARS_FILE,
    opencore_config, prepare_boot, smbios_model,
};
pub use compat::{ExtraKext, ExtraKexts};
pub use extrakexts::{download_extra_kexts, extra_kexts_ready, kext_switches};
pub use reimskext::{REIMS_UTIL_COMMIT, download_reims_kext, reims_kext_ready};
pub use rootpatch::{
    ROOT_PATCH_MARKER, clear_root_patch_marker, download_root_patch_kexts, is_kepler,
    root_patch_kexts_ready, root_patch_version,
};

/// What can go wrong; the messages follow the Go app's.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A download failed (refused URL, network, status, size limit).
    #[error("{0}")]
    Http(String),
    /// Invalid input or content: settings.yaml, an archive, a hash, missing files.
    #[error("{0}")]
    Invalid(String),
    /// The cancel flag was set.
    #[error("cancelled")]
    Cancelled,
    /// Another error with what was being done.
    #[error("{context}: {source}")]
    Context { context: String, source: Box<Error> },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Adds what was being done to an error, like Go's `fmt.Errorf("...: %w", err)`.
pub(crate) trait Context<T> {
    fn context(self, f: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E: Into<Error>> Context<T> for std::result::Result<T, E> {
    fn context(self, f: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|e| Error::Context {
            context: f(),
            source: Box::new(e.into()),
        })
    }
}

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}
