//! Modal dialogs as data (PhotoCraft's "dialogs as data", MIT OR Apache-2.0): each dialog is a
//! value holding its own state, drawn every frame and closed by returning an outcome. The pages
//! are the Go app's `settings.go`, `network.go`, `wizard.go` and the Preferences dialog.

use egui::{Align, Layout, RichText, Ui, vec2};
use rg_core::Nic;
use rg_core::config::{self, Config};

use crate::app::{Action, App, Tab, VERSIONS, version_label, version_title};
use crate::backend::{HostInfo, Status, Task, VmEntry, VmId};
use crate::icons;
use crate::theme::ThemeMode;
use crate::widgets::{
    self, Footer, bold, caption, form_row, modal, note, section_band, slider_row, warning,
};

/// A dialog and its state.
#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    Info {
        title: String,
        text: String,
    },
    Error {
        title: String,
        text: String,
    },
    Confirm {
        title: String,
        text: String,
        action: Box<Action>,
    },
    Remove {
        id: VmId,
        name: String,
        delete_files: bool,
    },
    Settings(Box<Settings>),
    NewVm(Box<Wizard>),
    Preferences(Prefs),
    /// Import, Export, Move, Offline installers and the copy progress (`files.rs`).
    File(Box<crate::files::FileDialog>),
}

impl Modal {
    pub fn info(title: &str, text: &str) -> Self {
        Self::Info {
            title: title.into(),
            text: text.into(),
        }
    }
    pub fn error(title: &str, text: &str) -> Self {
        Self::Error {
            title: title.into(),
            text: text.into(),
        }
    }
    pub fn confirm(title: &str, text: &str, action: Action) -> Self {
        Self::Confirm {
            title: title.into(),
            text: text.into(),
            action: Box::new(action),
        }
    }
    pub fn confirm_force_stop() -> Self {
        Self::confirm(
            "Force stop",
            "This is like pulling the power cord: unsaved data in macOS will be lost. Continue?",
            Action::ForceStopNow,
        )
    }
    pub fn confirm_remove(v: &VmEntry) -> Self {
        Self::Remove {
            id: v.id.clone(),
            name: v.display_name().into(),
            delete_files: false,
        }
    }
    pub fn settings(app: &App) -> Option<Self> {
        let v = app.current()?;
        let mut s = Settings::new(v, app.backend.host_info(), app.cache.reims_status.installed);
        let i = &app.cache.install;
        s.disk = i.disk.clone().unwrap_or_else(|| "not created yet".into());
        s.media = i.recovery.clone();
        s.can_remove_media = app.busy.is_none();
        Some(Self::Settings(Box::new(s)))
    }
    pub fn new_vm(app: &App) -> Self {
        let reims_ok = app.cache.reims_status.installed
            && app.cache.reims.iter().all(|c| c.status != Status::Fail);
        Self::NewVm(Box::new(Wizard::new(
            app.backend.recommended(),
            app.backend.host_info(),
            app.backend.workspace_root(),
            reims_ok,
            app.backend.random_mac(),
        )))
    }
    pub fn preferences(app: &App) -> Self {
        Self::Preferences(Prefs {
            workspace: app.backend.workspace_root(),
            theme: app.mode,
        })
    }
}

/// What a dialog asked for this frame.
#[derive(Default)]
pub(crate) struct Outcome {
    pub close: bool,
    pub actions: Vec<Action>,
    pub modals: Vec<Modal>,
}

