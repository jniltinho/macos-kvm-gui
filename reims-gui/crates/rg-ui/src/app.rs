//! The main window: menu bar, toolbar with the state chip, VM list, tab strip, status bar,
//! modals and toasts (the Go app's `gui.go` and `layout.go`, VirtualBox's layout).
//!
//! Shell structure (panels, actions queued during the frame and applied after it, menu table)
//! adapted from rustXterm `crates/ui/src/app.rs` and `menu.rs` (MIT).

use std::time::Duration;

use egui::{
    Align, Align2, Color32, Frame, Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, Rect,
    RichText, Sense, TextureHandle, TextureOptions, Ui, vec2,
};

use crate::backend::{
    Backend, Check, Event, Gpu, InstallStatus, PreviewUpdate, ReimsStatus, ScreenArea, Task,
    VmEntry, VmId, VmState,
};
use crate::dialogs::Modal;
use crate::files::FileDialog;
use crate::icons;
use crate::theme::{self, ThemeMode, Tokens};
use crate::widgets::{self, anim, bold, bold_font};

/// The window's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Details,
    Screen,
    Install,
    Host,
    Gpu,
    Log,
}

impl Tab {
    pub const ALL: [(Self, &'static str, &'static str); 6] = [
        (Self::Details, "Details", "info"),
        (Self::Screen, "Screen", "monitor"),
        (Self::Install, "Install", "download"),
        (Self::Host, "Host", "house"),
        (Self::Gpu, "GPU", "gpu-tab"),
        (Self::Log, "Log", "list"),
    ];
}

/// What a click, a key or a menu asks for; applied after the frame is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    New,
    Import,
    Export,
    Settings,
    Move,
    Remove,
    Start {
        verbose: bool,
    },
    Shutdown,
    ForceStop,
    /// Confirmed Force stop.
    ForceStopNow,
    ResetSettings,
    Window,
    FullScreen,
    Preferences,
    OfflineInstallers,
    About,
    Url(&'static str),
    Select(VmId),
    Tab(Tab),
    Run(Task),
    Recheck,
    Modal(Modal),
}

/// Keyboard shortcuts, shown in menus and tooltips.
pub mod keys {
    use super::*;
    pub const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
    pub const IMPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::I);
    pub const EXPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
    pub const SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
    pub const PREFERENCES: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
    pub const START: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
    pub const SHUTDOWN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::H);
    pub const REMOVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Delete);
    pub const FULL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F11);
}

/// Data from slow backend calls, refreshed on selection, events and "Check again".
#[derive(Default)]
pub(crate) struct Cache {
    pub host: Vec<Check>,
    pub reims: Vec<Check>,
    pub reims_status: ReimsStatus,
    pub gpus: Vec<Gpu>,
    pub reserved: String,
    pub install: InstallStatus,
}

/// A short message that slides in at the bottom right and fades out.
struct Toast {
    text: String,
    ok: bool,
    shown: f64,
}

pub struct App {
    pub(crate) backend: Box<dyn Backend>,
    pub(crate) mode: ThemeMode,
    pub(crate) t: Tokens,
    pub(crate) vms: Vec<VmEntry>,
    pub(crate) cur: Option<VmId>,
    pub(crate) tab: Tab,
    pub(crate) modals: Vec<Modal>,
    pub(crate) log: Vec<String>,
    pub(crate) busy: Option<Task>,
    pub(crate) progress: Option<(String, Option<f32>)>,
    pub(crate) cache: Cache,
    pub(crate) preview: Option<TextureHandle>,
    toasts: Vec<Toast>,
    fonts_set: bool,
    /// The VM whose embedded screen is in a window of its own (Window / Full screen).
    pub(crate) detached: bool,
    /// The Screen area sent to the backend this frame.
    pub(crate) screen_area: ScreenArea,
    /// The embedded or VNC screen, Full screen (design D3, D4).
    pub(crate) screen: crate::screen::Screen,
    /// The file chooser request of the Import/Export/Move dialogs.
    pub(crate) picker: crate::files::Picker,
}

