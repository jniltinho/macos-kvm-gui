//! MCP `vm_ssh` (design D6): one command in a running VM's macOS through the system `ssh`,
//! with the user's keys and agent, or a password handed over by the `ssh-askpass` helper.
//!
//! The password never goes on a command line, in the environment or in a file: ssh runs
//! `SSH_ASKPASS` (a script in a fresh 0700 folder that calls `reims-gui ssh-askpass`), which
//! reads it from a Unix socket in that folder. The socket answers once, and only to a process
//! whose parent is the `ssh` we started: its pid comes from SO_PEERCRED (tokio's safe
//! `UnixStream::peer_cred`) and its parent from `/proc/<pid>/stat`. OpenSSH forks the askpass
//! program itself, so that is the only process that passes.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// The environment variable that gives the askpass helper its socket.
pub const ASKPASS_SOCKET_ENV: &str = "REIMS_GUI_ASKPASS_SOCKET";
/// Output kept per stream; the rest is read and dropped.
pub const OUTPUT_CAP: usize = 1 << 20;

/// One `vm_ssh` run.
pub struct Request<'a> {
    /// The ssh client (`ssh` from PATH).
    pub ssh: &'a Path,
    pub port: u16,
    pub user: &'a str,
    pub command: &'a str,
    pub password: Option<&'a str>,
    /// The app's own known_hosts (`<config dir>/known_hosts`).
    pub known_hosts: &'a Path,
    /// The program `reims-gui ssh-askpass` runs from (the current executable).
    pub askpass: &'a Path,
    pub timeout: Duration,
}

/// What the command printed and how it ended.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    /// The exit status; -1 when ssh was killed (timeout) or died of a signal.
    pub exit_code: i32,
    /// A stream passed [`OUTPUT_CAP`] and was cut.
    pub truncated: bool,
    pub timed_out: bool,
}

/// ssh's arguments (design D6). The password is never one of them.
pub fn ssh_args(r: &Request<'_>) -> Vec<OsString> {
    let mut known = OsString::from("UserKnownHostsFile=");
    known.push(r.known_hosts);
    let batch = if r.password.is_some() { "no" } else { "yes" };
    [
        "-p".into(),
        r.port.to_string().into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        format!("BatchMode={batch}").into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        known,
        format!("{}@127.0.0.1", r.user).into(),
        "--".into(),
        r.command.into(),
    ]
    .into()
}