impl App {
    /// Draws the open dialogs, the last one on top.
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context, out: &mut Vec<Action>) {
        let mut modals = std::mem::take(&mut self.modals);
        let mut keep = Vec::with_capacity(modals.len());
        let mut pushed = Vec::new();
        let top = modals.len().saturating_sub(1);
        for (n, m) in modals.iter_mut().enumerate() {
            let o = self.dialog(ctx, n, m, n == top);
            out.extend(o.actions);
            pushed.extend(o.modals);
            if !o.close {
                keep.push(n);
            }
        }
        let mut remaining: Vec<Modal> = modals
            .into_iter()
            .enumerate()
            .filter(|(n, _)| keep.contains(n))
            .map(|(_, m)| m)
            .collect();
        remaining.append(&mut self.modals); // opened by actions inside a dialog
        remaining.extend(pushed);
        self.modals = remaining;
    }

    fn dialog(&mut self, ctx: &egui::Context, n: usize, m: &mut Modal, top: bool) -> Outcome {
        let mut o = Outcome::default();
        let id = format!("modal-{n}");
        let err = matches!(m, Modal::Error { .. });
        match m {
            Modal::Info { title, text } | Modal::Error { title, text } => {
                let t = self.t;
                let f = modal(ctx, &id, title, vec2(460.0, 0.0), Some("OK"), None, |ui| {
                    ui.horizontal_top(|ui| {
                        let (icon, c) = if err {
                            ("circle-x", t.error_fg)
                        } else {
                            ("info", t.accent_fg)
                        };
                        ui.add(icons::lucide(icon, 22.0, c));
                        ui.add(egui::Label::new(text.as_str()).wrap());
                    });
                });
                o.close = f != Footer::None;
            }
            Modal::Confirm {
                title,
                text,
                action,
            } => {
                let t = self.t;
                let f = modal(
                    ctx,
                    &id,
                    title,
                    vec2(460.0, 0.0),
                    Some("OK"),
                    Some("Cancel"),
                    |ui| {
                        ui.horizontal_top(|ui| {
                            ui.add(icons::lucide("triangle-alert", 22.0, t.warning_fg));
                            ui.add(egui::Label::new(text.as_str()).wrap());
                        });
                    },
                );
                if f == Footer::Ok {
                    o.actions.push((**action).clone());
                }
                o.close = f != Footer::None;
            }
            Modal::Remove {
                id: vm,
                name,
                delete_files,
            } => {
                let f = modal(
                    ctx,
                    &id,
                    "Remove VM",
                    vec2(460.0, 0.0),
                    Some("Remove"),
                    Some("Cancel"),
                    |ui| {
                        ui.label(format!("Remove {name} from the list?"));
                        ui.checkbox(
                            delete_files,
                            "Also delete its folder (disk, firmware variables and boot files)",
                        );
                        if *delete_files {
                            warning(ui, "The VM's disk is deleted: this cannot be undone.");
                        }
                    },
                );
                if f == Footer::Ok {
                    match self.backend.remove_vm(vm, *delete_files) {
                        Ok(()) => o.actions.push(Action::Recheck),
                        Err(e) => o.modals.push(Modal::error("Remove VM", &e)),
                    }
                    self.cur = None;
                    self.reload_vms();
                }
                o.close = f != Footer::None;
            }
            Modal::Settings(s) => {
                let title = format!("{} - Settings", s.name);
                let f = modal(
                    ctx,
                    &id,
                    &title,
                    vec2(880.0, 560.0),
                    Some("OK"),
                    Some("Cancel"),
                    |ui| s.ui(ui),
                );
                if std::mem::take(&mut s.remove_media) {
                    // As Install step 5, with its confirmation (the Go app's Storage "Remove…").
                    o.close = true;
                    o.modals.push(Modal::confirm(
                        "Remove install media",
                        "Confirm that macOS already boots from the installed disk. BaseSystem.img will be deleted.",
                        Action::Run(Task::RemoveMedia),
                    ));
                } else if f == Footer::Ok {
                    match s
                        .collect()
                        .and_then(|c| self.backend.save_config(&s.id, &c))
                    {
                        Ok(()) => {
                            o.close = true;
                            self.reload_vms();
                        }
                        Err(e) => o.modals.push(Modal::error("Settings", &e)),
                    }
                } else {
                    o.close = f == Footer::Cancel;
                }
            }
            Modal::NewVm(w) => {
                let names: Vec<String> = self
                    .vms
                    .iter()
                    .map(|v| v.config.name.to_lowercase())
                    .collect();
                let f = modal(
                    ctx,
                    &id,
                    "New Virtual Machine",
                    vec2(860.0, 600.0),
                    Some("Finish"),
                    Some("Cancel"),
                    |ui| w.ui(ui),
                );
                if f == Footer::Ok {
                    match w.collect(&names).and_then(|(c, disk)| {
                        self.backend
                            .create_vm(&c)
                            .map(|vm| (vm, disk))
                            .map_err(|e| (0, e))
                    }) {
                        Ok((vm, disk_gib)) => {
                            self.backend.select(&vm);
                            self.cur = Some(vm.clone());
                            self.reload_vms();
                            o.actions.push(Action::Tab(Tab::Install));
                            o.actions.push(Action::Run(Task::CreateVmFiles {
                                disk_gib,
                                download: w.download,
                            }));
                            o.close = true;
                        }
                        Err((section, e)) => {
                            w.open = section;
                            o.modals.push(Modal::error("New VM", &e));
                        }
                    }
                } else {
                    o.close = f == Footer::Cancel;
                }
            }
            Modal::Preferences(p) => {
                let file = self.backend.settings_file();
                let mut open_file = false;
                let mut reset = false;
                let f = modal(
                    ctx,
                    &id,
                    "Preferences",
                    vec2(640.0, 0.0),
                    Some("OK"),
                    Some("Cancel"),
                    |ui| {
                        (open_file, reset) = p.ui(ui, &file);
                    },
                );
                if open_file {
                    self.backend.open_settings_file();
                }
                if reset {
                    o.modals.push(Modal::confirm(
                        "Reset settings.yaml",
                        &format!("Overwrite {file} with the default, which sets nothing?"),
                        Action::ResetSettings,
                    ));
                }
                if f == Footer::Ok {
                    match self.backend.set_workspace_root(&p.workspace) {
                        Ok(()) => {
                            self.backend.set_theme(p.theme);
                            self.mode = p.theme;
                            o.close = true;
                        }
                        Err(e) => o.modals.push(Modal::error("Preferences", &e)),
                    }
                } else {
                    o.close = f == Footer::Cancel;
                }
            }
            Modal::File(d) => o = self.file_dialog(ctx, &id, d, top),
        }
        o
    }
}

