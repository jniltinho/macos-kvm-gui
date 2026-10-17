//! The Screen tab (design D3, D4): QEMU's own window embedded over X11 (Reims vGPU's, or QEMU's
//! SDL window for VMware SVGA), or QEMU's VNC socket drawn by the app when there is no X11;
//! Show in a window and Full screen.
//!
//! The app reads its X11 window id from eframe's frame ([`Screen::attach`]); every frame the tab
//! is drawn ([`App::screen_tab`]) and then the embedding follows it ([`App::sync_screen`]):
//! placed on the tab's rectangle in physical pixels, hidden on other tabs, for other VMs and
//! under any modal, menu, popup or tooltip that reaches the tab.

pub mod backend_choice;
mod embed;
pub(crate) mod vnc;

use egui::{Color32, Id, Order, Rect, Sense, Ui, vec2};

use crate::app::App;
use crate::backend::{Backend, ScreenArea, ScreenSource, VmEntry, VmId, VmState};
use crate::icons;
use crate::widgets::{self, bold};
pub(crate) use embed::{Slot, State as EmbedState, Target, Why};

/// The Screen tab's state, owned by the app.
#[derive(Default)]
pub(crate) struct Screen {
    attached: bool,
    /// The app's X11 window: embedding is on.
    pub parent: Option<u32>,
    pub embeds: embed::Embeds,
    pub vnc: vnc::VncView,
    session: Option<vnc::Session>,
    /// The VM the VNC view shows.
    vnc_vm: Option<VmId>,
    /// The current VM's screen source, refreshed every frame.
    pub source: Option<ScreenSource>,
    /// Full screen: the app's window shows only the screen.
    pub full: bool,
}

impl Screen {
    /// Reads the app's X11 window id once (eframe's frame); on X11 the backend starts VMs for
    /// embedding.
    pub fn attach(&mut self, frame: &eframe::Frame, backend: &dyn Backend) {
        if std::mem::replace(&mut self.attached, true) {
            return;
        }
        self.parent = backend_choice::x11_window(frame);
        if self.parent.is_some() {
            backend.set_x11_embed(true);
        }
    }
}

/// Whether the VM has a screen: running here, or started by the CLI or MCP.
fn live(v: &VmEntry) -> bool {
    v.state != VmState::Stopped || v.outside
}

/// Whether a menu, popup, tooltip or modal (anything above the panels but the toasts) reaches
/// `rect`.
fn overlay_over(ctx: &egui::Context, rect: Rect) -> bool {
    let toast = |id: Id| (0..16).any(|n| id == Id::new("toast").with(n));
    ctx.memory(|m| {
        m.areas().visible_layer_ids().into_iter().any(|l| {
            matches!(l.order, Order::Foreground | Order::Tooltip)
                && !toast(l.id)
                && m.area_rect(l.id).is_some_and(|r| r.intersects(rect))
        })
    })
}

impl App {
    /// The Screen tab's body; returns its rectangle (design D3).
    pub(crate) fn screen_tab(&mut self, ui: &mut Ui) -> Rect {
        let (rect, _) =
            ui.allocate_exact_size(ui.available_size() - vec2(0.0, 4.0), Sense::hover());
        let Some(v) = self.current().cloned() else {
            return rect;
        };
        let name = v.display_name().to_owned();
        if !live(&v) {
            let body = if v.config.reims() {
                "Start the VM: Reims vGPU draws macOS on the host's GPU right in this tab. Click it to type; Ctrl+Alt+Super releases the keyboard."
            } else if v.config.in_app() {
                "Start the VM: its screen appears in this tab. Click it to type."
            } else {
                "This VM shows its screen in QEMU's own window (Settings › Display)."
            };
            let title = if v.config.reims() {
                "The Reims vGPU display appears here while the VM runs"
            } else {
                "The VM screen appears here while it runs"
            };
            self.screen_message(ui, rect, false, "monitor", title, body, None);
            return rect;
        }
        let reims = v.config.reims();
        match self.screen.source.clone() {
            None => self.screen_waiting(ui, rect, &format!("Starting {name}…")),
            Some(ScreenSource::Vnc(_)) if self.detached => {
                if self.screen_message(
                    ui,
                    rect,
                    true,
                    "monitor",
                    "The screen is in its own window",
                    "Close that window, or bring it back here.",
                    Some(("monitor", "Show it here")),
                ) {
                    self.detached = false;
                }
            }
            Some(ScreenSource::Vnc(_)) => {
                let waiting = match self.screen.session.as_ref().and_then(|s| s.error()) {
                    Some(e) if !self.screen.vnc.has_frame() => {
                        format!("Waiting for {name}'s screen ({e})")
                    }
                    _ => format!("Connecting to {name}'s screen…"),
                };
                let input = self
                    .screen
                    .vnc
                    .show(ui, rect, Id::new("vnc-screen"), &waiting);
                if let Some(s) = &self.screen.session {
                    s.send(input);
                }
            }
            Some(ScreenSource::Standalone) => {
                let (title, body) = if reims && v.config.in_app() {
                    (
                        "Reims vGPU opened its own window",
                        "No X server is reachable, so the app runs on Wayland and cannot hold the Reims vGPU window: it is on the desktop, drawn by the GPU as usual. Ctrl+Alt+Super releases the keyboard there.",
                    )
                } else {
                    (
                        "The screen is in QEMU's window",
                        "This VM shows its screen in QEMU's own window on the desktop.",
                    )
                };
                self.screen_message(ui, rect, true, "monitor", title, body, None);
            }
            Some(ScreenSource::Window { .. }) => self.screen_embedded(ui, rect, &v, &name),
        }
        rect
    }

