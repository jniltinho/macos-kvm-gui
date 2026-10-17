//! Embeds QEMU's display window (Reims vGPU's SDL3 window, or QEMU's GTK window for VMware SVGA)
//! into the app's Screen tab by X11 reparenting (design D3). See `API.md`.
//!
//! Each [`Embedded`] owns its own X connection, so its events (the child's ConfigureNotify and
//! DestroyNotify) reach only it; the UI calls [`Embedded::pump`] once per frame, which never blocks.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux,
    EventMask, InputFocus, KeyButMask, MapState, PropMode, SetMode, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot connect to the X server: {0}")]
    Connect(#[from] x11rb::errors::ConnectError),
    #[error("X11 connection error: {0}")]
    Connection(#[from] x11rb::errors::ConnectionError),
    #[error("X11 request failed: {0}")]
    Reply(#[from] x11rb::errors::ReplyError),
    #[error("X11 id allocation failed: {0}")]
    Id(#[from] x11rb::errors::ReplyOrIdError),
    #[error("no {kind:?} window for pid {pid} within {timeout:?}")]
    NotFound {
        pid: u32,
        kind: WindowKind,
        timeout: Duration,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Which QEMU display window to look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// Reims vGPU's SDL3 window: title starts with "Reims vGPU" (the library changes the rest,
    /// ports append " (port N)").
    Reims,
    /// QEMU's own window (`-display gtk`, VMware SVGA): "QEMU" or "QEMU (<vm name>)", with
    /// " [Stopped]" or grab hints appended.
    Qemu,
}

impl WindowKind {
    pub fn matches(self, title: &str) -> bool {
        match self {
            WindowKind::Reims => title.starts_with("Reims vGPU"),
            WindowKind::Qemu => title.starts_with("QEMU"),
        }
    }
}

/// Whether an X server is reachable (`None` = `$DISPLAY`).
pub fn probe(display: Option<&str>) -> bool {
    x11rb::connect(display).is_ok()
}

/// The app's top-level X11 window (from raw-window-handle's Xlib/Xcb handle).
pub struct Embedder {
    display: Option<String>,
    conn: RustConnection,
    root: Window,
    parent: Window,
    atoms: [u32; 6],
}

/// Set on every container the embedder creates: `find_window` never looks inside one, so a
/// second instance of the app cannot take a window another instance holds.
const CONTAINER_MARK: &[u8] = b"_REIMS_GUI_CONTAINER";

/// How long `find_window` waits for a mapped `Qemu` window before taking an unmapped one.
const UNMAPPED_GRACE: Duration = Duration::from_secs(2);
/// How long `embed` waits for a reparenting window manager to give an unmapped window back.
const WM_RELEASE: Duration = Duration::from_secs(1);

impl Embedder {
    /// Connects to `display` (`None` = `$DISPLAY`) and checks that `parent` exists.
    pub fn new(display: Option<&str>, parent: u32) -> Result<Self> {
        let (conn, screen) = x11rb::connect(display)?;
        let root = conn.setup().roots[screen].root;
        conn.get_window_attributes(parent)?.reply()?;
        let mut atoms = [0; 6];
        for (a, name) in atoms.iter_mut().zip([
            &b"_NET_WM_PID"[..],
            b"_NET_WM_NAME",
            b"UTF8_STRING",
            b"_NET_WM_STATE",
            CONTAINER_MARK,
            b"_NET_ACTIVE_WINDOW",
        ]) {
            *a = conn.intern_atom(false, name)?.reply()?.atom;
        }
        Ok(Self {
            display: display.map(str::to_owned),
            conn,
            root,
            parent,
            atoms,
        })
    }

    /// Polls (every 50 ms, blocking the caller) for a mapped window with `_NET_WM_PID == pid`
    /// whose title matches `kind`. Searches the whole tree, so window-manager frames are fine,
    /// but never inside a container of an embedder (this app's or another instance's).
    /// QEMU's display may keep its window unmapped when it has no text consoles (the app runs
    /// it with `-serial none -parallel none`), so after a short grace an unmapped `Qemu`
    /// window is taken too: embedding maps it.
    pub fn find_window(&self, pid: u32, kind: WindowKind, timeout: Duration) -> Result<u32> {
        let start = Instant::now();
        let deadline = start + timeout;
        loop {
            let unmapped_ok = kind == WindowKind::Qemu && start.elapsed() >= UNMAPPED_GRACE;
            if let Some(w) = self.scan(self.root, pid, kind, unmapped_ok)? {
                return Ok(w);
            }
            if Instant::now() >= deadline {
                return Err(Error::NotFound { pid, kind, timeout });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn scan(
        &self,
        w: Window,
        pid: u32,
        kind: WindowKind,
        unmapped_ok: bool,
    ) -> Result<Option<Window>> {
        // Windows may vanish between QueryTree and the next request: treat errors as "not it".
        let Ok(tree) = self.conn.query_tree(w)?.reply() else {
            return Ok(None);
        };
        for &c in &tree.children {
            if self.is_container(c)? {
                continue;
            }
            if self.is_target(c, pid, kind, unmapped_ok)? {
                return Ok(Some(c));
            }
            if let Some(found) = self.scan(c, pid, kind, unmapped_ok)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn is_container(&self, w: Window) -> Result<bool> {
        Ok(self
            .conn
            .get_property(false, w, self.atoms[4], AtomEnum::ANY, 0, 0)?
            .reply()
            .is_ok_and(|p| p.type_ != u32::from(AtomEnum::NONE)))
    }

    fn is_target(&self, w: Window, pid: u32, kind: WindowKind, unmapped_ok: bool) -> Result<bool> {
        let Ok(p) = self
            .conn
            .get_property(false, w, self.atoms[0], AtomEnum::CARDINAL, 0, 1)?
            .reply()
        else {
            return Ok(false);
        };
        if p.value32().and_then(|mut v| v.next()) != Some(pid) {
            return Ok(false);
        }
        let Ok(attrs) = self.conn.get_window_attributes(w)?.reply() else {
            return Ok(false);
        };
        let shown = attrs.map_state == MapState::VIEWABLE
            || (unmapped_ok && attrs.map_state == MapState::UNMAPPED);
        Ok(shown && kind.matches(&self.title(w)?))
    }

    fn title(&self, w: Window) -> Result<String> {
        let (name, utf8) = (self.atoms[1], self.atoms[2]);
        for (prop, ty) in [
            (name, utf8),
            (AtomEnum::WM_NAME.into(), AtomEnum::ANY.into()),
        ] {
            if let Ok(r) = self.conn.get_property(false, w, prop, ty, 0, 1024)?.reply()
                && !r.value.is_empty()
            {
                return Ok(String::from_utf8_lossy(&r.value).into_owned());
            }
        }
        Ok(String::new())
    }

    /// Creates a container child of the parent window (unmapped, 1x1 at 0,0), reparents `child`
    /// into it at (0,0) and maps both. Call [`Embedded::set_rect`] right after.
    pub fn embed(&self, child: u32) -> Result<Embedded> {
        let (conn, _) = x11rb::connect(self.display.as_deref())?;
        let container = conn.generate_id()?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            container,
            self.parent,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            // No background: the server never clears it, so resizes do not flash.
            &CreateWindowAux::new().background_pixmap(x11rb::NONE),
        )?
        .check()?;
        conn.change_property32(
            PropMode::REPLACE,
            container,
            self.atoms[4],
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?
        .check()?;
        // Save-set: if the app dies, the server puts the child back on the root instead of
        // destroying it with the container.
        conn.change_save_set(SetMode::INSERT, child)?.check()?;
        conn.change_window_attributes(
            child,
            // EnterWindow: the pointer entering QEMU's window gives it the keyboard (see `pump`);
            // ButtonPress cannot be shared with SDL, so a click cannot be watched instead.
            // StructureNotify also reports another client reparenting it away (ownership lost).
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::ENTER_WINDOW),
        )?
        .check()?;
        // FocusChange on the app's window (this connection's own mask; egui's is untouched): a
        // click on QEMU's window makes Mutter focus the app's toplevel, see `pump`.
        conn.change_window_attributes(
            self.parent,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::FOCUS_CHANGE),
        )?
        .check()?;
        // Unmap first so a window manager lets go of it (its frame goes away), and drop any
        // fullscreen/maximized state it set, so a later release does not bring it back.
        let framed = conn.query_tree(child)?.reply()?.parent != self.root;
        conn.unmap_window(child)?;
        conn.delete_property(child, self.atoms[3])?;
        conn.flush()?;
        // A reparenting window manager (Mutter frames XWayland windows) puts the window back on
        // the root when it lets go, asynchronously: reparenting before that, it would take the
        // window back out of the container.
        let deadline = Instant::now() + WM_RELEASE;
        while framed
            && conn.query_tree(child)?.reply()?.parent != self.root
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        conn.reparent_window(child, container, 0, 0)?.check()?;
        conn.map_window(child)?.check()?;
        conn.map_window(container)?.check()?;
        // Events so far come from the move into the container, not from the child's life.
        let mut alive = true;
        while let Some(ev) = conn.poll_for_event()? {
            match ev {
                Event::DestroyNotify(e) if e.window == child => alive = false,
                Event::ReparentNotify(e) if e.window == child => alive = e.parent == container,
                _ => {}
            }
        }
        Ok(Embedded {
            conn,
            active_atom: self.atoms[5],
            root: self.root,
            parent: self.parent,
            container,
            child,
            rect: None,
            size: (1, 1),
            sent: None,
            latency: None,
            alive,
            released: false,
        })
    }
}

/// QEMU's window inside the app. Dropping it releases the child (see [`Embedded::release`]).
pub struct Embedded {
    conn: RustConnection,
    active_atom: u32,
    root: Window,
    parent: Window,
    container: Window,
    child: Window,
    rect: Option<(i32, i32, u32, u32)>,
    size: (u32, u32),
    /// When the last child configure was sent, until its ConfigureNotify arrives.
    sent: Option<Instant>,
    latency: Option<Duration>,
    alive: bool,
    released: bool,
}

impl Embedded {
    pub fn child(&self) -> u32 {
        self.child
    }

    /// Moves/resizes the container to the tab's rectangle (physical pixels, relative to the
    /// parent; see [`to_physical`]) and the child to fill it; the size is kept and re-applied by
    /// [`Embedded::pump`]. Call it every frame: it sends nothing when the rect is unchanged.
    pub fn set_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        // X rejects zero sizes.
        let (w, h) = (w.max(1), h.max(1));
        if self.rect == Some((x, y, w, h)) {
            return Ok(());
        }
        self.rect = Some((x, y, w, h));
        self.size = (w, h);
        self.conn.configure_window(
            self.container,
            &ConfigureWindowAux::new().x(x).y(y).width(w).height(h),
        )?;
        if self.alive {
            self.configure_child()?;
        }
        self.conn.flush()?;
        Ok(())
    }

    fn configure_child(&mut self) -> Result<()> {
        let (w, h) = self.size;
        self.sent = Some(Instant::now());
        self.conn.configure_window(
            self.child,
            &ConfigureWindowAux::new().x(0).y(0).width(w).height(h),
        )?;
        Ok(())
    }

    /// Processes pending X events without blocking: a child that moved or resized itself (the
    /// library on a guest mode change) is configured back; DestroyNotify marks it dead; the
    /// pointer entering the child gives it the keyboard (no window manager does it for a child
    /// window, and SDL's ButtonPress selection is exclusive, so clicks cannot be watched), but
    /// only while the app is the active window ([`Embedded::app_active`]), so hovering it while
    /// another application is in front never steals the keyboard. A child that another client
    /// reparented away is lost: marked not alive and never taken back.
    /// A storm of ConfigureNotify events is coalesced: only the last one counts.
    pub fn pump(&mut self) -> Result<()> {
        let mut last = None;
        while let Some(ev) = self.conn.poll_for_event()? {
            match ev {
                Event::ConfigureNotify(e) if e.window == self.child => {
                    let geom = (e.x, e.y, u32::from(e.width), u32::from(e.height));
                    let (w, h) = self.size;
                    if geom == (0, 0, w, h)
                        && let Some(t) = self.sent.take()
                    {
                        self.latency = Some(t.elapsed());
                    }
                    last = Some(geom);
                }
                Event::DestroyNotify(e) if e.window == self.child => self.alive = false,
                Event::ReparentNotify(e)
                    if e.window == self.child && e.parent != self.container =>
                {
                    self.alive = false;
                }
                Event::EnterNotify(e)
                    if e.event == self.child && self.alive && self.app_active() =>
                {
                    // Best effort: it may be unmapping right now.
                    let _ = self
                        .conn
                        .set_input_focus(InputFocus::PARENT, self.child, CURRENT_TIME);
                    self.conn.flush()?;
                }
                // A window manager answers a click on QEMU's window by focusing the app's
                // toplevel (Mutter calls XSetInputFocus on click-to-focus), taking the keyboard
                // back from QEMU: with the pointer over QEMU's window, the click was meant for it.
                // Only while the app is still the active one: a FocusIn queued before the user
                // switched to another application must not pull the keyboard back.
                Event::FocusIn(e)
                    if e.event == self.parent
                        && self.alive
                        && self.pointer_on_child()
                        && self.app_active() =>
                {
                    let _ = self
                        .conn
                        .set_input_focus(InputFocus::PARENT, self.child, CURRENT_TIME);
                    self.conn.flush()?;
                }
                // Errors from fire-and-forget requests on a child that just died.
                _ => {}
            }
        }
        let (w, h) = self.size;
        if self.alive && last.is_some_and(|g| g != (0, 0, w, h)) {
            self.configure_child()?;
            self.conn.flush()?;
        }
        Ok(())
    }

    /// Whether the app's window is the active one: `_NET_ACTIVE_WINDOW` on the root when a
    /// window manager sets it, else the keyboard focus being inside the app's window.
    pub fn app_active(&self) -> bool {
        let active = self
            .conn
            .get_property(false, self.root, self.active_atom, AtomEnum::WINDOW, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .filter(|p| p.type_ != u32::from(AtomEnum::NONE));
        if let Some(p) = active {
            return p.value32().and_then(|mut v| v.next()) == Some(self.parent);
        }
        let Some(mut w) = self
            .conn
            .get_input_focus()
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|f| f.focus)
        else {
            return false;
        };
        // 0 = None, 1 = PointerRoot: no window of the app holds the keyboard.
        while w > 1 && w != self.root {
            if w == self.parent {
                return true;
            }
            let Some(t) = self.conn.query_tree(w).ok().and_then(|c| c.reply().ok()) else {
                return false;
            };
            w = t.parent;
        }
        false
    }

    /// Whether the pointer is over QEMU's window (and it is shown).
    fn pointer_on_child(&self) -> bool {
        let (w, h) = self.size;
        self.conn
            .query_pointer(self.child)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|p| {
                p.same_screen
                    && (0..i32::try_from(w).unwrap_or(i32::MAX)).contains(&i32::from(p.win_x))
                    && (0..i32::try_from(h).unwrap_or(i32::MAX)).contains(&i32::from(p.win_y))
            })
            && self
                .conn
                .get_window_attributes(self.container)
                .ok()
                .and_then(|c| c.reply().ok())
                .is_some_and(|a| a.map_state == MapState::VIEWABLE)
    }

    /// A click on the app's own UI while the guest has the keyboard: if the keyboard is in the
    /// guest, a pointer button is held, and the pointer is over the app's window but not over the
    /// guest, gives the keyboard to the app's window. For hosts that cannot see their own clicks
    /// (the Go app: GLFW holds the window's ButtonPress selection, which X11 grants to one client
    /// only); call it every frame. Returns whether it moved the keyboard.
    pub fn return_focus_on_app_click(&self) -> Result<bool> {
        if !self.alive || self.conn.get_input_focus()?.reply()?.focus != self.child {
            return Ok(false);
        }
        let p = self.conn.query_pointer(self.parent)?.reply()?;
        let buttons = KeyButMask::BUTTON1 | KeyButMask::BUTTON2 | KeyButMask::BUTTON3;
        if !p.same_screen || u16::from(p.mask) & u16::from(buttons) == 0 {
            return Ok(false);
        }
        let g = self.conn.get_geometry(self.parent)?.reply()?;
        let inside = (0..i32::from(g.width)).contains(&i32::from(p.win_x))
            && (0..i32::from(g.height)).contains(&i32::from(p.win_y));
        if !inside || self.pointer_on_child() {
            return Ok(false);
        }
        self.unfocus()?;
        Ok(true)
    }

    /// Unmaps the container (dialogs over the tab, tab switch); focus reverts to the app.
    pub fn hide(&self) -> Result<()> {
        self.conn.unmap_window(self.container)?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn show(&self) -> Result<()> {
        self.conn.map_window(self.container)?;
        self.conn.flush()?;
        Ok(())
    }

    /// Gives the child keyboard focus (on a click in the tab). Fails if it is not viewable.
    pub fn focus(&self) -> Result<()> {
        self.conn
            .set_input_focus(InputFocus::PARENT, self.child, CURRENT_TIME)?
            .check()?;
        Ok(())
    }

    /// Gives the keyboard back to the app's window (a click on the app outside the tab).
    pub fn unfocus(&self) -> Result<()> {
        self.conn
            .set_input_focus(InputFocus::PARENT, self.parent, CURRENT_TIME)?
            .check()?;
        Ok(())
    }

    /// Time from the last configure of the child to its ConfigureNotify (diagnostics).
    pub fn last_latency(&self) -> Option<Duration> {
        self.latency
    }

    /// False once QEMU's window is destroyed (QEMU exited) or another client took it (reparented
    /// it elsewhere). Updated by [`Embedded::pump`].
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Puts a live child back on the root as a standalone window and destroys the container.
    pub fn release(mut self) -> Result<()> {
        self.do_release()
    }

    fn do_release(&mut self) -> Result<()> {
        if std::mem::replace(&mut self.released, true) {
            return Ok(());
        }
        if std::env::var_os("RG_DEBUG").is_some() {
            eprintln!(
                "[rg-x11] release child {:#x} (alive {})",
                self.child, self.alive
            );
        }
        self.pump()?;
        if self.alive {
            self.alive = false;
            self.conn.unmap_window(self.child)?;
            self.conn.reparent_window(self.child, self.root, 0, 0)?;
            self.conn.change_save_set(SetMode::DELETE, self.child)?;
            self.conn.map_window(self.child)?;
        }
        self.conn.destroy_window(self.container)?;
        self.conn.sync()?;
        Ok(())
    }
}

impl Drop for Embedded {
    fn drop(&mut self) {
        // Best effort: the connection may already be gone.
        let _ = self.do_release();
    }
}

/// Converts a rect in egui points to X physical pixels at `pixels_per_point`, rounding the
/// edges (not the size) so adjacent rects never leave a gap or overlap.
pub fn to_physical(x: f32, y: f32, w: f32, h: f32, pixels_per_point: f32) -> (i32, i32, u32, u32) {
    let px = |v: f32| (v * pixels_per_point).round() as i32;
    let (l, t, r, b) = (px(x), px(y), px(x + w), px(y + h));
    (
        l,
        t,
        r.saturating_sub(l).max(0).unsigned_abs(),
        b.saturating_sub(t).max(0).unsigned_abs(),
    )
}

#[cfg(test)]
mod tests {
    use super::{WindowKind, to_physical};

    #[test]
    fn physical_rects() {
        assert_eq!(to_physical(10.0, 20.0, 100.0, 50.0, 1.0), (10, 20, 100, 50));
        assert_eq!(
            to_physical(10.0, 20.0, 100.0, 50.0, 2.0),
            (20, 40, 200, 100)
        );
        // 1.25: edges 12.5→13 (round half away) and 137.5→138: width from edges, not 125.
        assert_eq!(to_physical(10.0, 0.0, 100.0, 1.0, 1.25), (13, 0, 125, 1));
        assert_eq!(to_physical(10.3, 0.0, 0.2, 0.0, 1.5), (15, 0, 1, 0));
    }

    #[test]
    fn titles() {
        assert!(WindowKind::Reims.matches("Reims vGPU"));
        assert!(WindowKind::Reims.matches("Reims vGPU — 60 fps (port 2)"));
        assert!(!WindowKind::Reims.matches("QEMU (mac)"));
        assert!(WindowKind::Qemu.matches("QEMU (mac) [Stopped]"));
        assert!(WindowKind::Qemu.matches("QEMU"));
        assert!(!WindowKind::Qemu.matches("Reims vGPU"));
    }
}