// ---- Settings ----

/// The machine Settings dialog's pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    General,
    System,
    Display,
    Storage,
    Audio,
    Network,
    Usb,
}

impl Page {
    pub const ALL: [(Self, &'static str, &'static str); 7] = [
        (Self::General, "General", "general"),
        (Self::System, "System", "system"),
        (Self::Display, "Display", "display"),
        (Self::Storage, "Storage", "storage"),
        (Self::Audio, "Audio", "audio"),
        (Self::Network, "Network", "network"),
        (Self::Usb, "USB", "usb"),
    ];
}

/// VirtualBox's machine Settings: nothing is saved until OK.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub id: VmId,
    pub name: String,
    pub c: Config,
    original_networks: Option<Vec<Nic>>,
    pub page: Page,
    system_tab: usize,
    adapter: usize,
    threads: u32,
    nics: Vec<Nic>,
    host: HostInfo,
    reims_installed: bool,
    /// Storage: the disk and the install media, as the Install tab shows them.
    pub disk: String,
    pub media: Option<String>,
    pub can_remove_media: bool,
    /// Storage "Remove…" was clicked.
    remove_media: bool,
}

const LABEL_W: f32 = 150.0;
const NIC_MODELS: [&str; 3] = ["virtio-net-pci", "vmxnet3", "e1000-82545em"];

/// VirtualBox-style names for the adapter types.
pub(crate) fn nic_label(model: &str) -> &str {
    match model {
        "virtio-net-pci" => "Paravirtualized Network (virtio-net)",
        "vmxnet3" => "VMware vmxnet3",
        "e1000-82545em" => "Intel PRO/1000 MT Desktop (82545EM)",
        m => m,
    }
}

fn version_index(short: &str) -> usize {
    VERSIONS.iter().position(|v| v.0 == short).unwrap_or(6)
}

fn default_nic_model(version: &str) -> &'static str {
    if version_index(version) >= 3 {
        "virtio-net-pci"
    } else {
        "vmxnet3"
    }
}

/// The Go app's `core.NICSupported`.
fn nic_supported(version: &str, model: &str) -> bool {
    let i = version_index(version);
    match model {
        "virtio-net-pci" => i >= 3,
        "e1000-82545em" => i < 5,
        m => m == "vmxnet3",
    }
}

fn wide(res: &str) -> bool {
    res.split_once('x')
        .and_then(|(w, _)| w.parse::<u32>().ok())
        .is_some_and(|w| w > 2048 && w < 3840)
}

impl Settings {
    pub fn new(v: &VmEntry, host: HostInfo, reims_installed: bool) -> Self {
        let mut nics = v.config.networks.clone().unwrap_or_else(|| {
            vec![Nic {
                enabled: true,
                mode: "nat".into(),
                model: default_nic_model(&v.config.version).into(),
                ..Nic::default()
            }]
        });
        nics.resize(
            4,
            Nic {
                mode: "nat".into(),
                model: default_nic_model(&v.config.version).into(),
                ..Nic::default()
            },
        );
        Self {
            id: v.id.clone(),
            name: v.display_name().into(),
            c: v.config.clone(),
            original_networks: v.config.networks.clone(),
            page: Page::General,
            system_tab: 0,
            adapter: 0,
            threads: (v.config.vcpus / v.config.cores.max(1)).clamp(1, 2),
            nics,
            host,
            reims_installed,
            disk: "not created yet".into(),
            media: None,
            can_remove_media: false,
            remove_media: false,
        }
    }

    /// The config to save, or what is wrong.
    pub fn collect(&self) -> Result<Config, String> {
        let mut n = self.c.clone();
        n.name = n.name.trim().to_owned();
        if n.name.is_empty() {
            return Err("Name: give the VM a name".into());
        }
        if !n.vcpus.is_multiple_of(self.threads) {
            return Err("Processors: with 2 threads per core, choose an even number".into());
        }
        n.cores = n.vcpus / self.threads;
        if !n.reims() {
            n.resolution.clear();
        }
        let used = self.original_networks.as_ref().map_or(1, Vec::len);
        let last = self
            .nics
            .iter()
            .enumerate()
            .filter(|(i, x)| x.enabled || *i < used)
            .map(|(i, _)| i + 1)
            .max()
            .unwrap_or(0);
        let nics: Vec<Nic> = self.nics[..last].to_vec();
        n.networks = match &self.original_networks {
            None if nics.len() == 1 && nics[0].enabled && nics[0].mac.is_empty() => None, // untouched legacy VM
            _ => Some(nics),
        };
        Ok(n)
    }

