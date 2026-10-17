//! kittest UI tests and snapshots (`tests/snapshots`). Regenerate with `UPDATE_SNAPSHOTS=1`.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::app::{App, Tab};
use crate::backend::{PreviewUpdate, Task, VmState};
use crate::dialogs::{Modal, Page};
use crate::fake::FakeBackend;
use crate::files::FileDialog;
use crate::screen::{EmbedState, Slot, Target, Why};
use crate::theme::ThemeMode;

fn harness(mode: ThemeMode, setup: impl FnOnce(&mut App)) -> Harness<'static, App> {
    let mut app = App::new(Box::new(FakeBackend::with_theme(mode)));
    setup(&mut app);
    let mut h = Harness::builder()
        .with_size(vec2(1280.0, 820.0))
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    h.run_steps(5);
    h
}

fn snap(name: &str, mode: ThemeMode, setup: impl FnOnce(&mut App)) {
    let mut h = harness(mode, setup);
    h.snapshot(name);
}

#[test]
fn snapshot_main_light() {
    snap("main_light", ThemeMode::Light, |_| {});
}

#[test]
fn snapshot_main_dark() {
    snap("main_dark", ThemeMode::Dark, |_| {});
}

#[test]
fn snapshot_host_light() {
    snap("host_light", ThemeMode::Light, |a| a.set_tab(Tab::Host));
}

#[test]
fn snapshot_host_dark() {
    snap("host_dark", ThemeMode::Dark, |a| a.set_tab(Tab::Host));
}

#[test]
fn snapshot_gpu_light() {
    snap("gpu_light", ThemeMode::Light, |a| a.set_tab(Tab::Gpu));
}

#[test]
fn snapshot_install_light() {
    snap("install_light", ThemeMode::Light, |a| {
        a.set_tab(Tab::Install)
    });
}

fn settings_display(a: &mut App) {
    if let Some(Modal::Settings(mut s)) = Modal::settings(a) {
        s.page = Page::Display;
        a.open(Modal::Settings(s));
    }
}

#[test]
fn snapshot_settings_display_light() {
    snap("settings_display_light", ThemeMode::Light, settings_display);
}

#[test]
fn snapshot_settings_display_dark() {
    snap("settings_display_dark", ThemeMode::Dark, settings_display);
}

#[test]
fn snapshot_wizard_light() {
    snap("wizard_light", ThemeMode::Light, |a| {
        a.open(Modal::new_vm(a))
    });
}

#[test]
fn snapshot_wizard_dark() {
    snap("wizard_dark", ThemeMode::Dark, |a| a.open(Modal::new_vm(a)));
}

#[test]
fn snapshot_preferences_light() {
    snap("preferences_light", ThemeMode::Light, |a| {
        a.open(Modal::preferences(a))
    });
}

#[test]
fn toolbar_start_runs_the_vm_and_shows_the_screen_tab() {
    let mut h = harness(ThemeMode::Light, |_| {});
    h.get_by_label("Start").click();
    h.run_steps(5);
    assert_eq!(h.state().state(), VmState::Running);
    assert_eq!(h.state().tab, Tab::Screen);
    assert!(h.query_by_label("Shut down").is_some());
}

#[test]
fn screen_area_is_reported_and_hidden_under_dialogs() {
    let mut h = harness(ThemeMode::Light, |a| a.set_tab(Tab::Screen));
    let area = h.state().screen_area;
    let r = area.rect.unwrap_or_default();
    assert!(r[2] > 600 && r[3] > 400, "{r:?}");
    assert!(!area.overlay_open);
    h.state_mut().open(Modal::info("Hi", "over the screen"));
    h.run_steps(5);
    assert!(h.state().screen_area.overlay_open);
    h.state_mut().set_tab(Tab::Details);
    h.run_steps(5);
    assert_eq!(h.state().screen_area.rect, None);
}

#[test]
fn settings_ok_saves_the_new_resolution() {
    let mut h = harness(ThemeMode::Light, settings_display);
    if let Some(Modal::Settings(s)) = h.state_mut().modals.last_mut() {
        s.c.resolution = "2560x1440".into();
    }
    h.run_steps(5);
    h.get_by_label("OK").click();
    h.run_steps(5);
    assert!(h.state().modals.is_empty());
    assert_eq!(h.state().backend.vms()[0].config.resolution, "2560x1440");
}

#[test]
fn wizard_defaults_sequoia_to_reims_and_refuses_an_empty_name() {
    let a = App::new(Box::new(FakeBackend::default()));
    let Modal::NewVm(mut w) = Modal::new_vm(&a) else {
        unreachable!()
    };
    assert!(!w.reims, "Sonoma keeps VMware SVGA");
    w.version = "sequoia".into();
    w.on_version();
    assert!(
        w.reims,
        "Sequoia defaults to Reims vGPU when the host can run it"
    );
    assert_eq!(
        w.collect(&[]).map(|_| ()),
        Err((0, "VM Name: give the VM a name".into()))
    );
    w.name = "Lab".into();
    let (c, disk) = w.collect(&[]).unwrap_or_else(|e| unreachable!("{e:?}"));
    assert!(c.reims() && disk == 80 && c.networks.as_ref().is_some_and(|n| n.len() == 1));
}

