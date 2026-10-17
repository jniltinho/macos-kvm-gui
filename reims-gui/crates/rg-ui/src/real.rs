//! The [`Backend`] on the real crates (design D6): the registry (rg-core), the operations that
//! span crates (rg-ops), the host (rg-host), Reims vGPU packages (rg-reims), installers and
//! recovery downloads (rg-macos), running-VM detection and the VNC Preview (rg-qemu).
//!
//! Nothing here blocks the UI thread for long: tasks, starts and host probes run on the tokio
//! runtime the backend owns (`spawn_blocking` for the blocking crates) and report back through a
//! bounded event channel; host checks, GPUs and the Reims status are cached and refreshed in the
//! background; a thread rescans `/proc` every 2 s for VMs started elsewhere (CLI, MCP, the Go app).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use egui::ColorImage;
use parking_lot::Mutex;
use rg_core::registry::PREF_THEME;
use rg_core::settings::MKG_THEME;
use rg_core::{AppDir, Registry, Workspace};
use rg_host::Host;
use rg_qemu::{Frame, Rect, VmEvent, Vnc};
use tokio::runtime::{Handle, Runtime};

use crate::backend::*;
use crate::theme::ThemeMode;

/// Pending events at most; log lines and progress past it are dropped, never the rest.
const EVENTS: usize = 4096;
/// How long host checks, GPUs and the Reims status stay fresh.
const CHECKS_TTL: Duration = Duration::from_secs(10);
/// How often `/proc` is scanned for VMs started elsewhere.
const SCAN: Duration = Duration::from_secs(2);
/// The wait before reconnecting a Preview whose VNC connection ended.
const PREVIEW_RETRY: Duration = Duration::from_secs(2);
/// More dirty rectangles than this and the Preview gets a whole frame instead.
const MAX_DIRTY: usize = 64;

/// The real engine behind the GUI. Dropping it stops its tasks; QEMU dies with the app.
pub struct RealBackend {
    inner: Arc<Inner>,
    events: Receiver<Event>,
    rt: Option<Runtime>,
}

struct Inner {
    app: AppDir,
    reg: Mutex<Registry>,
    /// `--config <file.json>`: the only VM, stored in that file.
    single: Option<Mutex<rg_core::VmEntry>>,
    /// The Workspace root (as the registry stores it) and the operations on it; rebuilt when it
    /// changes, here or in the shared database (the Go app, the CLI).
    ops: Mutex<(String, rg_ops::Ops)>,
    /// Tests swap the host for a fake one.
    host: Option<Host>,
    rt: Handle,
    tx: SyncSender<Event>,
    /// Important events the full channel could not take.
    overflow: Mutex<VecDeque<Event>>,
    ctx: OnceLock<egui::Context>,
    x11_embed: AtomicBool,
    /// Set when the backend is dropped: long calls stop.
    cancel: AtomicBool,
    /// VMs this app started.
    own: Mutex<HashMap<VmId, Own>>,
    starting: Mutex<HashSet<VmId>>,
    /// What the last scan (or an own VM's event) found per VM.
    seen: Mutex<HashMap<VmId, Seen>>,
    checks: Mutex<Checks>,
    preview: Mutex<Option<Preview>>,
    screen: Mutex<ScreenArea>,
}

struct Own {
    vm: rg_qemu::Vm,
    vnc: Option<PathBuf>,
    /// The install line when the full installer disc is attached (Install step 4).
    installer_command: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Seen {
    state: VmState,
    outside: bool,
    vnc: Option<PathBuf>,
    /// The control socket of a VM started by another app instance (shut down through it).
    control: bool,
    /// QEMU's pid, and what shows its screen (the Screen tab's [`ScreenSource`]).
    pid: u32,
    reims: bool,
    in_app: bool,
}

/// The slow host facts, refreshed in the background.
#[derive(Default)]
struct Checks {
    /// The config they were computed for, and its [`check_key`].
    cfg: rg_core::Config,
    key: String,
    host: Vec<Check>,
    reims: Vec<Check>,
    gpus: Vec<Gpu>,
    reserved: String,
    reims_status: Option<ReimsStatus>,
    fresh: Option<Instant>,
    busy: bool,
}

/// The Preview's VNC connection for one VM.
struct Preview {
    id: VmId,
    sock: PathBuf,
    shared: Arc<Mutex<Pending>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
struct Pending {
    vnc: Option<Vnc>,
    full: bool,
    dirty: Vec<Rect>,
    ended: Option<Instant>,
}

/// What the host checks depend on.
fn check_key(c: &rg_core::Config) -> String {
    format!("{}|{}|{}", c.version, c.graphics, c.ram_mib)
}

/// Go's `previewInterval`: about twice the time QEMU takes to read a Reims vGPU frame (0.22 µs
/// per pixel measured), at most 5 frames a second.
fn preview_interval(w: u32, h: u32) -> Duration {
    Duration::from_millis(200).max(Duration::from_nanos(2 * 220 * u64::from(w) * u64::from(h)))
}

fn threads() -> u32 {
    std::thread::available_parallelism().map_or(1, |n| u32::try_from(n.get()).unwrap_or(u32::MAX))
}

fn status(s: rg_host::Status) -> Status {
    match s {
        rg_host::Status::Ok => Status::Ok,
        rg_host::Status::Warn => Status::Warn,
        rg_host::Status::Fail => Status::Fail,
        rg_host::Status::Unknown => Status::Unknown,
    }
}

fn check(c: rg_host::Check) -> Check {
    Check {
        status: status(c.status),
        name: c.name,
        detail: c.detail,
        hint: c.hint,
    }
}

fn gpu(g: rg_host::Gpu) -> Gpu {
    Gpu {
        compat: g.compat.to_string(),
        kepler: g.compat == rg_host::Compat::Kepler,
        addr: g.addr,
        name: g.name,
        id: g.id,
        driver: g.driver,
        group: g.group,
        eligible: g.eligible,
        reason: g.reason,
    }
}

fn vm_state(s: rg_qemu::VmState) -> VmState {
    match s {
        rg_qemu::VmState::Stopped => VmState::Stopped,
        rg_qemu::VmState::Running => VmState::Running,
        rg_qemu::VmState::Paused => VmState::Paused,
    }
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1u64 << 30) as f64
}

/// The whole frame as an egui image.
fn image(f: &Frame) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.pixels)
}

