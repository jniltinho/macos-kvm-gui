//! A QMP client over QEMU's Unix socket: one command at a time, events on a channel (Go
//! `qmp.go`; the command format follows reims-ctl's `qmp.rs`).

use crate::qcode::Ev;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{Mutex, mpsc};

/// How long a command waits for its reply.
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// Connecting retries this often while QEMU creates the socket (50 × 100 ms).
pub(crate) const DIAL_TRIES: u32 = 50;
pub(crate) const DIAL_BACKOFF: Duration = Duration::from_millis(100);

/// An asynchronous QEMU event (STOP, RESUME, BLOCK_IO_ERROR, POWERDOWN...).
#[derive(Clone, Debug, PartialEq)]
pub struct QmpEvent {
    pub name: String,
    /// The event's `data` object, or `Value::Null`.
    pub data: Value,
}

struct Conn {
    writer: OwnedWriteHalf,
    replies: mpsc::Receiver<Value>,
}

/// A QMP connection. Commands are serialized; events go to the receiver [`Qmp::dial`] returns,
/// which closes when the connection drops.
pub struct Qmp {
    conn: Mutex<Conn>,
}

/// Connects to a Unix socket, retrying while QEMU creates it.
pub(crate) async fn dial_unix(path: &Path) -> std::io::Result<UnixStream> {
    let mut last = std::io::Error::other("no attempt");
    for _ in 0..DIAL_TRIES {
        match UnixStream::connect(path).await {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
        tokio::time::sleep(DIAL_BACKOFF).await;
    }
    Err(last)
}

impl Qmp {
    /// Waits for the socket to appear, negotiates capabilities and starts delivering events.
    pub async fn dial(path: &Path) -> Result<(Qmp, mpsc::UnboundedReceiver<QmpEvent>)> {
        let s = dial_unix(path)
            .await
            .map_err(|e| Error::msg(format!("QMP unavailable: {e}")))?;
        Self::handshake(s).await
    }

    /// Negotiates capabilities on a connected stream.
    pub async fn handshake(s: UnixStream) -> Result<(Qmp, mpsc::UnboundedReceiver<QmpEvent>)> {
        let (r, mut writer) = s.into_split();
        let mut lines = BufReader::new(r).lines();
        let negotiate = async {
            let greeting = read_msg(&mut lines).await?;
            if greeting.get("QMP").is_none() {
                return Err(Error::msg("invalid QMP greeting"));
            }
            writer
                .write_all(b"{\"execute\":\"qmp_capabilities\"}\n")
                .await?;
            loop {
                let m = read_msg(&mut lines)
                    .await
                    .map_err(|e| Error::msg(format!("QMP negotiation failed: {e}")))?;
                if let Some(e) = m.get("error") {
                    return Err(Error::msg(format!("QMP negotiation refused: {}", desc(e))));
                }
                if m.get("return").is_some() {
                    return Ok(());
                }
            }
        };
        tokio::time::timeout(TIMEOUT, negotiate)
            .await
            .map_err(|_| Error::msg("QMP negotiation timed out"))??;

        let (reply_tx, replies) = mpsc::channel(1);
        let (event_tx, events) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok(m) = read_msg(&mut lines).await {
                if let Some(name) = m.get("event").and_then(Value::as_str) {
                    let data = m.get("data").cloned().unwrap_or(Value::Null);
                    let _ = event_tx.send(QmpEvent {
                        name: name.into(),
                        data,
                    });
                    continue;
                }
                // A reply nobody waits for (its command timed out) is dropped.
                let _ = reply_tx.try_send(m);
            }
            // Dropping both senders reports the disconnect to commands and to the events reader.
        });
        Ok((
            Qmp {
                conn: Mutex::new(Conn { writer, replies }),
            },
            events,
        ))
    }

    /// Sends a command and waits for the reply's `return` value, up to [`TIMEOUT`].
    pub async fn execute(&self, cmd: &str, arguments: Option<Value>) -> Result<Value> {
        let mut c = self.conn.lock().await;
        while c.replies.try_recv().is_ok() {} // a late reply from a previous command
        let mut msg = json!({ "execute": cmd });
        if let Some(a) = arguments {
            msg["arguments"] = a;
        }
        let mut bytes = serde_json::to_vec(&msg).map_err(|e| Error::msg(e.to_string()))?;
        bytes.push(b'\n');
        match tokio::time::timeout(TIMEOUT, c.writer.write_all(&bytes)).await {
            Ok(Ok(())) => {}
            _ => return Err(Error::QmpClosed),
        }
        match tokio::time::timeout(TIMEOUT, c.replies.recv()).await {
            Err(_) => Err(Error::Timeout(cmd.into())),
            Ok(None) => Err(Error::QmpClosed),
            Ok(Some(m)) => match m.get("error") {
                Some(e) => Err(Error::Qemu {
                    cmd: cmd.into(),
                    desc: desc(e),
                }),
                None => Ok(m.get("return").cloned().unwrap_or(Value::Null)),
            },
        }
    }

    /// Asks the guest to shut down (ACPI power button).
    pub async fn system_powerdown(&self) -> Result<()> {
        self.execute("system_powerdown", None).await.map(drop)
    }

    /// Ends QEMU. QEMU usually exits before it replies, so a closed connection is success.
    pub async fn quit(&self) -> Result<()> {
        match self.execute("quit", None).await {
            Ok(_) | Err(Error::QmpClosed) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// `query-status`'s `status` ("running", "paused"...).
    pub async fn status(&self) -> Result<String> {
        let v = self.execute("query-status", None).await?;
        Ok(v.get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    /// Writes the display (of `device`, or the first) to `file` as PNG.
    pub async fn screendump(&self, file: &Path, device: Option<&str>) -> Result<()> {
        let mut a = json!({ "filename": file.to_string_lossy(), "format": "png" });
        if let Some(d) = device {
            a["device"] = json!(d);
        }
        self.execute("screendump", Some(a)).await.map(drop)
    }

    /// Sends key, button and tablet events (see [`crate::qcode`]).
    pub async fn input_send_event(&self, events: &[Ev]) -> Result<()> {
        let mut out = Vec::new();
        for e in events {
            e.json(&mut out);
        }
        self.execute("input-send-event", Some(json!({ "events": out })))
            .await
            .map(drop)
    }
}

async fn read_msg(
    lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
) -> Result<Value> {
    loop {
        let Some(line) = lines.next_line().await? else {
            return Err(Error::QmpClosed);
        };
        if line.trim().is_empty() {
            continue;
        }
        return serde_json::from_str(&line)
            .map_err(|e| Error::msg(format!("bad QMP message: {e}")));
    }
}

fn desc(e: &Value) -> String {
    e.get("desc")
        .and_then(Value::as_str)
        .unwrap_or("error")
        .to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use tokio::net::UnixListener;

    /// A minimal QMP server: greeting, capabilities and scripted replies (lines to write) per
    /// command; "hangup" closes the connection.
    pub(crate) fn fake_qmp(dir: &Path, handle: fn(&str) -> Vec<Value>) -> std::io::Result<PathBuf> {
        let sock = dir.join("q.sock");
        let l = UnixListener::bind(&sock)?;
        tokio::spawn(async move {
            let Ok((s, _)) = l.accept().await else { return };
            let (r, mut w) = s.into_split();
            let mut lines = BufReader::new(r).lines();
            let send = |v: Value| format!("{v}\n");
            let _ = w
                .write_all(send(json!({"QMP": {"version": {}}})).as_bytes())
                .await;
            while let Ok(Some(line)) = lines.next_line().await {
                let m: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                let cmd = m["execute"].as_str().unwrap_or_default().to_string();
                if cmd == "qmp_capabilities" {
                    let _ = w.write_all(send(json!({"return": {}})).as_bytes()).await;
                    continue;
                }
                for v in handle(&cmd) {
                    let _ = w.write_all(send(v).as_bytes()).await;
                }
                if cmd == "hangup" {
                    return;
                }
            }
        });
        Ok(sock)
    }

    fn script(cmd: &str) -> Vec<Value> {
        match cmd {
            // The event arrives before the reply: the client must tell them apart.
            "system_powerdown" => vec![json!({"event": "POWERDOWN"}), json!({"return": {}})],
            "broken" => vec![json!({"error": {"class": "GenericError", "desc": "nope"}})],
            "query-status" => vec![json!({"return": {"status": "running", "running": true}})],
            _ => vec![], // "mute": no reply
        }
    }

    #[tokio::test]
    async fn qmp_commands_events_errors_timeout_and_disconnect() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sock = fake_qmp(dir.path(), script)?;
        let (q, mut events) = Qmp::dial(&sock).await?;
        q.system_powerdown().await?;
        assert_eq!(
            events.recv().await.map(|e| e.name),
            Some("POWERDOWN".into())
        );
        assert_eq!(q.status().await?, "running");
        match q.execute("broken", None).await {
            Err(e) => assert!(e.to_string().contains("nope"), "{e}"),
            Ok(v) => panic!("QEMU error must be passed through: {v}"),
        }
        let start = std::time::Instant::now();
        assert!(matches!(
            q.execute("mute", None).await,
            Err(Error::Timeout(_))
        ));
        assert!(start.elapsed() >= TIMEOUT);
        let _ = q.execute("hangup", None).await;
        let closed = tokio::time::timeout(Duration::from_secs(2), events.recv()).await;
        assert!(
            matches!(closed, Ok(None)),
            "disconnect must close the events"
        );
        assert!(matches!(q.system_powerdown().await, Err(Error::QmpClosed)));
        Ok(())
    }
}
