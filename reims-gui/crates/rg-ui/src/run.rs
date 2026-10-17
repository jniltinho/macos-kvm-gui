//! Opening the main window: eframe's options, the backend's attachment and `--self-test`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use crate::{App, Backend};

/// How the window runs.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions {
    /// Embed QEMU's windows when the app runs on X11 (design D3).
    pub x11_embed: bool,
    /// Draw a few frames, close and fail if none was drawn (`reims-gui --self-test`).
    pub self_test: bool,
}

/// Frames `--self-test` draws before closing.
const SELF_TEST_FRAMES: u32 = 3;
/// `--self-test` fails when the window draws nothing for this long.
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(60);
/// After SIGTERM, SIGINT or SIGHUP, how long the window gets to let go of QEMU's windows.
const TERM_GRACE: Duration = Duration::from_secs(2);

/// Opens the main window on `backend` and blocks until it closes.
pub fn run(backend: Box<dyn Backend>, opts: RunOptions) -> anyhow::Result<()> {
    // Design D3: X11 (XWayland on Wayland desktops) when an X server answers, so the app has an
    // X11 window to embed QEMU's into; otherwise winit's choice and standalone QEMU windows.
    let x11_embed = opts.x11_embed && crate::x11_available();
    let mut options = crate::native_options(x11_embed);
    options.persist_window = !opts.self_test;
    let frames = Arc::new(AtomicU32::new(0));
    if opts.self_test {
        std::thread::spawn(|| {
            std::thread::sleep(SELF_TEST_TIMEOUT);
            eprintln!("Error: self-test: the window drew no frame in {SELF_TEST_TIMEOUT:?}");
            std::process::exit(1);
        });
    }
    let counter = frames.clone();
    let term = Arc::new(AtomicBool::new(false));
    let term_seen = term.clone();
    eframe::run_native(
        "reims-gui",
        options,
        Box::new(move |cc| {
            backend.attach(&cc.egui_ctx, x11_embed);
            on_term_signal(cc.egui_ctx.clone(), term);
            Ok(Box::new(Window {
                app: App::new(backend),
                frames: counter,
                self_test: opts.self_test,
                term: term_seen,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("could not open the window: {e}"))?;
    let drawn = frames.load(Ordering::Relaxed);
    if opts.self_test {
        anyhow::ensure!(
            drawn >= SELF_TEST_FRAMES,
            "self-test: the window closed after {drawn} frame(s)"
        );
        println!("self-test: the window drew {drawn} frames");
    }
    Ok(())
}

/// On SIGTERM, SIGINT or SIGHUP, sets `term` for the next frame, which puts QEMU's windows
/// back on the root and exits (QEMU follows through its death signal). Dying with them
/// embedded leaves them to the X save-set, which crashes Xwayland 24.1 (Ubuntu 26.04) when
/// the app's window is in a Mutter frame. If no frame comes, exits anyway.
fn on_term_signal(ctx: egui::Context, term: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
        else {
            return;
        };
        rt.block_on(async {
            use tokio::signal::unix::{SignalKind, signal};
            let (Ok(mut t), Ok(mut i), Ok(mut h)) = (
                signal(SignalKind::terminate()),
                signal(SignalKind::interrupt()),
                signal(SignalKind::hangup()),
            ) else {
                return;
            };
            tokio::select! { _ = t.recv() => {}, _ = i.recv() => {}, _ = h.recv() => {} }
        });
        term.store(true, Ordering::Relaxed);
        ctx.request_repaint();
        std::thread::sleep(TERM_GRACE);
        std::process::exit(143);
    });
}

struct Window {
    app: App,
    frames: Arc<AtomicU32>,
    self_test: bool,
    term: Arc<AtomicBool>,
}

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.term.load(Ordering::Relaxed) {
            self.app.screen.embeds.slots.clear(); // dropping each releases its window
            std::process::exit(143);
        }
        self.app.screen.attach(frame);
        self.app.show(ui);
        let n = self.frames.fetch_add(1, Ordering::Relaxed) + 1;
        if self.self_test {
            if n >= SELF_TEST_FRAMES {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ui.ctx().request_repaint();
            }
        }
    }
}