    fn ui(&mut self, ui: &mut Ui) {
        let t = widgets::tokens(ui);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(200.0, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (page, label, icon) in Page::ALL {
                    let on = self.page == page;
                    let (r, resp) = ui.allocate_exact_size(vec2(196.0, 32.0), egui::Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, label));
                    let hover = widgets::anim(ui, resp.id, resp.hovered() && !on);
                    if on {
                        ui.painter().rect_filled(r, 3, t.selection);
                    } else if hover > 0.0 {
                        ui.painter().rect_filled(r, 3, t.hover.gamma_multiply(hover));
                    }
                    egui::Image::new(icons::app(icon)).paint_at(ui, egui::Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(16.0, 16.0)));
                    ui.painter().text(r.left_center() + vec2(32.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), t.text);
                    if resp.clicked() {
                        self.page = page;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                let (_, label, icon) = Page::ALL.iter().copied().find(|p| p.0 == self.page).unwrap_or(Page::ALL[0]);
                section_band(ui, icons::app(icon), label);
                ui.add_space(8.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match self.page {
                    Page::General => self.general(ui),
                    Page::System => self.system(ui),
                    Page::Display => self.display(ui),
                    Page::Storage => {
                        form_row(ui, LABEL_W, "Controller", |ui| ui.label("SATA (ICH9 AHCI)"));
                        form_row(ui, LABEL_W, "SATA Port 0", |ui| ui.label(&self.disk));
                        form_row(ui, LABEL_W, "SATA Port 1", |ui| {
                            ui.label(self.media.as_deref().unwrap_or("Empty"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                self.remove_media = widgets::button(ui, "trash", "Remove…", false, self.can_remove_media && self.media.is_some())
                                    .on_hover_text("Delete the install media once macOS boots from its disk")
                                    .clicked();
                            });
                        });
                        form_row(ui, LABEL_W, "Boot Disk", |ui| ui.label("OpenCore (built in, read-only)"));
                        note(ui, "Remove the install media once macOS boots from its disk. Create the disk and download the media in the Install tab.");
                    }
                    Page::Audio => {
                        form_row(ui, LABEL_W, "Host Driver", |ui| ui.label("QEMU default"));
                        form_row(ui, LABEL_W, "Controller", |ui| ui.label(if self.c.reims() { "Intel HD Audio, USB Audio" } else { "Intel HD Audio" }));
                    }
                    Page::Network => self.network(ui),
                    Page::Usb => {
                        form_row(ui, LABEL_W, "USB Controller", |ui| ui.label("xHCI (USB 3.0)"));
                        form_row(ui, LABEL_W, "Devices", |ui| ui.label("USB keyboard and tablet (no host USB passthrough)"));
                    }
                });
            });
        });
    }

    fn general(&mut self, ui: &mut Ui) {
        form_row(ui, LABEL_W, "Name", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.c.name)
                    .desired_width(ui.available_width() - 8.0),
            )
        });
        form_row(ui, LABEL_W, "Operating System", |ui| {
            let opts: Vec<(String, String)> = VERSIONS
                .iter()
                .map(|v| (v.0.to_owned(), version_label(v.0)))
                .collect();
            let opts: Vec<(String, &str)> =
                opts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
            widgets::combo(
                ui,
                "settings-version",
                &mut self.c.version,
                &opts,
                ui.available_width() - 64.0,
            );
            ui.add(
                egui::Image::new(icons::version(&self.c.version))
                    .fit_to_exact_size(vec2(40.0, 40.0)),
            );
        });
        form_row(ui, LABEL_W, "Folder", |ui| ui.label(&self.c.workspace));
    }

    fn system(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            for (n, l) in ["Motherboard", "Processor", "Acceleration"]
                .iter()
                .enumerate()
            {
                if ui.selectable_label(self.system_tab == n, *l).clicked() {
                    self.system_tab = n;
                }
            }
        });
        ui.separator();
        let host_mem = self.host.mem_mib.max(config::MIN_RAM_MIB * 2);
        match self.system_tab {
            0 => {
                form_row(ui, LABEL_W, "Base Memory", |ui| {
                    slider_row(
                        ui,
                        &mut self.c.ram_mib,
                        config::MIN_RAM_MIB,
                        host_mem,
                        512,
                        "MB",
                    )
                });
                form_row(ui, LABEL_W, "Chipset", |ui| ui.label("Q35 (ICH9)"));
                form_row(ui, LABEL_W, "Firmware", |ui| {
                    ui.label("UEFI (OVMF) with OpenCore")
                });
            }
            1 => {
                form_row(ui, LABEL_W, "Processors", |ui| {
                    slider_row(ui, &mut self.c.vcpus, 1, self.host.cpus.max(1), 1, "CPUs")
                });
                form_row(ui, LABEL_W, "Threads per core", |ui| {
                    widgets::combo(
                        ui,
                        "threads",
                        &mut self.threads,
                        &[(1, "1"), (2, "2")],
                        80.0,
                    )
                });
                form_row(ui, LABEL_W, "CPU Model", |ui| {
                    ui.label("Skylake-Client (follows the macOS version)")
                });
                note(
                    ui,
                    "Without a supported GPU the CPU draws the screen: more processors make it smoother.",
                );
            }
            _ => {
                form_row(ui, LABEL_W, "Hypervisor", |ui| ui.label("KVM"));
                form_row(ui, LABEL_W, "Paging", |ui| {
                    ui.label("Nested paging (host CPU feature)")
                });
            }
        }
    }

    fn display(&mut self, ui: &mut Ui) {
        let reims = self.c.reims();
        form_row(ui, LABEL_W, "Shown", |ui| {
            ui.add_enabled_ui(!reims, |ui| {
                widgets::combo(
                    ui,
                    "shown",
                    &mut self.c.display,
                    &[
                        (String::new(), DISPLAY_APP),
                        (config::DISPLAY_WINDOW.to_owned(), "QEMU window"),
                    ],
                    320.0,
                );
            });
        });
        form_row(ui, LABEL_W, "Graphics Controller", |ui| {
            let mut g = reims;
            ui.add_enabled_ui(self.c.gpu.is_empty(), |ui| {
                if widgets::combo(
                    ui,
                    "graphics",
                    &mut g,
                    &[(false, "VMware SVGA"), (true, "Reims vGPU (experimental)")],
                    320.0,
                ) {
                    self.c.graphics = if g {
                        config::GRAPHICS_REIMS.into()
                    } else {
                        String::new()
                    };
                    if g {
                        self.c.display.clear();
                    }
                }
            });
        });
        form_row(ui, LABEL_W, "Resolution", |ui| {
            ui.add_enabled_ui(reims, |ui| {
                let mut r = self.c.reims_resolution().to_owned();
                let opts: Vec<(String, &str)> = config::RESOLUTIONS
                    .iter()
                    .map(|r| ((*r).to_owned(), *r))
                    .collect();
                if widgets::combo(ui, "resolution", &mut r, &opts, 320.0) {
                    self.c.resolution = r;
                }
            });
        });
        if reims && wide(self.c.reims_resolution()) && self.c.version != "tahoe" {
            warning(
                ui,
                "Between 2048 and 3840 px wide, WindowServer crashed at every login on macOS Sequoia (ReimsVgpu.kext, which fixes it, loads on macOS 26 only). 3840x2160 works there as a HiDPI desktop.",
            );
        }
        if !self.c.gpu.is_empty() {
            note(
                ui,
                "GPU passthrough is set for this VM: choose None (emulated display) in the GPU tab to use Reims vGPU.",
            );
        }
        if reims {
            reims_box(ui, self.reims_installed);
        }
    }

    fn network(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            for n in 0..4 {
                if ui
                    .selectable_label(self.adapter == n, format!("Adapter {}", n + 1))
                    .clicked()
                {
                    self.adapter = n;
                }
            }
        });
        ui.separator();
        let version = self.c.version.clone();
        let (bridges, helper) = (self.host.bridges.clone(), self.host.bridge_helper);
        let mac = format!(
            "52:54:00:{:02x}:{:02x}:{:02x}",
            self.adapter * 17 + 3,
            self.c.ssh_port % 256,
            self.adapter + 1
        );
        let Some(n) = self.nics.get_mut(self.adapter) else {
            return;
        };
        if ui
            .checkbox(&mut n.enabled, "Enable Network Adapter")
            .changed()
            && n.enabled
            && n.mac.is_empty()
        {
            n.mac = mac; // a new adapter gets its own address
        }
        ui.add_enabled_ui(n.enabled, |ui| {
            nic_form(ui, n, &bridges, &format!("nic{}", self.adapter))
        });
        let mut msg = Vec::new();
        if n.enabled && !nic_supported(&version, &n.model) {
            msg.push(format!("{} has no built-in driver for this adapter type; choose another unless the guest has one.", version_title(&version)));
        }
        if n.enabled && n.mode == "bridged" {
            if !helper {
                msg.push("qemu-bridge-helper is not installed.".into());
            }
            msg.push("Bridged adapters need qemu-bridge-helper to be setuid root and the bridge allowed in /etc/qemu/bridge.conf.".into());
        }
        if !msg.is_empty() {
            warning(ui, &msg.join(" "));
        }
        ui.add_space(6.0);
        form_row(ui, LABEL_W, "SSH port on 127.0.0.1", |ui| {
            ui.add(egui::DragValue::new(&mut self.c.ssh_port).range(1..=65535))
        });
        caption(
            ui,
            "The SSH port forward (127.0.0.1 to guest port 22) goes on the first enabled NAT adapter. Changing the first adapter's MAC or type can make macOS see a new network interface.",
        );
    }
}

