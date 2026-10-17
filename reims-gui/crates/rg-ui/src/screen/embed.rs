//! QEMU's window inside the Screen tab over X11 (design D3), one slot per running VM: found by
//! pid off the UI thread, reparented, placed on the tab every frame, unmapped under overlays,
//! on other tabs and for other VMs, released when the VM goes away or the app closes.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use rg_x11::{Embedded, Embedder, WindowKind};

use crate::backend::VmId;

/// How long QEMU gets to open its window.
const FIND_TIMEOUT: Duration = Duration::from_secs(30);

/// The window to embed: QEMU's pid and which window it opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub pid: u32,
    pub reims: bool,
}

impl Target {
    fn kind(self) -> WindowKind {
        if self.reims {
            WindowKind::Reims
        } else {
            WindowKind::QemuSdl
        }
    }
}

/// Why QEMU's window is not in the tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Why {
    /// Not found in time, or reparenting failed.
    Failed(String),
    /// The user moved it to a window of its own (Machine › Show in a window).
    Detached,
    /// QEMU destroyed it (the VM stopped).
    Closed,
}

pub(crate) enum State {
    Finding(Receiver<Result<u32, String>>),
    Embedded { win: Box<Embedded>, shown: bool },
    Standalone(Why),
}

pub(crate) struct Slot {
    pub target: Target,
    pub state: State,
}

/// Every VM's embedded window.
#[derive(Default)]
pub(crate) struct Embeds {
    pub slots: HashMap<VmId, Slot>,
}

/// What the tab wants this frame.
pub(crate) struct Want<'a> {
    pub ctx: &'a egui::Context,
    /// The app's X11 window.
    pub parent: u32,
    /// The VM shown and its window, when it is a window to embed.
    pub current: Option<(&'a VmId, Target)>,
    /// The tab's rectangle in physical pixels; None hides the window (other tab, overlay).
    pub rect: Option<[i32; 4]>,
    pub detached: bool,
    /// A pointer press reached the app's own window: give it the keyboard back.
    pub app_pressed: bool,
}

fn find(ctx: &egui::Context, parent: u32, t: Target) -> State {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("x11-find".into())
        .spawn(move || {
            let r = Embedder::new(None, parent)
                .and_then(|e| e.find_window(t.pid, t.kind(), FIND_TIMEOUT))
                .map_err(|e| e.to_string());
            let _ = tx.send(r); // the slot may be gone already
            ctx.request_repaint();
        });
    match spawned {
        Ok(_) => State::Finding(rx),
        Err(e) => State::Standalone(Why::Failed(e.to_string())),
    }
}

fn embed(parent: u32, win: u32) -> State {
    match Embedder::new(None, parent).and_then(|e| e.embed(win)) {
        Ok(win) => State::Embedded {
            win: Box::new(win),
            shown: true,
        },
        Err(e) => State::Standalone(Why::Failed(e.to_string())),
    }
}

impl Embeds {
    /// The state of `id`'s window, for the tab's placeholder.
    pub fn state(&self, id: &str) -> Option<&State> {
        self.slots.get(id).map(|s| &s.state)
    }

    /// Looks for `id`'s window again (the tab's "Try again").
    pub fn retry(&mut self, id: &str) {
        self.slots.remove(id);
    }

    /// Once per frame, after the UI is drawn.
    pub fn sync(&mut self, w: Want<'_>) {
        // Windows of VMs that stopped: QEMU destroyed them.
        for s in self.slots.values_mut() {
            if let State::Embedded { win, .. } = &mut s.state {
                let _ = win.pump(); // a broken connection shows as !is_alive
                if !win.is_alive() {
                    s.state = State::Standalone(Why::Closed);
                }
            }
        }
        if let Some((id, target)) = w.current {
            let slot = self.slots.entry(id.clone()).or_insert_with(|| Slot {
                target,
                state: find(w.ctx, w.parent, target),
            });
            if slot.target != target {
                // Restarted: a new QEMU, a new window (the old slot releases on drop).
                *slot = Slot {
                    target,
                    state: find(w.ctx, w.parent, target),
                };
            }
            match &mut slot.state {
                State::Finding(rx) => match rx.try_recv() {
                    Ok(Ok(win)) => slot.state = embed(w.parent, win),
                    Ok(Err(e)) => slot.state = State::Standalone(Why::Failed(e)),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        slot.state = State::Standalone(Why::Failed("the search stopped".into()))
                    }
                },
                State::Embedded { .. } if w.detached => {
                    slot.state = State::Standalone(Why::Detached); // dropping releases it
                }
                State::Standalone(Why::Detached) if !w.detached => {
                    slot.state = find(w.ctx, w.parent, target);
                }
                _ => {}
            }
        }
        let mut busy = false;
        for (id, s) in &mut self.slots {
            let here = w.current.is_some_and(|(c, _)| c == id);
            match &mut s.state {
                State::Embedded { win, shown } => {
                    match w.rect.filter(|_| here) {
                        Some([x, y, wd, h]) => {
                            let _ = win.set_rect(x, y, wd.max(1) as u32, h.max(1) as u32);
                            if !*shown {
                                *shown = win.show().is_ok();
                            }
                            if w.app_pressed {
                                let _ = win.unfocus();
                            }
                            busy = true;
                        }
                        None if *shown => *shown = win.hide().is_err(),
                        None => {}
                    }
                }
                State::Finding(_) => busy = true,
                State::Standalone(_) => {}
            }
        }
        // Closed windows of VMs no longer shown are forgotten.
        self.slots.retain(|id, s| {
            w.current.is_some_and(|(c, _)| c == id) || !matches!(s.state, State::Standalone(_))
        });
        if busy {
            // X events (QEMU's own resizes, the pointer entering it) reach `pump` only on a
            // frame: keep a slow tick while a window is shown or searched for.
            w.ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
