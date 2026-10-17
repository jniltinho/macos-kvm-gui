//! The UI-to-engine boundary (design D6, the rustXterm `Backend` pattern): everything the GUI
//! asks of the core crates goes through [`Backend`]. Calls return at once; long operations are
//! [`Task`]s whose progress and result arrive as [`Event`]s that the UI polls every frame.

use egui::ColorImage;
pub use rg_core::{Config, Nic};

/// A VM's registry id.
pub type VmId = String;

/// What QEMU is doing with a VM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VmState {
    #[default]
    Stopped,
    Running,
    Paused,
}

impl VmState {
    /// VirtualBox's wording.
    pub fn text(self) -> &'static str {
        match self {
            Self::Stopped => "Powered Off",
            Self::Running => "Running",
            Self::Paused => "Paused",
        }
    }
}

/// One row of the VM list.
#[derive(Clone, Debug, PartialEq)]
pub struct VmEntry {
    pub id: VmId,
    pub config: Config,
    pub state: VmState,
    /// Started by another process (CLI, MCP): the app cannot shut it down.
    pub outside: bool,
}

impl VmEntry {
    /// The name shown in the list (the folder's name when the VM has none).
    pub fn display_name(&self) -> &str {
        if self.config.name.is_empty() {
            self.config
                .workspace
                .rsplit('/')
                .next()
                .unwrap_or(&self.config.workspace)
        } else {
            &self.config.name
        }
    }
}

/// A host check's outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
    #[default]
    Unknown,
}

impl Status {
    pub fn text(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "Warning",
            Self::Fail => "Failed",
            Self::Unknown => "Unknown",
        }
    }
}

/// One host check (Host tab card).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Check {
    pub name: String,
    pub detail: String,
    /// How to fix it, shown under the card when not OK.
    pub hint: String,
    pub status: Status,
}

/// A PCI display adapter (GPU tab).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub addr: String,
    pub name: String,
    /// `vendor:device`.
    pub id: String,
    /// Compatibility class, e.g. "not supported by macOS".
    pub compat: String,
    pub driver: String,
    pub group: String,
    pub eligible: bool,
    pub kepler: bool,
    /// Why it is not eligible.
    pub reason: String,
}

/// The Reims vGPU build in use (Host tab).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReimsStatus {
    pub installed: bool,
    /// "In use: …", the build and commit, or why it cannot run.
    pub info: String,
}

/// A VM's installation progress (Install tab).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstallStatus {
    /// The VM folder, when created.
    pub folder: Option<String>,
    /// The recovery media, e.g. "BaseSystem.img (sequoia recovery)".
    pub recovery: Option<String>,
    /// The disk, e.g. "mac_hdd_ng.img · 0.00 GB used on the host".
    pub disk: Option<String>,
    /// The cached full installer for this version, e.g. "macOS 15.8.1 (24H32), 15.7 GB".
    pub installer: Option<String>,
    /// "OpenCore 1.0.8, Lilu 1.7.2, …".
    pub boot_files: String,
}

impl InstallStatus {
    /// Steps 1–3 done.
    pub fn done(&self) -> usize {
        [
            self.folder.is_some(),
            self.recovery.is_some(),
            self.disk.is_some(),
        ]
        .iter()
        .filter(|d| **d)
        .count()
    }
}

/// A full installer, cached or offered (File › Offline installers).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Installer {
    pub version: String,
    pub build: String,
    pub size_gb: f32,
    pub cached: bool,
    /// The VMs whose macOS version installs from it (the Go app's `CachedInstallerFor`).
    pub used_by: Vec<String>,
}

/// Host facts the dialogs need.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostInfo {
    pub cpus: u32,
    pub mem_mib: u32,
    /// Linux bridges, for bridged adapters.
    pub bridges: Vec<String>,
    /// Whether qemu-bridge-helper is installed.
    pub bridge_helper: bool,
}