const DISPLAY_APP: &str = "In the app (VNC on a private socket)";

/// Attached to, Name, Adapter Type and MAC Address of one adapter.
fn nic_form(ui: &mut Ui, n: &mut Nic, bridges: &[String], id: &str) {
    form_row(ui, LABEL_W, "Attached to", |ui| {
        widgets::combo(
            ui,
            &format!("{id}-mode"),
            &mut n.mode,
            &[
                ("nat".to_owned(), "NAT"),
                ("bridged".to_owned(), "Bridged Adapter"),
            ],
            320.0,
        );
    });
    form_row(ui, LABEL_W, "Name", |ui| {
        ui.add_enabled_ui(n.mode == "bridged", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut n.bridge)
                    .hint_text(bridges.first().map_or("br0", String::as_str))
                    .desired_width(320.0),
            );
        });
    });
    form_row(ui, LABEL_W, "Adapter Type", |ui| {
        let opts: Vec<(String, &str)> = NIC_MODELS
            .iter()
            .map(|m| ((*m).to_owned(), nic_label(m)))
            .collect();
        widgets::combo(ui, &format!("{id}-model"), &mut n.model, &opts, 320.0);
    });
    form_row(ui, LABEL_W, "MAC Address", |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut n.mac)
                .font(egui::TextStyle::Monospace)
                .desired_width(320.0),
        );
    });
}

