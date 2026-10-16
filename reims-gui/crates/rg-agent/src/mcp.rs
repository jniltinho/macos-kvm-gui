// The rmcp server shape (ServerHandler with hand-written schemas, stdio serving, the in-memory
// `exchange`/`handshake` and `self_test`) is adapted from rustXterm's crates/agent/src/mcp.rs
// (https://github.com/jniltinho/rustXterm), MIT License, Copyright (c) 2026 Nilton OS.
// The tools are the Go app's internal/mcp/server.go.

//! The MCP server (port of the Go app's `internal/mcp`): AI agents see a VM's screen, type and
//! click in it, start and stop it and install macOS from the offline installer. It runs over
//! stdio only and drives VMs through their private control socket (QMP: screendump and
//! input-send-event).

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use rg_core::registry::find_vm;
use rg_core::{Config, VmEntry};
use rg_qemu::qcode::{self, Ev};
use rg_qemu::{KeyCode, Qmp, RunInfo, find_running};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam,
    ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{Ctx, KextsSnapshot, assets_kexts, go_duration, parent_exists};

const VM_DESC: &str = "VM name or id; default: the VM selected in the app";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hint {
    None,
    ReadOnly,
    Destructive,
}

struct ToolDef {
    name: &'static str,
    desc: &'static str,
    hint: Hint,
    input: fn() -> Value,
    output: Option<fn() -> Value>,
}