impl App {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        let mode = backend.theme();
        let mut app = Self {
            backend,
            mode,
            t: Tokens::light(),
            vms: Vec::new(),
            cur: None,
            tab: Tab::Details,
            modals: Vec::new(),
            log: Vec::new(),
            busy: None,
            progress: None,
            cache: Cache::default(),
            preview: None,
            toasts: Vec::new(),
            fonts_set: false,
            detached: false,
            screen_area: ScreenArea::default(),
            screen: Default::default(),
            picker: Default::default(),
        };
        app.reload_vms();
        app
    }

    /// Opens a dialog (tests and the embedding layer use it too).
    pub fn open(&mut self, modal: Modal) {
        self.modals.push(modal);
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    pub(crate) fn current(&self) -> Option<&VmEntry> {
        self.cur
            .as_ref()
            .and_then(|id| self.vms.iter().find(|v| &v.id == id))
    }

    pub(crate) fn state(&self) -> VmState {
        self.current().map_or(VmState::Stopped, |v| v.state)
    }

    pub(crate) fn reload_vms(&mut self) {
        self.vms = self.backend.vms();
        let keep = self.cur.clone().or_else(|| self.backend.selected());
        self.cur = keep
            .filter(|id| self.vms.iter().any(|v| &v.id == id))
            .or_else(|| self.vms.first().map(|v| v.id.clone()));
        self.refresh_cache();
    }

    pub(crate) fn refresh_cache(&mut self) {
        let b = &self.backend;
        let cfg = self
            .current()
            .map(|v| v.config.clone())
            .unwrap_or_else(rg_core::Config::defaults);
        self.cache = Cache {
            host: b.host_checks(&cfg),
            reims: b.reims_checks(&cfg),
            reims_status: b.reims_status(),
            gpus: b.gpus(),
            reserved: b.reserved_gpus(),
            install: self
                .cur
                .as_deref()
                .map(|id| b.install_status(id))
                .unwrap_or_default(),
        };
    }

    pub(crate) fn append_log(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > 1000 {
            let extra = self.log.len() - 1000;
            self.log.drain(..extra);
        }
    }

    fn toast(&mut self, ctx: &egui::Context, text: String, ok: bool) {
        self.toasts.push(Toast {
            text,
            ok,
            shown: ctx.input(|i| i.time),
        });
    }

    fn resolve_theme(&mut self, ctx: &egui::Context) {
        let dark = match self.mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => ctx.system_theme() != Some(egui::Theme::Light),
        };
        let t = if dark {
            Tokens::dark()
        } else {
            Tokens::light()
        };
        if !self.fonts_set {
            ctx.set_fonts(theme::fonts());
            egui_extras::install_image_loaders(ctx);
            self.fonts_set = true;
            theme::apply(ctx, &t);
        } else if t != self.t {
            theme::apply(ctx, &t);
        }
        self.t = t;
        widgets::set_tokens(ctx, t);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let mut reload = false;
        while let Some(e) = self.backend.poll_event() {
            match e {
                Event::Log(l) => self.append_log(l),
                Event::Progress { stage, fraction } => self.progress = Some((stage, fraction)),
                Event::TaskDone { task, result } => {
                    self.busy = None;
                    self.progress = None;
                    match result {
                        Ok(msg) => {
                            self.append_log(format!("ok: {}", task.title()));
                            let text = if msg.is_empty() {
                                format!("{}: done", task.title())
                            } else {
                                msg
                            };
                            self.toast(ctx, text, true);
                        }
                        Err(err) => {
                            self.append_log(format!("error: {err}"));
                            self.modals.push(Modal::error(&task.title(), &err));
                        }
                    }
                    reload = true;
                }
                Event::State { id, state, message } => {
                    if let Some(v) = self.vms.iter_mut().find(|v| v.id == id) {
                        v.state = state;
                    }
                    if let Some(m) = message {
                        self.append_log(format!("[VM] {m}"));
                        if state != VmState::Running {
                            self.modals
                                .push(Modal::info(&format!("VM: {}", state.text()), &m));
                        }
                    }
                    if state == VmState::Stopped {
                        self.preview = None;
                        self.detached = false;
                        if std::mem::take(&mut self.screen.full) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                        }
                    }
                }
                Event::VmsChanged => reload = true,
            }
        }
        if reload {
            self.reload_vms();
        }
        if let Some(id) = self.cur.clone()
            && let Some(u) = self.backend.preview(&id)
        {
            self.apply_preview(ctx, u);
        }
    }

    /// Uploads new Preview pixels: a whole frame, or only the dirty rectangles.
    fn apply_preview(&mut self, ctx: &egui::Context, u: PreviewUpdate) {
        match (u, &mut self.preview) {
            (PreviewUpdate::Full(img), Some(tex)) if tex.size() == img.size => {
                tex.set(img, TextureOptions::LINEAR)
            }
            (PreviewUpdate::Full(img), slot) => {
                *slot = Some(ctx.load_texture("preview", img, TextureOptions::LINEAR))
            }
            (PreviewUpdate::Dirty(rects), Some(tex)) => {
                for (pos, img) in rects {
                    tex.set_partial(pos, img, TextureOptions::LINEAR);
                }
            }
            (PreviewUpdate::Dirty(_), None) => {} // no frame yet: wait for a full one
        }
    }

    /// Draws one frame into `ui` (the whole window).
    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let first = !self.fonts_set;
        self.resolve_theme(&ctx);
        if first {
            // The fonts bind at the next pass: draw from then on.
            ctx.request_repaint();
            return;
        }
        self.poll(&ctx);
        let mut out = Vec::new();
        if self.modals.is_empty() {
            self.shortcuts(&ctx, &mut out);
        }
        let t = self.t;
        let band = Frame::NONE.fill(t.bg);
        // Full screen slides the shell away and leaves the screen (and a thin bar).
        let full = self.screen.full;
        egui::Panel::top("full-bar")
            .exact_size(30.0)
            .frame(band.inner_margin(Margin::symmetric(10, 0)))
            .show_collapsible(ui, &mut full.clone(), |ui| {
                if self.full_screen_bar(ui) {
                    out.push(Action::FullScreen);
                }
            });
        egui::Panel::top("menu")
            .exact_size(26.0)
            .frame(band.inner_margin(Margin::symmetric(6, 0)))
            .show_separator_line(false)
            .show_collapsible(ui, &mut !full, |ui| self.menu_bar(ui, &mut out));
        egui::Panel::top("toolbar")
            .exact_size(64.0)
            .frame(band.inner_margin(Margin::symmetric(4, 4)))
            .show_collapsible(ui, &mut !full, |ui| self.toolbar(ui, &mut out));
        egui::Panel::bottom("status")
            .exact_size(28.0)
            .frame(band.inner_margin(Margin::symmetric(10, 0)))
            .show_collapsible(ui, &mut !full, |ui| self.status_bar(ui));
        egui::Panel::left("vms")
            .resizable(true)
            .drag_to_open(false)
            .default_size(320.0)
            .size_range(220.0..=560.0)
            .frame(Frame::NONE.fill(t.list_bg))
            .show_collapsible(ui, &mut !full, |ui| self.vm_list(ui, &mut out));
        let mut screen = None;
        let margin = if full { 0 } else { 4 };
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(t.bg).inner_margin(Margin::symmetric(margin, 0)))
            .show(ui, |ui| {
                if full {
                    screen = Some(self.screen_tab(ui));
                } else {
                    self.tab_strip(ui, &mut out);
                    screen = self.tab_body(ui, &mut out);
                }
            });
        self.dialogs(&ctx, &mut out);
        self.toasts(&ctx);
        widgets::focus_ring(&ctx, &t);
        // design D3: QEMU's window is placed here and hidden under overlays (screen module).
        self.screen_area = self.sync_screen(&ctx, screen);
        self.backend.set_screen_area(self.screen_area);
        for a in out {
            self.apply(&ctx, a);
        }
        if self.state() == VmState::Running && !cfg!(test) {
            ctx.request_repaint_after(Duration::from_millis(50)); // the status dot pulses
        }
    }

    fn shortcuts(&self, ctx: &egui::Context, out: &mut Vec<Action>) {
        let en = self.enabled();
        let table = [
            (keys::NEW, en.new, Action::New),
            (keys::IMPORT, en.new, Action::Import),
            (keys::EXPORT, en.settings, Action::Export),
            (keys::SETTINGS, en.settings, Action::Settings),
            (keys::PREFERENCES, true, Action::Preferences),
            (keys::START, en.start, Action::Start { verbose: false }),
            (keys::SHUTDOWN, en.shutdown, Action::Shutdown),
            (keys::REMOVE, en.new, Action::Remove),
            (keys::FULL, en.full, Action::FullScreen),
        ];
        ctx.input_mut(|i| {
            for (k, on, a) in table {
                if i.consume_shortcut(&k) && on {
                    out.push(a);
                }
            }
            for (n, (tab, _, _)) in Tab::ALL.iter().enumerate() {
                let key = [
                    Key::Num1,
                    Key::Num2,
                    Key::Num3,
                    Key::Num4,
                    Key::Num5,
                    Key::Num6,
                ][n];
                if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, key)) {
                    out.push(Action::Tab(*tab));
                }
            }
        });
    }

    /// Which commands can run now (the Go app's `refresh` rules).
    pub(crate) fn enabled(&self) -> Enabled {
        let state = self.state();
        let running = state != VmState::Stopped;
        let has_vm = self.current().is_some();
        let free = self.busy.is_none() && !running;
        let cfg = self.current().map(|v| &v.config);
        // Reims vGPU and in-app VMware SVGA screens are in the Screen tab (design D3, D4).
        let in_app = cfg.is_some_and(|c| c.in_app());
        let live = state != VmState::Stopped || self.current().is_some_and(|v| v.outside);
        Enabled {
            new: !running,
            settings: has_vm && !running,
            start: has_vm && free && self.cache.install.disk.is_some(),
            shutdown: state == VmState::Running,
            kill: running,
            window: in_app && live && !self.screen.full,
            full: in_app && live,
        }
    }

    // ---- menu bar and toolbar ----

    fn menu_bar(&self, ui: &mut Ui, out: &mut Vec<Action>) {
        let en = self.enabled();
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            let item = |ui: &mut Ui,
                        label: &str,
                        sc: Option<KeyboardShortcut>,
                        on: bool,
                        a: Action,
                        out: &mut Vec<Action>| {
                let mut b = egui::Button::new(label);
                if let Some(sc) = sc {
                    b = b.shortcut_text(ctx.format_shortcut(&sc));
                }
                if ui.add_enabled(on, b).clicked() {
                    out.push(a);
                }
            };
            ui.menu_button("File", |ui| {
                ui.set_min_width(240.0);
                item(
                    ui,
                    "Preferences…",
                    Some(keys::PREFERENCES),
                    true,
                    Action::Preferences,
                    out,
                );
                item(
                    ui,
                    "Offline installers…",
                    None,
                    true,
                    Action::OfflineInstallers,
                    out,
                );
                ui.separator();
                item(
                    ui,
                    "Import VM…",
                    Some(keys::IMPORT),
                    en.new,
                    Action::Import,
                    out,
                );
                item(
                    ui,
                    "Export VM…",
                    Some(keys::EXPORT),
                    en.settings,
                    Action::Export,
                    out,
                );
            });
            ui.menu_button("Machine", |ui| {
                ui.set_min_width(260.0);
                item(ui, "New…", Some(keys::NEW), en.new, Action::New, out);
                item(
                    ui,
                    "Settings…",
                    Some(keys::SETTINGS),
                    en.settings,
                    Action::Settings,
                    out,
                );
                item(ui, "Move…", None, en.settings, Action::Move, out);
                item(
                    ui,
                    "Remove…",
                    Some(keys::REMOVE),
                    en.new && self.cur.is_some(),
                    Action::Remove,
                    out,
                );
                ui.separator();
                item(
                    ui,
                    "Start",
                    Some(keys::START),
                    en.start,
                    Action::Start { verbose: false },
                    out,
                );
                item(
                    ui,
                    "Start with verbose boot (debug)",
                    None,
                    en.start,
                    Action::Start { verbose: true },
                    out,
                );
                item(
                    ui,
                    "Shut down",
                    Some(keys::SHUTDOWN),
                    en.shutdown,
                    Action::Shutdown,
                    out,
                );
                item(ui, "Force stop", None, en.kill, Action::ForceStop, out);
                ui.separator();
                item(ui, "Show in a window", None, en.window, Action::Window, out);
                item(
                    ui,
                    "Full screen",
                    Some(keys::FULL),
                    en.full,
                    Action::FullScreen,
                    out,
                );
            });
            ui.menu_button("Help", |ui| {
                ui.set_min_width(220.0);
                item(
                    ui,
                    "Online Documentation",
                    None,
                    true,
                    Action::Url(DOCS),
                    out,
                );
                item(ui, "Web Site", None, true, Action::Url(SITE), out);
                item(ui, "Bug Tracker", None, true, Action::Url(ISSUES), out);
                ui.separator();
                item(ui, "About reims-gui", None, true, Action::About, out);
            });
        });
    }

    fn toolbar(&self, ui: &mut Ui, out: &mut Vec<Action>) {
        let en = self.enabled();
        let ctx = ui.ctx().clone();
        let tip = |text: &str, sc: Option<KeyboardShortcut>| match sc {
            Some(sc) => format!("{text} ({})", ctx.format_shortcut(&sc)),
            None => text.to_owned(),
        };
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let gap = |ui: &mut Ui| {
                ui.add_space(3.0);
                let (r, _) = ui.allocate_exact_size(vec2(1.0, 40.0), Sense::hover());
                ui.painter().rect_filled(r, 0, self.t.separator);
                ui.add_space(3.0);
            };
            let buttons: [&[ToolButton]; 4] = [
                &[
                    (
                        "new",
                        "New",
                        en.new,
                        Action::New,
                        "Create a virtual machine",
                        Some(keys::NEW),
                    ),
                    (
                        "import",
                        "Import",
                        en.new,
                        Action::Import,
                        "Import a VM exported as JSON or a .tar bundle",
                        Some(keys::IMPORT),
                    ),
                ],
                &[
                    (
                        "settings",
                        "Settings",
                        en.settings,
                        Action::Settings,
                        "Change the selected VM's settings",
                        Some(keys::SETTINGS),
                    ),
                    (
                        "remove",
                        "Remove",
                        en.new && self.cur.is_some(),
                        Action::Remove,
                        "Remove the selected VM",
                        Some(keys::REMOVE),
                    ),
                ],
                &[
                    (
                        "start",
                        "Start",
                        en.start,
                        Action::Start { verbose: false },
                        "Start the selected VM",
                        Some(keys::START),
                    ),
                    (
                        "shutdown",
                        "Shut down",
                        en.shutdown,
                        Action::Shutdown,
                        "Send the ACPI power button to macOS",
                        Some(keys::SHUTDOWN),
                    ),
                    (
                        "forcestop",
                        "Force stop",
                        en.kill,
                        Action::ForceStop,
                        "Stop the VM at once, like pulling the power cord",
                        None,
                    ),
                ],
                &[
                    (
                        "window",
                        "Window",
                        en.window,
                        Action::Window,
                        "Show the screen in a window of its own",
                        None,
                    ),
                    (
                        "fullscreen",
                        "Full screen",
                        en.full,
                        Action::FullScreen,
                        "Show the screen full screen",
                        Some(keys::FULL),
                    ),
                ],
            ];
            for (i, group) in buttons.iter().enumerate() {
                if i > 0 {
                    gap(ui);
                }
                for (icon, label, on, action, text, sc) in group.iter() {
                    if widgets::tool_button(ui, icon, label, *on, &tip(text, *sc)).clicked() {
                        out.push(action.clone());
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                self.state_chip(ui);
            });
        });
    }

    /// "● Running" at the right of the toolbar; the dot pulses while the VM runs.
    fn state_chip(&self, ui: &mut Ui) {
        let (text, color) = self.state_text();
        let running = self.state() == VmState::Running || self.current().is_some_and(|v| v.outside);
        let font = bold_font(13.0);
        let w =
            ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), color).size().x) + 34.0;
        let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("State: {text}"))
        });
        let p = ui.painter();
        p.rect_filled(r, 14, color.gamma_multiply(0.12));
        let dot = r.left_center() + vec2(14.0, 0.0);
        if running && !cfg!(test) {
            let phase =
                (ui.input(|i| i.time) * std::f64::consts::TAU / 1.6).sin() as f32 * 0.5 + 0.5;
            p.circle_filled(
                dot,
                4.0 + 4.0 * phase,
                color.gamma_multiply(0.35 * (1.0 - phase)),
            );
        }
        p.circle_filled(dot, 4.0, color);
        p.text(
            dot + vec2(10.0, 0.0),
            Align2::LEFT_CENTER,
            text,
            font,
            color,
        );
    }

    pub(crate) fn state_text(&self) -> (String, Color32) {
        let t = &self.t;
        match self.current() {
            Some(v) if v.outside && v.state == VmState::Stopped => {
                ("Running (outside the app)".into(), t.success_fg)
            }
            Some(v) => (
                v.state.text().into(),
                match v.state {
                    VmState::Running => t.success_fg,
                    VmState::Paused => t.warning_fg,
                    VmState::Stopped => t.text,
                },
            ),
            None => ("No VM".into(), t.text_muted),
        }
    }

    // ---- VM list ----

    fn vm_list(&self, ui: &mut Ui, out: &mut Vec<Action>) {
        let t = self.t;
        if self.vms.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.add(icons::lucide("monitor", 40.0, t.text_muted));
                ui.add_space(8.0);
                ui.label(bold("No virtual machines yet"));
                ui.label(RichText::new("Click New to create one, or Import.").color(t.text_muted));
            });
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for v in &self.vms {
                let selected = self.cur.as_ref() == Some(&v.id);
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
                let name = v.display_name().to_owned();
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        selected,
                        &name,
                    )
                });
                let hover = anim(ui, resp.id, resp.hovered() && !selected);
                let p = ui.painter();
                if selected {
                    p.rect_filled(rect.shrink2(vec2(4.0, 1.0)), 3, t.selection);
                } else if hover > 0.0 {
                    p.rect_filled(
                        rect.shrink2(vec2(4.0, 1.0)),
                        3,
                        t.hover.gamma_multiply(hover),
                    );
                }
                let icon = Rect::from_min_size(rect.min + vec2(10.0, 13.0), vec2(36.0, 36.0));
                egui::Image::new(icons::version(&v.config.version)).paint_at(ui, icon);
                let x = icon.max.x + 10.0;
                let title = version_title(&v.config.version);
                let short = title.trim_start_matches("macOS ");
                let job = {
                    let mut j = egui::text::LayoutJob::default();
                    j.append(
                        &name,
                        0.0,
                        egui::TextFormat::simple(bold_font(13.0), t.text),
                    );
                    j.append(
                        &format!(" ({short})"),
                        0.0,
                        egui::TextFormat::simple(egui::FontId::proportional(13.0), t.text),
                    );
                    j.wrap.max_width = rect.max.x - x - 8.0;
                    j.wrap.max_rows = 1;
                    j
                };
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                p.galley(egui::pos2(x, rect.min.y + 6.0), galley, t.text);
                let (state_text, state_color) = match (v.state, v.outside) {
                    (VmState::Stopped, true) => ("Running (outside the app)", t.success_fg),
                    (VmState::Running, _) => ("Running", t.success_fg),
                    (VmState::Paused, _) => ("Paused", t.warning_fg),
                    (VmState::Stopped, false) => ("Powered Off", t.text),
                };
                let state_icon = match v.state {
                    VmState::Running => "state-running",
                    VmState::Paused => "state-paused",
                    VmState::Stopped => "state-off",
                };
                egui::Image::new(icons::app(state_icon)).paint_at(
                    ui,
                    Rect::from_min_size(egui::pos2(x, rect.min.y + 25.0), vec2(14.0, 14.0)),
                );
                let state_color = if selected { t.text } else { state_color };
                p.text(
                    egui::pos2(x + 20.0, rect.min.y + 32.0),
                    Align2::LEFT_CENTER,
                    state_text,
                    egui::FontId::proportional(13.0),
                    state_color,
                );
                let gpu = if v.config.reims() {
                    "Reims vGPU"
                } else {
                    "VMware SVGA"
                };
                let res = format!(
                    "{} GB · {} CPUs · {gpu}",
                    v.config.ram_mib / 1024,
                    v.config.vcpus
                );
                p.text(
                    egui::pos2(x, rect.min.y + 50.0),
                    Align2::LEFT_CENTER,
                    res,
                    egui::FontId::proportional(11.0),
                    if selected { t.text } else { t.text_muted },
                );
                p.hline(
                    rect.x_range().shrink(4.0),
                    rect.max.y - 0.5,
                    egui::Stroke::new(1.0, t.separator.gamma_multiply(0.6)),
                );
                let resp = resp.on_hover_text(format!("{name}\n{}\n{}", v.config.workspace, title));
                if resp.clicked() && !selected {
                    out.push(Action::Select(v.id.clone()));
                }
            }
        });
    }

    // ---- tabs ----

    fn tab_strip(&self, ui: &mut Ui, out: &mut Vec<Action>) {
        let t = self.t;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (n, (tab, label, icon)) in Tab::ALL.iter().enumerate() {
                let on = self.tab == *tab;
                let font = egui::FontId::proportional(13.0);
                let w = ui.fonts_mut(|f| {
                    f.layout_no_wrap((*label).into(), bold_font(13.0), t.text)
                        .size()
                        .x
                }) + 40.0;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, *label)
                });
                let sel = anim(ui, resp.id.with("sel"), on);
                let hover = anim(ui, resp.id, resp.hovered() && !on);
                let p = ui.painter();
                p.rect_filled(
                    r,
                    egui::CornerRadius {
                        nw: 3,
                        ne: 3,
                        sw: 0,
                        se: 0,
                    },
                    t.accent.gamma_multiply(0.12 * sel),
                );
                if hover > 0.0 {
                    p.rect_filled(r, 3, t.hover.gamma_multiply(hover));
                }
                let color = if on { t.accent_fg } else { t.text };
                let ir =
                    Rect::from_center_size(r.left_center() + vec2(16.0, 0.0), vec2(16.0, 16.0));
                icons::lucide(icon, 16.0, color).paint_at(ui, ir);
                p.text(
                    r.left_center() + vec2(30.0, 0.0),
                    Align2::LEFT_CENTER,
                    *label,
                    if on { bold_font(13.0) } else { font },
                    color,
                );
                if sel > 0.0 {
                    let line = Rect::from_center_size(
                        r.center_bottom() - vec2(0.0, 1.0),
                        vec2(r.width() * sel, 2.0),
                    );
                    p.rect_filled(line, 0, t.accent);
                }
                let tip = format!(
                    "{label} ({})",
                    ui.ctx().format_shortcut(&KeyboardShortcut::new(
                        Modifiers::COMMAND,
                        [
                            Key::Num1,
                            Key::Num2,
                            Key::Num3,
                            Key::Num4,
                            Key::Num5,
                            Key::Num6
                        ][n]
                    ))
                );
                if resp.on_hover_text(tip).clicked() {
                    out.push(Action::Tab(*tab));
                }
            }
        });
        let r = ui.cursor();
        ui.painter()
            .hline(r.x_range(), r.min.y, egui::Stroke::new(1.0, t.separator));
        ui.add_space(4.0);
    }

    fn status_bar(&self, ui: &mut Ui) {
        let (text, color) = self.state_text();
        let reims = self.current().is_some_and(|v| v.config.reims());
        let in_app = self.current().is_some_and(|v| v.config.in_app());
        let hint = match self.state() {
            _ if self
                .current()
                .is_some_and(|v| v.outside && v.state == VmState::Stopped) =>
            {
                "  ·  Started by the command line or an AI agent (MCP). Shut it down in macOS or where it was started."
            }
            VmState::Stopped => "",
            _ if reims => {
                "  ·  The Reims vGPU display is in the Screen tab; click it to type, Ctrl+Alt+Super releases the keyboard."
            }
            _ if in_app => "  ·  Click the screen to type; keys go to macOS while it has focus.",
            _ => "  ·  In the QEMU window.",
        };
        ui.horizontal_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, color);
            ui.label(
                RichText::new(format!("{text}{hint}"))
                    .italics()
                    .color(color),
            );
            if let Some(task) = &self.busy {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(egui::Spinner::new().size(14.0));
                    let stage = self
                        .progress
                        .as_ref()
                        .map_or(task.title(), |(s, _)| s.clone());
                    ui.label(RichText::new(stage).color(self.t.text_muted));
                });
            }
        });
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let life = 3.5;
        self.toasts.retain(|t| now - t.shown < life);
        if self.toasts.is_empty() {
            return;
        }
        let t = self.t;
        for (n, toast) in self.toasts.iter().enumerate() {
            let age = (now - toast.shown) as f32;
            // 160 ms ease-out in, 300 ms out.
            let alpha = if cfg!(test) {
                1.0
            } else {
                (age / 0.16)
                    .min(1.0)
                    .min((life as f32 - age) / 0.3)
                    .clamp(0.0, 1.0)
            };
            let slide = (1.0 - alpha) * 16.0;
            egui::Area::new(Id::new("toast").with(n))
                .order(egui::Order::Tooltip)
                .anchor(
                    Align2::RIGHT_BOTTOM,
                    vec2(-16.0 + slide, -40.0 - n as f32 * 52.0),
                )
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_opacity(alpha);
                    Frame::new()
                        .fill(t.overlay)
                        .corner_radius(6)
                        .stroke(egui::Stroke::new(1.0, t.separator))
                        .shadow(t.popup_shadow())
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (icon, c) = if toast.ok {
                                    ("check", t.success_fg)
                                } else {
                                    ("circle-x", t.error_fg)
                                };
                                ui.add(icons::lucide(icon, 16.0, c));
                                ui.label(&toast.text);
                            });
                        });
                });
        }
        if !cfg!(test) {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    // ---- actions ----

    fn apply(&mut self, ctx: &egui::Context, a: Action) {
        let id = self.cur.clone();
        let result = match a {
            Action::New => {
                self.modals.push(Modal::new_vm(self));
                Ok(())
            }
            Action::Settings => {
                if let Some(m) = Modal::settings(self) {
                    self.modals.push(m);
                }
                Ok(())
            }
            Action::Preferences => {
                self.modals.push(Modal::preferences(self));
                Ok(())
            }
            Action::Import => {
                self.modals.push(FileDialog::import());
                Ok(())
            }
            Action::Export | Action::Move => {
                let ws = self.backend.workspace_root();
                if let Some(v) = self.current() {
                    let m = if a == Action::Export {
                        FileDialog::export(v, &ws)
                    } else {
                        FileDialog::move_vm(v, &ws)
                    };
                    self.modals.push(m);
                }
                Ok(())
            }
            Action::OfflineInstallers => {
                self.modals.push(FileDialog::installers(self));
                Ok(())
            }
            Action::Remove => {
                if let Some(v) = self.current() {
                    self.modals.push(Modal::confirm_remove(v));
                }
                Ok(())
            }
            Action::Start { verbose } => id.map_or(Ok(()), |id| {
                if self.current().is_some_and(|v| v.config.in_app()) {
                    self.tab = Tab::Screen;
                }
                self.backend.start(&id, verbose)
            }),
            Action::Shutdown => id.map_or(Ok(()), |id| {
                let r = self.backend.shutdown(&id);
                if r.is_ok() {
                    self.modals.push(Modal::info("Shut down", SHUTDOWN_NOTE));
                }
                r
            }),
            Action::ForceStop => {
                self.modals.push(Modal::confirm_force_stop());
                Ok(())
            }
            Action::ForceStopNow => id.map_or(Ok(()), |id| self.backend.force_stop(&id)),
            Action::ResetSettings => self.backend.reset_settings(),
            Action::Window => {
                // The embedded window goes back to the desktop; a VNC screen opens a window.
                self.detached = true;
                Ok(())
            }
            Action::FullScreen => {
                self.screen.full = !self.screen.full;
                if self.screen.full {
                    self.detached = false;
                    self.tab = Tab::Screen;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.screen.full));
                Ok(())
            }
            Action::About => {
                self.modals.push(Modal::info(
                    "About reims-gui",
                    &format!(
                        "reims-gui {}\n\n{}\n\n{HOME_LAB}",
                        env!("CARGO_PKG_VERSION"),
                        self.backend.about()
                    ),
                ));
                Ok(())
            }
            Action::Url(u) => {
                self.backend.open_url(u);
                Ok(())
            }
            Action::Select(id) => {
                self.backend.select(&id);
                self.cur = Some(id);
                self.preview = None;
                self.detached = false;
                self.refresh_cache();
                Ok(())
            }
            Action::Tab(tab) => {
                self.tab = tab;
                Ok(())
            }
            Action::Run(task) => {
                if matches!(task, Task::InstallReims | Task::BuildReims) {
                    self.tab = Tab::Log;
                }
                self.append_log(format!("== {}", task.title()));
                self.busy = Some(task.clone());
                self.backend.run(id.as_deref(), task);
                Ok(())
            }
            Action::Recheck => {
                self.refresh_cache();
                Ok(())
            }
            Action::Modal(m) => {
                self.modals.push(m);
                Ok(())
            }
        };
        if let Err(e) = result {
            self.modals.push(Modal::error("Error", &e));
        }
        self.vms = self.backend.vms();
    }
}

