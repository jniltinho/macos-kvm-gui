//! QEMU arguments, process runtime, running-VM detection, QMP and RFB clients, key mapping (port of
//! the Go app's `vm.go`, `running.go`, `qmp.go`, `vnc.go`, `keys.go` and `network.go`'s `netArgs`,
//! with reims-ctl's `qmp.rs` and `view/keys.rs`). See `API.md`.
//!
//! The QMP and VNC clients are async (tokio, over `UnixStream`); [`blocking`] wraps them for the
//! CLI. [`Vm`] runs one QEMU and reports its state and output through a callback.

pub mod args;
pub mod blocking;
pub mod keys;
pub mod qcode;
pub mod qmp;
pub mod running;
pub mod vm;
pub mod vnc;

pub use args::{ArgsEnv, build_args, control_socket, vnc_socket};
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

/// Whether `qemu` has its GTK display (on Ubuntu, the optional `qemu-system-gui` package),
/// from `-display help`. Without it a VMware SVGA VM shows on the VNC screen instead of QEMU's
/// own window. Asked once per binary.
// ponytail: cached for the app's lifetime; installing qemu-system-gui takes effect on restart.
pub fn gtk_display(qemu: &std::path::Path) -> bool {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<(std::path::PathBuf, bool)>> = Mutex::new(Vec::new());
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, gtk)) = seen.iter().find(|(p, _)| p == qemu) {
        return *gtk;
    }
    let gtk = std::process::Command::new(qemu)
        .args(["-display", "help"])
        .output()
        .is_ok_and(|o| lists_gtk(&String::from_utf8_lossy(&o.stdout)));
    seen.push((qemu.to_path_buf(), gtk));
    gtk
}

fn lists_gtk(help: &str) -> bool {
    help.lines().any(|l| l.trim() == "gtk")
}

#[cfg(test)]
mod tests {
    #[test]
    fn gtk_is_listed_only_with_qemu_system_gui() {
        // Ubuntu 24.04's `qemu-system-x86_64 -display help` without and with qemu-system-gui.
        assert!(!super::lists_gtk(
            "Available display backend types:\nnone\ncurses\n"
        ));
        assert!(super::lists_gtk(
            "Available display backend types:\nnone\ngtk\nsdl\negl-headless\ncurses\ndbus\n"
        ));
    }
}
