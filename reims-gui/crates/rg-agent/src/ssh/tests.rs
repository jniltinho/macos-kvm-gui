use super::*;
use anyhow::ensure;
use std::os::unix::net::UnixStream;

const CHILD_ENV: &str = "RG_TEST_ASKPASS_CHILD";

fn rt() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

fn request<'a>(ssh: &'a Path, password: Option<&'a str>, timeout: Duration) -> Request<'a> {
    Request {
        ssh,
        port: 2223,
        user: "nilton",
        command: "sw_vers -productVersion",
        password,
        known_hosts: Path::new("/cfg/known_hosts"),
        askpass: Path::new("/usr/bin/reims-gui"),
        timeout,
    }
}

#[test]
fn argv_follows_the_design_and_never_holds_the_password() {
    let ssh = Path::new("ssh");
    let args = |pw| -> Vec<String> {
        ssh_args(&request(ssh, pw, Duration::from_secs(60)))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(
        args(None).join(" "),
        "-p 2223 -o ConnectTimeout=10 -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
-o UserKnownHostsFile=/cfg/known_hosts nilton@127.0.0.1 -- sw_vers -productVersion"
    );
    let with = args(Some("s3cret pw"));
    assert!(with.contains(&"BatchMode=no".to_string()));
    assert!(!with.iter().any(|a| a.contains("s3cret")));
}

/// Not a test on its own: the child process [`socket_answers_only_its_ssh_s_child_once`]
/// starts, acting as the askpass helper.
#[test]
fn askpass_child() -> Result<()> {
    if std::env::var_os(CHILD_ENV).is_none() {
        return Ok(());
    }
    let mut out = Vec::new();
    askpass(&mut out)?;
    std::io::stdout().write_all(b"PASSWORD=")?;
    std::io::stdout().write_all(&out)?;
    Ok(())
}

/// Runs the test binary as an askpass helper child of this process.
fn child_askpass(sock: &Path) -> Result<String> {
    let out = std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", "ssh::tests::askpass_child", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(ASKPASS_SOCKET_ENV, sock)
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn socket_answers_only_its_ssh_s_child_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sock = dir.path().join("s");
    rt()?.block_on(async {
        let listener = UnixListener::bind(&sock)?;
        // This process stands for ssh: only its children get the password.
        let server = tokio::spawn(serve_password(
            listener,
            sock.clone(),
            "pw".into(),
            std::process::id(),
        ));
        // Not a child of "ssh" (this process itself): closed unanswered.
        let mut buf = String::new();
        let s2 = sock.clone();
        let got = tokio::task::spawn_blocking(move || -> Result<String> {
            UnixStream::connect(&s2)?.read_to_string(&mut buf)?;
            Ok(buf)
        })
        .await??;
        ensure!(got.is_empty(), "a non-child got {got:?}");
        let s2 = sock.clone();
        let first = tokio::task::spawn_blocking(move || child_askpass(&s2)).await??;
        ensure!(first.contains("PASSWORD=pw\n"), "{first}");
        server.await?;
        ensure!(!sock.exists(), "the socket is removed after answering");
        let second = child_askpass(&sock)?;
        ensure!(!second.contains("PASSWORD="), "answered twice: {second}");
        Ok(())
    })
}

#[test]
fn parent_of_reads_proc() {
    assert_eq!(
        parent_of(std::process::id()),
        Some(std::os::unix::process::parent_id())
    );
    assert_eq!(parent_of(u32::MAX), None);
}

fn script(dir: &Path, body: &str) -> Result<PathBuf> {
    let p = dir.join("fake-ssh");
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n"))?;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
    Ok(p)
}

#[test]
fn output_is_capped_and_the_exit_code_kept() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let ssh = script(
        dir.path(),
        "head -c 2000000 /dev/zero | tr '\\0' a; echo oops >&2; exit 3",
    )?;
    let o = rt()?.block_on(run(&request(&ssh, None, Duration::from_secs(30))))?;
    ensure!(o.exit_code == 3 && o.truncated && !o.timed_out, "{o:?}");
    ensure!(o.stdout.len() < OUTPUT_CAP + 100 && o.stdout.ends_with("truncated at 1 MiB]"));
    ensure!(o.stderr == "oops\n", "{:?}", o.stderr);
    Ok(())
}

#[test]
fn timeout_kills_the_process_group() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // A grandchild holding the output pipes open: only a group kill ends it.
    let ssh = script(dir.path(), "sleep 60 & echo started; wait")?;
    let t = std::time::Instant::now();
    let o = rt()?.block_on(run(&request(&ssh, None, Duration::from_secs(1))))?;
    ensure!(o.timed_out && o.exit_code == -1, "{o:?}");
    ensure!(o.stdout == "started\n", "{o:?}");
    ensure!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    Ok(())
}

#[test]
fn a_password_needs_the_runtime_dir() -> Result<()> {
    if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
        return Ok(()); // the variable cannot be unset safely in a threaded test
    }
    let err = rt()?
        .block_on(run(&request(
            Path::new("true"),
            Some("pw"),
            Duration::from_secs(5),
        )))
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    ensure!(err.contains("XDG_RUNTIME_DIR"), "{err}");
    Ok(())
}