/// METAL_ERROR_MODE 5, set once per macOS install: without it the desktop never appears on Reims vGPU.
const METAL_DESKTOP: &str =
    "sudo defaults write /Library/Preferences/.GlobalPreferences METAL_ERROR_MODE -int 5";
const METAL_RECOVERY: &str = "cd /Volumes/*Data/Library/Preferences\ndefaults write \"$PWD/.GlobalPreferences\" METAL_ERROR_MODE -int 5\nchmod 644 .GlobalPreferences.plist\nreboot";

/// The Reims vGPU guidance shown when it is the Graphics Controller.
fn reims_box(ui: &mut Ui, installed: bool) {
    note(
        ui,
        "Upstream measured macOS 26 (Tahoe) guests only; this project validated macOS Sequoia 15.8.1 (Apple Paravirtualized Graphics, Metal, 1920x1080). Other versions are untested. Switching back to VMware SVGA keeps the VM as it was.",
    );
    note(
        ui,
        "The display is drawn by Reims vGPU on the host's GPU and shown in the Screen tab; the Preview still updates.",
    );
    if !installed {
        warning(
            ui,
            "Reims vGPU is not installed on this host: install it or build it in the Host tab.",
        );
    }
    ui.add_space(4.0);
    ui.label(bold(
        "Set METAL_ERROR_MODE once per macOS install, or the desktop never appears",
    ));
    caption(ui, "On a working desktop (before switching), in Terminal:");
    widgets::command(ui, METAL_DESKTOP);
    caption(
        ui,
        "On a new install, from the recovery's Utilities › Terminal (after macOS is installed):",
    );
    widgets::command(ui, METAL_RECOVERY);
    note(
        ui,
        "Tahoe VMs load ReimsVgpu.kext (downloaded at the first start), which macOS 26 needs for displays wider than 2048 pixels.",
    );
}

// ---- New VM wizard ----

/// The New VM wizard in VirtualBox's expert style: four folding sections.
#[derive(Clone, Debug, PartialEq)]
pub struct Wizard {
    /// The open section (one at a time).
    pub open: usize,
    pub name: String,
    pub folder: String,
    folder_edited: bool,
    root: String,
    pub version: String,
    pub ram: u32,
    pub cpus: u32,
    pub window: bool,
    pub reims: bool,
    graphics_picked: bool,
    reims_ok: bool,
    pub nic: Nic,
    model_picked: bool,
    pub disk_gib: u32,
    pub download: bool,
    host: HostInfo,
}

impl Wizard {
    pub fn new(rec: Config, host: HostInfo, root: String, reims_ok: bool, mac: String) -> Self {
        let mut w = Self {
            open: 0,
            name: String::new(),
            folder: format!("{}/macOS", root.trim_end_matches('/')),
            folder_edited: false,
            root,
            version: "sonoma".into(),
            ram: rec.ram_mib.max(config::MIN_RAM_MIB),
            cpus: rec.vcpus.max(1),
            window: false,
            reims: false,
            graphics_picked: false,
            reims_ok,
            nic: Nic {
                enabled: true,
                mode: "nat".into(),
                model: String::new(),
                mac,
                ..Nic::default()
            },
            model_picked: false,
            disk_gib: 80,
            download: true,
            host,
        };
        w.on_version();
        w
    }

