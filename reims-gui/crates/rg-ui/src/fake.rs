//! A [`Backend`] with sample data, for tests and `--demo`: a Sequoia VM "Nilton" on Reims vGPU,
//! a Sonoma VM on VMware SVGA, a host that passes its checks and Reims vGPU installed.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use egui::{Color32, ColorImage};
use rg_core::Nic;
use rg_core::config::{self, Config};

use crate::backend::*;
use crate::theme::ThemeMode;

pub struct FakeBackend {
    vms: RefCell<Vec<VmEntry>>,
    selected: RefCell<Option<VmId>>,
    events: RefCell<VecDeque<Event>>,
    theme: Cell<ThemeMode>,
    workspace: RefCell<String>,
    /// The Screen area the UI reported last (tests read it).
    pub screen: Cell<ScreenArea>,
    preview_sent: Cell<bool>,
    /// The `x11_embed` that [`Backend::attach`] was given.
    pub x11: Cell<bool>,
    /// QEMU's pid in [`ScreenSource::Window`]; 0 = a made-up one per VM.
    pub screen_pid: Cell<u32>,
    next_id: Cell<u32>,
    mac: Cell<u8>,
}

fn nic(model: &str, mac: &str) -> Nic {
    Nic {
        enabled: true,
        mode: "nat".into(),
        bridge: String::new(),
        model: model.into(),
        mac: mac.into(),
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        let nilton = Config {
            name: "Nilton".into(),
            workspace: "/home/nilton/VMs/nilton".into(),
            version: "sequoia".into(),
            ram_mib: 16384,
            cores: 4,
            vcpus: 8,
            ssh_port: 2222,
            networks: Some(vec![nic("virtio-net-pci", "52:54:00:12:34:56")]),
            graphics: config::GRAPHICS_REIMS.into(),
            resolution: "1920x1080".into(),
            ..Config::default()
        };
        let sonoma = Config {
            name: "Sonoma".into(),
            workspace: "/home/nilton/VMs/sonoma".into(),
            version: "sonoma".into(),
            ram_mib: 8192,
            cores: 2,
            vcpus: 4,
            ssh_port: 2223,
            networks: Some(vec![nic("virtio-net-pci", "52:54:00:ab:cd:ef")]),
            ..Config::default()
        };
        let entry = |id: &str, config| VmEntry {
            id: id.into(),
            config,
            state: VmState::Stopped,
            outside: false,
        };
        Self {
            vms: RefCell::new(vec![entry("1", nilton), entry("2", sonoma)]),
            selected: RefCell::new(Some("1".into())),
            events: RefCell::default(),
            theme: Cell::new(ThemeMode::Light),
            workspace: RefCell::new("/home/nilton/VMs".into()),
            screen: Cell::default(),
            preview_sent: Cell::new(false),
            x11: Cell::new(false),
            screen_pid: Cell::new(0),
            next_id: Cell::new(3),
            mac: Cell::new(0x10),
        }
    }
}

impl FakeBackend {
    /// A fake whose saved theme is `mode`.
    pub fn with_theme(mode: ThemeMode) -> Self {
        let f = Self::default();
        f.theme.set(mode);
        f
    }

    fn push(&self, e: Event) {
        self.events.borrow_mut().push_back(e);
    }

    fn set_state(&self, id: &str, state: VmState) -> Result<(), String> {
        let mut vms = self.vms.borrow_mut();
        let vm = vms.iter_mut().find(|v| v.id == id).ok_or("no such VM")?;
        vm.state = state;
        self.preview_sent.set(false);
        self.push(Event::State {
            id: id.into(),
            state,
            message: None,
        });
        Ok(())
    }
}

fn check(name: &str, detail: &str, status: Status) -> Check {
    Check {
        name: name.into(),
        detail: detail.into(),
        hint: String::new(),
        status,
    }
}

impl Backend for FakeBackend {
    fn vms(&self) -> Vec<VmEntry> {
        self.vms.borrow().clone()
    }
    fn selected(&self) -> Option<VmId> {
        self.selected.borrow().clone()
    }
    fn select(&self, id: &str) {
        *self.selected.borrow_mut() = Some(id.into());
        self.preview_sent.set(false);
    }
    fn save_config(&self, id: &str, c: &Config) -> Result<(), String> {
        if c.name.trim().is_empty() {
            return Err("Name: give the VM a name".into());
        }
        let mut vms = self.vms.borrow_mut();
        let vm = vms.iter_mut().find(|v| v.id == id).ok_or("no such VM")?;
        vm.config = c.clone();
        Ok(())
    }
    fn create_vm(&self, c: &Config) -> Result<VmId, String> {
        let mut c = c.clone();
        if c.ssh_port == 0 {
            c.ssh_port = self.next_ssh_port("").ok_or("no free SSH port")?;
        }
        let id = self.next_id.get().to_string();
        self.next_id.set(self.next_id.get() + 1);
        self.vms.borrow_mut().push(VmEntry {
            id: id.clone(),
            config: c,
            state: VmState::Stopped,
            outside: false,
        });
        self.push(Event::VmsChanged);
        Ok(id)
    }
    fn recommended(&self) -> Config {
        Config {
            ram_mib: 16384,
            cores: 4,
            vcpus: 8,
            ..Config::defaults()
        }
    }
    fn host_info(&self) -> HostInfo {
        HostInfo {
            cpus: 24,
            mem_mib: 65536,
            bridges: vec!["br0".into()],
            bridge_helper: true,
        }
    }
    fn random_mac(&self) -> String {
        let n = self.mac.get().wrapping_add(1);
        self.mac.set(n);
        format!("52:54:00:5e:{n:02x}:{:02x}", n.wrapping_mul(7))
    }