/// A JSON schema object as Go's jsonschema builds it from a struct: fields without
/// `omitempty` are required, no other properties are allowed.
fn object(props: &[(&str, Value, bool)]) -> Value {
    let mut o = json!({ "type": "object", "additionalProperties": false });
    if !props.is_empty() {
        o["properties"] = props
            .iter()
            .map(|(n, s, _)| ((*n).to_string(), s.clone()))
            .collect();
    }
    let required: Vec<&str> = props.iter().filter(|p| p.2).map(|p| p.0).collect();
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

fn string(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}
fn boolean(desc: &str) -> Value {
    json!({ "type": "boolean", "description": desc })
}
fn integer(desc: &str) -> Value {
    json!({ "type": "integer", "description": desc })
}
/// A Go slice: null when nil.
fn array(items: Value) -> Value {
    json!({ "type": ["null", "array"], "items": items })
}
fn vm_prop() -> (&'static str, Value, bool) {
    ("vm", string(VM_DESC), false)
}
fn no_args() -> Value {
    object(&[])
}
fn vm_only() -> Value {
    object(&[vm_prop()])
}
fn vm_info_props() -> Vec<(&'static str, Value, bool)> {
    vec![
        ("id", json!({"type": "string"}), true),
        ("name", json!({"type": "string"}), true),
        ("version", json!({"type": "string"}), true),
        ("running", json!({"type": "boolean"}), true),
    ]
}

const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "vm_list",
        desc: "List the VMs and whether each is running.",
        hint: Hint::ReadOnly,
        input: no_args,
        output: Some(|| object(&[("vms", array(object(&vm_info_props())), true)])),
    },
    ToolDef {
        name: "vm_status",
        desc: "Show one VM: QEMU's state, whether the offline installer disc is attached, the keyboard layout macOS's recovery uses, and the display mode. guest_phase is always unknown: look at screen_capture to see what macOS shows.",
        hint: Hint::ReadOnly,
        input: vm_only,
        output: Some(|| {
            let mut p = vm_info_props();
            p.extend([
                ("qemu_state", json!({"type": "string"}), true),
                ("guest_phase", json!({"type": "string"}), true),
                ("offline_disc_attached", json!({"type": "boolean"}), true),
                ("keyboard_layout", json!({"type": "string"}), true),
                ("screen_available", json!({"type": "boolean"}), true),
                ("uptime", json!({"type": "string"}), false),
            ]);
            object(&p)
        }),
    },
    ToolDef {
        name: "vm_start",
        desc: "Start a VM with its screen available to these tools. It keeps running when this server exits. Do not send keys while it boots (keys during boot change how macOS boots): wait until screen_capture shows macOS.",
        hint: Hint::None,
        input: || {
            object(&[
                vm_prop(),
                (
                    "verbose_boot",
                    boolean("show macOS's boot log instead of the Apple logo, for this start only"),
                    false,
                ),
            ])
        },
        output: None,
    },
    ToolDef {
        name: "vm_shutdown",
        desc: "Press the VM's power button (ACPI): macOS shuts down by itself, or asks first. Setup Assistant and the recovery may ignore it: then shut down from macOS's menu, or ask the user about vm_force_stop.",
        hint: Hint::None,
        input: vm_only,
        output: None,
    },
    ToolDef {
        name: "vm_force_stop",
        desc: "Stop a VM at once, like pulling the power cord: unsaved data in macOS is lost. Needs confirm: true; ask the user first.",
        hint: Hint::Destructive,
        input: || {
            object(&[
                vm_prop(),
                (
                    "confirm",
                    boolean("must be true: the user agreed to lose unsaved data in macOS"),
                    false,
                ),
            ])
        },
        output: None,
    },
    ToolDef {
        name: "screen_capture",
        desc: "The VM's screen as a PNG image. Coordinates for pointer are the guest's, given in the text with the image's scale.",
        hint: Hint::ReadOnly,
        input: || {
            object(&[
                vm_prop(),
                (
                    "max_width",
                    integer(
                        "scale the image down to at most this width (default 1280; 0 or more than the screen keeps it)",
                    ),
                    false,
                ),
            ])
        },
        output: None,
    },
    ToolDef {
        name: "send_keys",
        desc: "Press key chords in order, for example [\"ctrl+f2\", \"right\", \"down\"] (the menu bar from the keyboard), [\"return\"], [\"cmd+q\"]. Names are U.S. key positions: letters, digits, f1-f12, return, esc, tab, space, backspace, delete, arrows, kp_divide and other kp_ keys; modifiers shift, ctrl, alt (option), cmd.",
        hint: Hint::None,
        input: || {
            object(&[
                vm_prop(),
                (
                    "keys",
                    json!({
                        "type": ["null", "array"],
                        "items": {"type": "string"},
                        "description": "chords to press in order, such as ctrl+f2, return or cmd+q"
                    }),
                    true,
                ),
            ])
        },
        output: None,
    },
    ToolDef {
        name: "type_text",
        desc: "Type text as physical keys in the VM's keyboard layout (U.S. or Brazilian ABNT2). It never presses Return: send it with send_keys. Characters the layout cannot type are refused before any key is sent.",
        hint: Hint::None,
        input: || {
            object(&[
                vm_prop(),
                ("text", string("the text; no line breaks"), true),
                (
                    "layout",
                    string(
                        "us or br (Brazilian ABNT2); default: the layout the recovery uses on this host",
                    ),
                    false,
                ),
            ])
        },
        output: None,
    },
    ToolDef {
        name: "pointer",
        desc: "Move the pointer or click at guest coordinates: action move, click, double_click, right_click, scroll_up or scroll_down.",
        hint: Hint::None,
        input: || {
            object(&[
                vm_prop(),
                (
                    "action",
                    string("move, click, double_click, right_click, scroll_up or scroll_down"),
                    true,
                ),
                ("x", integer("guest x coordinate"), true),
                ("y", integer("guest y coordinate"), true),
            ])
        },
        output: None,
    },
    ToolDef {
        name: "installer_list",
        desc: "List the cached full installers (the offline install media), one per macOS build.",
        hint: Hint::ReadOnly,
        input: no_args,
        output: Some(|| {
            let item = object(&[
                ("version", json!({"type": "string"}), true),
                ("build", json!({"type": "string"}), true),
                ("size_gb", json!({"type": "number"}), true),
                ("path", json!({"type": "string"}), true),
            ]);
            object(&[("installers", array(item), true)])
        }),
    },
    ToolDef {
        name: "installer_download",
        desc: "Download Apple's full installer for a macOS version (Big Sur and later, 12-18 GB) into the shared cache, check it and write the offline disc. It resumes an interrupted download. A VM gets the disc when it starts with its install media, so download before vm_start.",
        hint: Hint::None,
        input: || {
            object(&[(
                "version",
                string("macOS version: big-sur, monterey, ventura, sonoma, sequoia or tahoe"),
                true,
            )])
        },
        output: None,
    },
    ToolDef {
        name: "installer_remove",
        desc: "Delete a cached full installer. Needs confirm: true; ask the user first.",
        hint: Hint::Destructive,
        input: || {
            object(&[
                ("build", string("the build, from installer_list"), true),
                (
                    "confirm",
                    boolean("must be true: the user agreed to delete it"),
                    false,
                ),
            ])
        },
        output: None,
    },
    ToolDef {
        name: "install_offline",
        desc: "Open the recovery's Terminal and type the line that installs macOS from the offline disc, as the app's Type it in the VM does. Call it only when screen_capture shows the recovery's window. Without unattended, erase the disk in Disk Utility first and continue in the installer. unattended: true installs with no questions: it ERASES the VM's empty disk (or uses \"Macintosh HD\") and ACCEPTS APPLE'S macOS LICENSE for the user, so it needs erase_disk: true and accept_license: true, which only the user can decide.",
        hint: Hint::Destructive,
        input: || {
            object(&[
                vm_prop(),
                (
                    "unattended",
                    boolean("install with no questions; needs erase_disk and accept_license"),
                    false,
                ),
                (
                    "erase_disk",
                    boolean(
                        "the user agreed that the VM's empty disk is erased (or Macintosh HD is installed over)",
                    ),
                    false,
                ),
                (
                    "accept_license",
                    boolean("the user accepts Apple's macOS software license agreement"),
                    false,
                ),
            ])
        },
        output: None,
    },
    ToolDef {
        name: "host_check",
        desc: "Check that this computer can run macOS VMs (KVM, QEMU, CPU), like the app's Host tab.",
        hint: Hint::ReadOnly,
        input: vm_only,
        output: Some(|| {
            let check = object(&[
                ("name", json!({"type": "string"}), true),
                ("status", json!({"type": "string"}), true),
                ("detail", json!({"type": "string"}), false),
                ("hint", json!({"type": "string"}), false),
            ]);
            object(&[
                ("ready", json!({"type": "boolean"}), true),
                ("checks", array(check), true),
            ])
        }),
    },
];

