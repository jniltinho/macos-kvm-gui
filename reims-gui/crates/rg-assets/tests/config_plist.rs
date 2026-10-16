//! config.plist must be byte-identical to the Go app's for the same inputs. The fixtures were
//! written by the Go `OpenCoreConfig` (internal/core) with the identity below; `keyboard.txt`
//! is the `RecoveryKeyboard()` the Go run used.

use rg_assets::{BootAssets, BootOptions, BootState, Identity, opencore_config};
use rg_core::Config;
use rg_core::ExtraKexts;
use rg_core::config::GRAPHICS_REIMS;

type R = Result<(), Box<dyn std::error::Error>>;

fn fixture(name: &str) -> std::io::Result<String> {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn identity() -> Identity {
    Identity {
        serial: "C02XK0ABJGH5".into(),
        mlb: "C02123456789ABCDE".into(),
        uuid: "0F5A3C2E-1B2D-4E6F-8A9B-0C1D2E3F4A5B".into(),
        rom: "a1b2c3d4e5f6".into(),
    }
}

fn check(name: &str, c: &Config, opts: BootOptions<'_>, state: &BootState) -> R {
    let kbd = fixture("keyboard.txt")?;
    let opts = BootOptions {
        keyboard: &kbd,
        ..opts
    };
    let got = opencore_config(&BootAssets::Embedded, c, &identity(), &opts, state)?;
    let want = fixture(&format!("{name}.plist"))?;
    assert!(got == want, "{name}: differs from the Go output");
    Ok(())
}

fn config(version: &str) -> Config {
    Config {
        version: version.into(),
        ..Config::defaults()
    }
}

#[test]
fn default_matches_go() -> R {
    check(
        "default",
        &Config::defaults(),
        BootOptions::default(),
        &BootState::default(),
    )
}

#[test]
fn install_media_verbose_gpu_matches_go() -> R {
    let c = Config {
        gpu: "0000:03:00.0".into(),
        ..Config::defaults()
    };
    let o = BootOptions {
        with_media: true,
        verbose_boot: true,
        ..Default::default()
    };
    check("install-verbose-gpu", &c, o, &BootState::default())
}

#[test]
fn root_patch_on_matches_go() -> R {
    let c = Config {
        root_patch: true,
        ..config("sequoia")
    };
    check(
        "rootpatch-on",
        &c,
        BootOptions::default(),
        &BootState::default(),
    )
}

#[test]
fn root_patch_clear_matches_go() -> R {
    let st = BootState {
        root_patch_was_on: true,
        ..Default::default()
    };
    check(
        "rootpatch-clear",
        &config("sequoia"),
        BootOptions::default(),
        &st,
    )
}

#[test]
fn tahoe_reims_with_extras_and_switches_matches_go() -> R {
    let c = Config {
        graphics: GRAPHICS_REIMS.into(),
        ..config("tahoe")
    };
    let ks = ExtraKexts {
        disabled: vec!["WhateverGreen".into(), "VirtualSMC".into()],
        ..Default::default()
    };
    let o = BootOptions {
        extras: Some(&ks),
        ..Default::default()
    };
    let st = BootState {
        reims_kext: true,
        extra_kexts: vec!["Foo".into(), "Bar".into()],
        ..Default::default()
    };
    check("tahoe-reims-extras", &c, o, &st)
}

#[test]
fn high_sierra_matches_go() -> R {
    check(
        "high-sierra",
        &config("high-sierra"),
        BootOptions::default(),
        &BootState::default(),
    )
}

#[test]
fn mojave_install_matches_go() -> R {
    let o = BootOptions {
        with_media: true,
        ..Default::default()
    };
    check("mojave-media", &config("mojave"), o, &BootState::default())
}
