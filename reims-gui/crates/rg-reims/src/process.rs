//! Running a command with its output as lines, a timeout and cancellation.

use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Error, Result};

/// How long an interrupted command gets before it is killed.
const INTERRUPT_GRACE: Duration = Duration::from_secs(30);

/// Spawns cmd, retrying while the executable is still open for writing in a child another thread
/// forked (a binary this process just unpacked).
pub(crate) fn spawn(cmd: &mut Command) -> io::Result<Child> {
    let mut tries = 0;
    loop {
        match cmd.spawn() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy && tries < 50 => {
                tries += 1;
                thread::sleep(Duration::from_millis(20));
            }
            r => return r,
        }
    }
}

/// Runs cmd with stdout and stderr merged into `line`, one call per line. Past `timeout` the
/// command is killed; setting `cancel` interrupts it as Ctrl+C would (docker passes that on to
/// the container), kills it 30 s later, and returns [`Error::Cancelled`].
pub(crate) fn run_lines(
    mut cmd: Command,
    stdin: Option<&[u8]>,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
    line: &mut (dyn FnMut(&str) + Send),
) -> Result<ExitStatus> {
    let (reader, writer) = io::pipe()?;
    cmd.stdout(writer.try_clone()?).stderr(writer);
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = spawn(&mut cmd)?;
    drop(cmd); // our copies of the pipe's write end: the reader ends when the command's close
    if let (Some(data), Some(mut w)) = (stdin, child.stdin.take()) {
        // ponytail: written before the output is read, fine for input within a pipe buffer
        // (the Dockerfile); a larger one needs a writer thread.
        let _ = w.write_all(data);
    }
    let start = Instant::now();
    let mut interrupted: Option<Instant> = None;
    let status = thread::scope(|s| {
        s.spawn(move || {
            let mut r = BufReader::new(reader);
            let mut buf = Vec::new();
            while r.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
                let l = String::from_utf8_lossy(&buf);
                line(l.trim_end_matches('\n').trim_end_matches('\r'));
                buf.clear();
            }
        });
        loop {
            if let Some(st) = child.try_wait()? {
                return Ok::<_, io::Error>(st);
            }
            let timed_out = timeout.is_some_and(|t| start.elapsed() > t);
            if timed_out || interrupted.is_some_and(|i| i.elapsed() > INTERRUPT_GRACE) {
                let _ = child.kill();
            } else if interrupted.is_none() && cancel.load(Ordering::Relaxed) {
                // ponytail: kill(1), as unsafe code (libc::kill) is forbidden here.
                let _ = Command::new("kill")
                    .args(["-INT", &child.id().to_string()])
                    .status();
                interrupted = Some(Instant::now());
            }
            thread::sleep(Duration::from_millis(50));
        }
    })?;
    if interrupted.is_some() {
        return Err(Error::Cancelled);
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_lines_merges_output_and_feeds_stdin() -> crate::testutil::TestResult {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "cat; echo err >&2; exit 2"]);
        let mut lines = Vec::new();
        let st = run_lines(
            cmd,
            Some(b"a\r\nb\n"),
            &AtomicBool::new(false),
            None,
            &mut |l| lines.push(l.to_string()),
        )?;
        assert_eq!(st.code(), Some(2));
        assert_eq!(lines, ["a", "b", "err"]);
        Ok(())
    }

    #[test]
    fn run_lines_interrupts_on_cancel_and_kills_on_timeout() -> crate::testutil::TestResult {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let r = run_lines(cmd, None, &AtomicBool::new(true), None, &mut |_| {});
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let st = run_lines(
            cmd,
            None,
            &AtomicBool::new(false),
            Some(Duration::from_millis(100)),
            &mut |_| {},
        )?;
        assert!(!st.success());
        Ok(())
    }
}
