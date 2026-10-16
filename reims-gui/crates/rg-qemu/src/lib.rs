//! QEMU arguments, process runtime, running-VM detection, QMP and RFB clients, key mapping (port of
//! the Go app's `vm.go`, `running.go`, `qmp.go`, `vnc.go`, `keys.go` and `network.go`'s `netArgs`,
//! with reims-ctl's `qmp.rs` and `view/keys.rs`). See `API.md`.
//!
//! The QMP and VNC clients are async (tokio, over `UnixStream`); [`blocking`] wraps them for the
//! CLI. [`Vm`] runs one QEMU and reports its state and output through a callback.

pub mod args;
pub mod blocking;
mod compat;
pub mod keys;
pub mod qcode;
pub mod qmp;
pub mod running;
pub mod vm;
pub mod vnc;

pub use args::{ArgsEnv, build_args, control_socket, vnc_socket};
pub use compat::{DISK_FILE, EFI_DIR, MEDIA_FILE, OVMF_CODE, VARS_FILE};
pub use keys::KeyCode;
pub use qmp::{Qmp, QmpEvent};
pub use running::{RunInfo, check_run_dir, find_running};
pub use vm::{StartSpec, Started, Vm, VmEvent, VmState, explain_exit, memlock_limit};
pub use vnc::{Frame, Rect, Update, Vnc, VncReader};

/// Errors of this crate. The messages are the Go app's, shown to the user as they are.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The QMP connection dropped (QEMU exited or closed it).
    #[error("QMP connection closed")]
    QmpClosed,
    /// QEMU did not answer a command in time.
    #[error("{0}: no reply from QEMU within 5s")]
    Timeout(String),
    /// QEMU answered a command with an error.
    #[error("{cmd}: {desc}")]
    Qemu { cmd: String, desc: String },
    /// Anything else, as a message for the user (protocol errors, refusals).
    #[error("{0}")]
    Msg(String),
}

impl Error {
    pub(crate) fn msg(s: impl Into<String>) -> Self {
        Self::Msg(s.into())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