/// `r` cut out of `f` (clamped to it), with its top-left corner.
fn sub_image(f: &Frame, r: &Rect) -> Option<([usize; 2], ColorImage)> {
    let (x, y) = (r.x.min(f.width) as usize, r.y.min(f.height) as usize);
    let w = (r.w as usize).min(f.width as usize - x);
    let h = (r.h as usize).min(f.height as usize - y);
    if w == 0 || h == 0 {
        return None;
    }
    let stride = f.width as usize * 4;
    let mut px = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        let start = row * stride + x * 4;
        px.extend_from_slice(f.pixels.get(start..start + w * 4)?);
    }
    Some(([x, y], ColorImage::from_rgba_unmultiplied([w, h], &px)))
}

/// Opens a file or URL with the desktop's handler, without waiting for it.
fn xdg_open(target: &str) -> Result<(), String> {
    let mut child = std::process::Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| format!("could not open {target}: {e}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The operations on the Workspace `root`, with `host` instead of the system's (tests).
fn new_ops(app: &AppDir, root: &str, host: Option<&Host>) -> rg_ops::Result<rg_ops::Ops> {
    let mut ops = rg_ops::Ops::new(app.clone(), root)?;
    if let Some(h) = host {
        ops.host = h.clone();
    }
    Ok(ops)
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl RealBackend {
    /// The backend on the config folder `app` (`~/.config/macos-kvm-gui`, or a temporary one);
    /// `config` is `--config`'s VM file, the only VM then. With an empty registry it adds the
    /// Go app's first VM, "macOS Sonoma".
    pub fn new(app: AppDir, config: Option<&Path>) -> anyhow::Result<Self> {
        Self::open(app, config, None)
    }

    /// [`RealBackend::new`] with `host` instead of the system (tests).
    pub fn with_host(app: AppDir, config: Option<&Path>, host: Host) -> anyhow::Result<Self> {
        Self::open(app, config, Some(host))
    }

    fn open(app: AppDir, config: Option<&Path>, host: Option<Host>) -> anyhow::Result<Self> {
        let reg = Registry::open(&app)?;
        let single = match config {
            Some(p) => Some(Mutex::new(rg_core::VmEntry {
                id: "config".into(),
                path: Some(p.to_path_buf()),
                config: rg_core::config::load_config(p)?,
            })),
            None => {
                if reg.list()?.is_empty() {
                    let mut c = rg_core::Config::defaults();
                    c.name = "macOS Sonoma".into();
                    c.workspace = reg.vm_folder(&c.name);
                    c.networks = Some(rg_core::network::new_networks(&c.version));
                    reg.create(c)?;
                }
                None
            }
        };
        let root = reg.workspace_root();
        let ops = new_ops(&app, &root, host.as_ref())?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("reims-gui-backend")
            .enable_all()
            .build()?;
        let (tx, events) = sync_channel(EVENTS);
        let inner = Arc::new(Inner {
            app,
            reg: Mutex::new(reg),
            single,
            ops: Mutex::new((root, ops)),
            host,
            rt: rt.handle().clone(),
            tx,
            overflow: Mutex::default(),
            ctx: OnceLock::new(),
            x11_embed: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            own: Mutex::default(),
            starting: Mutex::default(),
            seen: Mutex::default(),
            checks: Mutex::default(),
            preview: Mutex::default(),
            screen: Mutex::default(),
        });
        inner.scan(); // the first list already shows VMs started elsewhere
        let weak = Arc::downgrade(&inner);
        std::thread::Builder::new()
            .name("reims-gui-scan".into())
            .spawn(move || scan_loop(&weak))?;
        Ok(Self {
            inner,
            events,
            rt: Some(rt),
        })
    }

    /// The Screen tab's area the UI reported last (the X11 embedder reads it).
    pub fn screen_area(&self) -> ScreenArea {
        *self.inner.screen.lock()
    }
}

impl Drop for RealBackend {
    fn drop(&mut self) {
        self.inner.cancel.store(true, Ordering::Relaxed);
        *self.inner.preview.lock() = None;
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

fn scan_loop(weak: &Weak<Inner>) {
    loop {
        std::thread::sleep(SCAN);
        match weak.upgrade() {
            Some(i) if !i.cancel.load(Ordering::Relaxed) => i.scan(),
            _ => return,
        }
    }
}

impl Inner {
    fn emit(&self, e: Event) {
        let droppable = matches!(e, Event::Log(_) | Event::Progress { .. });
        if let Err(std::sync::mpsc::TrySendError::Full(e)) = self.tx.try_send(e)
            && !droppable
        {
            // ponytail: unbounded only for the rare important events; logs are dropped instead.
            self.overflow.lock().push_back(e);
        }
        if let Some(c) = self.ctx.get() {
            c.request_repaint();
        }
    }

    fn log(&self, line: impl Into<String>) {
        self.emit(Event::Log(line.into()));
    }

    /// The operations for the registry's current Workspace. The Go app or the CLI may have
    /// changed it in the shared database: then they are rebuilt (Reims lookup, installer and
    /// kext folders) and the cached host facts dropped.
    fn ops(&self) -> rg_ops::Ops {
        let root = self.reg.lock().workspace_root();
        let mut cur = self.ops.lock();
        if cur.0 == root {
            return cur.1.clone();
        }
        match new_ops(&self.app, &root, self.host.as_ref()) {
            Ok(o) => cur.1 = o,
            // Kept on the old folders; logged once (the root is remembered either way).
            Err(e) => self.log(format!("error: Workspace {root}: {e}")),
        }
        cur.0 = root;
        let ops = cur.1.clone();
        drop(cur);
        self.invalidate_checks();
        ops
    }

    fn entries(&self) -> Result<Vec<rg_core::VmEntry>, String> {
        match &self.single {
            Some(s) => Ok(vec![s.lock().clone()]),
            None => self.reg.lock().list().map_err(err),
        }
    }

    fn entry(&self, id: &str) -> Result<rg_core::VmEntry, String> {
        self.entries()?
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("VM {id} no longer exists"))
    }

    fn seen(&self, id: &str) -> Seen {
        self.seen.lock().get(id).cloned().unwrap_or_default()
    }

    /// Finds every VM's QEMU (this app's, the CLI's, the MCP server's, the Go app's) and reports
    /// the changes.
    fn scan(&self) {
        let Ok(list) = self.entries() else { return };
        let ops = self.ops();
        let starting = self.starting.lock().clone();
        let mut found = HashMap::new();
        for e in list {
            if starting.contains(&e.id) {
                found.insert(e.id.clone(), self.seen(&e.id));
                continue;
            }
            let s = if let Some(o) = self.own.lock().get(&e.id) {
                Seen {
                    state: vm_state(o.vm.state()),
                    vnc: o.vnc.clone(),
                    pid: o.vm.pid().unwrap_or(0),
                    reims: e.config.reims(),
                    in_app: e.config.in_app(),
                    ..Seen::default()
                }
            } else if let Some(r) = ops.find_running(&e.config) {
                Seen {
                    state: VmState::Running,
                    outside: true,
                    control: r.control.is_some(),
                    vnc: r.vnc_socket,
                    pid: r.pid,
                    reims: r.reims,
                    in_app: e.config.in_app(),
                }
            } else {
                Seen::default()
            };
            found.insert(e.id, s);
        }
        let old = std::mem::replace(&mut *self.seen.lock(), found.clone());
        let mut list_changed = false;
        for (id, s) in found {
            let before = old.get(&id).cloned().unwrap_or_default();
            if before.state != s.state {
                self.emit(Event::State {
                    id: id.clone(),
                    state: s.state,
                    message: None,
                });
            }
            list_changed |= before.outside != s.outside;
        }
        if list_changed {
            self.emit(Event::VmsChanged);
        }
    }

    fn set_seen(&self, id: &str, s: Seen) {
        self.seen.lock().insert(id.into(), s);
    }

    // ---- host facts ----

    /// The cached host facts, refreshed in the background when stale or for another config
    /// (`None`: the last one).
    fn checks(
        self: &Arc<Self>,
        c: Option<&rg_core::Config>,
    ) -> parking_lot::MutexGuard<'_, Checks> {
        let mut ch = self.checks.lock();
        let cfg = c.cloned().unwrap_or_else(|| ch.cfg.clone());
        let key = check_key(&cfg);
        let stale = ch.fresh.is_none_or(|t| t.elapsed() > CHECKS_TTL) || ch.key != key;
        if stale && !ch.busy {
            ch.busy = true;
            let inner = self.clone();
            self.rt
                .spawn_blocking(move || inner.compute_checks(cfg, key));
        }
        ch
    }

    fn compute_checks(&self, cfg: rg_core::Config, key: String) {
        let ops = self.ops();
        let h = &ops.host;
        let reims: Vec<Check> = h.reims_checks_for(&cfg).into_iter().map(check).collect();
        let mut host: Vec<Check> = h.checks_for(&cfg).into_iter().map(check).collect();
        host.truncate(host.len().saturating_sub(reims.len()));
        let gpus = h.gpus().into_iter().map(gpu).collect();
        let reserved = h.reserved_ids();
        let reims_status = match ops.reims().installed() {
            Err(_) => ReimsStatus {
                installed: false,
                info: "Not installed.".into(),
            },
            Ok(i) => {
                let mut info = format!("In use: {}", i.qemu.display());
                if !i.build.is_empty() {
                    info += &format!("\nBuild {}, upstream commit {}", i.build, i.commit);
                }
                if let Err(e) = i.apple_gfx {
                    info += &format!("\n{e}");
                }
                ReimsStatus {
                    installed: true,
                    info,
                }
            }
        };
        *self.checks.lock() = Checks {
            cfg,
            key,
            host,
            reims,
            gpus,
            reserved,
            reims_status: Some(reims_status),
            fresh: Some(Instant::now()),
            busy: false,
        };
        self.emit(Event::VmsChanged); // the UI rereads the host tabs
    }

    /// A byte copy's progress (bundle export and import), at most one event per 0.5 %.
    fn bytes(&self, last: &mut u64, stage: &str, done: u64, total: u64) {
        let total = total.max(1);
        let tick = done.saturating_mul(200) / total;
        if std::mem::replace(last, tick) != tick {
            self.emit(Event::Progress {
                stage: format!(
                    "{stage}: {:.1} of {:.1} GB",
                    done as f64 / 1e9,
                    total as f64 / 1e9
                ),
                fraction: Some(done as f32 / total as f32),
            });
        }
    }

    fn disk_warning(&self, warning: impl Into<String>) {
        self.emit(Event::Notice {
            title: "Disk created with a warning".into(),
            text: warning.into(),
        });
    }

    fn save_config(&self, id: &str, config: &Config) -> Result<(), String> {
        let e = self.entry(id)?;
        self.reg.lock().save(&e, config.clone()).map_err(err)?;
        if let Some(s) = &self.single {
            s.lock().config = config.clone();
        }
        Ok(())
    }

    fn invalidate_checks(&self) {
        self.checks.lock().fresh = None;
    }

    // ---- tasks ----

    fn task(&self, id: Option<&str>, task: &Task) -> Result<String, String> {
        let ops = self.ops();
        let vm =
            || -> Result<rg_core::VmEntry, String> { self.entry(id.ok_or("select a VM first")?) };
        let cancel = &self.cancel;
        let mut last = (String::new(), -1i64);
        let mut progress = |p: rg_macos::Progress| {
            // At most one event per 0.1 %.
            let tick = p.pct().map_or(-1, |pct| (pct * 10.0) as i64);
            if (p.phase.as_str(), tick) != (last.0.as_str(), last.1) {
                last = (p.phase.clone(), tick);
                self.emit(Event::Progress {
                    fraction: p.pct().map(|pct| (pct / 100.0) as f32),
                    stage: p.phase,
                });
            }
        };
        let mut log = |l: &str| self.log(l);
        match task {
            Task::PrepareFolder => {
                let c = vm()?.config;
                ops.prepare_workspace(&c).map_err(err)?;
                Ok(format!(
                    "VM folder ready: {}",
                    c.workspace_dir().map_err(err)?.display()
                ))
            }
            Task::DownloadRecovery => {
                let c = vm()?.config;
                ops.download_media(&c, &c.version, cancel, &mut progress, &mut log)
                    .map_err(err)?;
                Ok(format!("Install media ready ({})", c.version))
            }
            Task::CreateDisk { gib } => {
                let w = Workspace::new(vm()?.config.workspace_dir().map_err(err)?);
                if let Some(warning) = w.create_disk(u64::from(*gib)).map_err(err)? {
                    log(&format!("[disk] {warning}"));
                    self.disk_warning(warning);
                }
                Ok(format!("Disk created: {gib} GB"))
            }
            Task::RemoveMedia => {
                let w = Workspace::new(vm()?.config.workspace_dir().map_err(err)?);
                let freed = w.remove_media().map_err(err)?;
                Ok(format!("Install media removed: {:.2} GB freed", gib(freed)))
            }
            Task::DownloadInstaller => {
                let c = vm()?.config;
                let ci = rg_macos::download_installer(
                    &ops.http,
                    &ops.installers_dir(),
                    &c.version,
                    cancel,
                    &mut progress,
                    &mut log,
                )
                .map_err(err)?;
                Ok(format!(
                    "Full installer ready: macOS {} ({})",
                    ci.product.version, ci.product.build
                ))
            }
            Task::DeleteInstaller => {
                let c = vm()?.config;
                let dir = ops.installers_dir();
                let ci = rg_macos::cached_installer_for(&dir, &c.version)
                    .ok_or("no full installer is cached for this version")?;
                rg_macos::remove_installer(&dir, &ci.product.build).map_err(err)?;
                Ok(format!(
                    "Deleted macOS {} ({})",
                    ci.product.version, ci.product.build
                ))
            }
            Task::CreateVmFiles { disk_gib, download } => {
                let e = vm()?;
                let c = &e.config;
                ops.prepare_workspace(c).map_err(err)?;
                let w = Workspace::new(c.workspace_dir().map_err(err)?);
                if let Some(warning) = w.create_disk(u64::from(*disk_gib)).map_err(err)? {
                    log(&format!("[disk] {warning}"));
                    self.disk_warning(warning);
                }
                if *download {
                    ops.download_media(c, &c.version, cancel, &mut progress, &mut log)
                        .map_err(err)?;
                }
                Ok(format!("{} created", e.display_name()))
            }
            Task::SetupHost => {
                let user = std::env::var("USER")
                    .or_else(|_| std::env::var("LOGNAME"))
                    .map_err(|_| "cannot tell who you are: $USER is not set")?;
                let msg = ops.host.setup_host(&user).map_err(err)?;
                self.invalidate_checks();
                Ok(msg)
            }
            Task::ReserveGpu { addr } => {
                let g = ops
                    .host
                    .gpus()
                    .into_iter()
                    .find(|g| &g.addr == addr)
                    .ok_or_else(|| format!("no GPU at {addr}"))?;
                ops.host.reserve_gpu(&g).map_err(err)?;
                self.invalidate_checks();
                Ok(format!("Reserved {addr} for vfio-pci: reboot the host"))
            }
            Task::ReleaseGpu => {
                ops.host.release_gpu().map_err(err)?;
                self.invalidate_checks();
                Ok("Reservation removed: reboot the host".into())
            }
            Task::InstallReims => {
                let r = ops.reims().install(cancel, &|l| self.log(l));
                self.invalidate_checks();
                r.map_err(err)?;
                Ok("Reims vGPU installed".into())
            }
            Task::BuildReims => {
                let r = ops.reims().build(None, cancel, &|l| self.log(l));
                self.invalidate_checks();
                r.map_err(err)?;
                Ok("Reims vGPU built".into())
            }
            Task::Import { path, folder } => {
                if self.single.is_some() {
                    return Err("importing needs the VM list (not with --config)".into());
                }
                let e = if folder.is_empty() {
                    self.reg
                        .lock()
                        .import_settings(Path::new(path))
                        .map_err(err)?
                } else {
                    let dir = rg_core::Config {
                        workspace: folder.clone(),
                        ..rg_core::Config::default()
                    }
                    .workspace_dir()
                    .map_err(err)?;
                    // Its own connection: the copy takes minutes and the UI reads the list.
                    let reg = Registry::open(&self.app).map_err(err)?;
                    let mut last = u64::MAX;
                    reg.import_bundle(Path::new(path), &dir, cancel, |done, total| {
                        self.bytes(&mut last, "Importing", done, total)
                    })
                    .map_err(err)?
                };
                let _ = self.reg.lock().select(&e.id);
                self.emit(Event::VmsChanged);
                Ok(format!(
                    "{} imported into {}",
                    e.display_name(),
                    e.config.workspace
                ))
            }
            Task::Export { path, full } => {
                let e = vm()?;
                if *full {
                    let mut last = u64::MAX;
                    ops.export_bundle(&e, Path::new(path), cancel, |done, total| {
                        self.bytes(&mut last, "Exporting", done, total)
                    })
                    .map_err(err)?;
                } else {
                    let json = rg_core::registry::export_vm(&e).map_err(err)?;
                    std::fs::write(path, json + "\n").map_err(err)?;
                }
                Ok(format!("{} exported to {path}", e.display_name()))
            }
            Task::Move { folder } => {
                let e = vm()?;
                let moved = match &self.single {
                    Some(_) => return Err("moving needs the VM list (not with --config)".into()),
                    None => ops
                        .move_vm(&Registry::open(&self.app).map_err(err)?, &e, folder)
                        .map_err(err)?,
                };
                self.emit(Event::VmsChanged);
                Ok(format!(
                    "{} moved to {}",
                    moved.display_name(),
                    moved.config.workspace
                ))
            }
            Task::RemoveVm { id, delete_files } => {
                if self.single.is_some() {
                    return Err("this window shows one VM file (--config)".into());
                }
                let e = self.entry(id)?;
                // Its own connection: the UI keeps using the shared one meanwhile.
                let reg = Registry::open(&self.app).map_err(err)?;
                for f in ops.delete_vm(&reg, &e, *delete_files).map_err(err)? {
                    log(&format!("deleted {f}"));
                }
                self.seen.lock().remove(id);
                self.emit(Event::VmsChanged);
                Ok(format!("{} removed", e.display_name()))
            }
            Task::RemoveInstaller { build } => {
                rg_macos::remove_installer(&ops.installers_dir(), build).map_err(err)?;
                Ok(format!("Deleted the offline installer {build}"))
            }
            Task::InstallOffline { unattended } => {
                // The Unattended check is the user's consent to erase the disk and accept the license.
                let u = *unattended;
                ops.install_offline(&vm()?.config, u, u, u).map_err(err)?;
                Ok("Install line typed in the VM's Terminal".into())
            }
            Task::RootPatch { on } => {
                let e = vm()?;
                if *on {
                    rg_assets::download_root_patch_kexts(&ops.kext_cache(), cancel).map_err(err)?;
                }
                // Reread: Settings may have been saved during the download.
                let mut c = self.entry(&e.id)?.config;
                c.root_patch = *on;
                self.save_config(&e.id, &c)?;
                Ok(format!(
                    "Root-patch mode {} for {}",
                    if *on { "on" } else { "off" },
                    e.display_name()
                ))
            }
        }
    }

    // ---- VMs ----

    /// Starts `e` off the UI thread; the outcome arrives as [`Event::State`].
    fn start(self: &Arc<Self>, e: rg_core::VmEntry, verbose: bool) {
        let inner = self.clone();
        self.rt.spawn_blocking(move || {
            let id = e.id.clone();
            let mut c = e.config;
            c.verbose_boot = verbose;
            let (reims, in_app) = (c.reims(), c.in_app());
            let ops = inner.ops();
            let res = ops.start_vm(
                inner.rt.clone(),
                c,
                inner.x11_embed.load(Ordering::Relaxed),
                &inner.cancel,
                &mut |l| inner.log(l),
            );
            let run = match res {
                Ok(r) => r,
                Err(err) => {
                    inner.starting.lock().remove(&id);
                    inner.emit(Event::State {
                        id,
                        state: VmState::Stopped,
                        message: Some(err.to_string()),
                    });
                    return;
                }
            };
            let rg_ops::Running {
                vm,
                started,
                mut events,
                installer_command,
            } = run;
            inner.log(format!("[VM] QEMU started (pid {})", started.pid));
            if let Some(cmd) = installer_command {
                inner.log(format!(
                    "[VM] the full installer disc is attached; to install from it, type in the recovery's Terminal: {cmd}"
                ));
            }
            let vnc = started.vnc_socket.clone();
            inner.own.lock().insert(
                id.clone(),
                Own {
                    vm,
                    vnc: vnc.clone(),
                    installer_command: installer_command.map(str::to_owned),
                },
            );
            inner.set_seen(
                &id,
                Seen {
                    state: VmState::Running,
                    vnc: vnc.clone(),
                    pid: started.pid,
                    reims,
                    in_app,
                    ..Seen::default()
                },
            );
            inner.starting.lock().remove(&id);
            inner.emit(Event::State {
                id: id.clone(),
                state: VmState::Running,
                message: None,
            });
            let fwd = inner.clone();
            let pid = started.pid;
            inner.rt.spawn(async move {
                while let Some(ev) = events.recv().await {
                    match ev {
                        VmEvent::Log(l) => fwd.log(l),
                        VmEvent::State(s, m) => {
                            let state = vm_state(s);
                            if state == VmState::Stopped {
                                fwd.own.lock().remove(&id);
                                fwd.set_seen(&id, Seen::default());
                            } else {
                                fwd.set_seen(
                                    &id,
                                    Seen {
                                        state,
                                        vnc: vnc.clone(),
                                        pid,
                                        reims,
                                        in_app,
                                        ..Seen::default()
                                    },
                                );
                            }
                            fwd.emit(Event::State {
                                id: id.clone(),
                                state,
                                message: (!m.is_empty()).then_some(m),
                            });
                            if state == VmState::Stopped {
                                return;
                            }
                        }
                    }
                }
            });
        });
    }

    /// A VM started by another app instance, through its control socket (blocking QMP on a
    /// plain thread: rg-qemu's blocking calls make their own runtime).
    fn outside(self: &Arc<Self>, id: &str, what: &'static str) -> Result<(), String> {
        let e = self.entry(id)?;
        let s = self.seen(id);
        if s.state == VmState::Stopped {
            return Err(format!("{} is not running", e.display_name()));
        }
        if !s.control {
            return Err(format!(
                "{} was not started by this app: shut it down from macOS or where it was started",
                e.display_name()
            ));
        }
        let inner = self.clone();
        std::thread::spawn(move || {
            let ops = inner.ops();
            let r = if what == "quit" {
                ops.force_stop(&e.config)
            } else {
                ops.shutdown(&e.config)
            };
            if let Err(err) = r {
                inner.log(format!("error: {err}"));
            }
            inner.scan();
        });
        Ok(())
    }

    // ---- Preview ----

    fn preview(self: &Arc<Self>, id: &str) -> Option<PreviewUpdate> {
        let seen = self.seen(id);
        let mut slot = self.preview.lock();
        let Some(sock) = seen.vnc.filter(|_| seen.state != VmState::Stopped) else {
            *slot = None;
            return None;
        };
        let restart = match slot.as_ref() {
            None => true,
            Some(p) if p.id != id || p.sock != sock => true,
            Some(p) => p
                .shared
                .lock()
                .ended
                .is_some_and(|t| t.elapsed() > PREVIEW_RETRY),
        };
        if restart {
            *slot = Some(self.connect_preview(id, sock));
            return None;
        }
        let p = slot.as_ref()?;
        let mut pend = p.shared.lock();
        let vnc = pend.vnc.clone()?;
        if pend.full || pend.dirty.len() > MAX_DIRTY {
            pend.full = false;
            pend.dirty.clear();
            drop(pend);
            return Some(PreviewUpdate::Full(vnc.with_frame(image)));
        }
        if pend.dirty.is_empty() {
            return None;
        }
        let rects = std::mem::take(&mut pend.dirty);
        drop(pend);
        Some(PreviewUpdate::Dirty(vnc.with_frame(|f| {
            rects.iter().filter_map(|r| sub_image(f, r)).collect()
        })))
    }

    fn connect_preview(&self, id: &str, sock: PathBuf) -> Preview {
        let shared = Arc::new(Mutex::new(Pending::default()));
        let (s, path, ctx) = (shared.clone(), sock.clone(), self.ctx.get().cloned());
        let task = self.rt.spawn(async move {
            let run = async {
                let (vnc, reader) = Vnc::dial(&path).await?;
                s.lock().vnc = Some(vnc);
                let mut first = true;
                reader
                    .with_pace(preview_interval)
                    .run(|u| {
                        let mut p = s.lock();
                        if first || u.resized {
                            first = false;
                            p.full = true;
                            p.dirty.clear();
                        } else {
                            p.dirty.extend_from_slice(&u.dirty);
                        }
                        if let Some(c) = &ctx {
                            c.request_repaint();
                        }
                    })
                    .await
            };
            let _ = run.await;
            s.lock().ended = Some(Instant::now());
        });
        Preview {
            id: id.into(),
            sock,
            shared,
            task,
        }
    }
}

impl Backend for RealBackend {
    fn attach(&self, ctx: &egui::Context, x11_embed: bool) {
        let _ = self.inner.ctx.set(ctx.clone());
        self.inner.x11_embed.store(x11_embed, Ordering::Relaxed);
    }

    fn vms(&self) -> Vec<VmEntry> {
        let Ok(list) = self.inner.entries() else {
            return Vec::new();
        };
        let seen = self.inner.seen.lock();
        list.into_iter()
            .map(|e| {
                let s = seen.get(&e.id).cloned().unwrap_or_default();
                VmEntry {
                    id: e.id,
                    config: e.config,
                    state: s.state,
                    outside: s.outside,
                }
            })
            .collect()
    }

    fn selected(&self) -> Option<VmId> {
        let list = self.inner.entries().ok()?;
        if self.inner.single.is_some() {
            return list.first().map(|e| e.id.clone());
        }
        self.inner.reg.lock().selected(&list).map(|e| e.id)
    }

    fn select(&self, id: &str) {
        if self.inner.single.is_none()
            && let Err(e) = self.inner.reg.lock().select(id)
        {
            self.inner.log(format!("error: {e}"));
        }
    }

    fn save_config(&self, id: &str, config: &Config) -> Result<(), String> {
        self.inner.save_config(id, config)
    }

    fn create_vm(&self, config: &Config) -> Result<VmId, String> {
        if self.inner.single.is_some() {
            return Err("this window shows one VM file (--config): add VMs without it".into());
        }
        let mut c = config.clone();
        if c.networks.is_none() {
            c.networks = Some(rg_core::network::new_networks(&c.version));
        }
        let reg = self.inner.reg.lock();
        let e = reg.create(c).map_err(err)?;
        reg.select(&e.id).map_err(err)?;
        drop(reg);
        self.inner.emit(Event::VmsChanged);
        Ok(e.id)
    }

    fn recommended(&self) -> Config {
        let mem = self.inner.ops().host.mem_total_mib();
        Config::defaults().recommended(threads(), mem)
    }

    fn host_info(&self) -> HostInfo {
        HostInfo {
            cpus: threads(),
            mem_mib: u32::try_from(self.inner.ops().host.mem_total_mib()).unwrap_or(u32::MAX),
            bridges: rg_core::network::host_bridges(),
            bridge_helper: rg_core::network::bridge_helper().is_some(),
        }
    }

    fn random_mac(&self) -> String {
        rg_core::network::random_mac()
    }

    fn start(&self, id: &str, verbose: bool) -> Result<(), String> {
        let e = self.inner.entry(id)?;
        if self.inner.seen(id).state != VmState::Stopped
            || !self.inner.starting.lock().insert(id.into())
        {
            return Err(format!("{} is already running", e.display_name()));
        }
        self.inner
            .log(format!("[VM] starting {}", e.display_name()));
        self.inner.start(e, verbose);
        Ok(())
    }

    fn shutdown(&self, id: &str) -> Result<(), String> {
        let own = self.inner.own.lock().get(id).map(|o| o.vm.clone());
        let Some(vm) = own else {
            return self.inner.outside(id, "system_powerdown");
        };
        let inner = self.inner.clone();
        self.inner.rt.spawn(async move {
            if let Err(e) = vm.powerdown().await {
                inner.log(format!("error: {e}"));
            }
        });
        Ok(())
    }

    fn force_stop(&self, id: &str) -> Result<(), String> {
        let own = self.inner.own.lock().get(id).map(|o| o.vm.clone());
        match own {
            Some(vm) => vm.kill().map_err(err),
            None => self.inner.outside(id, "quit"),
        }
    }

    fn host_checks(&self, config: &Config) -> Vec<Check> {
        let ch = self.inner.checks(Some(config));
        if ch.key == check_key(config) {
            return ch.host.clone();
        }
        vec![Check {
            name: "Checking the host…".into(),
            ..Check::default()
        }]
    }

    fn reims_checks(&self, config: &Config) -> Vec<Check> {
        let ch = self.inner.checks(Some(config));
        if ch.key == check_key(config) {
            ch.reims.clone()
        } else {
            Vec::new()
        }
    }

    fn reims_status(&self) -> ReimsStatus {
        self.inner
            .checks(None)
            .reims_status
            .clone()
            .unwrap_or_else(|| ReimsStatus {
                installed: false,
                info: "Checking…".into(),
            })
    }

    fn gpus(&self) -> Vec<Gpu> {
        self.inner.checks(None).gpus.clone()
    }

    fn reserved_gpus(&self) -> String {
        self.inner.checks(None).reserved.clone()
    }

    fn install_status(&self, id: &str) -> InstallStatus {
        let ops = self.inner.ops();
        let boot_files = rg_assets::asset_versions(ops.assets_dir.as_deref());
        let Ok(e) = self.inner.entry(id) else {
            return InstallStatus {
                boot_files,
                ..InstallStatus::default()
            };
        };
        let c = &e.config;
        let Ok(dir) = c.workspace_dir() else {
            return InstallStatus {
                boot_files,
                ..InstallStatus::default()
            };
        };
        let w = Workspace::new(&dir);
        let used = |f: &str| gib(rg_core::workspace::allocated(&w.path(f)));
        InstallStatus {
            folder: w.exists().then(|| dir.display().to_string()),
            recovery: w.has_media().then(|| {
                format!(
                    "BaseSystem.img · {} · {:.2} GB",
                    w.media_version(),
                    used(rg_core::workspace::MEDIA_FILE)
                )
            }),
            disk: w.has_disk().then(|| {
                format!(
                    "mac_hdd_ng.img · {:.2} GB used on the host",
                    used(rg_core::workspace::DISK_FILE)
                )
            }),
            installer: rg_macos::cached_installer_for(&ops.installers_dir(), &c.version).map(
                |ci| {
                    format!(
                        "macOS {} ({}), {:.1} GB, cached for every VM of this version",
                        ci.product.version,
                        ci.product.build,
                        ci.product.size as f64 / 1e9
                    )
                },
            ),
            boot_files,
        }
    }

    fn installers(&self) -> Vec<Installer> {
        let dir = self.inner.ops().installers_dir();
        let vms = self.inner.entries().unwrap_or_default();
        rg_macos::cached_installers(&dir)
            .into_iter()
            .map(|ci| {
                // The VMs whose version installs from this build (the Go app's dialog).
                let used_by = vms
                    .iter()
                    .filter(|e| {
                        rg_macos::cached_installer_for(&dir, &e.config.version)
                            .is_some_and(|c| c.product.build == ci.product.build)
                    })
                    .map(|e| e.display_name().to_owned())
                    .collect();
                Installer {
                    version: ci.product.version,
                    build: ci.product.build,
                    size_gb: (ci.product.size as f64 / 1e9) as f32,
                    cached: true,
                    used_by,
                }
            })
            .collect()
    }

    fn about(&self) -> String {
        let ops = self.inner.ops();
        format!(
            "Boot files: {}",
            rg_assets::asset_versions(ops.assets_dir.as_deref())
        )
    }

    fn run(&self, id: Option<&str>, task: Task) {
        let inner = self.inner.clone();
        let id = id.map(str::to_owned);
        self.inner.rt.spawn_blocking(move || {
            let result = inner.task(id.as_deref(), &task);
            inner.emit(Event::TaskDone { task, result });
        });
    }

    fn poll_event(&self) -> Option<Event> {
        self.events
            .try_recv()
            .ok()
            .or_else(|| self.inner.overflow.lock().pop_front())
    }

    fn preview(&self, id: &str) -> Option<PreviewUpdate> {
        self.inner.preview(id)
    }

    fn set_screen_area(&self, area: ScreenArea) {
        *self.inner.screen.lock() = area;
    }

    fn installer_command(&self, id: &str) -> Option<(String, bool)> {
        let own = self.inner.own.lock();
        let o = own.get(id)?;
        Some((o.installer_command.clone()?, o.vnc.is_some()))
    }

    fn screen_source(&self, id: &str) -> Option<ScreenSource> {
        let s = self.inner.seen(id);
        if s.state == VmState::Stopped {
            return None;
        }
        let x11 = self.inner.x11_embed.load(Ordering::Relaxed);
        Some(if !s.in_app {
            ScreenSource::Standalone
        } else if s.reims {
            // Reims vGPU's window is on X11 when this app (or an X11 session) started it.
            if x11 && s.pid != 0 {
                ScreenSource::Window {
                    pid: s.pid,
                    reims: true,
                }
            } else {
                ScreenSource::Standalone
            }
        } else {
            // VMware SVGA: QEMU's VNC socket drawn in the tab (rg-ops never asks QEMU for its
            // SDL window, see `display` in rg-ops' start.rs).
            s.vnc.map_or(ScreenSource::Standalone, ScreenSource::Vnc)
        })
    }

    fn workspace_root(&self) -> String {
        self.inner.reg.lock().workspace_root()
    }

    fn set_workspace_root(&self, path: &str) -> Result<(), String> {
        let root = self.inner.reg.lock().workspace_root();
        if path.trim() == root {
            return Ok(());
        }
        // Validate the folder before saving it.
        new_ops(&self.inner.app, path, self.inner.host.as_ref()).map_err(err)?;
        self.inner
            .reg
            .lock()
            .set_workspace_root(path)
            .map_err(err)?;
        self.inner.ops(); // rebuilds them
        Ok(())
    }

    fn settings_file(&self) -> String {
        self.inner.app.settings_file().display().to_string()
    }

    fn open_settings_file(&self) {
        // Loading writes the commented default when the file is missing.
        let r = self
            .inner
            .app
            .load_settings()
            .map_err(err)
            .and_then(|_| xdg_open(&self.settings_file()));
        if let Err(e) = r {
            self.inner.log(format!("error: {e}"));
        }
    }

    fn reset_settings(&self) -> Result<(), String> {
        self.inner.app.reset_settings().map_err(err)
    }

    fn theme(&self) -> ThemeMode {
        let mut t = self.inner.reg.lock().preference(PREF_THEME);
        if t.is_empty() {
            t = self.inner.app.setting(MKG_THEME);
        }
        ThemeMode::ALL
            .iter()
            .map(|(m, _)| *m)
            .find(|m| m.as_str() == t.trim().to_lowercase())
            .unwrap_or_default()
    }

    fn set_theme(&self, mode: ThemeMode) {
        if let Err(e) = self
            .inner
            .reg
            .lock()
            .set_preference(PREF_THEME, mode.as_str())
        {
            self.inner.log(format!("error: {e}"));
        }
    }

    fn open_url(&self, url: &str) {
        if let Err(e) = xdg_open(url) {
            self.inner.log(format!("error: {e}"));
        }
    }
}

#[cfg(test)]
mod tests;
