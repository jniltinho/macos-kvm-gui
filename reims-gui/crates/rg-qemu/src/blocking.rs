//! Blocking wrappers for the CLI: each runs the async call on a private current-thread runtime.
//! Do not call them from inside a tokio runtime (tokio panics on a nested `block_on`); async
//! code uses [`crate::Qmp`] and [`crate::Vnc`] directly.

use crate::keys::KeyCode;
use crate::running::check_run_dir;
use crate::vnc::Frame;
use crate::{Error, Qmp, Result, Vnc};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

fn block_on<T>(f: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(f)
}

/// Runs one QMP command on a socket of a private run directory (a VM's control socket).
pub fn qmp(socket: &Path, cmd: &str, arguments: Option<Value>) -> Result<Value> {
    check_run_dir(socket)?;
    block_on(async {
        let (q, _events) = Qmp::dial(socket).await?;
        match cmd {
            "quit" => q.quit().await.map(|()| Value::Null),
            _ => q.execute(cmd, arguments).await,
        }
    })
}

/// A complete frame of the VM's screen (see [`Vnc::snapshot`]).
pub fn capture(vnc_socket: &Path, timeout: Duration) -> Result<Frame> {
    check_run_dir(vnc_socket)?;
    block_on(Vnc::capture(vnc_socket, timeout))
}

/// Sends key chords over VNC, `pause` apart.
pub fn send_chords(vnc_socket: &Path, chords: &[Vec<KeyCode>], pause: Duration) -> Result<()> {
    check_run_dir(vnc_socket)?;
    block_on(async {
        let (v, reader) = Vnc::dial(vnc_socket).await?;
        let run = tokio::spawn(reader.run(|_| {}));
        // QEMU says it takes extended key events in its first update.
        tokio::time::sleep(Duration::from_millis(300)).await;
        for c in chords {
            v.chord(c).await?;
            tokio::time::sleep(pause).await;
        }
        run.abort();
        Ok::<_, Error>(())
    })
}