    /// New Sequoia and Tahoe VMs default to Reims vGPU when the host can run it.
    pub(crate) fn on_version(&mut self) {
        if !self.graphics_picked {
            self.reims = self.reims_ok && matches!(self.version.as_str(), "sequoia" | "tahoe");
        }
        if !self.model_picked {
            self.nic.model = default_nic_model(&self.version).into();
        }
    }

    /// The new VM and its disk size, or the section to open and what is wrong.
    pub fn collect(&self, existing: &[String]) -> Result<(Config, u32), (usize, String)> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err((0, "VM Name: give the VM a name".into()));
        }
        if existing.iter().any(|e| e == &name.to_lowercase()) {
            return Err((0, "VM Name: a VM with this name already exists".into()));
        }
        if self.folder.trim().is_empty() {
            return Err((0, "VM Folder: choose a folder".into()));
        }
        if self.nic.mode == "bridged" && self.nic.bridge.trim().is_empty() {
            return Err((2, "Network: name the host bridge (for example br0)".into()));
        }
        if self.disk_gib < 40 {
            return Err((3, "Disk Size: at least 40 GB".into()));
        }
        let cores = if self.cpus.is_multiple_of(2) && self.host.cpus > 1 {
            self.cpus / 2
        } else {
            self.cpus
        };
        let c = Config {
            name: name.into(),
            workspace: self.folder.trim().into(),
            version: self.version.clone(),
            ram_mib: self.ram,
            cores,
            vcpus: self.cpus,
            ssh_port: 2222,
            display: if self.window && !self.reims {
                config::DISPLAY_WINDOW.into()
            } else {
                String::new()
            },
            graphics: if self.reims {
                config::GRAPHICS_REIMS.into()
            } else {
                String::new()
            },
            networks: Some(vec![Nic {
                mac: self.nic.mac.trim().to_lowercase(),
                bridge: self.nic.bridge.trim().into(),
                ..self.nic.clone()
            }]),
            ..Config::default()
        };
        Ok((c, self.disk_gib))
    }

    fn ui(&mut self, ui: &mut Ui) {
        let t = widgets::tokens(ui);
        ui.horizontal_top(|ui| {
            let h = ui.available_height();
            let (r, _) = ui.allocate_exact_size(vec2(150.0, h), egui::Sense::hover());
            ui.painter().rect_filled(r, 4, t.header);
            egui::Image::new(icons::version(&self.version)).paint_at(
                ui,
                egui::Rect::from_center_size(r.center(), vec2(128.0, 128.0)),
            );
            ui.add_space(8.0);
            ui.vertical(|ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        let titles = [
                            "Virtual machine name and operating system",
                            "Specify virtual hardware",
                            "Specify network",
                            "Specify virtual hard disk",
                        ];
                        for (n, title) in titles.iter().enumerate() {
                            if n > 0 {
                                ui.separator();
                            }
                            let open = self.open == n;
                            let resp = ui.add(
                                egui::Button::image_and_text(
                                    icons::lucide(
                                        if open {
                                            "chevron-down"
                                        } else {
                                            "chevron-right"
                                        },
                                        14.0,
                                        t.text,
                                    ),
                                    bold(*title),
                                )
                                .frame(false)
                                .min_size(vec2(ui.available_width(), 30.0)),
                            );
                            if resp.clicked() {
                                self.open = n;
                            }
                            if open {
                                ui.indent(n, |ui| match n {
                                    0 => self.identity(ui),
                                    1 => self.hardware(ui),
                                    2 => self.network(ui),
                                    _ => self.disk(ui),
                                });
                            }
                        }
                    });
            });
        });
    }

    fn identity(&mut self, ui: &mut Ui) {
        form_row(ui, 110.0, "VM Name", |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .hint_text("for example macOS Tahoe lab")
                    .desired_width(ui.available_width() - 8.0),
            );
            if r.changed() && !self.folder_edited {
                let n = self.name.trim();
                self.folder = format!(
                    "{}/{}",
                    self.root.trim_end_matches('/'),
                    if n.is_empty() { "macOS" } else { n }
                );
            }
        });
        form_row(ui, 110.0, "VM Folder", |ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.folder)
                        .desired_width(ui.available_width() - 8.0),
                )
                .changed()
            {
                self.folder_edited = true;
            }
        });
        form_row(ui, 110.0, "OS Version", |ui| {
            let opts: Vec<(String, String)> = VERSIONS
                .iter()
                .map(|v| (v.0.to_owned(), version_label(v.0)))
                .collect();
            let opts: Vec<(String, &str)> =
                opts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
            if widgets::combo(
                ui,
                "wizard-version",
                &mut self.version,
                &opts,
                ui.available_width() - 52.0,
            ) {
                self.on_version();
            }
            ui.add(
                egui::Image::new(icons::version(&self.version)).fit_to_exact_size(vec2(36.0, 36.0)),
            );
        });
        note(
            ui,
            "Only Sonoma has been validated end to end. Newer versions need more memory and disk.",
        );
    }

    fn hardware(&mut self, ui: &mut Ui) {
        let mem = self.host.mem_mib.max(config::MIN_RAM_MIB * 2);
        form_row(ui, 130.0, "Base Memory", |ui| {
            slider_row(ui, &mut self.ram, config::MIN_RAM_MIB, mem, 512, "MB")
        });
        form_row(ui, 130.0, "Number of CPUs", |ui| {
            slider_row(ui, &mut self.cpus, 1, self.host.cpus.max(1), 1, "CPUs")
        });
        form_row(ui, 130.0, "Display", |ui| {
            ui.add_enabled_ui(!self.reims, |ui| {
                widgets::combo(
                    ui,
                    "wizard-display",
                    &mut self.window,
                    &[(false, DISPLAY_APP), (true, "QEMU window")],
                    320.0,
                )
            });
        });
        form_row(ui, 130.0, "Graphics", |ui| {
            if widgets::combo(
                ui,
                "wizard-graphics",
                &mut self.reims,
                &[(false, "VMware SVGA"), (true, "Reims vGPU (experimental)")],
                320.0,
            ) {
                self.graphics_picked = true;
            }
        });
        if self.reims {
            reims_box(ui, self.reims_ok);
        }
        note(
            ui,
            &format!(
                "Recommended for this host: {} MB and {} CPUs. Without a GPU for macOS the CPU draws the screen, so more CPUs make it smoother.",
                self.ram, self.cpus
            ),
        );
    }

    fn network(&mut self, ui: &mut Ui) {
        let before = self.nic.model.clone();
        nic_form(ui, &mut self.nic, &self.host.bridges, "wizard-nic");
        if self.nic.model != before {
            self.model_picked = true;
        }
        if !nic_supported(&self.version, &self.nic.model) {
            warning(
                ui,
                &format!(
                    "{} has no built-in driver for this adapter type.",
                    version_title(&self.version)
                ),
            );
        }
        note(
            ui,
            "This is Adapter 1. Add more adapters later in Settings › Network. NAT also forwards SSH from 127.0.0.1.",
        );
    }

    fn disk(&mut self, ui: &mut Ui) {
        form_row(ui, 110.0, "Location", |ui| {
            ui.label(format!("{}/mac_hdd_ng.img", self.folder.trim()))
        });
        form_row(ui, 110.0, "Disk Size", |ui| {
            slider_row(ui, &mut self.disk_gib, 40, 2048, 8, "GB")
        });
        form_row(ui, 110.0, "Format", |ui| {
            ui.label("QCOW2 (QEMU Copy-On-Write), dynamically allocated")
        });
        ui.checkbox(
            &mut self.download,
            "Download the macOS recovery from Apple now",
        );
    }
}