/// (icon, label, enabled, action, tooltip, shortcut).
type ToolButton = (
    &'static str,
    &'static str,
    bool,
    Action,
    &'static str,
    Option<KeyboardShortcut>,
);

#[derive(Clone, Copy, Debug)]
pub(crate) struct Enabled {
    pub new: bool,
    pub settings: bool,
    pub start: bool,
    pub shutdown: bool,
    pub kill: bool,
    pub window: bool,
    pub full: bool,
}

/// "sonoma" → "macOS Sonoma 14".
pub fn version_title(short: &str) -> String {
    VERSIONS.iter().find(|v| v.0 == short).map_or_else(
        || format!("macOS {short}"),
        |(_, label)| {
            let (name, num) = label
                .trim_end_matches(')')
                .split_once(" (")
                .unwrap_or((label, ""));
            format!("macOS {name} {num}")
        },
    )
}

/// The Go app's `core.Versions`.
// ponytail: move to rg-core once its version table lands.
pub const VERSIONS: [(&str, &str); 9] = [
    ("high-sierra", "High Sierra (10.13)"),
    ("mojave", "Mojave (10.14)"),
    ("catalina", "Catalina (10.15)"),
    ("big-sur", "Big Sur (11)"),
    ("monterey", "Monterey (12)"),
    ("ventura", "Ventura (13)"),
    ("sonoma", "Sonoma (14)"),
    ("sequoia", "Sequoia (15)"),
    ("tahoe", "Tahoe (26)"),
];

