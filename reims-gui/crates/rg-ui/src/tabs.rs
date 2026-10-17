//! The tab bodies: Details (sections and the Preview), Screen (the embedding area), Install,
//! Host (checks and Reims vGPU), GPU and Log (the Go app's `layout.go`, `install.go`, `gui.go`).

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, ScrollArea, Sense,
    Ui, vec2,
};

use crate::app::{Action, App, Tab, smbios_model, version_title};
use crate::backend::{Check, Status, Task, VmState};
use crate::dialogs::{Modal, nic_label};
use crate::icons;
use crate::widgets::{self, Badge, bold, caption, item_card, kv_grid, note, section_band, surface};

impl App {
    /// The selected tab's body; returns the Screen tab's rectangle when it is shown.
    pub(crate) fn tab_body(&mut self, ui: &mut Ui, out: &mut Vec<Action>) -> Option<Rect> {
        if self.current().is_none() && !matches!(self.tab, Tab::Host | Tab::Log) {
            empty_state(
                ui,
                "monitor",
                "No VM selected",
                "Create a VM with New, or pick one in the list.",
            );
            return None;
        }
        match self.tab {
            Tab::Details => self.details(ui),
            Tab::Screen => return Some(self.screen(ui)),
            Tab::Install => self.install(ui, out),
            Tab::Host => self.host(ui, out),
            Tab::Gpu => self.gpu(ui, out),
            Tab::Log => self.log_tab(ui),
        }
        None
    }