/// Forgets the host key learned for `127.0.0.1:port` (`reset_host_key`), as `ssh-keygen -R`.
pub fn reset_host_key(known_hosts: &Path, port: u16) -> Result<()> {
    if !known_hosts.exists() {
        return Ok(());
    }
    let out = std::process::Command::new("ssh-keygen")
        .arg("-R")
        .arg(format!("[127.0.0.1]:{port}"))
        .arg("-f")
        .arg(known_hosts)
        .output()
        .context("ssh-keygen (OpenSSH) is needed to forget the host key")?;
    if !out.status.success() {
        bail!(
            "ssh-keygen -R: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let _ = std::fs::remove_file(known_hosts.with_extension("old"));
    Ok(())
}

/// A private folder removed with everything in it when dropped.
struct PrivateDir(PathBuf);

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A new 0700 folder under `$XDG_RUNTIME_DIR` for one call's socket and askpass script.
fn private_dir() -> Result<PrivateDir> {
    static N: AtomicU64 = AtomicU64::new(0);
    let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) else {
        bail!(
            "$XDG_RUNTIME_DIR is not set, so the password cannot be handed to ssh privately: use SSH keys"
        );
    };
    let dir = PathBuf::from(run).join(format!(
        "reims-gui-ssh-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(PrivateDir(dir))
}

/// `sh` quoting for one word.
fn quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', r"'\''"))
}

/// The parent pid of `pid`, from `/proc/<pid>/stat` (after the command name, which may hold
/// spaces and parentheses).
pub fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Answers the first connection whose process is a child of `parent` with `password`, then
/// removes the socket. Other connections are closed unanswered.
pub async fn serve_password(listener: UnixListener, sock: PathBuf, password: String, parent: u32) {
    while let Ok((mut s, _)) = listener.accept().await {
        let peer = s.peer_cred().ok().and_then(|c| c.pid());
        let ok = peer.and_then(|p| u32::try_from(p).ok()).and_then(parent_of) == Some(parent);
        if ok {
            let _ = s.write_all(password.as_bytes()).await;
            let _ = s.shutdown().await;
            break;
        }
    }
    drop(listener);
    let _ = std::fs::remove_file(sock);
}

/// Reads a stream to its end, keeping the first [`OUTPUT_CAP`] bytes.
async fn read_capped(r: Option<impl AsyncRead + Unpin>) -> (String, bool) {
    let Some(mut r) = r else {
        return (String::new(), false);
    };
    let (mut kept, mut buf, mut cut) = (Vec::new(), vec![0u8; 64 << 10], false);
    while let Ok(n) = r.read(&mut buf).await {
        if n == 0 {
            break;
        }
        let room = OUTPUT_CAP - kept.len();
        kept.extend_from_slice(&buf[..n.min(room)]);
        cut |= n > room;
    }
    let mut s = String::from_utf8_lossy(&kept).into_owned();
    if cut {
        s += "\n[output truncated at 1 MiB]";
    }
    (s, cut)
}

/// Runs the command over ssh. On timeout ssh's whole process group is killed.
pub async fn run(r: &Request<'_>) -> Result<Output> {
    let mut cmd = tokio::process::Command::new(r.ssh);
    cmd.args(ssh_args(r))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut secret = None;
    if let Some(pw) = r.password {
        let dir = private_dir()?;
        let sock = dir.0.join("askpass.sock");
        let script = dir.0.join("askpass");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nexec {} ssh-askpass \"$@\"\n", quote(r.askpass)),
        )?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
        let listener = UnixListener::bind(&sock)?;
        cmd.env("SSH_ASKPASS", &script)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(ASKPASS_SOCKET_ENV, &sock);
        secret = Some((dir, listener, sock, pw.to_owned()));
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("cannot run {}", r.ssh.display()))?;
    let pid = child.id().unwrap_or(0);
    let server = secret.map(|(dir, listener, sock, pw)| {
        (dir, tokio::spawn(serve_password(listener, sock, pw, pid)))
    });
    let out = tokio::spawn(read_capped(child.stdout.take()));
    let err = tokio::spawn(read_capped(child.stderr.take()));
    let status = match tokio::time::timeout(r.timeout, child.wait()).await {
        Ok(s) => Some(s?),
        Err(_) => {
            if let Some(p) = i32::try_from(pid)
                .ok()
                .and_then(rustix::process::Pid::from_raw)
            {
                let _ = rustix::process::kill_process_group(p, rustix::process::Signal::KILL);
            }
            let _ = child.wait().await;
            None
        }
    };
    if let Some((_dir, task)) = server {
        task.abort();
    }
    let (stdout, t1) = out.await?;
    let (stderr, t2) = err.await?;
    Ok(Output {
        stdout,
        stderr,
        exit_code: status.and_then(|s| s.code()).unwrap_or(-1),
        truncated: t1 || t2,
        timed_out: status.is_none(),
    })
}

/// `reims-gui ssh-askpass`: prints the password the socket in [`ASKPASS_SOCKET_ENV`] gives.
pub fn askpass(out: &mut (dyn Write + Send)) -> Result<()> {
    let Some(sock) = std::env::var_os(ASKPASS_SOCKET_ENV) else {
        bail!("ssh-askpass is run by ssh for the MCP server's vm_ssh, not by hand");
    };
    let mut s = std::os::unix::net::UnixStream::connect(&sock)
        .context("the password was already given or the call ended")?;
    let mut pw = String::new();
    s.read_to_string(&mut pw)?;
    if pw.is_empty() {
        bail!("no password for this process");
    }
    writeln!(out, "{pw}")?;
    Ok(())
}

#[cfg(test)]
mod tests;