/// Every version but Sonoma (the validated one) is experimental in the Go app.
pub fn version_label(short: &str) -> String {
    let label = VERSIONS
        .iter()
        .find(|v| v.0 == short)
        .map_or(short, |v| v.1);
    if short == "sonoma" {
        label.to_owned()
    } else {
        format!("{label} (experimental)")
    }
}

/// The Go app's `core.SMBIOSModel`.
pub fn smbios_model(short: &str) -> &'static str {
    match short {
        "high-sierra" => "iMac18,3",
        "mojave" => "iMac19,1",
        _ => "iMac20,1",
    }
}

pub const DOCS: &str = "https://github.com/jniltinho/macos-kvm-gui/tree/main/docs";
pub const SITE: &str = "https://github.com/jniltinho/macos-kvm-gui";
pub const ISSUES: &str = "https://github.com/jniltinho/macos-kvm-gui/issues";
pub const HOME_LAB: &str = "For home lab use. macOS is Apple's software: its license only allows it on Apple-branded hardware, so make sure your use complies with it.";
const SHUTDOWN_NOTE: &str = "The power button was sent to macOS. Many macOS versions treat it as Sleep, which does nothing in a VM.\n\nIf macOS keeps running, shut it down from the Apple menu (Apple › Shut Down) inside the VM: the app notices when it powers off. Use Force stop only if macOS is frozen.";

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        self.screen.attach(frame, self.backend.as_ref());
        self.show(ui);
    }
}