    fn details(&mut self, ui: &mut Ui) {
        let Some(v) = self.current().cloned() else {
            return;
        };
        let c = &v.config;
        let disk = self
            .cache
            .install
            .disk
            .clone()
            .unwrap_or_else(|| "mac_hdd_ng.img (not created yet)".into());
        let media = self
            .cache
            .install
            .recovery
            .clone()
            .unwrap_or_else(|| "Empty".into());
        let disc = match (&self.cache.install.recovery, &self.cache.install.installer) {
            (Some(_), Some(i)) => {
                format!("InstallAssistant.iso (offline installer, {i}, read-only)")
            }
            _ => "Empty".into(),
        };
        let mut os = version_title(&c.version);
        if c.version != "sonoma" {
            os += " (experimental)";
        }
        let (shown, ctrl, remote) = if c.reims() {
            (
                "Screen tab (Reims vGPU window, embedded)",
                "Reims vGPU (apple-gfx-pci)",
                "VNC on a private socket (Preview only)",
            )
        } else if c.in_app() {
            ("In the app", "VMware SVGA", "VNC on a private socket")
        } else {
            ("QEMU window", "VMware SVGA", "Disabled")
        };
        let res = if c.reims() {
            c.reims_resolution().to_owned()
        } else {
            "1920x1080".into()
        };
        let adapters = c.networks.clone().unwrap_or_else(|| {
            vec![rg_core::Nic {
                enabled: true,
                mode: "nat".into(),
                model: "vmxnet3".into(),
                ..Default::default()
            }]
        });
        let adapter = |i: usize| {
            adapters
                .get(i)
                .filter(|n| n.enabled)
                .map(nic_summary)
                .unwrap_or_default()
        };
        let (gpu, accel) = if !c.gpu.is_empty() {
            (
                format!("{} (vfio-pci)", c.gpu),
                "Metal through the passed-through GPU",
            )
        } else if c.reims() {
            (
                "None".into(),
                "Metal through Reims vGPU (the host's Vulkan driver)",
            )
        } else {
            ("None".into(), "Disabled: the CPU draws the screen")
        };
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let left_w = ui.available_width() - 248.0;
                ui.allocate_ui_with_layout(vec2(left_w, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    section_band(ui, icons::app("general"), "General");
                    kv_grid(
                        ui,
                        "general",
                        &[
                            ("Name", v.display_name().to_owned()),
                            ("Operating System", os),
                            (
                                "Mac Model",
                                format!("{} (SMBIOS)", smbios_model(&c.version)),
                            ),
                        ],
                    );
                    section_band(ui, icons::app("system"), "System");
                    kv_grid(
                        ui,
                        "system",
                        &[
                            ("Base Memory", format!("{} MB", c.ram_mib)),
                            (
                                "Processors",
                                format!(
                                    "{} ({} cores × {} threads)",
                                    c.vcpus,
                                    c.cores,
                                    c.vcpus / c.cores.max(1)
                                ),
                            ),
                            ("CPU Model", "Skylake-Client".into()),
                            ("Firmware", "UEFI (OVMF) with OpenCore".into()),
                            ("Acceleration", "KVM, Q35 chipset".into()),
                        ],
                    );
                });
                ui.allocate_ui_with_layout(vec2(242.0, 0.0), Layout::top_down(Align::Min), |ui| {
                    section_band(ui, icons::app("preview"), "Preview");
                    ui.add_space(6.0);
                    self.preview_box(ui, v.display_name(), v.state);
                });
            });
            let rows: [Section; 6] = [
                (
                    "display",
                    "Display",
                    "display",
                    vec![
                        ("Shown", shown.into()),
                        ("Graphics Controller", ctrl.into()),
                        ("Resolution", res),
                        ("Remote Display", remote.into()),
                    ],
                ),
                (
                    "storage",
                    "Storage",
                    "storage",
                    vec![
                        ("Controller", "SATA (ICH9 AHCI)".into()),
                        ("SATA Port 0", disk),
                        ("SATA Port 1", media),
                        ("SATA Port 2", disc),
                        ("Boot Disk", "OpenCore (built in, read-only)".into()),
                    ],
                ),
                (
                    "audio",
                    "Audio",
                    "audio",
                    vec![(
                        "Controller",
                        if c.reims() {
                            "Intel HD Audio, USB Audio"
                        } else {
                            "Intel HD Audio"
                        }
                        .into(),
                    )],
                ),
                (
                    "network",
                    "Network",
                    "network",
                    vec![
                        ("Adapter 1", adapter(0)),
                        ("Adapter 2", adapter(1)),
                        ("Adapter 3", adapter(2)),
                        ("Adapter 4", adapter(3)),
                        (
                            "Port Forwarding",
                            format!("SSH 127.0.0.1:{} to guest port 22", c.ssh_port),
                        ),
                    ],
                ),
                (
                    "usb",
                    "USB",
                    "usb",
                    vec![
                        ("Controller", "xHCI (USB 3.0)".into()),
                        ("Devices", "Keyboard, Tablet".into()),
                    ],
                ),
                (
                    "gpu",
                    "GPU Passthrough",
                    "gpu",
                    vec![("GPU", gpu), ("3D Acceleration", accel.into())],
                ),
            ];
            for (id, title, icon, kv) in &rows {
                section_band(ui, icons::app(icon), title);
                kv_grid(ui, id, kv);
            }
        });
    }

    /// A rounded 16:9 screen with a soft inner shadow: the live picture, or the VM's name.
    fn preview_box(&self, ui: &mut Ui, name: &str, state: VmState) {
        let t = self.t;
        let (outer, resp) = ui.allocate_exact_size(vec2(236.0, 138.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 8, t.frame);
        let screen = outer.shrink(6.0);
        let radius = CornerRadius::same(5);
        p.rect_filled(screen, radius, Color32::BLACK);
        match (&self.preview, state) {
            (Some(tex), VmState::Running) => {
                let size = tex.size_vec2();
                let scale = (screen.width() / size.x).min(screen.height() / size.y);
                let r = Rect::from_center_size(screen.center(), size * scale);
                egui::Image::new((tex.id(), r.size()))
                    .corner_radius(radius)
                    .paint_at(ui, r);
            }
            (None, VmState::Running) => {
                egui::Spinner::new()
                    .size(20.0)
                    .color(Color32::from_gray(200))
                    .paint_at(
                        ui,
                        Rect::from_center_size(screen.center(), vec2(20.0, 20.0)),
                    );
            }
            _ => {
                p.text(
                    screen.center(),
                    Align2::CENTER_CENTER,
                    name,
                    widgets::bold_font(17.0),
                    Color32::WHITE,
                );
            }
        }
        // Inner shadow: darker edges fading inward.
        for i in 0..4 {
            let a = 60 - i * 14;
            p.rect_stroke(
                screen.shrink(i as f32),
                radius,
                egui::Stroke::new(1.0, Color32::from_black_alpha(a as u8)),
                egui::StrokeKind::Inside,
            );
        }
        resp.on_hover_text(if state == VmState::Running {
            "Live preview of the VM's screen"
        } else {
            "The VM is powered off"
        });
    }

    /// The Screen tab: the area QEMU's window is embedded into (design D3).
    fn screen(&mut self, ui: &mut Ui) -> Rect {
        self.screen_tab(ui)
    }

    fn install(&mut self, ui: &mut Ui, out: &mut Vec<Action>) {
        let t = self.t;
        let s = self.cache.install.clone();
        let free = self.busy.is_none() && self.state() == VmState::Stopped;
        let done = s.done();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            section_band(ui, icons::app("storage"), "Installation");
            Frame::NONE.inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.label(bold(format!("{done} of 5 steps done")));
                widgets::progress_line(ui, egui::Id::new("install-progress"), done as f32 / 5.0);
                if let Some((stage, frac)) = &self.progress {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(stage);
                        if let Some(f) = frac {
                            ui.label(RichText::new(format!("{:.0}%", f * 100.0)).color(t.text_muted));
                        }
                    });
                }
                caption(ui, &format!("Boot files: {}", s.boot_files));
            });
            let step = |n: usize, done: bool, next: bool| -> (Badge<'static>, Color32, (&'static str, Color32), bool) {
                let num = ["1", "2", "3", "4", "5"][n];
                if done {
                    (Badge::Icon("check"), t.success, ("Done", t.success_fg), false)
                } else if next {
                    (Badge::Number(num), t.accent, ("Next", t.accent_fg), true)
                } else {
                    (Badge::Number(num), t.neutral, ("Waiting", t.text_muted), false)
                }
            };
            let first_missing = [s.folder.is_some(), s.recovery.is_some(), s.disk.is_some()].iter().position(|d| !d).unwrap_or(3);
            let (b, c, st, acc) = step(0, s.folder.is_some(), first_missing == 0);
            item_card(ui, b, c, "VM folder", "Creates the VM's folder and extracts the built-in OpenCore and OVMF.", st, s.folder.as_deref().unwrap_or(""), acc, |ui| {
                if s.folder.is_none() {
                    right(ui, |ui| {
                        if widgets::button(ui, "folder-open", "Create VM folder", true, free).clicked() {
                            out.push(Action::Run(Task::PrepareFolder));
                        }
                    });
                }
            });
            let (b, c, st, acc) = step(1, s.recovery.is_some(), first_missing == 1);
            item_card(ui, b, c, "macOS recovery", "Downloaded from Apple; the signature and every chunk's SHA-256 are verified. Optionally also download the full installer (12–18 GB) once for every VM of this version.", st, s.recovery.as_deref().unwrap_or("Not downloaded yet"), acc, |ui| {
                if let Some(i) = &s.installer {
                    caption(ui, &format!("Full installer: {i}"));
                }
                right(ui, |ui| {
                    if s.recovery.is_none() && widgets::button(ui, "download", "Download macOS recovery", true, free).clicked() {
                        out.push(Action::Run(Task::DownloadRecovery));
                    }
                    if s.installer.is_none() {
                        if widgets::button(ui, "download", "Download full installer", false, free).clicked() {
                            out.push(Action::Run(Task::DownloadInstaller));
                        }
                    } else if widgets::button(ui, "trash", "Delete installer", false, free).clicked() {
                        out.push(Action::Modal(Modal::confirm("Delete full installer", "Delete the cached installer? Every VM of this version uses it.", Action::Run(Task::DeleteInstaller))));
                    }
                });
            });
            let (b, c, st, acc) = step(2, s.disk.is_some(), first_missing == 2);
            item_card(ui, b, c, "Virtual hard disk", "A qcow2 disk that only takes the space macOS uses.", st, s.disk.as_deref().unwrap_or(""), acc, |ui| {
                if s.disk.is_none() {
                    right(ui, |ui| {
                        if widgets::button(ui, "folder-open", "Create disk (80 GB)", true, free).clicked() {
                            out.push(Action::Run(Task::CreateDisk { gib: 80 }));
                        }
                    });
                }
            });
            let ready = first_missing == 3;
            let (b, c, st, acc) = step(3, false, ready);
            item_card(ui, b, c, "Install macOS", "Start the VM, erase the disk in Disk Utility and run Reinstall macOS, or install from the full installer.", st, "", acc, |ui| {
                ui.collapsing("Show the steps inside macOS", |ui| note(ui, MANUAL_STEPS));
                caption(ui, "Stuck at the Apple logo? Start with verbose boot shows macOS's boot log instead of the logo, for that start only.");
                right(ui, |ui| {
                    let en = self.enabled();
                    if widgets::button(ui, "play", "Start the VM", true, en.start).clicked() {
                        out.push(Action::Start { verbose: false });
                    }
                    if widgets::button(ui, "list", "Start with verbose boot", false, en.start).clicked() {
                        out.push(Action::Start { verbose: true });
                    }
                });
            });
            let (b, c, st, acc) = step(4, false, false);
            item_card(ui, b, c, "Clean up", "Once macOS boots from its disk, remove the install media to free space.", st, "", acc, |ui| {
                right(ui, |ui| {
                    if widgets::button(ui, "trash", "Remove install media", false, free && s.recovery.is_some()).clicked() {
                        out.push(Action::Modal(Modal::confirm("Remove install media", "Confirm that macOS already boots from the installed disk. BaseSystem.img will be deleted.", Action::Run(Task::RemoveMedia))));
                    }
                });
            });
        });
    }

    fn host(&mut self, ui: &mut Ui, out: &mut Vec<Action>) {
        let t = self.t;
        let checks = self.cache.host.clone();
        let reims = self.cache.reims.clone();
        let reims_vm = self.current().is_some_and(|v| v.config.reims());
        let name = self
            .current()
            .map(|v| v.display_name().to_owned())
            .unwrap_or_default();
        let free = self.busy.is_none();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            section_band(ui, icons::app("system"), "Host checks");
            let ok = checks.iter().filter(|c| c.status == Status::Ok).count();
            Frame::NONE.inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let summary = if ok == checks.len() { format!("All {ok} checks passed: the host is ready") } else { format!("{ok} of {} checks passed", checks.len()) };
                    let color = if ok == checks.len() { t.success_fg } else { t.warning_fg };
                    ui.add(icons::lucide(if ok == checks.len() { "check" } else { "triangle-alert" }, 16.0, color));
                    ui.label(bold(summary));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button(ui, "settings", "Set up host", true, free).on_hover_text("Install QEMU, load KVM and set the kvm group (asks for your password once)").clicked() {
                            out.push(Action::Run(Task::SetupHost));
                        }
                        if widgets::button(ui, "rotate-cw", "Check again", false, true).on_hover_text("Run the checks again").clicked() {
                            out.push(Action::Recheck);
                        }
                    });
                });
                caption(ui, "Set up host asks for your password once, then installs missing QEMU packages, loads the KVM module, adds you to the kvm group and enables ignore_msrs. The libvirt and input groups are not needed.");
            });
            for c in &checks {
                check_card(ui, c);
            }
            ui.add_space(6.0);
            section_band(ui, icons::lucide("gpu-tab", 16.0, t.accent), "Reims vGPU (experimental)");
            let rok = reims.iter().filter(|c| c.status != Status::Fail).count();
            Frame::NONE.inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let summary = if rok == reims.len() {
                        "Ready for Reims vGPU VMs".to_owned()
                    } else if reims_vm {
                        format!("{rok} of {} checks passed: {name} cannot start yet", reims.len())
                    } else {
                        format!("{rok} of {} checks passed (only needed for Reims vGPU VMs)", reims.len())
                    };
                    ui.label(bold(summary));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button(ui, "monitor", "Build from source (Docker)", false, free).on_hover_text("Build Reims vGPU's QEMU in a Docker container (10–20 minutes)").clicked() {
                            out.push(Action::Modal(Modal::confirm(
                                "Build Reims vGPU",
                                "Build Reims vGPU's QEMU from source in a Docker container for this host's Ubuntu release? It downloads Rust, QEMU and SDL3 sources and takes about 10 to 20 minutes the first time.",
                                Action::Run(Task::BuildReims),
                            )));
                        }
                        let label = if self.cache.reims_status.installed { "Reinstall" } else { "Install Reims vGPU" };
                        if widgets::button(ui, "download", label, !self.cache.reims_status.installed, free).on_hover_text("Install the prebuilt package for this Ubuntu release").clicked() {
                            out.push(Action::Run(Task::InstallReims));
                        }
                    });
                });
                let info = if self.cache.reims_status.installed { self.cache.reims_status.info.as_str() } else { "Not installed." };
                ui.label(RichText::new(info).monospace().size(11.5));
                caption(ui, "Reims vGPU is an experimental paravirtual GPU: Metal in macOS through the host's Vulkan driver, without passing a GPU through. It runs on its own QEMU build, installed in <Workspace>/.reims from a prebuilt package (Ubuntu 24.04 and 26.04) or built in Docker. Choose it per VM in Settings › Display.");
            });
            for c in &reims {
                check_card(ui, c);
            }
            surface(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.add(icons::lucide("info", 16.0, t.warning_fg));
                    ui.add(egui::Label::new(RichText::new(crate::app::HOME_LAB).color(t.warning_fg)).wrap());
                });
            });
        });
    }

    fn gpu(&mut self, ui: &mut Ui, out: &mut Vec<Action>) {
        let t = self.t;
        let Some(v) = self.current().cloned() else {
            return;
        };
        let running = v.state != VmState::Stopped;
        let gpus = self.cache.gpus.clone();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            section_band(ui, icons::lucide("gpu-tab", 16.0, t.accent), "GPU passthrough");
            Frame::NONE.inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("GPU for the VM");
                    let mut choice = v.config.gpu.clone();
                    let mut opts: Vec<(String, String)> = vec![(String::new(), NO_GPU.into())];
                    opts.extend(gpus.iter().filter(|g| g.eligible || g.addr == v.config.gpu).map(|g| (g.addr.clone(), format!("{} {}", g.addr, g.name))));
                    let opts_ref: Vec<(String, &str)> = opts.iter().map(|(a, l)| (a.clone(), l.as_str())).collect();
                    let enabled = !running && !v.config.reims();
                    ui.add_enabled_ui(enabled, |ui| {
                        if widgets::combo(ui, "gpu-choice", &mut choice, &opts_ref, ui.available_width() - 8.0) {
                            let mut c = v.config.clone();
                            c.gpu = choice;
                            if let Err(e) = self.backend.save_config(&v.id, &c) {
                                out.push(Action::Modal(Modal::error("GPU", &e)));
                            }
                            self.vms = self.backend.vms();
                        }
                    });
                });
                ui.horizontal(|ui| {
                    let mut status = if self.cache.reserved.is_empty() { "No GPU is reserved for VFIO.".to_owned() } else { format!("Reserved for vfio-pci: {}. If you just changed this, reboot the host.", self.cache.reserved) };
                    if v.config.reims() {
                        status = format!("This VM uses Reims vGPU (Settings › Display): choose VMware SVGA there to pass a GPU through. {status}");
                    }
                    let w = ui.available_width() - 330.0;
                    ui.allocate_ui_with_layout(vec2(w.max(100.0), 20.0), Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(status).wrap());
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button(ui, "x", "Release reservation", false, !running && !self.cache.reserved.is_empty()).clicked() {
                            out.push(Action::Run(Task::ReleaseGpu));
                        }
                        let eligible = gpus.iter().any(|g| g.addr == v.config.gpu && g.eligible);
                        if widgets::button(ui, "check", "Reserve for VFIO", false, !running && eligible).clicked() {
                            out.push(Action::Modal(Modal::confirm("Reserve GPU", "vfio-pci will claim the GPU at the next boot, so the host can no longer use it. A host reboot is required. Continue?", Action::Run(Task::ReserveGpu { addr: v.config.gpu.clone() }))));
                        }
                    });
                });
                caption(ui, "macOS-supported AMD GPUs and NVIDIA Kepler GPUs can be passed to the VM. Experimental: untested on real hardware.");
            });
            if gpus.is_empty() {
                surface(ui, |ui| empty_state(ui, "monitor", "No PCI display adapter found", ""));
            }
            for g in &gpus {
                let desc = format!("{} · {} · {} · driver {} · IOMMU group {}", g.addr, g.id, g.compat, g.driver, g.group);
                if g.eligible {
                    let detail = if g.kepler { "NVIDIA Kepler: native up to Big Sur; Monterey to Sequoia with root-patch mode below" } else { "Can be passed to the VM" };
                    item_card(ui, Badge::Icon("check"), t.success, &g.name, &desc, ("Eligible", t.success_fg), detail, g.addr == v.config.gpu, |_| {});
                } else {
                    item_card(ui, Badge::Icon("x"), t.neutral, &g.name, &desc, ("Not eligible", t.text_muted), &g.reason, false, |_| {});
                }
            }
            surface(ui, |ui| {
                ui.label(bold("Root-patch mode (experimental)"));
                let available = matches!(v.config.version.as_str(), "monterey" | "ventura" | "sonoma" | "sequoia");
                let mut on = v.config.root_patch;
                ui.add_enabled_ui(available && !running && self.busy.is_none(), |ui| {
                    if ui.checkbox(&mut on, "Root-patch mode for an NVIDIA Kepler GPU (OpenCore Legacy Patcher)").changed() {
                        let mut c = v.config.clone();
                        c.root_patch = on;
                        if let Err(e) = self.backend.save_config(&v.id, &c) {
                            out.push(Action::Modal(Modal::error("Root-patch mode", &e)));
                        }
                        self.vms = self.backend.vms();
                    }
                });
                caption(ui, if !available && !v.config.root_patch {
                    "Not needed or not available for this macOS version: Big Sur and older drive Kepler GPUs natively, and Tahoe has no stable OCLP support yet."
                } else if v.config.root_patch {
                    "On for this VM: SIP 0x803, AMFIPass, RSRHelper, RestrictEvents, KDKlessWorkaround. Turning it off restores full SIP at the next start."
                } else {
                    "Off. For Monterey to Sequoia with a passed-through Kepler GPU; other VMs do not need it."
                });
                ui.collapsing("Root-patch steps inside macOS", |ui| note(ui, ROOT_PATCH_STEPS));
            });
            surface(ui, |ui| {
                ui.collapsing("About GPU passthrough", |ui| note(ui, GPU_NOTES));
            });
        });
    }

    fn log_tab(&mut self, ui: &mut Ui) {
        let t = self.t;
        section_band(
            ui,
            icons::lucide("list", 16.0, t.accent),
            "QEMU and app log",
        );
        caption(ui, "Last 1000 lines");
        if self.log.is_empty() {
            empty_state(
                ui,
                "list",
                "Nothing logged yet",
                "QEMU's output and the app's tasks appear here.",
            );
            return;
        }
        Frame::new()
            .fill(t.input)
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ScrollArea::both()
                    .auto_shrink(false)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.log {
                            let color = if line.starts_with("error") {
                                t.error_fg
                            } else if line.starts_with("ok:") {
                                t.success_fg
                            } else {
                                t.text
                            };
                            ui.label(RichText::new(line).monospace().color(color));
                        }
                    });
            });
    }
}

