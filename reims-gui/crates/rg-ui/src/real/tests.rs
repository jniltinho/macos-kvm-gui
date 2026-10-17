//! [`RealBackend`] on temporary folders and a fake host.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rg_core::{AppDir, Registry};
use rg_host::Host;

use super::RealBackend;
use crate::backend::*;
use crate::theme::ThemeMode;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A host whose files live in `root` and whose commands answer like Ubuntu 24.04's QEMU.
fn fake_host(root: &Path) -> std::io::Result<Host> {
    let w = |p: PathBuf, s: &str| -> std::io::Result<()> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, s)
    };
    let h = Host {
        proc_root: root.join("proc"),
        sys_root: root.join("sys"),
        etc_root: root.join("etc"),
        dev_kvm: root.join("kvm"),
        dev_vfio: root.join("dev-vfio"),
        share_root: root.join("share"),
        dev_udmabuf: root.join("udmabuf"),
        look_path: Arc::new(|s| Some(PathBuf::from("/usr/bin").join(s))),
        output: Arc::new(|_, _| Ok(b"QEMU emulator version 8.2.2".to_vec())),
        run: Arc::new(|_, _| Ok(())),
        reims_qemu: Arc::new(|| Err("not installed".into())),
        extra_kexts: Arc::new(|| Ok(0)),
        settings_file: root.join("settings.yaml").to_string_lossy().into_owned(),
    };
    w(
        h.proc_root.join("cpuinfo"),
        "processor : 0\nflags\t\t: fpu vmx sse4_1 avx2\n",
    )?;
    w(h.proc_root.join("meminfo"), "MemTotal:       32768000 kB\n")?;
    w(h.dev_kvm.clone(), "")?;
    Ok(h)
}

/// A backend on `t`: config folder `t/app`, Workspace `t/VMs`.
fn backend(t: &tempfile::TempDir) -> Result<RealBackend, Box<dyn std::error::Error>> {
    let app = AppDir::at(t.path().join("app"));
    Registry::open(&app)?.set_workspace_root(&t.path().join("VMs").to_string_lossy())?;
    Ok(RealBackend::with_host(app, None, fake_host(t.path())?)?)
}

