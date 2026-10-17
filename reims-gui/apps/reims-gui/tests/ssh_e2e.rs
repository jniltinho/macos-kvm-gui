//! MCP `vm_ssh` end to end (design D6): `reims-gui mcp` runs the system ssh against a real
//! sshd, and the password goes through `reims-gui ssh-askpass` and the per-call socket. A
//! copy of /bin/sh named qemu-system-x86_64 stands for the VM's QEMU (its command line has the
//! VM's disk and the SSH forward).
//!
//! Needs `ssh` and an sshd on 127.0.0.1, so it is ignored by default. Run it with a throwaway
//! sshd container, for example:
//!
//! ```text
//! RG_SSHD_PORT=2299 RG_SSHD_USER=tester RG_SSHD_PASSWORD=... \
//!     cargo test -p reims-gui --test ssh_e2e -- --ignored
//! ```

use std::io::{BufRead, BufReader, Lines, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const BIN: &str = env!("CARGO_BIN_EXE_reims-gui");

fn app(home: &Path) -> Command {
    let mut c = Command::new(BIN);
    c.env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_RUNTIME_DIR", home.join("run"));
    c
}

/// Runs a CLI command; returns its stdout.
fn cli(home: &Path, args: &[&str]) -> Res<String> {
    let out = app(home).args(args).output()?;
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        return Err(format!("{args:?}: {}{s}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(s)
}

/// One JSON-RPC session with `reims-gui mcp`.
struct Mcp {
    w: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Mcp {
    fn send(&mut self, msg: &Value) -> Res<()> {
        Ok(writeln!(self.w, "{msg}")?)
    }

    fn call(&mut self, id: u64, method: &str, params: Value) -> Res<Value> {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        for line in self.lines.by_ref() {
            let v: Value = serde_json::from_str(&line?)?;
            if v["id"] == id {
                return Ok(v);
            }
        }
        Err("the MCP server closed".into())
    }
}

/// Kills the stand-in QEMU when the test ends, passed or not.
struct Kill(std::process::Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "needs ssh and a test sshd: set RG_SSHD_PORT, RG_SSHD_USER and RG_SSHD_PASSWORD"]
fn vm_ssh_runs_a_command_with_a_password() -> Res<()> {
    let port = std::env::var("RG_SSHD_PORT")?;
    let user = std::env::var("RG_SSHD_USER")?;
    let password = std::env::var("RG_SSHD_PASSWORD")?;
    let t = tempfile::tempdir()?;
    let home = t.path();
    std::fs::create_dir_all(home.join("run"))?;
    std::fs::set_permissions(home.join("run"), std::fs::Permissions::from_mode(0o700))?;
    cli(
        home,
        &["workspace", "set", &home.join("VMs").to_string_lossy()],
    )?;
    // The sshd holds the port: an explicit port no other VM has is kept anyway.
    let out = cli(home, &["vm", "create", "E2E", "--ssh-port", &port])?;
    assert!(out.contains(&format!("SSH port {port}")), "{out}");
    let disk = home.join("VMs/e2e/mac_hdd_ng.img");
    std::fs::write(&disk, "")?;

    let qemu = home.join("qemu-system-x86_64");
    std::fs::copy("/bin/sh", &qemu)?;
    let _qemu = Kill(
        Command::new(&qemu)
            .args(["-c", "sleep 120; true", "qemu", "-drive"])
            .arg(format!("file={}", disk.display()))
            .arg("-netdev")
            .arg(format!("user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:22"))
            .spawn()?,
    );

    let mut child = app(home)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut mcp = Mcp {
        w: child.stdin.take().ok_or("stdin")?,
        lines: BufReader::new(child.stdout.take().ok_or("stdout")?).lines(),
    };
    mcp.call(
        0,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {},
               "clientInfo": {"name": "e2e", "version": "1"}}),
    )?;
    mcp.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
    let ssh = |pw: Option<&str>| {
        let mut a = json!({"vm": "E2E", "user": user, "confirm": true,
            "command": "echo hello-$((6*7)); id -un", "timeout_sec": 30});
        if let Some(pw) = pw {
            a["password"] = pw.into();
        }
        json!({"name": "vm_ssh", "arguments": a})
    };

    let ok = mcp.call(1, "tools/call", ssh(Some(&password)))?;
    let r = &ok["result"]["structuredContent"];
    println!("with the password: {r}");
    assert_eq!(r["exit_code"], 0, "{ok}");
    assert_eq!(r["stdout"], format!("hello-42\n{user}\n"), "{ok}");
    assert_eq!(
        (r["truncated"].clone(), r["timed_out"].clone()),
        (json!(false), json!(false))
    );

    let bad = mcp.call(2, "tools/call", ssh(Some("not-the-password")))?;
    let r = &bad["result"]["structuredContent"];
    println!("with a wrong password: {r}");
    assert!(r["exit_code"] == 255 && r["stdout"] == "", "{bad}");

    let keys = mcp.call(3, "tools/call", ssh(None))?;
    let r = &keys["result"]["structuredContent"];
    println!("without a password (BatchMode=yes, no keys): {r}");
    assert!(r["exit_code"] == 255 && r["stdout"] == "", "{keys}");

    let mut again = ssh(Some(&password));
    again["arguments"]["reset_host_key"] = true.into();
    let reset = mcp.call(4, "tools/call", again)?;
    let r = &reset["result"]["structuredContent"];
    println!("with reset_host_key: {r}");
    assert!(
        r["exit_code"] == 0
            && r["stderr"]
                .as_str()
                .is_some_and(|e| e.contains("Permanently added")),
        "the key is learned again: {reset}"
    );

    drop(mcp);
    let out = child.wait_with_output()?;
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(
        !log.contains(&password),
        "the password is in the server's stderr"
    );
    // The app's own known_hosts learned the guest's key; nothing in the config folder holds
    // the password; the per-call folders are gone.
    let cfg = home.join(".config/macos-kvm-gui");
    // (ssh-keygen -F also finds hashed entries: Ubuntu's ssh_config sets HashKnownHosts.)
    let found = Command::new("ssh-keygen")
        .args(["-F", &format!("[127.0.0.1]:{port}"), "-f"])
        .arg(cfg.join("known_hosts"))
        .output()?;
    assert!(
        found.status.success() && !found.stdout.is_empty(),
        "{found:?}"
    );
    for e in std::fs::read_dir(&cfg)? {
        let p = e?.path();
        if p.is_file() {
            let b = std::fs::read(&p)?;
            assert!(
                !b.windows(password.len()).any(|w| w == password.as_bytes()),
                "{} holds the password",
                p.display()
            );
        }
    }
    for e in std::fs::read_dir(home.join("run"))? {
        let name = e?.file_name();
        assert!(
            !name.to_string_lossy().starts_with("reims-gui-ssh-"),
            "{name:?} remains"
        );
    }
    Ok(())
}