    fn screen_embedded(&mut self, ui: &mut Ui, rect: Rect, v: &VmEntry, name: &str) {
        let what = if v.config.reims() {
            "Reims vGPU's window"
        } else {
            "QEMU's window"
        };
        if self.screen.parent.is_none() {
            // A source for embedding without an X11 window (tests, or the backend guessed).
            self.screen_waiting(ui, rect, &format!("Placing {what} here…"));
            return;
        }
        match self.screen.embeds.state(&v.id) {
            None | Some(EmbedState::Finding(_)) => {
                self.screen_waiting(ui, rect, &format!("Placing {what} here…"))
            }
            Some(EmbedState::Embedded { .. }) => {
                // QEMU's window covers this; it shows only while something hides the window.
                ui.painter().rect_filled(rect, 0, Color32::BLACK);
                if self.screen_area.overlay_open {
                    let c = Color32::from_gray(150);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{name} is hidden while a dialog or menu is open"),
                        egui::FontId::proportional(13.0),
                        c,
                    );
                }
            }
            Some(EmbedState::Standalone(Why::Detached)) => {
                if self.screen_message(
                    ui,
                    rect,
                    true,
                    "monitor",
                    "The screen is in its own window",
                    &format!("{what} is on the desktop. Bring it back into this tab here; closing it there would stop the VM."),
                    Some(("monitor", "Show it here")),
                ) {
                    self.detached = false;
                }
            }
            Some(EmbedState::Standalone(Why::Failed(e))) => {
                let body = format!("{what} could not be placed in this tab, so it stays on the desktop.\n{e}");
                if self.screen_message(
                    ui,
                    rect,
                    true,
                    "triangle-alert",
                    "The display stayed in its own window",
                    &body,
                    Some(("rotate-cw", "Try again")),
                ) {
                    self.screen.embeds.retry(&v.id);
                }
            }
            Some(EmbedState::Standalone(Why::Closed)) => {
                self.screen_message(
                    ui,
                    rect,
                    true,
                    "monitor",
                    &format!("QEMU closed {what}"),
                    "It comes back here when the VM starts again.",
                    None,
                );
            }
        }
    }

    /// A spinner and a line on the black screen.
    fn screen_waiting(&self, ui: &mut Ui, rect: Rect, text: &str) {
        ui.painter().rect_filled(rect, 0, Color32::BLACK);
        vnc::centered(ui, rect, |ui| {
            ui.add(egui::Spinner::new().size(22.0).color(Color32::from_gray(200)));
            ui.label(egui::RichText::new(text).color(Color32::from_gray(220)));
        });
    }

    /// An icon, a title, a text and an optional button, centered; true when it was clicked.
    #[allow(clippy::too_many_arguments)]
    fn screen_message(
        &self,
        ui: &mut Ui,
        rect: Rect,
        dark: bool,
        icon: &str,
        title: &str,
        body: &str,
        button: Option<(&str, &str)>,
    ) -> bool {
        let t = self.t;
        ui.painter()
            .rect_filled(rect, 0, if dark { Color32::BLACK } else { t.input });
        let (fg, muted) = if dark {
            (Color32::WHITE, Color32::from_gray(200))
        } else {
            (t.text, t.text_muted)
        };
        let mut clicked = false;
        vnc::centered(ui, rect, |ui| {
            ui.add(icons::lucide(icon, 48.0, muted));
            ui.label(bold(title).size(15.0).color(fg));
            ui.add(egui::Label::new(egui::RichText::new(body).color(muted)).wrap());
            if let Some((icon, label)) = button {
                ui.add_space(4.0);
                clicked = widgets::button(ui, icon, label, true, true).clicked();
            }
        });
        clicked
    }

    /// After the frame is drawn: where the tab is, whether something covers it, and the
    /// embedding and VNC connection that follow (design D3).
    pub(crate) fn sync_screen(&mut self, ctx: &egui::Context, screen: Option<Rect>) -> ScreenArea {
        let overlay_open =
            !self.modals.is_empty() || screen.is_some_and(|r| overlay_over(ctx, r));
        let ppp = ctx.pixels_per_point();
        let rect = screen.filter(|_| !self.detached).map(|r| {
            let (x, y, w, h) = rg_x11::to_physical(r.min.x, r.min.y, r.width(), r.height(), ppp);
            [x, y, w as i32, h as i32]
        });
        let cur = self.current().filter(|v| live(v)).map(|v| v.id.clone());
        self.screen.source = cur.as_deref().and_then(|id| self.backend.screen_source(id));
        // The VNC connection follows the current VM's socket (it appears shortly after start).
        let s = &mut self.screen;
        if s.vnc_vm.is_some() && s.vnc_vm != cur {
            s.vnc.reset();
            s.vnc_vm.clone_from(&cur);
        }
        match &s.source {
            Some(ScreenSource::Vnc(path)) => {
                if s.session.as_ref().is_none_or(|x| &x.path != path) && path.exists() {
                    s.vnc.reset();
                    s.session = Some(vnc::Session::start(path.clone(), ctx.clone()));
                }
            }
            _ => s.session = None,
        }
        if let Some(u) = s.session.as_ref().and_then(vnc::Session::take) {
            s.vnc.pending.push(u);
        }
        if let Some(parent) = s.parent {
            let target = match &s.source {
                Some(ScreenSource::Window { pid, reims }) => Some(Target {
                    pid: *pid,
                    reims: *reims,
                }),
                _ => None,
            };
            s.embeds.sync(embed::Want {
                ctx,
                parent,
                current: cur.as_ref().zip(target),
                rect: rect.filter(|_| !overlay_open),
                detached: self.detached,
                app_pressed: ctx.input(|i| i.pointer.any_pressed()),
            });
        }
        if self.detached && matches!(self.screen.source, Some(ScreenSource::Vnc(_))) {
            self.vnc_window(ctx);
        }
        ScreenArea { rect, overlay_open }
    }

    /// Show in a window for the VNC screen: a window of the app's own.
    fn vnc_window(&mut self, ctx: &egui::Context) {
        let title = self
            .current()
            .map_or("Screen".to_owned(), |v| v.display_name().to_owned());
        let mut close = false;
        let s = &mut self.screen;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("vnc-window"),
            egui::ViewportBuilder::default()
                .with_title(&title)
                .with_inner_size([1024.0, 640.0]),
            |ctx, _| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(Color32::BLACK))
                    .show(ctx, |ui| {
                        let rect = ui.max_rect();
                        let input = s.vnc.show(
                            ui,
                            rect,
                            Id::new("vnc-window-screen"),
                            "Connecting to the VM's screen…",
                        );
                        if let Some(x) = &s.session {
                            x.send(input);
                        }
                    });
                close = ctx.input(|i| i.viewport().close_requested());
            },
        );
        if close {
            self.detached = false;
        }
    }

    /// The thin bar over the screen in full screen.
    pub(crate) fn full_screen_bar(&self, ui: &mut Ui) -> bool {
        let name = self
            .current()
            .map_or(String::new(), |v| v.display_name().to_owned());
        let mut exit = false;
        ui.horizontal_centered(|ui| {
            ui.add(icons::lucide("monitor", 14.0, self.t.text_muted));
            ui.label(bold(name));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                exit = widgets::button(ui, "minimize-2", "Exit full screen", false, true)
                    .on_hover_text("F11")
                    .clicked();
            });
        });
        exit
    }
}
