//! Real windows under Xvfb: an SDL3 window (Reims vGPU's toolkit) and a GTK3 window (QEMU's
//! -display gtk) are found by pid, embedded, resized, fought over, hidden and killed.
//! Needs Xvfb, the sdl_window example (built by `cargo test`) and python3-gi (see the Dockerfile).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rg_x11::{Embedder, WindowKind, probe};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, MapState, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts Xvfb on a free display; returns it and ":N".
fn xvfb() -> (Kill, String) {
    let mut c = Command::new("Xvfb")
        .args([
            "-displayfd",
            "1",
            "-screen",
            "0",
            "1280x800x24",
            "-nolisten",
            "tcp",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("Xvfb");
    let mut line = String::new();
    BufReader::new(c.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    (Kill(c), format!(":{}", line.trim()))
}

/// The app's top-level window.
fn app_window(display: &str) -> (RustConnection, u32) {
    let (conn, screen) = x11rb::connect(Some(display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let w = conn.generate_id().unwrap();
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        w,
        root,
        0,
        0,
        1000,
        700,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )
    .unwrap();
    conn.map_window(w).unwrap();
    conn.sync().unwrap();
    (conn, w)
}

fn sdl_window(display: &str, title: &str) -> Kill {
    // target/<profile>/deps/embed-xxx -> target/<profile>/examples/sdl_window
    let exe = std::env::current_exe().unwrap();
    let helper = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/sdl_window");
    let c = Command::new(helper)
        .arg(title)
        .env("DISPLAY", display)
        .env("SDL_VIDEO_DRIVER", "x11")
        .spawn()
        .expect("sdl_window example (run through cargo test)");
    Kill(c)
}

fn gtk_window(display: &str, title: &str) -> Kill {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/gtk_window.py");
    let c = Command::new("python3")
        .args([script, title])
        .env("DISPLAY", display)
        .env("GDK_BACKEND", "x11")
        .env("NO_AT_BRIDGE", "1")
        .spawn()
        .expect("python3");
    Kill(c)
}

/// Window geometry relative to its parent, and the parent.
fn geometry(conn: &RustConnection, w: u32) -> ((i32, i32, u32, u32), u32) {
    let g = conn.get_geometry(w).unwrap().reply().unwrap();
    let parent = conn.query_tree(w).unwrap().reply().unwrap().parent;
    (
        (g.x.into(), g.y.into(), g.width.into(), g.height.into()),
        parent,
    )
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn scenario(kind: WindowKind, spawn: fn(&str, &str) -> Kill, title: &str) {
    let (_x, display) = xvfb();
    assert!(probe(Some(&display)));
    let (conn, app) = app_window(&display);
    let mut child = spawn(&display, title);
    let embedder = Embedder::new(Some(&display), app).unwrap();

    // Found by pid and title while it is still starting up.
    let win = embedder
        .find_window(child.0.id(), kind, Duration::from_secs(30))
        .unwrap();
    let other = if kind == WindowKind::Reims {
        WindowKind::QemuGtk
    } else {
        WindowKind::Reims
    };
    assert!(
        embedder
            .find_window(child.0.id(), other, Duration::from_millis(200))
            .is_err()
    );
    assert!(
        embedder
            .find_window(1, kind, Duration::from_millis(200))
            .is_err()
    );

    let mut e = embedder.embed(win).unwrap();
    e.set_rect(10, 20, 640, 360).unwrap();
    wait("child configured", || {
        e.pump().unwrap();
        e.last_latency().is_some()
    });
    let (g, parent) = geometry(&conn, win);
    assert_eq!(g, (0, 0, 640, 360));
    let (cg, app_parent) = geometry(&conn, parent);
    assert_eq!((cg, app_parent), ((10, 20, 640, 360), app));

    // The child resizes itself (the library on a guest mode change): it is set back.
    conn.configure_window(win, &ConfigureWindowAux::new().width(300).height(200))
        .unwrap();
    conn.sync().unwrap();
    wait("child set back", || {
        e.pump().unwrap();
        geometry(&conn, win).0 == (0, 0, 640, 360)
    });

    // New rect (tab resized, HiDPI 1.5).
    let (x, y, w, h) = rg_x11::to_physical(10.0, 20.0, 400.0, 300.0, 1.5);
    e.set_rect(x, y, w, h).unwrap();
    wait("resized", || {
        e.pump().unwrap();
        geometry(&conn, win).0 == (0, 0, 600, 450)
    });

    let state = |w| {
        conn.get_window_attributes(w)
            .unwrap()
            .reply()
            .unwrap()
            .map_state
    };
    e.hide().unwrap();
    wait("hidden", || state(win) == MapState::UNVIEWABLE);
    e.show().unwrap();
    wait("shown", || state(win) == MapState::VIEWABLE);
    e.focus().unwrap();
    assert_eq!(conn.get_input_focus().unwrap().reply().unwrap().focus, win);

    // QEMU exits: its window goes with it.
    drop(std::mem::replace(
        &mut child,
        Kill(Command::new("true").spawn().unwrap()),
    ));
    wait("child destroyed", || {
        e.pump().unwrap();
        !e.is_alive()
    });
    e.release().unwrap();
}

#[test]
fn sdl3_reims_window() {
    scenario(WindowKind::Reims, sdl_window, "Reims vGPU (port 2)");
}

#[test]
fn gtk_qemu_window() {
    scenario(WindowKind::QemuGtk, gtk_window, "QEMU (mac-test)");
}

#[test]
fn release_puts_child_back_on_root() {
    let (_x, display) = xvfb();
    let (conn, app) = app_window(&display);
    let child = sdl_window(&display, "Reims vGPU");
    let embedder = Embedder::new(Some(&display), app).unwrap();
    let win = embedder
        .find_window(child.0.id(), WindowKind::Reims, Duration::from_secs(30))
        .unwrap();
    let root = conn.setup().roots[0].root;
    let mut e = embedder.embed(win).unwrap();
    e.set_rect(0, 0, 200, 100).unwrap();
    assert_ne!(geometry(&conn, win).1, root);
    e.release().unwrap();
    assert_eq!(geometry(&conn, win).1, root);
    let viewable = || {
        conn.get_window_attributes(win)
            .unwrap()
            .reply()
            .unwrap()
            .map_state
            == MapState::VIEWABLE
    };
    wait("standalone again", viewable);
}

#[test]
fn no_x_server() {
    assert!(!probe(Some(":9999")));
}