    fn start(&self, id: &str, verbose: bool) -> Result<(), String> {
        self.push(Event::Log(format!(
            "[VM] starting{}",
            if verbose { " with verbose boot" } else { "" }
        )));
        self.set_state(id, VmState::Running)
    }
    fn shutdown(&self, id: &str) -> Result<(), String> {
        self.push(Event::Log("[VM] ACPI power button sent to macOS".into()));
        self.set_state(id, VmState::Stopped)
    }
    fn force_stop(&self, id: &str) -> Result<(), String> {
        self.set_state(id, VmState::Stopped)
    }

    fn host_checks(&self, _: &Config) -> Vec<Check> {
        vec![
            check("CPU virtualization", "VT-x/AMD-V present", Status::Ok),
            check("/dev/kvm", "read and write allowed", Status::Ok),
            check("qemu-system-x86_64", "found", Status::Ok),
            check("qemu-img", "found", Status::Ok),
            check("QEMU version", "8.2.2", Status::Ok),
            check(
                "CPU compatible with version",
                "SSE4.1 and AVX2 present",
                Status::Ok,
            ),
            check("kvm ignore_msrs", "enabled", Status::Ok),
            check("Other hypervisors", "none loaded", Status::Ok),
        ]
    }
    fn reims_checks(&self, _: &Config) -> Vec<Check> {
        vec![
            check(
                "Reims vGPU QEMU",
                "/home/nilton/VMs/.reims/current/bin/qemu-system-x86_64",
                Status::Ok,
            ),
            check(
                "Vulkan driver",
                "NVIDIA GeForce RTX 4070 (Vulkan 1.4)",
                Status::Ok,
            ),
            Check {
                hint: "Give the VM at least 8192 MB, or close other programs.".into(),
                ..check("Free memory", "the host has 41 GB free", Status::Warn)
            },
        ]
    }
    fn reims_status(&self) -> ReimsStatus {
        ReimsStatus {
            installed: true,
            info: "In use: /home/nilton/VMs/.reims/current/bin/qemu-system-x86_64\nBuild 2026.10.1, upstream commit 4f1c2ab"
                .into(),
        }
    }
    fn gpus(&self) -> Vec<Gpu> {
        let nv = |addr: &str, name: &str, id: &str, group: &str| Gpu {
            addr: addr.into(),
            name: name.into(),
            id: id.into(),
            compat: "not supported by macOS".into(),
            driver: "nvidia".into(),
            group: group.into(),
            reason: "macOS has no driver for this GPU".into(),
            ..Gpu::default()
        };
        vec![
            nv(
                "0000:01:00.0",
                "NVIDIA Corporation AD104 [GeForce RTX 4070] (rev a1)",
                "10de:2786",
                "13",
            ),
            nv(
                "0000:0a:00.0",
                "NVIDIA Corporation GV100GL [Tesla V100 PCIe 32GB] (rev a1)",
                "10de:1db6",
                "17",
            ),
            Gpu {
                addr: "0000:75:00.0".into(),
                name: "Advanced Micro Devices, Inc. [AMD/ATI] Device 13c0 (rev c2)".into(),
                id: "1002:13c0".into(),
                compat: "unknown".into(),
                driver: "amdgpu".into(),
                group: "29".into(),
                reason:
                    "not in the list of macOS-supported GPUs; AMD Ryzen iGPUs have no macOS driver"
                        .into(),
                ..Gpu::default()
            },
        ]
    }
    fn reserved_gpus(&self) -> String {
        String::new()
    }
    fn install_status(&self, id: &str) -> InstallStatus {
        let vms = self.vms.borrow();
        let ws = vms
            .iter()
            .find(|v| v.id == id)
            .map(|v| v.config.workspace.clone())
            .unwrap_or_default();
        InstallStatus {
            folder: Some(ws),
            recovery: (id == "1").then(|| "BaseSystem.img (sequoia recovery)".into()),
            disk: Some("mac_hdd_ng.img · 21.40 GB used on the host".into()),
            installer: (id == "1").then(|| "macOS 15.8.1 (24H32), 15.7 GB, cached for every VM of this version".into()),
            boot_files: "OpenCore 1.0.8, Lilu 1.7.2, VirtualSMC 1.3.8, WhateverGreen 1.7.1, OVMF 2025.02-8+deb13u1 (built in)"
                .into(),
        }
    }
    fn installers(&self) -> Vec<Installer> {
        vec![Installer {
            version: "15.8.1".into(),
            build: "24H32".into(),
            size_gb: 15.7,
            cached: true,
            used_by: vec!["Nilton".into()],
        }]
    }
    /// The Nilton VM has the full installer cached, so its disc is attached while it runs.
    fn installer_command(&self, id: &str) -> Option<(String, bool)> {
        self.vms
            .borrow()
            .iter()
            .any(|v| v.id == id && v.id == "1" && v.state == VmState::Running)
            .then(|| (rg_macos::DISC_COMMAND.into(), true))
    }
    fn about(&self) -> String {
        "Boot files: OpenCore 1.0.8, Lilu 1.7.2, VirtualSMC 1.3.8, WhateverGreen 1.7.1".into()
    }

