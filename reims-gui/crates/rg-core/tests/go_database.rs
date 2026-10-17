//! Opens a database written by the Go app (schema 5) and round-trips it.
//!
//! `fixtures/go-schema5.db` was made by a throwaway Go program using `internal/core`: three VMs
//! (a legacy one without an adapter list, a Reims vGPU Tahoe one with two adapters, display
//! window and 2560x1440, and a root-patch Sequoia one with an empty adapter list), the selection
//! on `tahoe-lab`, theme `dark` and the Workspace `/srv/vms`.

use rg_core::registry::find_vm;
use rg_core::{AppDir, Registry};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn open_copy() -> Result<(tempfile::TempDir, AppDir), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let app = AppDir::at(tmp.path().join("macos-kvm-gui"));
    std::fs::create_dir_all(app.path())?;
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/go-schema5.db"),
        app.db_path(),
    )?;
    Ok((tmp, app))
}

#[test]
fn reads_the_go_database() -> TestResult {
    let (_tmp, app) = open_copy()?;
    let reg = Registry::open(&app)?;
    let vms = reg.list()?;
    let ids: Vec<_> = vms.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        ["kepler", "legacy-sonoma", "tahoe-lab"],
        "sorted by name"
    );

    let legacy = find_vm(&vms, "legacy-sonoma").ok_or("legacy")?;
    assert!(legacy.config.networks.is_none());
    assert_eq!(
        legacy.config.adapters()[0].mac,
        rg_core::network::LEGACY_MAC
    );

    let tahoe = find_vm(&vms, "Tahoe Lab").ok_or("tahoe")?;
    let c = &tahoe.config;
    assert!(c.reims() && c.reims_kext() && !c.in_app());
    assert_eq!(
        (c.ram_mib, c.cores, c.vcpus, c.ssh_port),
        (16384, 2, 4, 2223)
    );
    assert_eq!(c.reims_resolution(), "2560x1440");
    let nets = c.networks.as_deref().ok_or("networks")?;
    assert_eq!(nets.len(), 2);
    assert_eq!(
        (
            nets[1].mode.as_str(),
            nets[1].bridge.as_str(),
            nets[1].enabled
        ),
        ("bridged", "br0", false)
    );
    assert!(c.validate().is_ok());

    let kepler = find_vm(&vms, "kepler").ok_or("kepler")?;
    assert!(kepler.config.root_patch);
    assert_eq!(kepler.config.networks, Some(vec![]));

    assert_eq!(reg.selected(&vms).map(|e| e.id), Some("tahoe-lab".into()));
    assert_eq!(reg.preference("theme"), "dark");
    assert_eq!(reg.workspace_root(), "/srv/vms");
    Ok(())
}

#[test]
fn round_trips_the_go_database() -> TestResult {
    let (_tmp, app) = open_copy()?;
    let reg = Registry::open(&app)?;
    let before = reg.list()?;
    for e in &before {
        reg.save(e, e.config.clone())?;
    }
    let mut tahoe = find_vm(&before, "tahoe-lab").ok_or("tahoe")?.config.clone();
    tahoe.resolution = "3840x2160".into();
    reg.save(find_vm(&before, "tahoe-lab").ok_or("tahoe")?, tahoe.clone())?;
    drop(reg);

    let reg = Registry::open(&app)?;
    let after = reg.list()?;
    assert_eq!(after.len(), before.len());
    for (a, b) in after.iter().zip(&before) {
        if a.id == "tahoe-lab" {
            assert_eq!(a.config, tahoe);
        } else {
            assert_eq!(a, b);
        }
    }
    Ok(())
}