#[test]
fn escape_closes_the_top_dialog() {
    let mut h = harness(ThemeMode::Light, |a| a.open(Modal::preferences(a)));
    h.key_press(egui::Key::Escape);
    h.run_steps(5);
    assert!(h.state().modals.is_empty());
}

// ---- Screen tab (design D3, D4) ----

/// The VM `id` selected, running and on the Screen tab; `x11` = the app embeds QEMU's windows.
fn running_on_screen(a: &mut App, id: &str, x11: bool) {
    a.backend.attach(&egui::Context::default(), x11);
    a.cur = Some(id.into());
    let _ = a.backend.start(id, false);
    a.refresh_cache();
    a.set_tab(Tab::Screen);
}

/// A guest desktop for the VNC screen: a wallpaper gradient and a menu bar.
fn guest_frame() -> egui::ColorImage {
    let (w, h) = (640, 400);
    let mut img = egui::ColorImage::filled([w, h], egui::Color32::BLACK);
    for y in 0..h {
        for x in 0..w {
            img.pixels[y * w + x] = if y < 14 {
                egui::Color32::from_gray(236)
            } else {
                egui::Color32::from_rgb((30 + x * 140 / w) as u8, (50 + y * 90 / h) as u8, 170)
            };
        }
    }
    img
}

fn slot(state: EmbedState) -> Slot {
    Slot {
        target: Target {
            pid: 4001,
            reims: true,
        },
        state,
    }
}

#[test]
fn snapshot_screen_embedding_light() {
    // Reims vGPU on X11 while QEMU's window is looked for.
    snap("screen_embedding_light", ThemeMode::Light, |a| {
        running_on_screen(a, "1", true)
    });
}

#[test]
fn snapshot_screen_vnc_dark() {
    // VMware SVGA without X11: the VNC screen, letterboxed.
    snap("screen_vnc_dark", ThemeMode::Dark, |a| {
        running_on_screen(a, "2", false);
        a.screen
            .vnc
            .pending
            .push(PreviewUpdate::Full(guest_frame()));
    });
}

#[test]
fn snapshot_screen_wayland_light() {
    // Reims vGPU without X11: its window stays on the desktop and the tab says so.
    snap("screen_wayland_light", ThemeMode::Light, |a| {
        running_on_screen(a, "1", false)
    });
}

#[test]
fn snapshot_screen_not_found_dark() {
    snap("screen_not_found_dark", ThemeMode::Dark, |a| {
        running_on_screen(a, "1", true);
        a.screen.parent = Some(1);
        a.screen.embeds.slots.insert(
            "1".into(),
            slot(EmbedState::Standalone(Why::Failed(
                "no Reims window for pid 4001 within 30s".into(),
            ))),
        );
    });
}

#[test]
fn vnc_screen_grabs_the_keyboard_on_click() {
    let mut h = harness(ThemeMode::Light, |a| {
        running_on_screen(a, "2", false);
        a.screen
            .vnc
            .pending
            .push(PreviewUpdate::Full(guest_frame()));
    });
    h.run_steps(5);
    h.get_by_label("VM screen").click();
    h.run_steps(5);
    assert!(h.state().screen.vnc.grabbed);
}

#[test]
fn full_screen_hides_the_shell_and_comes_back() {
    let mut h = harness(ThemeMode::Light, |a| running_on_screen(a, "1", true));
    h.get_by_label("Full screen").click();
    h.run_steps(5);
    assert!(h.state().screen.full);
    assert!(h.query_by_label("New").is_none(), "toolbar hidden");
    let r = h.state().screen_area.rect.unwrap_or_default();
    assert!(
        r[2] >= 1260 && r[3] >= 760,
        "the screen fills the window: {r:?}"
    );
    h.get_by_label("Exit full screen").click();
    h.run_steps(5);
    assert!(!h.state().screen.full);
    assert!(h.query_by_label("New").is_some());
}

#[test]
fn show_in_a_window_detaches_and_comes_back() {
    let mut h = harness(ThemeMode::Light, |a| {
        running_on_screen(a, "1", true);
        a.screen.parent = Some(1);
        a.screen
            .embeds
            .slots
            .insert("1".into(), slot(EmbedState::Standalone(Why::Detached)));
        a.detached = true;
    });
    assert_eq!(h.state().screen_area.rect, None);
    h.get_by_label("Show it here").click();
    h.run_steps(5);
    assert!(!h.state().detached);
    assert!(h.state().screen_area.rect.is_some());
}

