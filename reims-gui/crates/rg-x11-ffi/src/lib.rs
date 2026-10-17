//! C ABI over [`rg_x11`] for the Go app's `internal/embed` (cgo). The header is `rg_x11.h`
//! next to this crate's `Cargo.toml`, written by hand; a test checks it declares exactly the
//! functions exported here.
//!
//! Contract:
//! - Handles are opaque pointers from `rgx_embedder_new` / `rgx_embed`, freed by
//!   `rgx_embedder_free` / `rgx_release`. Null handles are refused, never dereferenced.
//! - A fallible function returns 0 on success or a negative code, and writes its error message
//!   into `err` (at most `errlen` bytes, NUL-terminated). The error comes back from the same call
//!   because a Go goroutine may change OS threads between two cgo calls.
//! - Panics are caught at the boundary (code -2) and never unwind into the caller.
//! - An `RgxEmbedded` is not thread-safe: one thread (Go's UI goroutine) uses it.
//!
//! Codes: 0 ok, -1 error (message in `err`), -2 panic, -3 null argument.

use std::ffi::{CStr, c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use rg_x11::{Embedded, Embedder, WindowKind};

pub struct RgxEmbedder(Embedder);
pub struct RgxEmbedded(Embedded);

const ERR: c_int = -1;
const PANIC: c_int = -2;
const NULL: c_int = -3;

/// Copies `msg` into `err` (truncated, NUL-terminated) when the caller gave a buffer.
///
/// # Safety
/// `err` is null or valid for `errlen` bytes.
unsafe fn write_err(err: *mut c_char, errlen: usize, msg: &str) {
    if err.is_null() || errlen == 0 {
        return;
    }
    let n = msg.len().min(errlen - 1);
    // SAFETY: the caller guarantees `err` holds `errlen` bytes, and n < errlen.
    unsafe {
        std::ptr::copy_nonoverlapping(msg.as_ptr().cast::<c_char>(), err, n);
        *err.add(n) = 0;
    }
}

/// Runs `f`, turning an error or a panic into a code and a message.
///
/// # Safety
/// As [`write_err`].
unsafe fn guard(err: *mut c_char, errlen: usize, f: impl FnOnce() -> Result<(), String>) -> c_int {
    let (code, msg) = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => return 0,
        Ok(Err(m)) => (ERR, m),
        Err(_) => (PANIC, "rg-x11-ffi: panic".to_owned()),
    };
    // SAFETY: forwarded from the caller.
    unsafe { write_err(err, errlen, &msg) };
    code
}

/// `display` as an optional string (null = `$DISPLAY`).
///
/// # Safety
/// `display` is null or a valid NUL-terminated string.
unsafe fn opt_str(display: *const c_char) -> Result<Option<String>, String> {
    if display.is_null() {
        return Ok(None);
    }
    // SAFETY: non-null, NUL-terminated per the caller.
    let s = unsafe { CStr::from_ptr(display) };
    s.to_str()
        .map(|s| Some(s.to_owned()))
        .map_err(|_| "display is not UTF-8".to_owned())
}

/// 1 if an X server answers on `display` (null = `$DISPLAY`), else 0.
///
/// # Safety
/// `display` is null or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_probe(display: *const c_char) -> c_int {
    catch_unwind(|| {
        // SAFETY: forwarded from the caller.
        match unsafe { opt_str(display) } {
            Ok(d) => c_int::from(rg_x11::probe(d.as_deref())),
            Err(_) => 0,
        }
    })
    .unwrap_or(0)
}

/// Connects to `display` (null = `$DISPLAY`) with `parent` (the app's top-level X11 window).
///
/// # Safety
/// `display` as in [`rgx_probe`]; `out` valid for a write; `err` as in the module docs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_embedder_new(
    display: *const c_char,
    parent: u32,
    out: *mut *mut RgxEmbedder,
    err: *mut c_char,
    errlen: usize,
) -> c_int {
    if out.is_null() {
        return NULL;
    }
    // SAFETY: the closure only touches arguments the caller vouched for.
    unsafe {
        guard(err, errlen, || {
            let d = opt_str(display)?;
            let e = Embedder::new(d.as_deref(), parent).map_err(|e| e.to_string())?;
            *out = Box::into_raw(Box::new(RgxEmbedder(e)));
            Ok(())
        })
    }
}

/// Frees an embedder (null is ignored). Embedded windows it made stay embedded.
///
/// # Safety
/// `e` is null or from [`rgx_embedder_new`], not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_embedder_free(e: *mut RgxEmbedder) {
    if !e.is_null() {
        // SAFETY: from Box::into_raw, freed once.
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(e) })));
    }
}

/// Blocks (polling) up to `timeout_ms` for QEMU's window of `pid`: `kind` 0 = Reims vGPU's
/// window, 1 = QEMU's own window. Writes the window id to `out`.
///
/// # Safety
/// `e` from [`rgx_embedder_new`]; `out` valid for a write; `err` as in the module docs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_find_window(
    e: *const RgxEmbedder,
    pid: u32,
    kind: c_int,
    timeout_ms: u32,
    out: *mut u32,
    err: *mut c_char,
    errlen: usize,
) -> c_int {
    if e.is_null() || out.is_null() {
        return NULL;
    }
    // SAFETY: valid handle and out pointer per the caller.
    unsafe {
        guard(err, errlen, || {
            let kind = match kind {
                0 => WindowKind::Reims,
                1 => WindowKind::Qemu,
                k => return Err(format!("unknown window kind {k}")),
            };
            let w = (*e)
                .0
                .find_window(pid, kind, Duration::from_millis(timeout_ms.into()))
                .map_err(|e| e.to_string())?;
            *out = w;
            Ok(())
        })
    }
}