    fn run(&self, id: Option<&str>, task: Task) {
        self.push(Event::Log(format!("== {}", task.title())));
        match &task {
            Task::Import { path, folder } => {
                let name = std::path::Path::new(path)
                    .file_stem()
                    .map_or("Imported".into(), |s| s.to_string_lossy().into_owned());
                let c = Config {
                    workspace: folder.clone(),
                    name,
                    ..Config::defaults()
                };
                if let Ok(new) = self.create_vm(&c) {
                    self.select(&new);
                }
            }
            Task::Move { folder } => {
                if let Some(v) = self
                    .vms
                    .borrow_mut()
                    .iter_mut()
                    .find(|v| Some(v.id.as_str()) == id)
                {
                    v.config.workspace = folder.clone();
                }
                self.push(Event::VmsChanged);
            }
            Task::RemoveVm { id, .. } => {
                self.vms.borrow_mut().retain(|v| &v.id != id);
                self.push(Event::VmsChanged);
            }
            Task::InstallOffline { .. } => self.push(Event::Log(format!("{task:?}"))),
            Task::RootPatch { on } => {
                if let Some(v) = self
                    .vms
                    .borrow_mut()
                    .iter_mut()
                    .find(|v| Some(v.id.as_str()) == id)
                {
                    v.config.root_patch = *on;
                }
            }
            _ => {}
        }
        let result = Ok(match task {
            Task::RemoveMedia => "Install media removed: 1.20 GB freed".into(),
            _ => String::new(),
        });
        self.push(Event::TaskDone { task, result });
    }
    fn poll_event(&self) -> Option<Event> {
        self.events.borrow_mut().pop_front()
    }
    fn preview(&self, id: &str) -> Option<PreviewUpdate> {
        let running = self
            .vms
            .borrow()
            .iter()
            .any(|v| v.id == id && v.state == VmState::Running);
        if !running || self.preview_sent.replace(true) {
            return None;
        }
        // A macOS-like desktop: a gradient wallpaper and a menu bar.
        let (w, h) = (320, 180);
        let mut img = ColorImage::filled([w, h], Color32::BLACK);
        for y in 0..h {
            for x in 0..w {
                let c = if y < 8 {
                    Color32::from_gray(235)
                } else {
                    Color32::from_rgb((40 + x * 120 / w) as u8, (60 + y * 80 / h) as u8, 160)
                };
                img.pixels[y * w + x] = c;
            }
        }
        Some(PreviewUpdate::Full(img))
    }
    fn set_screen_area(&self, area: ScreenArea) {
        self.screen.set(area);
    }
    fn attach(&self, _ctx: &egui::Context, x11_embed: bool) {
        self.x11.set(x11_embed);
    }
    fn screen_source(&self, id: &str) -> Option<ScreenSource> {
        let vms = self.vms.borrow();
        let v = vms
            .iter()
            .find(|v| v.id == id && (v.state != VmState::Stopped || v.outside))?;
        let pid = match self.screen_pid.get() {
            0 => 4000 + id.parse::<u32>().unwrap_or(0),
            p => p,
        };
        Some(
            match (v.config.reims(), v.config.in_app(), self.x11.get()) {
                (_, false, _) | (true, _, false) => ScreenSource::Standalone,
                (reims, true, true) => ScreenSource::Window { pid, reims },
                (false, true, false) => ScreenSource::Vnc("/nonexistent/vnc.sock".into()),
            },
        )
    }

    fn workspace_root(&self) -> String {
        self.workspace.borrow().clone()
    }
    fn set_workspace_root(&self, path: &str) -> Result<(), String> {
        if path.trim().is_empty() {
            return Err("Workspace: choose a folder".into());
        }
        *self.workspace.borrow_mut() = path.trim().into();
        Ok(())
    }
    fn settings_file(&self) -> String {
        "/home/nilton/.config/macos-kvm-gui/settings.yaml".into()
    }
    fn open_settings_file(&self) {}
    fn reset_settings(&self) -> Result<(), String> {
        Ok(())
    }
    fn theme(&self) -> ThemeMode {
        self.theme.get()
    }
    fn set_theme(&self, mode: ThemeMode) {
        self.theme.set(mode);
    }
    fn open_url(&self, url: &str) {
        self.push(Event::Log(format!("open {url}")));
    }
}
