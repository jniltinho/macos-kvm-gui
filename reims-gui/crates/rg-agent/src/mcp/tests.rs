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
    ] {
        ensure!(get(n).is_some(), "missing tool {n}");
    }
    ensure!(tools.len() == 14);
    for n in ["vm_force_stop", "installer_remove", "install_offline"] {
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
    ensure!(self_test(env.ctx.clone())?.len() == 14);
    Ok(())
}

#[test]
fn scale_down_keeps_small_frames() {
    let (img, s) = scale_down(image::RgbaImage::new(800, 600), 1280);
    assert_eq!((img.width(), s), (800, 1.0));
    let (img, s) = scale_down(image::RgbaImage::new(2560, 1440), 1280);
    assert_eq!((img.width(), img.height(), s), (1280, 720, 0.5));
}
