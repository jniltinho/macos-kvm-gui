#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ffi::{CStr, CString, c_char};
use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};
use std::ptr::{null, null_mut};
use std::time::Duration;

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, PropMode, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

use super::*;

#[test]
fn the_header_declares_exactly_the_exported_functions() {
    let names = |text: &str, prefix: &str| -> Vec<String> {
        let mut v: Vec<String> = text
            .lines()
            .filter_map(|l| l.split_once(prefix).map(|(_, rest)| rest))
            .filter_map(|rest| rest.split('(').next())
            .map(|n| format!("rgx_{}", n.trim()))
            .collect();
        v.sort();
        v
    };
    let header = names(include_str!("../rg_x11.h"), " rgx_");
    let exported = names(include_str!("lib.rs"), "extern \"C\" fn rgx_");
    assert!(!exported.is_empty());
    assert_eq!(header, exported);
}

#[test]
fn null_arguments_are_refused() {
    let mut buf = [0 as c_char; 64];
    unsafe {
        assert_eq!(rgx_pump(null_mut(), buf.as_mut_ptr(), buf.len()), NULL);
        assert_eq!(rgx_is_alive(null()), 0);
        assert_eq!(rgx_release(null_mut(), null_mut(), 0), 0);
        rgx_embedder_free(null_mut());
        let mut w = 0;
        assert_eq!(
            rgx_find_window(null(), 1, 0, 1, &mut w, null_mut(), 0),
            NULL
        );
        assert_eq!(rgx_embedder_new(null(), 1, null_mut(), null_mut(), 0), NULL);
    }
}

#[test]
fn errors_come_back_in_the_caller_s_buffer() {
    let display = CString::new(":9999").unwrap();
    let mut out = null_mut();
    let mut buf = [0 as c_char; 16];
    let code =
        unsafe { rgx_embedder_new(display.as_ptr(), 1, &mut out, buf.as_mut_ptr(), buf.len()) };
    assert_eq!(code, ERR);
    assert!(out.is_null());
    let msg = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap();
    assert_eq!(
        msg.len(),
        15,
        "truncated to the buffer, NUL included: {msg:?}"
    );
    assert!(msg.starts_with("cannot connect"), "{msg}");
    assert_eq!(unsafe { rgx_probe(display.as_ptr()) }, 0);
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn xvfb() -> (Kill, String) {
    let mut c = Command::new("Xvfb")
        .args([
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
    (Kill(c), format!(":{}", line.trim()))
}

/// The whole C ABI against a real X server: a window titled like Reims vGPU's, owned by this
/// process's PID, is found, embedded, moved, hidden, shown and released back to the root.
#[test]
fn embed_through_the_c_abi() {
    let (_x, display) = xvfb();
    let (conn, screen) = (0..50)
        .find_map(|_| {
            x11rb::connect(Some(&display)).ok().or_else(|| {
                std::thread::sleep(Duration::from_millis(100));
                None
            })
        })
        .expect("X server");
    let root = conn.setup().roots[screen].root;
    let window = |parent, title: Option<&str>| {
        let w = conn.generate_id().unwrap();
        conn.create_window(
            0,
            w,
            parent,
            0,
            0,
            640,
            400,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap();
        if let Some(t) = title {
            let pid = conn
                .intern_atom(false, b"_NET_WM_PID")
                .unwrap()
                .reply()
                .unwrap()
                .atom;
            conn.change_property32(
                PropMode::REPLACE,
                w,
                pid,
                AtomEnum::CARDINAL,
                &[std::process::id()],
            )
            .unwrap();
            conn.change_property8(
                PropMode::REPLACE,
                w,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                t.as_bytes(),
            )
            .unwrap();
        }
        conn.map_window(w).unwrap();
        conn.sync().unwrap();
        w
    };
    let app = window(root, None);
    let guest = window(root, Some("Reims vGPU (port 2)"));

    let d = CString::new(display).unwrap();
    let mut buf = [0 as c_char; 256];
    let (b, n) = (buf.as_mut_ptr(), buf.len());
    unsafe {
        assert_eq!(rgx_probe(d.as_ptr()), 1);
        let mut e = null_mut();
        assert_eq!(rgx_embedder_new(d.as_ptr(), app, &mut e, b, n), 0);
        let mut found = 0;
        assert_eq!(
            rgx_find_window(e, std::process::id(), 0, 5000, &mut found, b, n),
            0
        );
        assert_eq!(found, guest);
        assert_eq!(
            rgx_find_window(e, std::process::id(), 7, 10, &mut found, b, n),
            ERR
        );
        let mut h = null_mut();
        assert_eq!(rgx_embed(e, guest, &mut h, b, n), 0);
        assert_eq!(rgx_set_rect(h, 10, 20, 320, 200, b, n), 0);
        assert_eq!(rgx_pump(h, b, n), 0);
        assert_eq!(rgx_is_alive(h), 1);
        let parent = |w| conn.query_tree(w).unwrap().reply().unwrap().parent;
        let container = parent(guest);
        assert_eq!(parent(container), app, "embedded under the app's window");
        assert_eq!(rgx_hide(h, b, n), 0);
        assert_eq!(rgx_show(h, b, n), 0);
        assert_eq!(rgx_release(h, b, n), 0);
        assert_eq!(parent(guest), root, "released back to the desktop");
        rgx_embedder_free(e);
    }
}