/// Long operations; each ends with [`Event::TaskDone`].
#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    PrepareFolder,
    DownloadRecovery,
    CreateDisk {
        gib: u32,
    },
    RemoveMedia,
    DownloadInstaller,
    DeleteInstaller,
    /// Folder, disk and (optionally) recovery of a VM just added by the wizard.
    CreateVmFiles {
        disk_gib: u32,
        download: bool,
    },
    SetupHost,
    ReserveGpu {
        addr: String,
    },
    ReleaseGpu,
    InstallReims,
    BuildReims,
    /// File › Import: a settings file (`.json`, `folder` empty) or a full bundle (`.tar`)
    /// unpacked into `folder` (rg-core `Registry::import_settings` / `import_bundle`). The new
    /// VM is selected and [`Event::VmsChanged`] sent.
    Import {
        path: String,
        folder: String,
    },
    /// File › Export of the VM: its settings as JSON, or the full `.tar` bundle with progress
    /// events (rg-ops `export_bundle`, refused while it runs).
    Export {
        path: String,
        full: bool,
    },
    /// Machine › Move: the VM's files into `folder` (rg-ops `move_vm`, refused while it runs).
    Move {
        folder: String,
    },
    /// File › Offline installers › Delete: the cached installer `build` (rg-macos
    /// `remove_installer`, refused while a VM uses its disc).
    RemoveInstaller {
        build: String,
    },
    /// Install step 4's "Type it in the VM": opens the recovery's Terminal and types the line
    /// of the attached installer disc (rg-ops `install_offline`); `unattended` is the user's
    /// consent to erase the empty disk and accept Apple's license.
    InstallOffline {
        unattended: bool,
    },
    /// GPU tab root-patch mode, confirmed: on downloads OCLP's kexts first, then saves.
    RootPatch {
        on: bool,
    },
}

impl Task {
    /// The name shown in the log and the status bar.
    pub fn title(&self) -> String {
        match self {
            Self::PrepareFolder => "Create VM folder".into(),
            Self::DownloadRecovery => "Download macOS recovery".into(),
            Self::CreateDisk { gib } => format!("Create disk ({gib} GB)"),
            Self::RemoveMedia => "Remove install media".into(),
            Self::DownloadInstaller => "Download full installer".into(),
            Self::DeleteInstaller => "Delete full installer".into(),
            Self::CreateVmFiles { .. } => "Create VM".into(),
            Self::SetupHost => "Set up host".into(),
            Self::ReserveGpu { .. } => "Reserve GPU".into(),
            Self::ReleaseGpu => "Release GPU".into(),
            Self::InstallReims => "Install Reims vGPU".into(),
            Self::BuildReims => "Build Reims vGPU".into(),
            Self::Import { .. } => "Import VM".into(),
            Self::Export { full: true, .. } => "Export VM bundle".into(),
            Self::Export { .. } => "Export VM settings".into(),
            Self::Move { .. } => "Move VM".into(),
            Self::RemoveInstaller { .. } => "Delete offline installer".into(),
            Self::InstallOffline { .. } => "Type the install line in the VM".into(),
            Self::RootPatch { on: true } => "Download OCLP's kexts for root-patch mode".into(),
            Self::RootPatch { on: false } => "Turn off root-patch mode".into(),
        }
    }
}

/// What the engine tells the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Log(String),
    /// `fraction` None: indeterminate.
    Progress {
        stage: String,
        fraction: Option<f32>,
    },
    TaskDone {
        task: Task,
        result: Result<String, String>,
    },
    /// A VM started or stopped; `message` explains an unexpected stop.
    State {
        id: VmId,
        state: VmState,
        message: Option<String>,
    },
    /// The VM list changed (created, removed, renamed, started elsewhere).
    VmsChanged,
    /// Something the user must read, shown as a dialog (e.g. "Disk created with a warning").
    Notice {
        title: String,
        text: String,
    },
}

/// New pixels for the Details Preview.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewUpdate {
    /// A whole frame (first frame, or the guest changed size).
    Full(ColorImage),
    /// Changed rectangles of the current frame: top-left `[x, y]` and the pixels.
    Dirty(Vec<([usize; 2], ColorImage)>),
}

/// The Screen tab's area for the embedding layer (design D3), sent every frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenArea {
    /// The Screen tab's rectangle in physical pixels of the app's window, `[x, y, w, h]`;
    /// None when the tab is not shown (QEMU's window is then unmapped).
    pub rect: Option<[i32; 4]>,
    /// A modal, menu or popup is open over the window: unmap the embedded window so it never
    /// covers them and clicks never reach the guest.
    pub overlay_open: bool,
}