fn as_object(v: Value) -> JsonObject {
    match v {
        Value::Object(o) => o,
        _ => JsonObject::new(),
    }
}

/// The tools, as `tools/list` gives them.
pub fn tools() -> Vec<Tool> {
    TOOLS
        .iter()
        .map(|t| {
            let mut tool = Tool::new(t.name, t.desc, Arc::new(as_object((t.input)())));
            if let Some(out) = t.output {
                tool = tool.with_raw_output_schema(Arc::new(as_object(out())));
            }
            match t.hint {
                Hint::None => tool,
                Hint::ReadOnly => tool.with_annotations(ToolAnnotations::new().read_only(true)),
                Hint::Destructive => {
                    tool.with_annotations(ToolAnnotations::new().destructive(true))
                }
            }
        })
        .collect()
}

// --- arguments -------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct VmArg {
    #[serde(default)]
    vm: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartIn {
    #[serde(default)]
    vm: String,
    #[serde(default)]
    verbose_boot: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmVm {
    #[serde(default)]
    vm: String,
    #[serde(default)]
    confirm: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureIn {
    #[serde(default)]
    vm: String,
    #[serde(default)]
    max_width: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysIn {
    #[serde(default)]
    vm: String,
    keys: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypeIn {
    #[serde(default)]
    vm: String,
    text: String,
    #[serde(default)]
    layout: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PointerIn {
    #[serde(default)]
    vm: String,
    action: String,
    x: i64,
    y: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadIn {
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveIn {
    build: String,
    #[serde(default)]
    confirm: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallIn {
    #[serde(default)]
    vm: String,
    #[serde(default)]
    unattended: bool,
    #[serde(default)]
    erase_disk: bool,
    #[serde(default)]
    accept_license: bool,
}

fn args<T: DeserializeOwned>(a: &JsonObject) -> Result<T> {
    serde_json::from_value(Value::Object(a.clone())).map_err(|e| anyhow!("invalid arguments: {e}"))
}

// --- the server ------------------------------------------------------------------------------

/// The MCP server.
#[derive(Clone)]
pub struct Server {
    ctx: Arc<Ctx>,
    /// One lock per VM screen, so two calls never interleave their keys.
    input: Arc<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>>,
}

fn text(s: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(s.into())])
}

impl Server {
    pub fn new(ctx: Ctx) -> Self {
        Self {
            ctx: Arc::new(ctx),
            input: Arc::default(),
        }
    }

    fn find_vm(&self, key: &str) -> Result<VmEntry> {
        let reg = self.ctx.registry()?;
        let vms = reg.list()?;
        if key.is_empty() {
            return reg
                .selected(&vms)
                .ok_or_else(|| anyhow!("no VM selected: pass vm (see vm_list)"));
        }
        find_vm(&vms, key)
            .cloned()
            .ok_or_else(|| anyhow!("no VM named {key:?} (see vm_list)"))
    }

    /// A VM's QEMU. Its sockets are used only inside this user's private run directory.
    fn running(&self, key: &str) -> Result<(VmEntry, RunInfo)> {
        let e = self.find_vm(key)?;
        let r = find_running(&e.config)
            .ok_or_else(|| anyhow!("{} is not running (vm_start starts it)", e.display_name()))?;
        Ok((e, r))
    }

    async fn call(
        &self,
        name: &str,
        a: &JsonObject,
        rc: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult> {
        match name {
            "vm_list" => self.vm_list(args(a)?),
            "vm_status" => self.vm_status(args(a)?).await,
            "vm_start" => self.vm_start(args(a)?).await,
            "vm_shutdown" => self.vm_shutdown(args(a)?).await,
            "vm_force_stop" => self.vm_force_stop(args(a)?).await,
            "screen_capture" => self.screen_capture(args(a)?).await,
            "send_keys" => self.send_keys(args(a)?, rc).await,
            "type_text" => self.type_text(args(a)?, rc).await,
            "pointer" => self.pointer(args(a)?).await,
            "installer_list" => self.installer_list(args(a)?),
            "installer_download" => self.installer_download(args(a)?, rc).await,
            "installer_remove" => self.installer_remove(args(a)?),
            "install_offline" => self.install_offline(args(a)?, rc).await,
            "host_check" => Ok(self.host_check(args(a)?)),
            _ => bail!("unknown tool {name:?}"),
        }
    }

    // --- VMs ---------------------------------------------------------------------------------

    fn vm_list(&self, _: NoArgs) -> Result<CallToolResult> {
        let vms: Vec<Value> = self
            .ctx
            .registry()?
            .list()?
            .iter()
            .map(|e| {
                json!({"id": e.id, "name": e.display_name(), "version": e.config.version,
                       "running": find_running(&e.config).is_some()})
            })
            .collect();
        Ok(CallToolResult::structured(json!({ "vms": vms })))
    }

    async fn vm_status(&self, a: VmArg) -> Result<CallToolResult> {
        let e = self.find_vm(&a.vm)?;
        let layout = rg_core::keyboard::recovery_keyboard();
        let mut out = json!({"id": e.id, "name": e.display_name(), "version": e.config.version, "running": false,
            "qemu_state": "stopped", "guest_phase": "unknown", "offline_disc_attached": false,
            "keyboard_layout": layout, "screen_available": false});
        if let Some(r) = find_running(&e.config) {
            out["running"] = true.into();
            out["offline_disc_attached"] = r.disc.into();
            out["screen_available"] = r.control.is_some().into();
            out["uptime"] = go_duration(r.uptime()).into();
            out["qemu_state"] = "running".into();
            if let Ok(q) = control(&r).await
                && let Ok(st) = q.status().await
                && !st.is_empty()
            {
                out["qemu_state"] = st.into();
            }
        }
        Ok(CallToolResult::structured(out))
    }

    /// Runs `reims-gui vm start` in its own process group: that process owns QEMU, so the VM
    /// outlives this server and the agent.
    async fn vm_start(&self, a: StartIn) -> Result<CallToolResult> {
        let e = self.find_vm(&a.vm)?;
        if find_running(&e.config).is_some() {
            return Ok(text(format!("{} is already running", e.display_name())));
        }
        let dir = e.config.workspace_dir()?;
        let me = std::env::current_exe()?;
        // Downloads happen here, under the client's request, so the wait below covers only QEMU.
        let ks = self.ctx.app.load_extra_kexts()?;
        let cache = self.ctx.kext_cache()?;
        let reims_kext = e.config.reims_kext();
        let aks = assets_kexts(&ks);
        tokio::task::spawn_blocking(move || -> Result<()> {
            let cancel = AtomicBool::new(false);
            rg_assets::download_extra_kexts(&aks, &cache, &cancel)?;
            if reims_kext {
                rg_assets::download_reims_kext(&cache, &cancel)?;
            }
            Ok(())
        })
        .await??;
        let snapshot = KextsSnapshot::json(&ks)?;
        let log_path = dir.join(rg_core::workspace::BOOT_DIR).join("mcp-start.log");
        if !parent_exists(&log_path) {
            bail!(
                "{}: no such folder (prepare the VM's folder first)",
                dir.join(rg_core::workspace::BOOT_DIR).display()
            );
        }
        let log = std::fs::File::create(&log_path)?;
        let mut cmd = std::process::Command::new(me);
        cmd.args([
            "vm",
            "start",
            "--vm",
            &e.id,
            "--display",
            rg_core::config::DISPLAY_APP,
            "--kexts-snapshot",
            &snapshot,
        ]);
        if a.verbose_boot {
            cmd.arg("--verbose-boot");
        }
        // ponytail: a new process group, not a session (setsid needs unsafe or a helper binary);
        // Ctrl+C and the agent's exit do not reach it, a closing terminal's SIGHUP could.
        use std::os::unix::process::CommandExt;
        cmd.stdin(std::process::Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0);
        let mut child = cmd.spawn()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if child.try_wait()?.is_some() {
                let b = std::fs::read_to_string(&log_path).unwrap_or_default();
                bail!("the VM did not start: {}", b.trim());
            }
            if let Some(r) = find_running(&e.config)
                && r.vnc_socket.is_some()
            {
                return Ok(text(format!(
                    "{} started (QEMU pid {}). It boots for a minute or two: check screen_capture before sending keys.",
                    e.display_name(),
                    r.pid
                )));
            }
        }
        bail!(
            "the VM did not show up within 30 s; see {}",
            log_path.display()
        )
    }

    async fn vm_shutdown(&self, a: VmArg) -> Result<CallToolResult> {
        let (e, r) = self.running(&a.vm)?;
        control(&r).await?.system_powerdown().await?;
        Ok(text(format!(
            "Pressed the power button of {}: macOS shuts down by itself, or asks on its screen.",
            e.display_name()
        )))
    }

    async fn vm_force_stop(&self, a: ConfirmVm) -> Result<CallToolResult> {
        if !a.confirm {
            bail!(
                "a forced stop loses unsaved data in macOS: ask the user, then call it with confirm: true"
            );
        }
        let (e, r) = self.running(&a.vm)?;
        control(&r).await?.quit().await?;
        Ok(text(format!("{} was stopped.", e.display_name())))
    }

    // --- screen and input --------------------------------------------------------------------

    /// Connects to a running VM's control socket and runs `f` with the VM's input lock held,
    /// so calls never interleave their keys.
    async fn with_screen<T>(
        &self,
        key: &str,
        f: impl AsyncFnOnce(&VmEntry, &Qmp, &Path) -> Result<T>,
    ) -> Result<T> {
        let (e, r) = self.running(key)?;
        let sock = r.control.clone().ok_or_else(no_control)?;
        let lock = {
            let mut m = self
                .input
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            m.entry(sock.clone()).or_default().clone()
        };
        let _held = lock.lock().await;
        let q = control(&r).await?;
        let run_dir = sock.parent().ok_or_else(no_control)?;
        f(&e, &q, run_dir).await
    }

    async fn screen_capture(&self, a: CaptureIn) -> Result<CallToolResult> {
        let (name, png) = self
            .with_screen(&a.vm, async |e, q, dir| {
                Ok((e.display_name(), screendump(q, dir).await?))
            })
            .await?;
        let frame = image::load_from_memory(&png)?.to_rgba8();
        let (w, h) = frame.dimensions();
        let max_w = if a.max_width == 0 { 1280 } else { a.max_width };
        let (img, scale) = scale_down(frame, max_w);
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)?;
        let mut note = format!("{name} screen, {w}x{h} guest pixels");
        if scale != 1.0 {
            note += &format!(
                "; image scaled by {scale:.3}: divide image coordinates by it for pointer"
            );
        }
        let data = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
        Ok(CallToolResult::success(vec![
            ContentBlock::image(data, "image/png"),
            ContentBlock::text(note),
        ]))
    }

    async fn send_keys(
        &self,
        a: KeysIn,
        rc: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult> {
        let keys = a.keys.unwrap_or_default();
        let chords = keys
            .iter()
            .map(|k| rg_qemu::keys::parse_chord(k))
            .collect::<Result<Vec<_>, _>>()?;
        if chords.is_empty() {
            bail!("keys is empty");
        }
        self.with_screen(&a.vm, async |_, q, _| {
            press_all(q, &chords, Duration::from_millis(80), rc).await
        })
        .await?;
        Ok(text(format!("Pressed {}.", keys.join(", "))))
    }

    async fn type_text(
        &self,
        a: TypeIn,
        rc: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult> {
        let layout = match a.layout.as_str() {
            "" => {
                let (l, v) = rg_core::keyboard::host_keyboard();
                rg_core::keyboard::mac_keyboard_id(&l, &v)
            }
            "us" => rg_qemu::keys::LAYOUT_US,
            "br" => rg_qemu::keys::LAYOUT_ABNT2,
            l => bail!("layout {l:?}: use us or br"),
        };
        let chords = rg_qemu::keys::type_keys(&a.text, layout)?;
        self.with_screen(&a.vm, async |_, q, _| {
            press_all(q, &chords, TYPING_GAP, rc).await
        })
        .await?;
        Ok(text(format!(
            "Typed {} characters (Return not pressed).",
            a.text.chars().count()
        )))
    }

    async fn pointer(&self, a: PointerIn) -> Result<CallToolResult> {
        let buttons: &[&'static str] = match a.action.as_str() {
            "move" => &[],
            "click" => &["left"],
            "double_click" => &["left", "left"],
            "right_click" => &["right"],
            "scroll_up" => &["wheel-up"],
            "scroll_down" => &["wheel-down"],
            other => bail!(
                "action {other:?}: use move, click, double_click, right_click, scroll_up or scroll_down"
            ),
        };
        let (x, y) = (a.x, a.y);
        self.with_screen(&a.vm, async |_, q, dir| {
            let png = screendump(q, dir).await?;
            let (w, h) = image::load_from_memory(&png)?.to_rgba8().dimensions();
            let (w, h) = (i64::from(w), i64::from(h));
            if x < 0 || y < 0 || x >= w || y >= h {
                bail!("({x}, {y}) is outside the {w}x{h} screen");
            }
            let at = qcode::abs(
                x as f64 / (w - 1).max(1) as f64,
                y as f64 / (h - 1).max(1) as f64,
            );
            q.input_send_event(&[at]).await?;
            for b in buttons {
                tokio::time::sleep(Duration::from_millis(60)).await;
                q.input_send_event(&[Ev::Btn(b, true)]).await?;
                tokio::time::sleep(Duration::from_millis(60)).await;
                q.input_send_event(&[Ev::Btn(b, false)]).await?;
            }
            Ok(())
        })
        .await?;
        Ok(text(format!("{} at ({x}, {y}).", a.action)))
    }

    // --- offline installer -------------------------------------------------------------------

    fn installer_list(&self, _: NoArgs) -> Result<CallToolResult> {
        let dir = rg_macos::installers_dir(&self.ctx.workspace_root()?);
        let list: Vec<Value> = rg_macos::cached_installers(&dir)
            .iter()
            .map(|c| {
                let p = &c.product;
                json!({"version": p.version, "build": p.build, "size_gb": p.size as f64 / 1e9, "path": c.path})
            })
            .collect();
        Ok(CallToolResult::structured(json!({ "installers": list })))
    }

    async fn installer_download(
        &self,
        a: DownloadIn,
        rc: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult> {
        if !rg_macos::installer_supported(&a.version) {
            bail!(
                "Apple has a full installer only for Big Sur and later, not {:?}",
                a.version
            );
        }
        let dir = rg_macos::installers_dir(&self.ctx.workspace_root()?);
        let token = rc.meta.get_progress_token();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(f64, String)>();
        let cancel = Arc::new(AtomicBool::new(false));
        let job = {
            let cancel = cancel.clone();
            tokio::task::spawn_blocking(move || {
                let (mut last, mut sent) = (-1.0, None::<Instant>);
                let mut progress = |p: rg_macos::Progress| {
                    let v = overall(&p);
                    if v <= last
                        || (sent.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) && v < 99.0)
                    {
                        return;
                    }
                    (last, sent) = (v, Some(Instant::now()));
                    let _ = tx.send((v, p.phase));
                };
                rg_macos::download_installer(
                    &rg_macos::Http::default(),
                    &dir,
                    &a.version,
                    &cancel,
                    &mut progress,
                    &mut |s| eprintln!("{s}"),
                )
            })
        };
        let mut job = std::pin::pin!(job);
        let ci = loop {
            tokio::select! {
                res = &mut job => break res??,
                Some((v, msg)) = rx.recv() => if let Some(t) = &token {
                    let p = ProgressNotificationParam::new(t.clone(), v).with_total(100.0).with_message(msg);
                    let _ = rc.peer.notify_progress(p).await;
                },
                () = rc.ct.cancelled(), if !cancel.load(Ordering::Relaxed) => cancel.store(true, Ordering::Relaxed),
            }
        };
        let p = &ci.product;
        Ok(text(format!(
            "Offline installer ready: macOS {} ({}), {:.1} GB, {}. VMs of this version get it when they start with their install media.",
            p.version,
            p.build,
            p.size as f64 / 1e9,
            ci.path.display()
        )))
    }

    fn installer_remove(&self, a: RemoveIn) -> Result<CallToolResult> {
        if !a.confirm {
            bail!(
                "deleting an installer means downloading 12-18 GB again: ask the user, then call it with confirm: true"
            );
        }
        rg_macos::remove_installer(
            &rg_macos::installers_dir(&self.ctx.workspace_root()?),
            &a.build,
        )?;
        Ok(text(format!("Deleted the installer {}.", a.build)))
    }

    async fn install_offline(
        &self,
        a: InstallIn,
        rc: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult> {
        if a.unattended && !a.accept_license {
            bail!(
                "the unattended install accepts Apple's macOS license for the user: ask them, then pass accept_license: true"
            );
        }
        if a.unattended && !a.erase_disk {
            bail!(
                "the unattended install erases the VM's empty disk: ask the user, then pass erase_disk: true"
            );
        }
        if !rg_core::keyboard::guest_typing_ok() {
            bail!(
                "this keyboard layout moves the letters of the line (Dvorak, Russian): type it in the recovery's Terminal yourself: {}",
                rg_macos::DISC_COMMAND
            );
        }
        let (e, r) = self.running(&a.vm)?;
        if !r.disc {
            bail!(
                "{} has no offline disc: download the installer (installer_download), then start the VM with its install media",
                e.display_name()
            );
        }
        let mut line = rg_macos::DISC_TYPED.to_string();
        if a.unattended {
            line += " -u";
        }
        let chords = rg_qemu::keys::guest_keys(&line)?;
        self.with_screen(&a.vm, async |_, q, _| {
            press_all(
                q,
                &rg_qemu::keys::recovery_terminal(),
                Duration::from_millis(120),
                rc,
            )
            .await?;
            tokio::time::sleep(Duration::from_secs(2)).await; // the Terminal window opens
            press_all(q, &chords, TYPING_GAP, rc).await
        })
        .await?;
        let msg = if a.unattended {
            "Typed the unattended install line. The VM erases its empty disk, installs and restarts by itself (about 30 minutes) up to Setup Assistant; follow it with screen_capture."
        } else {
            "Typed the install line in the recovery's Terminal. It copies the installer from the disc and opens it: continue in the installer."
        };
        Ok(text(format!(
            "{msg} Check screen_capture: if the Terminal did not open, the keys went to the recovery's window."
        )))
    }

    fn host_check(&self, a: VmArg) -> CallToolResult {
        let version = self
            .find_vm(&a.vm)
            .map_or_else(|_| Config::defaults().version, |e| e.config.version);
        let mut ready = true;
        let checks: Vec<Value> = self
            .ctx
            .host()
            .checks(&version)
            .into_iter()
            .map(|c| {
                ready &= c.status != rg_host::Status::Fail;
                let mut o = json!({"name": c.name, "status": c.status.to_string()});
                if !c.detail.is_empty() {
                    o["detail"] = c.detail.into();
                }
                if !c.hint.is_empty() {
                    o["hint"] = c.hint.into();
                }
                o
            })
            .collect();
        CallToolResult::structured(json!({"ready": ready, "checks": checks}))
    }
}

/// The pause between typed characters: the guest's USB keyboard queue holds only a few events.
// ponytail: fixed pace; measure the guest's HID polling if typing ever drops keys.
const TYPING_GAP: Duration = Duration::from_millis(30);

fn no_control() -> anyhow::Error {
    anyhow!(
        "this VM was started without a control socket (by an older version or outside the app): use the app"
    )
}

async fn control(r: &RunInfo) -> Result<Qmp> {
    let sock = r.control.as_deref().ok_or_else(no_control)?;
    rg_qemu::check_run_dir(sock)?;
    Ok(Qmp::dial(sock).await?.0)
}

/// QEMU writes the screen as PNG into the VM's private run directory; returns its bytes.
async fn screendump(q: &Qmp, dir: &Path) -> Result<Vec<u8>> {
    static N: AtomicU64 = AtomicU64::new(0);
    let file = dir.join(format!(
        "screen-{}-{}.png",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let res = q.screendump(&file, None).await;
    let png = res
        .map_err(anyhow::Error::from)
        .and_then(|()| Ok(std::fs::read(&file)?));
    let _ = std::fs::remove_file(&file);
    png
}

/// Shrinks the frame to at most `max_w` wide (nearest pixel, like the app's preview).
fn scale_down(frame: image::RgbaImage, max_w: i64) -> (image::DynamicImage, f64) {
    let (w, h) = frame.dimensions();
    match u32::try_from(max_w) {
        Ok(mw) if mw > 0 && w > mw => {
            let nh = (u64::from(h) * u64::from(mw) / u64::from(w)) as u32;
            let img = image::imageops::resize(
                &frame,
                mw,
                nh.max(1),
                image::imageops::FilterType::Nearest,
            );
            (img.into(), f64::from(mw) / f64::from(w))
        }
        _ => (frame.into(), 1.0),
    }
}

/// One chord over QMP: its keys go down in order and come up in reverse (scancodes, so the
/// guest's layout applies).
async fn chord(q: &Qmp, keys: &[KeyCode]) -> Result<()> {
    let ev = |k: &KeyCode, down| json!({"type": "key", "data": {"down": down, "key": {"type": "number", "data": k.scancode}}});
    let events: Vec<Value> = keys
        .iter()
        .map(|k| ev(k, true))
        .chain(keys.iter().rev().map(|k| ev(k, false)))
        .collect();
    q.execute("input-send-event", Some(json!({ "events": events })))
        .await?;
    Ok(())
}

/// Presses chords in order; a cancelled call stops between chords (each chord is pressed and
/// released whole, so no key stays down).
async fn press_all(
    q: &Qmp,
    chords: &[Vec<KeyCode>],
    gap: Duration,
    rc: &RequestContext<RoleServer>,
) -> Result<()> {
    for c in chords {
        if rc.ct.is_cancelled() {
            bail!("cancelled");
        }
        chord(q, c).await?;
        tokio::time::sleep(gap).await;
    }
    Ok(())
}

/// Maps the download's phases to one value that only grows (0-100).
fn overall(p: &rg_macos::Progress) -> f64 {
    let pct = p.pct().unwrap_or(0.0);
    if p.phase.starts_with("Downloading") {
        1.0 + pct * 0.89
    } else if p.phase.starts_with("Checking") {
        90.0 + pct * 0.09
    } else if p.phase.starts_with("Writing") {
        99.0
    } else {
        0.0
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new("reims-gui", self.ctx.version.clone());
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        if !TOOLS.iter().any(|t| t.name == name) {
            return Err(ErrorData::invalid_params(
                format!("unknown tool {name:?}"),
                None,
            ));
        }
        let a = request.arguments.unwrap_or_default();
        let result = match self.call(&name, &a, &context).await {
            Ok(r) => r,
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
        };
        Ok(result.into())
    }
}

/// Serves MCP on stdin and stdout until the client disconnects. Nothing else may write to
/// stdout.
pub fn serve_stdio(ctx: Ctx) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let running = Server::new(ctx).serve(rmcp::transport::io::stdio()).await?;
        running.waiting().await?;
        Ok(())
    })
}

/// Runs `messages` (JSON-RPC objects) against an in-process server over an in-memory duplex and
/// returns the responses to the messages that have an `id`, in order.
pub async fn exchange(ctx: Ctx, messages: Vec<Value>) -> Result<Vec<Value>> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (ours, theirs) = tokio::io::duplex(1 << 20);
    let server = tokio::spawn(async move {
        if let Ok(running) = Server::new(ctx).serve(theirs).await {
            let _ = running.waiting().await;
        }
    });
    let (r, mut w) = tokio::io::split(ours);
    let mut lines = BufReader::new(r).lines();
    let mut out = Vec::new();
    for msg in messages {
        let id = msg.get("id").cloned();
        w.write_all(format!("{msg}\n").as_bytes()).await?;
        let Some(id) = id else { continue };
        loop {
            let line = tokio::time::timeout(Duration::from_secs(30), lines.next_line())
                .await
                .map_err(|_| anyhow!("MCP server did not answer in 30 s"))??
                .ok_or_else(|| anyhow!("MCP server closed the stream"))?;
            let v: Value = serde_json::from_str(&line)?;
            if v.get("id") == Some(&id) {
                out.push(v);
                break;
            }
        }
    }
    server.abort();
    Ok(out)
}

/// JSON-RPC `initialize` + `initialized` for [`exchange`].
pub fn handshake(client: &str) -> Vec<Value> {
    vec![
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": client, "version": "1.0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    ]
}

/// `--self-test`: initialize + tools/list in-process; returns the tool names.
pub fn self_test(ctx: Ctx) -> Result<Vec<String>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut msgs = handshake("reims-gui-self-test");
    msgs.push(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
    let replies = rt.block_on(exchange(ctx, msgs))?;
    let server = replies
        .first()
        .and_then(|r| r.pointer("/result/serverInfo/name"))
        .and_then(Value::as_str);
    if server != Some("reims-gui") {
        bail!("unexpected initialize reply: {:?}", replies.first());
    }
    let tools = replies
        .get(1)
        .and_then(|r| r.pointer("/result/tools"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("no tool list"))?;
    Ok(tools
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect())
}

#[cfg(test)]
mod tests;