// ---- Preferences ----

/// App-wide settings: the Workspace and the theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub workspace: String,
    pub theme: ThemeMode,
}

impl Prefs {
    /// Returns (open settings.yaml, reset it) clicks.
    fn ui(&mut self, ui: &mut Ui, file: &str) -> (bool, bool) {
        section_band(ui, icons::app("general"), "General");
        ui.add_space(4.0);
        form_row(ui, 100.0, "Workspace", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.workspace)
                    .desired_width(ui.available_width() - 8.0),
            )
        });
        note(
            ui,
            "New and imported VMs get their own folder inside the Workspace (<Workspace>/<name>). Existing VMs stay where they are; use Machine › Move to move one.",
        );
        ui.add_space(6.0);
        section_band(ui, icons::app("display"), "Interface");
        ui.add_space(4.0);
        form_row(ui, 100.0, "Theme", |ui| {
            widgets::combo(ui, "theme", &mut self.theme, &ThemeMode::ALL, 200.0)
        });
        note(
            ui,
            "System follows the desktop's light or dark setting. theme in settings.yaml (or MKG_THEME) overrides it.",
        );
        ui.add_space(6.0);
        section_band(ui, icons::app("general"), "Settings file");
        ui.add_space(4.0);
        let (mut open, mut reset) = (false, false);
        ui.horizontal(|ui| {
            open = widgets::button(ui, "folder-open", "Open settings.yaml", false, true)
                .on_hover_text(file)
                .clicked();
            reset = widgets::button(ui, "rotate-cw", "Reset to default", false, true).clicked();
        });
        ui.label(RichText::new(file).monospace().size(11.5));
        note(
            ui,
            "settings.yaml holds the theme, the boot files override and the extra kexts OpenCore loads in every VM. Environment variables win over it.",
        );
        (open, reset)
    }
}
