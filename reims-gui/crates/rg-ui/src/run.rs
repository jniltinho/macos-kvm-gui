//! Opening the main window: eframe's options, the backend's attachment and `--self-test`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
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

/// Opens the main window on `backend` and blocks until it closes.
pub fn run(backend: Box<dyn Backend>, opts: RunOptions) -> anyhow::Result<()> {
    // ponytail: $DISPLAY stands in for the X11 probe and event-loop choice of design D3.
    let x11_embed = opts.x11_embed && std::env::var_os("DISPLAY").is_some();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("reims-gui")
            .with_app_id("io.github.jniltinho.reims-gui")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 600.0]),
        persist_window: !opts.self_test,
        ..Default::default()
    };
    let frames = Arc::new(AtomicU32::new(0));
    if opts.self_test {
        std::thread::spawn(|| {
            std::thread::sleep(SELF_TEST_TIMEOUT);
            eprintln!("Error: self-test: the window drew no frame in {SELF_TEST_TIMEOUT:?}");
            std::process::exit(1);
        });
    }
    let counter = frames.clone();
    eframe::run_native(
        "reims-gui",
        options,
        Box::new(move |cc| {
            backend.attach(&cc.egui_ctx, x11_embed);
            Ok(Box::new(Window {
                app: App::new(backend),
                frames: counter,
                self_test: opts.self_test,
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

struct Window {
    app: App,
    frames: Arc<AtomicU32>,
    self_test: bool,
}

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