/// (grid id, title, icon, rows).
type Section = (
    &'static str,
    &'static str,
    &'static str,
    Vec<(&'static str, String)>,
);

/// Right-aligned row of buttons.
fn right(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 32.0),
        Layout::right_to_left(Align::Center),
        add,
    );
}

/// A centered icon, title and hint for empty lists and tabs.
pub(crate) fn empty_state(ui: &mut Ui, icon: &str, title: &str, hint: &str) {
    let t = widgets::tokens(ui);
    ui.vertical_centered(|ui| {
        ui.add_space(48.0);
        ui.add(icons::lucide(icon, 40.0, t.text_muted));
        ui.add_space(8.0);
        ui.label(bold(title));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).color(t.text_muted));
        }
    });
}

fn check_card(ui: &mut Ui, c: &Check) {
    let t = widgets::tokens(ui);
    let (icon, badge, fg) = match c.status {
        Status::Ok => ("check", t.success, t.success_fg),
        Status::Warn => ("triangle-alert", t.warning, t.warning_fg),
        Status::Fail => ("circle-x", t.error, t.error_fg),
        Status::Unknown => ("info", t.neutral, t.text_muted),
    };
    item_card(
        ui,
        Badge::Icon(icon),
        badge,
        &c.name,
        &c.detail,
        (c.status.text(), fg),
        &c.hint,
        c.status == Status::Fail,
        |_| {},
    );
}

