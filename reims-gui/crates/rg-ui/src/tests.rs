//! kittest UI tests and snapshots (`tests/snapshots`). Regenerate with `UPDATE_SNAPSHOTS=1`.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::app::{App, Tab};
use crate::backend::VmState;
use crate::dialogs::{Modal, Page};
use crate::fake::FakeBackend;
use crate::theme::ThemeMode;

fn harness(mode: ThemeMode, setup: impl FnOnce(&mut App)) -> Harness<'static, App> {
    let mut app = App::new(Box::new(FakeBackend::with_theme(mode)));
    setup(&mut app);
    let mut h = Harness::builder()
        .with_size(vec2(1280.0, 820.0))
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    h.run();
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
    h.run();
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
    h.run();
    assert!(h.state().screen_area.overlay_open);
    h.state_mut().set_tab(Tab::Details);
    h.run();
    assert_eq!(h.state().screen_area.rect, None);
}

#[test]
fn settings_ok_saves_the_new_resolution() {
    let mut h = harness(ThemeMode::Light, settings_display);
    if let Some(Modal::Settings(s)) = h.state_mut().modals.last_mut() {
        s.c.resolution = "2560x1440".into();
    }
    h.run();
    h.get_by_label("OK").click();
    h.run();
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
    h.run();
    assert!(h.state().modals.is_empty());
}