/// Polls events until `want` matches one (20 s at most).
fn wait(b: &RealBackend, mut want: impl FnMut(&Event) -> bool) -> Option<Event> {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        while let Some(e) = b.poll_event() {
            if want(&e) {
                return Some(e);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn creates_lists_saves_and_removes_vms() -> TestResult {
    let t = tempfile::tempdir()?;
    let b = backend(&t)?;
    // An empty registry gets the Go app's first VM.
    let vms = b.vms();
    assert_eq!(vms.len(), 1);
    assert_eq!(vms[0].config.name, "macOS Sonoma");
    assert_eq!(vms[0].state, VmState::Stopped);
    assert_eq!(b.selected(), Some(vms[0].id.clone()));

    let mut c = b.recommended();
    c.name = "Nilton".into();
    c.workspace = t.path().join("VMs/nilton").to_string_lossy().into_owned();
    let id = b.create_vm(&c)?;
    assert!(wait(&b, |e| *e == Event::VmsChanged).is_some());
    assert_eq!(b.vms().len(), 2);
    assert_eq!(b.selected(), Some(id.clone()));
    let nilton = b.vms().into_iter().find(|v| v.id == id).ok_or("no VM")?;
    assert!(nilton.config.networks.is_some(), "default adapters added");

    c.name = "Renamed".into();
    b.save_config(&id, &c)?;
    assert!(b.vms().iter().any(|v| v.config.name == "Renamed"));
    c.name = " ".into();
    assert!(b.save_config(&id, &c).is_err());

    b.select(&vms[0].id);
    assert_eq!(b.selected(), Some(vms[0].id.clone()));
    // Remove with purge runs off the UI thread and reports through events.
    let folder = t.path().join("VMs/nilton");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("mac_hdd_ng.img"), "disk")?;
    b.run(
        None,
        Task::RemoveVm {
            id: id.clone(),
            delete_files: true,
        },
    );
    let done = wait(&b, |e| matches!(e, Event::TaskDone { .. })).ok_or("no TaskDone")?;
    let Event::TaskDone { result, .. } = done else {
        return Err("unreachable".into());
    };
    assert!(result?.contains("removed"));
    assert_eq!(b.vms().len(), 1);
    assert!(!folder.exists());
    Ok(())
}

#[test]
fn workspace_changed_elsewhere_rebuilds_the_operations() -> TestResult {
    let t = tempfile::tempdir()?;
    let b = backend(&t)?;
    let c = b.vms()[0].config.clone();
    b.host_checks(&c);
    assert!(wait(&b, |e| *e == Event::VmsChanged).is_some());
    assert!(b.inner.checks.lock().fresh.is_some());
    // The Go app (or the CLI) sets another Workspace in the shared database.
    let other = t.path().join("Elsewhere");
    Registry::open(&AppDir::at(t.path().join("app")))?
        .set_workspace_root(&other.to_string_lossy())?;
    assert_eq!(b.inner.ops().workspace_root, other);
    assert!(
        b.inner.checks.lock().fresh.is_none(),
        "host facts recomputed"
    );
    Ok(())
}

#[test]
fn host_facts_load_in_the_background() -> TestResult {
    let t = tempfile::tempdir()?;
    let b = backend(&t)?;
    let c = b.vms()[0].config.clone();
    // The first call answers at once and starts the probes.
    assert_eq!(b.host_checks(&c)[0].status, Status::Unknown);
    assert!(wait(&b, |e| *e == Event::VmsChanged).is_some());
    let checks = b.host_checks(&c);
    let reims = b.reims_checks(&c);
    assert!(checks.iter().any(|ch| ch.name == "/dev/kvm"), "{checks:?}");
    assert!(!reims.is_empty());
    assert!(
        reims
            .iter()
            .all(|r| checks.iter().all(|ch| ch.name != r.name)),
        "the Reims checks are not repeated"
    );
    assert!(!b.reims_status().installed);
    assert!(b.gpus().is_empty(), "no GPUs in the fake sysfs");
    assert_eq!(b.reserved_gpus(), "");
    assert!(b.host_info().mem_mib > 30000);
    Ok(())
}

#[test]
fn tasks_and_starts_report_through_events() -> TestResult {
    let t = tempfile::tempdir()?;
    let b = backend(&t)?;
    let v = b.vms().remove(0);
    assert_eq!(b.install_status(&v.id).folder, None);
    assert!(!b.install_status(&v.id).boot_files.is_empty());

    b.run(Some(&v.id), Task::PrepareFolder);
    let done = wait(&b, |e| matches!(e, Event::TaskDone { .. })).ok_or("no TaskDone")?;
    let Event::TaskDone { task, result } = done else {
        return Err("unreachable".into());
    };
    assert_eq!(task, Task::PrepareFolder);
    assert!(result?.starts_with("VM folder ready"));
    assert!(b.install_status(&v.id).folder.is_some());
    assert_eq!(b.install_status(&v.id).disk, None);

    // No disk yet: the start fails off the UI thread and says why.
    b.start(&v.id, false)?;
    let ev = wait(&b, |e| matches!(e, Event::State { .. })).ok_or("no State")?;
    let Event::State { state, message, .. } = ev else {
        return Err("unreachable".into());
    };
    assert_eq!(state, VmState::Stopped);
    assert!(message.unwrap_or_default().contains("does not exist"));
    assert!(b.preview(&v.id).is_none());

    b.run(None, Task::DeleteInstaller);
    let done = wait(&b, |e| matches!(e, Event::TaskDone { .. })).ok_or("no TaskDone")?;
    assert!(matches!(done, Event::TaskDone { result: Err(_), .. }));
    assert!(b.installers().is_empty());
    Ok(())
}

#[test]
fn preferences_live_in_the_database_and_settings_yaml() -> TestResult {
    let t = tempfile::tempdir()?;
    let b = backend(&t)?;
    assert_eq!(b.theme(), ThemeMode::default());
    b.set_theme(ThemeMode::Light);
    assert_eq!(b.theme(), ThemeMode::Light);
    let app = AppDir::at(t.path().join("app"));
    assert_eq!(
        Registry::open(&app)?.preference(rg_core::registry::PREF_THEME),
        "light"
    );

    let other = t.path().join("Other").to_string_lossy().into_owned();
    b.set_workspace_root(&other)?;
    assert_eq!(b.workspace_root(), other);
    assert!(b.set_workspace_root("relative/path").is_err());

    assert_eq!(b.settings_file(), app.settings_file().display().to_string());
    b.reset_settings()?;
    assert!(app.settings_file().exists());
    assert!(b.about().starts_with("Boot files: "));
    Ok(())
}

#[test]
fn a_config_file_is_the_only_vm() -> TestResult {
    let t = tempfile::tempdir()?;
    let file = t.path().join("vm.json");
    let c = rg_core::Config {
        name: "File VM".into(),
        workspace: t.path().join("vm").to_string_lossy().into_owned(),
        ..rg_core::Config::defaults()
    };
    rg_core::config::save_config(&file, &c)?;
    let b = RealBackend::with_host(
        AppDir::at(t.path().join("app")),
        Some(&file),
        fake_host(t.path())?,
    )?;
    let vms = b.vms();
    assert_eq!(vms.len(), 1);
    assert_eq!(b.selected().as_deref(), Some("config"));
    let mut c2 = vms[0].config.clone();
    c2.ram_mib = 12288;
    b.save_config("config", &c2)?;
    assert_eq!(rg_core::config::load_config(&file)?.ram_mib, 12288);
    assert!(b.create_vm(&c).is_err());
    Ok(())
}

#[test]
fn preview_rects_are_cut_from_the_frame() {
    let f = rg_qemu::Frame {
        width: 4,
        height: 2,
        pixels: (0..32).map(|i| if i % 4 == 3 { 255 } else { i }).collect(),
    };
    let r = |x, y, w, h| rg_qemu::Rect { x, y, w, h };
    let (pos, img) = super::sub_image(&f, &r(1, 1, 2, 5)).unwrap_or_default();
    assert_eq!(pos, [1, 1]);
    assert_eq!(img.size, [2, 1]);
    assert_eq!(img.pixels[0].to_array(), [20, 21, 22, 255]);
    assert!(super::sub_image(&f, &r(4, 0, 1, 1)).is_none());
    assert_eq!(
        super::preview_interval(640, 480),
        Duration::from_millis(200)
    );
    assert_eq!(
        super::preview_interval(1920, 1080),
        Duration::from_nanos(912_384_000)
    );
}

#[test]
fn only_vms_started_here_are_embedded() -> TestResult {
    use std::sync::atomic::Ordering;
    let t = tempfile::tempdir()?;
    let b = backend(&t)?;
    b.inner.x11_embed.store(true, Ordering::Relaxed);
    let vnc = PathBuf::from("/run/vnc.sock");
    let source = |reims, outside| {
        b.inner.set_seen(
            "v",
            super::Seen {
                state: VmState::Running,
                outside,
                vnc: Some(vnc.clone()),
                pid: 42,
                reims,
                in_app: true,
                ..Default::default()
            },
        );
        b.screen_source("v")
    };
    let window = |reims| Some(ScreenSource::Window { pid: 42, reims });
    assert_eq!(source(true, false), window(true));
    assert_eq!(source(false, false), window(false));
    // Started by the CLI, MCP or the Go app: their window stays theirs.
    assert_eq!(source(true, true), Some(ScreenSource::Standalone));
    assert_eq!(source(false, true), Some(ScreenSource::Vnc(vnc.clone())));
    Ok(())
}
