//! The Go app's command lines, byte for byte: `testdata/buildargs-svga.golden` is the Go
//! repository's `internal/core/testdata` file; `buildargs-reims.golden` was written by the Go
//! `BuildArgs` with the same inputs as below.

use rg_core::config::{DISPLAY_WINDOW, GRAPHICS_REIMS, GRAPHICS_SVGA};
use rg_core::{Config, Nic};
use rg_qemu::{ArgsEnv, build_args};

fn golden(name: &str) -> std::io::Result<String> {
    std::fs::read_to_string(format!("{}/testdata/{name}", env!("CARGO_MANIFEST_DIR")))
}

fn env(audio: &str) -> ArgsEnv {
    ArgsEnv {
        workspace: "/vm/a,b".into(),
        qmp_socket: "/run/macos-kvm-gui-x/qmp.sock".into(),
        audio: audio.into(),
        ..ArgsEnv::default()
    }
}

#[test]
fn svga_matches_go() -> std::io::Result<()> {
    let mut c = Config {
        ssh_port: 2222,
        ..Config::defaults()
    };
    let app = build_args(&c, &env("none")).join("\n");
    c.graphics = GRAPHICS_SVGA.into();
    c.display = DISPLAY_WINDOW.into();
    let mut e = env("none");
    e.gpu_funcs = vec!["0000:03:00.0".into(), "0000:03:00.1".into()];
    let win = build_args(&c, &e).join("\n");
    assert_eq!(
        format!("{app}\n--\n{win}\n"),
        golden("buildargs-svga.golden")?
    );
    Ok(())
}

#[test]
fn reims_matches_go() -> std::io::Result<()> {
    let mut c = Config {
        ssh_port: 2222,
        ..Config::defaults()
    };
    c.graphics = GRAPHICS_REIMS.into();
    let mut e = env("pipewire");
    e.with_media = true;
    let a = build_args(&c, &e).join("\n");
    c.display = DISPLAY_WINDOW.into();
    c.ram_mib = 16384;
    c.resolution = "2560x1440".into();
    c.version = "tahoe".into();
    let nic = |model: &str, mac: &str| Nic {
        enabled: true,
        mode: "nat".into(),
        bridge: String::new(),
        model: model.into(),
        mac: mac.into(),
    };
    c.networks = Some(vec![
        nic("virtio-net-pci", "52:54:00:00:00:01"),
        nic("vmxnet3", "52:54:00:AA:00:02"),
    ]);
    let b = build_args(&c, &env("none")).join("\n");
    assert_eq!(format!("{a}\n--\n{b}\n"), golden("buildargs-reims.golden")?);
    Ok(())
}
