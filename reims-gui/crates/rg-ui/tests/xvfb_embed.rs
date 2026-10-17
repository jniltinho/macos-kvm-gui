//! The whole embedding path under Xvfb (design D3): the real eframe window on X11 (wgpu on
//! lavapipe), a real SDL3 window titled like Reims vGPU's (rg-x11's `sdl_window` example), found
//! by pid and reparented into the Screen tab.
//!
//! `harness = false`: winit wants the main thread, and the process re-runs itself with `DISPLAY`
//! set to its own Xvfb (no environment mutation). Skips (passing) without Xvfb or the example,
//! which `cargo test --workspace` builds.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rg_ui::backend::Backend;
use rg_ui::{App, FakeBackend, Tab};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

const CHILD: &str = "RG_UI_XVFB_CHILD";
const TIMEOUT: Duration = Duration::from_secs(60);

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn main() -> ExitCode {
    if std::env::var_os(CHILD).is_some() {
        return child();
    }
    let Ok(mut x) = Command::new("Xvfb")
        .args([
            "-noreset",
            "-displayfd",
            "1",
            "-screen",
            "0",
            "1600x1000x24",
            "-nolisten",
            "tcp",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        println!("xvfb_embed: skipped (no Xvfb)");
        return ExitCode::SUCCESS;
    };
    let mut line = String::new();
    BufReader::new(x.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _x = Kill(x);
    let status = Command::new(std::env::current_exe().unwrap())
        .env("DISPLAY", format!(":{}", line.trim()))
        .env_remove("WAYLAND_DISPLAY")
        .env(CHILD, "1")
        .status()
        .unwrap();
    if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// QEMU's stand-in: target/<profile>/deps/xvfb_embed-xxx -> target/<profile>/examples/sdl_window.
fn sdl_window() -> Option<Kill> {
    let exe = std::env::current_exe().ok()?;
    let helper = exe.parent()?.parent()?.join("examples/sdl_window");
    let c = Command::new(helper)
        .arg("Reims vGPU (port 2)")
        .env("SDL_VIDEO_DRIVER", "x11")
        .spawn()
        .ok()?;
    Some(Kill(c))
}

fn child() -> ExitCode {
    // A slow runner's Xvfb may drop connections while it finishes starting.
    let up = (0..50).any(|_| {
        rg_ui::x11_available() || {
            std::thread::sleep(Duration::from_millis(100));
            false
        }
    });
    assert!(up, "Xvfb answers");
    let Some(qemu) = sdl_window() else {
        println!("xvfb_embed: skipped (build rg-x11's sdl_window example: cargo test --workspace)");
        return ExitCode::SUCCESS;
    };
    let pid = qemu.0.id();
    let fake = FakeBackend::default();
    fake.screen_pid.set(pid);
    fake.start("1", false).unwrap();
    let ok = Arc::new(AtomicBool::new(false));
    let done = ok.clone();
    let res = eframe::run_native(
        "xvfb-embed",
        rg_ui::native_options(true),
        Box::new(move |cc| {
            fake.attach(&cc.egui_ctx, true);
            let mut app = App::new(Box::new(fake));
            app.set_tab(Tab::Screen);
            Ok(Box::new(Probe {
                app,
                pid,
                done,
                since: Instant::now(),
                conn: x11rb::connect(None).unwrap().0,
                closing: false,
            }))
        }),
    );
    drop(qemu);
    if let Err(e) = res {
        eprintln!("xvfb_embed: the window failed: {e}");
        return ExitCode::FAILURE;
    }
    if ok.load(Ordering::Relaxed) {
        println!("xvfb_embed: QEMU's window is inside the Screen tab");
        ExitCode::SUCCESS
    } else {
        eprintln!("xvfb_embed: QEMU's window was not embedded within {TIMEOUT:?}");
        ExitCode::FAILURE
    }
}

struct Probe {
    app: App,
    pid: u32,
    done: Arc<AtomicBool>,
    since: Instant,
    conn: x11rb::rust_connection::RustConnection,
    /// Close sent: the app no longer gets frames, or it would refuse to close while its VM runs.
    closing: bool,
}

impl Probe {
    /// The app window has a container whose child is the SDL window of `pid`, sized like a tab.
    fn embedded(&self, app: u32) -> bool {
        let atom = self
            .conn
            .intern_atom(false, b"_NET_WM_PID")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let kids = |w| {
            self.conn
                .query_tree(w)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|t| t.children)
                .unwrap_or_default()
        };
        kids(app).into_iter().any(|container| {
            kids(container).into_iter().any(|w| {
                let pid = self
                    .conn
                    .get_property(false, w, atom, AtomEnum::CARDINAL, 0, 1)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .and_then(|p| p.value32().and_then(|mut v| v.next()));
                let g = self.conn.get_geometry(w).ok().and_then(|c| c.reply().ok());
                pid == Some(self.pid) && g.is_some_and(|g| g.width > 400 && g.height > 300)
            })
        })
    }
}

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        eframe::App::ui(&mut self.app, ui, frame);
        let app = rg_ui::screen::backend_choice::x11_window(frame).expect("an X11 window");
        if self.embedded(app) {
            self.done.store(true, Ordering::Relaxed);
            self.closing = true;
        } else if self.since.elapsed() > TIMEOUT {
            self.closing = true;
        }
        if self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