/// A Details line for an adapter, like VirtualBox's "Intel PRO/1000 MT Desktop (NAT)".
fn nic_summary(n: &rg_core::Nic) -> String {
    if n.mode == "bridged" {
        format!("{} (Bridged Adapter, {})", nic_label(&n.model), n.bridge)
    } else {
        format!("{} (NAT)", nic_label(&n.model))
    }
}

pub(crate) const NO_GPU: &str = "None (emulated display)";

const MANUAL_STEPS: &str = "1. In the OpenCore menu, macOS Base System is selected and starts by itself after five seconds.\n2. Open Disk Utility, enable View › Show All Devices, select the QEMU HARDDISK disk and Erase it (APFS, GUID).\n3. Close Disk Utility and choose Reinstall macOS. The VM restarts a few times: in OpenCore pick macOS Installer, then the installed disk.\n4. Once macOS boots from the disk, remove the install media here to free space.";

const ROOT_PATCH_STEPS: &str = "1. Turn FileVault off, and keep a backup of the VM's folder.\n2. Download OCLP and open it. Never use \"Build and Install OpenCore\".\n3. Click Post-Install Root Patch, then Start Root Patching.\n4. Restart. Metal should now use the Kepler card.\n5. Every macOS update removes the patch: update on purpose, then patch again.\n6. To stop using it: Revert Root Patches in OCLP, restart, then turn the mode off here.";

const GPU_NOTES: &str = "Gives the VM a real GPU, the only way to get Metal without Reims vGPU. The GPU's display output then goes to a monitor plugged into that card.\n\n• Supported: RX 460–590, Vega 56/64, Radeon VII, RX 5500/5600/5700, RX 6600/6800/6900.\n• NVIDIA Kepler: native up to Big Sur; Monterey to Sequoia with root-patch mode.\n• Not supported: RX 7000, RX 6400/6500, NVIDIA Maxwell and newer, Intel and AMD iGPUs.\n• Reserve writes the VFIO settings and needs a host reboot; Release undoes it.";