/// Reparents `child` into a container over the parent window; writes the handle to `out`.
///
/// # Safety
/// As [`rgx_find_window`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_embed(
    e: *const RgxEmbedder,
    child: u32,
    out: *mut *mut RgxEmbedded,
    err: *mut c_char,
    errlen: usize,
) -> c_int {
    if e.is_null() || out.is_null() {
        return NULL;
    }
    // SAFETY: valid handle and out pointer per the caller.
    unsafe {
        guard(err, errlen, || {
            let em = (*e).0.embed(child).map_err(|e| e.to_string())?;
            *out = Box::into_raw(Box::new(RgxEmbedded(em)));
            Ok(())
        })
    }
}

/// Runs `f` on a live handle.
///
/// # Safety
/// `h` is null or from [`rgx_embed`], not released; `err` as in the module docs.
unsafe fn with(
    h: *mut RgxEmbedded,
    err: *mut c_char,
    errlen: usize,
    f: impl FnOnce(&mut Embedded) -> Result<(), rg_x11::Error>,
) -> c_int {
    if h.is_null() {
        return NULL;
    }
    // SAFETY: valid, exclusively used handle per the caller.
    unsafe { guard(err, errlen, || f(&mut (*h).0).map_err(|e| e.to_string())) }
}

/// Moves and resizes the embedded window (physical pixels, relative to the parent).
///
/// # Safety
/// As `with`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_set_rect(
    h: *mut RgxEmbedded,
    x: i32,
    y: i32,
    w: u32,
    hgt: u32,
    err: *mut c_char,
    errlen: usize,
) -> c_int {
    // SAFETY: forwarded.
    unsafe { with(h, err, errlen, |e| e.set_rect(x, y, w, hgt)) }
}

/// Handles pending X events; never blocks. Call it every frame.
///
/// # Safety
/// As `with`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_pump(h: *mut RgxEmbedded, err: *mut c_char, errlen: usize) -> c_int {
    // SAFETY: forwarded.
    unsafe { with(h, err, errlen, Embedded::pump) }
}

/// Hides the embedded window (a dialog or menu over it, another tab).
///
/// # Safety
/// As `with`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_hide(h: *mut RgxEmbedded, err: *mut c_char, errlen: usize) -> c_int {
    // SAFETY: forwarded.
    unsafe { with(h, err, errlen, |e| e.hide()) }
}

/// Shows it again.
///
/// # Safety
/// As `with`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_show(h: *mut RgxEmbedded, err: *mut c_char, errlen: usize) -> c_int {
    // SAFETY: forwarded.
    unsafe { with(h, err, errlen, |e| e.show()) }
}

/// Gives the guest the keyboard (a click on it).
///
/// # Safety
/// As `with`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_focus(h: *mut RgxEmbedded, err: *mut c_char, errlen: usize) -> c_int {
    // SAFETY: forwarded.
    unsafe { with(h, err, errlen, |e| e.focus()) }
}

/// Gives the keyboard back to the app's window (a click on the app's UI).
///
/// # Safety
/// As `with`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_unfocus(
    h: *mut RgxEmbedded,
    err: *mut c_char,
    errlen: usize,
) -> c_int {
    // SAFETY: forwarded.
    unsafe { with(h, err, errlen, |e| e.unfocus()) }
}

/// A click on the app's own UI while the guest has the keyboard gives it back to the app
/// (call it every frame). Writes 1 to `moved` when it did, else 0.
///
/// # Safety
/// As `with`; `moved` valid for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_return_focus_on_app_click(
    h: *mut RgxEmbedded,
    moved: *mut c_int,
    err: *mut c_char,
    errlen: usize,
) -> c_int {
    if moved.is_null() {
        return NULL;
    }
    // SAFETY: forwarded; `moved` checked above.
    unsafe {
        with(h, err, errlen, |e| {
            *moved = c_int::from(e.return_focus_on_app_click()?);
            Ok(())
        })
    }
}

/// 1 while QEMU's window is alive and still in the container, 0 otherwise (or for null).
///
/// # Safety
/// `h` is null or from [`rgx_embed`], not released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_is_alive(h: *const RgxEmbedded) -> c_int {
    if h.is_null() {
        return 0;
    }
    // SAFETY: valid handle per the caller.
    catch_unwind(AssertUnwindSafe(|| {
        c_int::from(unsafe { (*h).0.is_alive() })
    }))
    .unwrap_or(0)
}

/// Puts a live window back on the desktop and frees the handle (null is a no-op returning 0).
///
/// # Safety
/// `h` is null or from [`rgx_embed`], not released yet; `err` as in the module docs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rgx_release(
    h: *mut RgxEmbedded,
    err: *mut c_char,
    errlen: usize,
) -> c_int {
    if h.is_null() {
        return 0;
    }
    // SAFETY: from Box::into_raw, released once.
    unsafe {
        let b = Box::from_raw(h);
        guard(err, errlen, || b.0.release().map_err(|e| e.to_string()))
    }
}

#[cfg(test)]
mod tests;