// ---- File dialogs ----

#[test]
fn snapshot_import_light() {
    snap("import_light", ThemeMode::Light, |a| {
        a.open(FileDialog::import());
        if let Some(Modal::File(d)) = a.modals.last_mut()
            && let FileDialog::Import { path, folder } = &mut **d
        {
            *path = "/home/nilton/Downloads/tahoe-lab.tar".into();
            *folder = "/home/nilton/VMs/tahoe-lab".into();
        }
    });
}

#[test]
fn snapshot_export_dark() {
    snap("export_dark", ThemeMode::Dark, |a| {
        let v = a.vms[0].clone();
        a.open(FileDialog::export(&v, "/home/nilton/Backups"));
    });
}

#[test]
fn snapshot_move_light() {
    snap("move_light", ThemeMode::Light, |a| {
        let v = a.vms[0].clone();
        a.open(FileDialog::move_vm(&v, "/mnt/fast/VMs"));
    });
}

#[test]
fn snapshot_installers_light() {
    snap("installers_light", ThemeMode::Light, |a| {
        a.open(FileDialog::installers(a))
    });
}

#[test]
fn snapshot_installers_dark() {
    snap("installers_dark", ThemeMode::Dark, |a| {
        a.open(FileDialog::installers(a))
    });
}

#[test]
fn snapshot_export_progress_light() {
    snap("export_progress_light", ThemeMode::Light, |a| {
        a.busy = Some(Task::Export {
            path: "/home/nilton/Backups/nilton.tar".into(),
            full: true,
        });
        a.progress = Some(("Copying the disk".into(), Some(0.42)));
        a.open(Modal::File(Box::new(FileDialog::Busy {
            title: "Exporting Nilton".into(),
            detail: "/home/nilton/Backups/nilton.tar".into(),
        })));
    });
}

fn settings_storage(a: &mut App) {
    if let Some(Modal::Settings(mut s)) = Modal::settings(a) {
        s.page = Page::Storage;
        a.open(Modal::Settings(s));
    }
}

#[test]
fn snapshot_settings_storage_light() {
    snap("settings_storage_light", ThemeMode::Light, settings_storage);
}

#[test]
fn storage_remove_asks_then_removes_the_media() {
    let mut h = harness(ThemeMode::Light, settings_storage);
    h.get_by_label("Remove…").click();
    h.run_steps(5);
    assert!(matches!(
        h.state().modals.as_slice(),
        [Modal::Confirm { .. }]
    ));
    h.get_by_label("OK").click();
    h.run_steps(5);
    assert!(
        h.state()
            .log
            .iter()
            .any(|l| l.contains("Remove install media"))
    );
}

#[test]
fn import_runs_the_task_and_selects_the_new_vm() {
    let dir = std::env::temp_dir().join(format!("rg-ui-import-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("lab.json");
    let _ = std::fs::write(&file, "{}");
    let mut h = harness(ThemeMode::Light, |a| a.open(FileDialog::import()));
    if let Some(Modal::File(d)) = h.state_mut().modals.last_mut()
        && let FileDialog::Import { path, .. } = &mut **d
    {
        *path = file.display().to_string();
    }
    h.run_steps(5);
    h.get_all_by_label("Import")
        .last()
        .unwrap_or_else(|| unreachable!())
        .click();
    h.run_steps(5);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(h.state().modals.is_empty(), "{:?}", h.state().modals);
    assert_eq!(h.state().current().map(|v| v.display_name()), Some("lab"));
}

#[test]
fn import_refuses_a_missing_file() {
    let mut h = harness(ThemeMode::Light, |a| a.open(FileDialog::import()));
    h.get_all_by_label("Import")
        .last()
        .unwrap_or_else(|| unreachable!())
        .click();
    h.run_steps(5);
    assert!(matches!(
        h.state().modals.last(),
        Some(Modal::Error { text, .. }) if text.starts_with("File:")
    ));
}

#[test]
fn move_points_the_vm_to_the_new_folder() {
    let mut h = harness(ThemeMode::Light, |a| {
        let v = a.vms[0].clone();
        a.open(FileDialog::move_vm(&v, "/mnt/fast/VMs"));
    });
    h.get_by_label("Move").click();
    h.run_steps(5);
    assert_eq!(
        h.state().current().map(|v| v.config.workspace.as_str()),
        Some("/mnt/fast/VMs/nilton")
    );
}

#[test]
fn installers_delete_asks_first() {
    let mut h = harness(ThemeMode::Light, |a| a.open(FileDialog::installers(a)));
    h.get_by_label("Delete").click();
    h.run_steps(5);
    assert_eq!(h.state().modals.len(), 2);
    h.get_by_label("OK").click();
    h.run_steps(5);
    assert!(
        h.state()
            .log
            .iter()
            .any(|l| l.contains("Delete offline installer"))
    );
}
