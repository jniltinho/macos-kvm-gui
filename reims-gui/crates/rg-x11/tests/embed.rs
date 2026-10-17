//! Real windows under Xvfb: SDL3 windows titled like Reims vGPU's and like QEMU's `-display sdl`
//! window are found by pid, embedded, resized, fought over, focused, hidden and killed.
//! Needs Xvfb and the sdl_window example (built by `cargo test`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rg_x11::{Embedder, WindowKind, probe};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, InputFocus, MapState,
    PropMode, WindowClass,
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

/// One X test at a time: parallel Xvfb starts on a slow runner drop the first connections.
static SERIAL: Mutex<()> = Mutex::new(());

/// A running Xvfb; the test lock is released after the server is killed.
struct Xvfb {
    _server: Kill,
    _serial: MutexGuard<'static, ()>,
}

/// Connects, retrying while the server finishes starting (up to 5 s).
fn connect(display: &str) -> (RustConnection, usize) {
    for _ in 0..50 {
        if let Ok(c) = x11rb::connect(Some(display)) {
            return c;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    x11rb::connect(Some(display)).expect("X server")
}

/// Starts Xvfb on a free display and waits until it accepts connections; returns it and ":N".
fn xvfb() -> (Xvfb, String) {
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = Command::new("Xvfb")
        .args([
            // Without it the server resets when its last client leaves (the readiness probe),
            // refusing connections meanwhile.
            "-noreset",
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
    let display = format!(":{}", line.trim());
    drop(connect(&display));
    (
        Xvfb {
            _server: Kill(c),
            _serial: serial,
        },
        display,
    )
}

/// The app's top-level window.
fn app_window(display: &str) -> (RustConnection, u32) {
    let (conn, screen) = connect(display);
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
        WindowKind::Qemu
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
    let focus = || conn.get_input_focus().unwrap().reply().unwrap().focus;
    e.focus().unwrap();
    assert_eq!(focus(), win);
    e.unfocus().unwrap();
    assert_eq!(focus(), app);
    // The pointer entering QEMU's window gives it the keyboard.
    conn.warp_pointer(x11rb::NONE, app, 0, 0, 0, 0, 100, 100)
        .unwrap();
    conn.sync().unwrap();
    wait("focused on enter", || {
        e.pump().unwrap();
        focus() == win
    });

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
fn sdl_qemu_window() {
    scenario(WindowKind::Qemu, sdl_window, "QEMU (mac-test)");
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
fn second_instance_cannot_take_an_embedded_window() {
    let (_x, display) = xvfb();
    let (conn, app1) = app_window(&display);
    let (_conn2, app2) = app_window(&display);
    let child = sdl_window(&display, "Reims vGPU");
    let pid = child.0.id();
    let first = Embedder::new(Some(&display), app1).unwrap();
    let win = first
        .find_window(pid, WindowKind::Reims, Duration::from_secs(30))
        .unwrap();
    let mut e = first.embed(win).unwrap();
    e.set_rect(0, 0, 200, 100).unwrap();

    let second = Embedder::new(Some(&display), app2).unwrap();
    assert!(
        second
            .find_window(pid, WindowKind::Reims, Duration::from_millis(300))
            .is_err(),
        "found the window the first instance holds"
    );

    // Another client takes it anyway: the first lets go and does not take it back.
    conn.reparent_window(win, app2, 0, 0).unwrap();
    conn.sync().unwrap();
    wait("ownership lost", || {
        e.pump().unwrap();
        !e.is_alive()
    });
    e.release().unwrap();
    assert_eq!(geometry(&conn, win).1, app2);
}

#[test]
fn enter_gives_focus_only_while_the_app_is_active() {
    let (_x, display) = xvfb();
    let (conn, app) = app_window(&display);
    let root = conn.setup().roots[0].root;
    // Another application's window, beside the app's.
    let other = conn.generate_id().unwrap();
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        other,
        root,
        1100,
        0,
        100,
        100,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )
    .unwrap();
    conn.map_window(other).unwrap();
    conn.sync().unwrap();
    let child = sdl_window(&display, "Reims vGPU");
    let embedder = Embedder::new(Some(&display), app).unwrap();
    let win = embedder
        .find_window(child.0.id(), WindowKind::Reims, Duration::from_secs(30))
        .unwrap();
    let mut e = embedder.embed(win).unwrap();
    e.set_rect(0, 0, 640, 360).unwrap();
    wait("child configured", || {
        e.pump().unwrap();
        e.last_latency().is_some()
    });
    let focus = || conn.get_input_focus().unwrap().reply().unwrap().focus;
    let set_focus = |w| {
        conn.set_input_focus(InputFocus::PARENT, w, x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
    };
    // The pointer leaves the child and comes back over it.
    let reenter = |e: &mut rg_x11::Embedded| {
        conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 900, 600)
            .unwrap();
        conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 100, 100)
            .unwrap();
        conn.sync().unwrap();
        let until = Instant::now() + Duration::from_millis(400);
        while Instant::now() < until {
            e.pump().unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    // No window manager: the other application holds the keyboard.
    set_focus(other);
    reenter(&mut e);
    assert_eq!(focus(), other, "focus stolen from another application");

    // A window manager says the app is active.
    let active = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let set_active = |w: u32| {
        conn.change_property32(PropMode::REPLACE, root, active, AtomEnum::WINDOW, &[w])
            .unwrap()
            .check()
            .unwrap();
    };
    set_active(app);
    reenter(&mut e);
    assert_eq!(focus(), win, "the active app gives QEMU the keyboard");

    // ... and then that another application is.
    set_active(other);
    set_focus(other);
    reenter(&mut e);
    assert_eq!(focus(), other, "focus stolen from the active application");
}

#[test]
fn no_x_server() {
    assert!(!probe(Some(":9999")));
}