/// Where a running VM's screen comes from (design D3, D4); the Screen tab follows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenSource {
    /// QEMU's own X11 window, to embed: Reims vGPU's (`reims`, title "Reims vGPU…") or QEMU's
    /// `-display sdl` window for VMware SVGA (title "QEMU…"). `pid` is the start result's
    /// (`rg_qemu::Started::pid`) or, for a VM started by the CLI or MCP, running detection's
    /// (`rg_qemu::RunInfo::pid`).
    Window { pid: u32, reims: bool },
    /// QEMU's VNC socket, drawn by the app (VMware SVGA without X11).
    Vnc(std::path::PathBuf),
    /// QEMU shows a window the app cannot embed (Reims vGPU on Wayland, `display=window`).
    Standalone,
}

/// The operations the GUI needs. Implementations must not block: anything slow is a [`Task`].
///
/// The Screen tab asks [`Backend::screen_source`]; the file dialogs run the [`Task`]s `Import`,
/// `Export`, `Move` and `RemoveInstaller`; Settings › Storage "Remove…" runs
/// [`Task::RemoveMedia`].
pub trait Backend {
    /// Called once the window exists: `ctx` wakes the UI when events arrive, `x11_embed` says
    /// whether the app embeds QEMU's windows over X11 (design D3).
    fn attach(&self, _ctx: &egui::Context, _x11_embed: bool) {}
    /// All VMs, in list order.
    fn vms(&self) -> Vec<VmEntry>;
    /// The VM selected at the last run.
    fn selected(&self) -> Option<VmId>;
    fn select(&self, id: &str);
    /// Validates and saves a VM's settings.
    fn save_config(&self, id: &str, config: &Config) -> Result<(), String>;
    /// Adds a VM (the wizard's Finish); its files come from [`Task::CreateVmFiles`].
    fn create_vm(&self, config: &Config) -> Result<VmId, String>;
    fn remove_vm(&self, id: &str, delete_files: bool) -> Result<(), String>;
    /// Defaults recommended for this host (memory, processors).
    fn recommended(&self) -> Config;
    fn host_info(&self) -> HostInfo;
    fn random_mac(&self) -> String;

    fn start(&self, id: &str, verbose: bool) -> Result<(), String>;
    /// ACPI power button.
    fn shutdown(&self, id: &str) -> Result<(), String>;
    fn force_stop(&self, id: &str) -> Result<(), String>;

    /// The host checks for `config`, without the Reims ones.
    fn host_checks(&self, config: &Config) -> Vec<Check>;
    fn reims_checks(&self, config: &Config) -> Vec<Check>;
    fn reims_status(&self) -> ReimsStatus;
    fn gpus(&self) -> Vec<Gpu>;
    /// The vfio-pci ids reserved, empty when none.
    fn reserved_gpus(&self) -> String;
    fn install_status(&self, id: &str) -> InstallStatus;
    fn installers(&self) -> Vec<Installer>;
    /// The line to type in the recovery's Terminal when the VM `id`, started by this app, has
    /// the full installer disc attached, and whether the app can type it (QEMU has a VNC
    /// socket: not with VMware SVGA in QEMU's own window); None otherwise. Called every frame
    /// on the Install tab.
    fn installer_command(&self, _id: &str) -> Option<(String, bool)> {
        None
    }
    /// The asset versions for About ("OpenCore 1.0.8, …").
    fn about(&self) -> String;

    /// Starts a long operation for the VM `id` (None for host-wide ones).
    fn run(&self, id: Option<&str>, task: Task);
    fn poll_event(&self) -> Option<Event>;
    /// New Preview pixels for `id` since the last call.
    fn preview(&self, id: &str) -> Option<PreviewUpdate>;
    /// Where the Screen tab is this frame (design D3). The UI embeds QEMU's window itself
    /// (`screen` module); this is informational.
    fn set_screen_area(&self, area: ScreenArea);
    /// Where the screen of the running VM `id` is shown; None when it is not running. Called
    /// every frame while the Screen tab shows a running VM: keep it cheap (cache the start
    /// result and the running detection).
    fn screen_source(&self, _id: &str) -> Option<ScreenSource> {
        None
    }

    fn workspace_root(&self) -> String;
    fn set_workspace_root(&self, path: &str) -> Result<(), String>;
    /// The path of `settings.yaml`.
    fn settings_file(&self) -> String;
    fn open_settings_file(&self);
    fn reset_settings(&self) -> Result<(), String>;
    fn theme(&self) -> crate::theme::ThemeMode;
    fn set_theme(&self, mode: crate::theme::ThemeMode);
    fn open_url(&self, url: &str);
}
