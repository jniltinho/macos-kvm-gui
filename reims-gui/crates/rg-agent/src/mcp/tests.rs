//! Port of the Go app's internal/mcp/server_test.go, over an in-memory MCP session.

use super::*;
use crate::testutil::Env;
use anyhow::ensure;
use rg_core::Registry;

fn rt() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

fn call(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

/// The replies to `calls` after the handshake (the initialize reply first).
fn session(env: &Env, calls: Vec<Value>) -> Result<Vec<Value>> {
    let mut msgs = handshake("test");
    msgs.extend(calls);
    rt()?.block_on(exchange(env.ctx.clone(), msgs))
}

/// A tool result's text and whether it is an error.
fn result(r: &Value) -> (String, bool) {
    let text = r["result"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<String>();
    (text, r["result"]["isError"] == true)
}

#[test]
fn tools() -> Result<()> {
    let env = Env::new()?;
    let r = session(
        &env,
        vec![json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})],
    )?;
    ensure!(
        r[0]["result"]["serverInfo"]["name"] == "reims-gui",
        "{}",
        r[0]
    );
    let tools = r[1]["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let get = |n: &str| tools.iter().find(|t| t["name"] == n);
    for n in [
        "vm_list",
        "vm_status",
        "vm_start",
        "vm_shutdown",
        "vm_force_stop",
        "screen_capture",
        "send_keys",
        "type_text",
        "pointer",
        "installer_list",
        "installer_download",
        "installer_remove",
        "install_offline",
        "host_check",
        "vm_config_get",
        "vm_config_set",
        "vm_create",
        "vm_ssh",
    ] {
        ensure!(get(n).is_some(), "missing tool {n}");
    }
    ensure!(tools.len() == 18);
    for n in ["vm_config_set", "vm_create", "vm_ssh"] {
        ensure!(
            get(n).is_some_and(|t| t["inputSchema"]["properties"].is_object()),
            "{n} has no input schema"
        );
    }
    for n in ["vm_config_set", "vm_create"] {
        ensure!(
            get(n).is_some_and(|t| t["annotations"].is_null()),
            "{n} is neither read-only nor destructive"
        );
    }
    let ssh = get("vm_ssh")
        .map(|t| t["inputSchema"].clone())
        .unwrap_or_default();
    ensure!(ssh["required"] == json!(["user", "command"]), "{ssh}");
    for n in [
        "vm_force_stop",
        "installer_remove",
        "install_offline",
        "vm_ssh",
    ] {
        ensure!(
            get(n).is_some_and(|t| t["annotations"]["destructiveHint"] == true),
            "{n} must be marked destructive"
        );
    }
    for n in [
        "vm_list",
        "vm_status",
        "screen_capture",
        "installer_list",
        "host_check",
        "vm_config_get",
    ] {
        ensure!(
            get(n).is_some_and(|t| t["annotations"]["readOnlyHint"] == true),
            "{n} must be read-only"
        );
    }
    let keys = get("send_keys")
        .map(|t| t["inputSchema"].clone())
        .unwrap_or_default();
    ensure!(
        keys["required"] == json!(["keys"]) && keys["additionalProperties"] == false,
        "{keys}"
    );
    ensure!(get("vm_list").is_some_and(|t| t["outputSchema"]["required"] == json!(["vms"])));
    Ok(())
}

// Consent and prerequisites are checked before anything happens: no VM even exists here.
#[test]
fn consent_first() -> Result<()> {
    let env = Env::new()?;
    let cases = [
        ("vm_force_stop", json!({}), "confirm: true"),
        (
            "installer_remove",
            json!({"build": "24H32"}),
            "confirm: true",
        ),
        (
            "install_offline",
            json!({"unattended": true, "erase_disk": true}),
            "license",
        ),
        (
            "install_offline",
            json!({"unattended": true, "accept_license": true}),
            "erase",
        ),
        ("type_text", json!({"text": "a\nb"}), "line break"),
        ("send_keys", json!({"keys": ["shift+nope"]}), "unknown key"),
        (
            "installer_download",
            json!({"version": "catalina"}),
            "Big Sur",
        ),
        ("vm_status", json!({"vm": "nope"}), "no VM"),
        ("vm_status", json!({"vm": "x", "agent": 1}), "unknown field"),
        (
            "vm_ssh",
            json!({"user": "u", "command": "true"}),
            "runs commands in the guest",
        ),
        (
            "vm_config_set",
            json!({"root-patch": false}),
            "use the app or `config set`",
        ),
        ("vm_config_set", json!({"workspace": "/x"}), "use the app"),
        ("vm_config_set", json!({"nic1.mode": "nat"}), "unknown key"),
    ];
    let calls = cases
        .iter()
        .enumerate()
        .map(|(i, (t, a, _))| call(i as u64 + 1, t, a.clone()))
        .collect();
    let r = session(&env, calls)?;
    for (i, (tool, a, want)) in cases.iter().enumerate() {
        let (msg, is_err) = result(&r[i + 1]);
        ensure!(
            is_err && msg.contains(want),
            "{tool} {a}: {msg:?} (error {is_err}), want {want:?}"
        );
    }
    let r = session(
        &env,
        vec![
            call(1, "installer_list", json!({})),
            call(2, "vm_list", json!({})),
        ],
    )?;
    ensure!(
        !result(&r[1]).1 && r[1]["result"]["structuredContent"]["installers"] == json!([]),
        "{}",
        r[1]
    );
    ensure!(
        r[2]["result"]["structuredContent"]["vms"] == json!([]),
        "{}",
        r[2]
    );
    Ok(())
}

#[test]
fn overall_progress_only_grows() -> Result<()> {
    let mut last = -1.0;
    for (phase, done, total) in [
        ("Reading Apple's software catalog", 0, 0),
        ("Downloading the macOS 15 installer", 0, 100),
        ("Downloading the macOS 15 installer", 100, 100),
        ("Checking the package", 0, 100),
        ("Checking the package", 100, 100),
        ("Writing the installer disc", 0, 0),
    ] {
        let v = overall(&rg_macos::Progress::new(phase, done, total));
        ensure!(
            v >= last && v <= 100.0,
            "{phase} {done}/{total} gives {v} after {last}"
        );
        last = v;
    }
    Ok(())
}

// vm_start downloads the extra kexts itself, before spawning the VM and its 30 s wait.
#[test]
fn vm_start_downloads_kexts_first() -> Result<()> {
    let env = Env::new()?;
    let mut c = Config::defaults();
    c.name = "Lab".into();
    c.version = "sequoia".into();
    c.workspace = env.path("lab").display().to_string();
    c.networks = Some(rg_core::network::new_networks(&c.version));
    let e = Registry::open(&env.ctx.app)?.create(c)?;
    env.ctx.ops.prepare_workspace(&e.config)?;
    std::fs::create_dir_all(env.ctx.app.path())?;
    std::fs::write(
        env.ctx.app.settings_file(),
        "kexts:\n  - url: https://127.0.0.1:1/x.zip\n",
    )?;
    let r = session(&env, vec![call(1, "vm_start", json!({"vm": e.id}))])?;
    let (msg, is_err) = result(&r[1]);
    ensure!(
        is_err && msg.contains("127.0.0.1:1"),
        "vm_start: {msg:?} (error {is_err})"
    );
    ensure!(
        !env.path("lab/macos-kvm-gui/mcp-start.log").exists(),
        "no VM process is spawned when a kext cannot be downloaded"
    );
    // A VM that is not running: shutdown and input tools say so.
    let r = session(
        &env,
        vec![
            call(1, "vm_shutdown", json!({})),
            call(2, "pointer", json!({"action": "click", "x": 1, "y": 1})),
        ],
    )?;
    for i in [1, 2] {
        let (msg, is_err) = result(&r[i]);
        ensure!(is_err && msg.contains("Lab is not running"), "{msg}");
    }
    Ok(())
}

#[test]
fn self_test_lists_the_tools() -> Result<()> {
    let env = Env::new()?;
    ensure!(self_test(env.ctx.clone())?.len() == 18);
    Ok(())
}

#[test]
fn scale_down_keeps_small_frames() {
    let (img, s) = scale_down(image::RgbaImage::new(800, 600), 1280);
    assert_eq!((img.width(), s), (800, 1.0));
    let (img, s) = scale_down(image::RgbaImage::new(2560, 1440), 1280);
    assert_eq!((img.width(), img.height(), s), (1280, 720, 0.5));
}

/// A process that running detection takes for the QEMU of the VM using `disk`: a copy of
/// /bin/sh named qemu-system-x86_64, sleeping, with the disk (and an SSH forward) on its
/// command line. Killed on drop.
struct FakeQemu(std::process::Child);

impl FakeQemu {
    fn start(dir: &Path, disk: &Path, hostfwd: Option<u16>) -> Result<Self> {
        let bin = dir.join("qemu-system-x86_64");
        if !bin.exists() {
            std::fs::copy("/bin/sh", &bin)?;
        }
        let mut cmd = std::process::Command::new(&bin);
        cmd.args(["-c", "sleep 60; true", "qemu", "-drive"])
            .arg(format!("file={}", disk.display()));
        if let Some(p) = hostfwd {
            cmd.args([
                "-netdev",
                &format!("user,id=net0,hostfwd=tcp:127.0.0.1:{p}-:22"),
            ]);
        }
        let q = Self(cmd.spawn()?);
        let comm = format!("/proc/{}/comm", q.0.id());
        for _ in 0..200 {
            if std::fs::read(&comm).is_ok_and(|c| c.starts_with(b"qemu-system")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(q)
    }
}

impl Drop for FakeQemu {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A VM in the list with its folder and an empty disk.
fn vm(env: &Env, name: &str, port: u16) -> Result<VmEntry> {
    let mut c = Config::defaults();
    c.name = name.into();
    c.workspace = env
        .path(rg_core::registry::slug(name))
        .display()
        .to_string();
    c.ssh_port = port;
    c.networks = Some(rg_core::network::new_networks(&c.version));
    let e = Registry::open(&env.ctx.app)?.create(c)?;
    let dir = e.config.workspace_dir()?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(rg_core::workspace::DISK_FILE), "")?;
    Ok(e)
}

fn stored(env: &Env, name: &str) -> Result<Config> {
    find_vm(&Registry::open(&env.ctx.app)?.list()?, name)
        .map(|e| e.config.clone())
        .ok_or_else(|| anyhow!("no {name}"))
}

#[test]
fn vm_config_get_and_set() -> Result<()> {
    let env = Env::new()?;
    vm(&env, "Nilton", 2222)?;
    vm(&env, "sonoma-test", 2300)?;
    let r = session(
        &env,
        vec![
            call(1, "vm_config_get", json!({"vm": "Nilton"})),
            call(
                2,
                "vm_config_set",
                json!({"vm": "sonoma-test", "ssh-port": 2222, "ram": 16384}),
            ),
            call(
                3,
                "vm_config_set",
                json!({"vm": "sonoma-test", "ram": 1024}),
            ),
            call(
                4,
                "vm_config_set",
                json!({"vm": "sonoma-test", "ssh-port": "abc"}),
            ),
            call(5, "vm_status", json!({"vm": "Nilton"})),
        ],
    )?;
    let got = &r[1]["result"]["structuredContent"];
    ensure!(
        got["ssh_port"] == 2222
            && got["name"] == "Nilton"
            && got["root_patch"] == false
            && got["networks"].as_array().is_some_and(|n| n.len() == 1)
            && got["warnings"] == json!([]),
        "{got}"
    );
    let set = &r[2]["result"]["structuredContent"];
    ensure!(
        set["ssh_port"] == 2222
            && set["ram_mib"] == 16384
            && set["warnings"] == json!(["Also used by Nilton: they cannot run at the same time"]),
        "{}",
        r[2]
    );
    let c = stored(&env, "sonoma-test")?;
    ensure!(
        c.ssh_port == 2222 && c.ram_mib == 16384,
        "saved for both apps"
    );
    let (msg, is_err) = result(&r[3]);
    ensure!(is_err && msg.contains("ram: minimum is 4096 MiB"), "{msg}");
    let (msg, is_err) = result(&r[4]);
    ensure!(
        is_err && msg.contains(r#"ssh-port: "abc" is not a whole number"#),
        "{msg}"
    );
    ensure!(
        stored(&env, "sonoma-test")?.ram_mib == 16384,
        "a refused change changes nothing"
    );
    let st = &r[5]["result"]["structuredContent"];
    ensure!(
        st["ssh_port"] == 2222
            && st["warning"] == "Also used by sonoma-test: they cannot run at the same time",
        "{st}"
    );
    Ok(())
}

#[test]
fn vm_config_set_and_vm_ssh_refuse_a_running_or_busy_vm() -> Result<()> {
    let env = Env::new()?;
    let e = vm(&env, "Run", 2222)?;
    let dir = e.config.workspace_dir()?;
    let set = || call(1, "vm_config_set", json!({"vm": "Run", "ram": 16384}));
    {
        // A start in progress (or the other app) holds the Workspace lock.
        let _held = rg_core::Workspace::new(&dir).lock()?;
        let (msg, is_err) = result(&session(&env, vec![set()])?[1]);
        ensure!(is_err && msg.contains("in use"), "{msg}");
    }
    let ssh = |id| {
        call(
            id,
            "vm_ssh",
            json!({"vm": "Run", "user": "u", "command": "true", "confirm": true}),
        )
    };
    let (msg, is_err) = result(&session(&env, vec![ssh(1)])?[1]);
    ensure!(is_err && msg.contains("Run is not running"), "{msg}");
    let _qemu = FakeQemu::start(
        env.tmp.path(),
        &dir.join(rg_core::workspace::DISK_FILE),
        None,
    )?;
    let r = session(&env, vec![set(), ssh(2)])?;
    let (msg, is_err) = result(&r[1]);
    ensure!(is_err && msg.contains("shut it down first"), "{msg}");
    let (msg, is_err) = result(&r[2]);
    ensure!(is_err && msg.contains("no SSH forward"), "{msg}");
    ensure!(stored(&env, "Run")?.ram_mib == 8192, "nothing changed");
    Ok(())
}

#[test]
fn vm_create_picks_a_free_port_and_is_ready_to_start() -> Result<()> {
    let env = Env::new()?;
    vm(&env, "Nilton", 2222)?;
    let r = session(
        &env,
        vec![
            call(
                1,
                "vm_create",
                json!({"name": "lab", "version": "sonoma", "ssh_port": 2222}),
            ),
            call(2, "vm_create", json!({"name": "auto"})),
            call(3, "vm_create", json!({"name": "low", "ssh_port": 80})),
        ],
    )?;
    let lab = &r[1]["result"]["structuredContent"];
    ensure!(
        lab["ssh_port"] == 2223 && lab["notice"] == "SSH port 2222 belongs to Nilton; using 2223",
        "{}",
        r[1]
    );
    let auto = &r[2]["result"]["structuredContent"];
    ensure!(
        auto["ssh_port"] == 2224 && auto["notice"].is_null(),
        "{}",
        r[2]
    );
    let (msg, is_err) = result(&r[3]);
    ensure!(
        is_err && msg.contains("use a port between 1024 and 65535"),
        "{msg}"
    );
    let reg = Registry::open(&env.ctx.app)?;
    let vms = reg.list()?;
    ensure!(find_vm(&vms, "low").is_none(), "a refused VM is not added");
    ensure!(reg.selected(&vms).is_some_and(|e| e.config.name == "auto"));
    // vm_start's preconditions: the boot folder (its log goes there) and the disk.
    let dir = stored(&env, "lab")?.workspace_dir()?;
    ensure!(dir == env.root().join("lab"), "{}", dir.display());
    ensure!(parent_exists(
        &dir.join(rg_core::workspace::BOOT_DIR).join("mcp-start.log")
    ));
    ensure!(dir.join(rg_core::workspace::DISK_FILE).exists());
    Ok(())
}
