//! The Screen tab without X11 (design D4): QEMU's VNC socket drawn into a texture updated from
//! the dirty rectangles, letterboxed, with a keyboard grab released by Ctrl+Alt and the pointer
//! sent as an absolute tablet. Adapted from rustXterm `crates/ui/src/desktop.rs` (MIT).
//!
//! Keys go by physical position (scancodes over QEMU's extended key events), so the guest's
//! layout applies; egui has no key for the keypad, ISO/JIS extras or Super, so those are lost
//! here (the X11 embedding passes them natively).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{
    Color32, ColorImage, CursorIcon, Event, EventFilter, Id, Key, Modifiers, MouseWheelUnit,
    PointerButton, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui, vec2,
};
use rg_qemu::KeyCode;
use rg_qemu::keys::{fit_rect, key_for, to_frame};
use rg_qemu::vnc::{BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BUTTON_WHEEL_DOWN, BUTTON_WHEEL_UP};
use rg_qemu::{Frame, Vnc};

use crate::backend::PreviewUpdate;

/// The grab hint stays this long after the keyboard is grabbed (seconds).
const HINT_SECS: f64 = 3.0;
/// Text on the black backdrop.
const BACKDROP_TEXT: Color32 = Color32::from_gray(0xf0);
/// More dirty rectangles than this in one take: upload the whole frame instead.
const MAX_RECTS: usize = 64;

/// What the view sends to the guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Key(bool, KeyCode),
    Pointer { buttons: u8, x: u32, y: u32 },
}

#[derive(Default)]
struct Shared {
    vnc: Option<Vnc>,
    dirty: Vec<rg_qemu::Rect>,
    full: bool,
    error: Option<String>,
}

/// A connection to a VM's VNC socket on its own thread (a current-thread tokio runtime); it
/// closes when dropped.
pub(crate) struct Session {
    pub path: PathBuf,
    tx: tokio::sync::mpsc::UnboundedSender<Input>,
    shared: Arc<Mutex<Shared>>,
}

impl Session {
    pub fn start(path: PathBuf, ctx: egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (sh, p) = (shared.clone(), path.clone());
        let spawned = std::thread::Builder::new()
            .name("vnc-screen".into())
            .spawn(move || {
                let r = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(serve(&p, &sh, &ctx, rx)),
                    Err(e) => Err(e.to_string()),
                };
                if let Ok(mut s) = sh.lock() {
                    s.error = Some(r.err().unwrap_or_else(|| "the connection closed".into()));
                }
                ctx.request_repaint();
            });
        if let Err(e) = spawned
            && let Ok(mut s) = shared.lock()
        {
            s.error = Some(e.to_string());
        }
        Self { path, tx, shared }
    }

    /// New pixels since the last call: the whole frame after a connect or a resize, else the
    /// changed rectangles.
    pub fn take(&self) -> Option<PreviewUpdate> {
        let (v, full, dirty) = {
            let mut s = self.shared.lock().ok()?;
            let v = s.vnc.clone()?;
            (v, std::mem::take(&mut s.full), std::mem::take(&mut s.dirty))
        };
        v.with_frame(|f| {
            let size = [f.width as usize, f.height as usize];
            if size[0] == 0 || size[1] == 0 || f.pixels.len() != size[0] * size[1] * 4 {
                return None;
            }
            if full || dirty.len() > MAX_RECTS {
                return Some(PreviewUpdate::Full(ColorImage::from_rgba_premultiplied(
                    size, &f.pixels,
                )));
            }
            let rects: Vec<_> = dirty.iter().filter_map(|r| crop(f, r)).collect();
            (!rects.is_empty()).then_some(PreviewUpdate::Dirty(rects))
        })
    }

    pub fn error(&self) -> Option<String> {
        self.shared.lock().ok()?.error.clone()
    }

    pub fn send(&self, input: Vec<Input>) {
        for i in input {
            // The session thread is gone only after an error, which the tab already shows.
            let _ = self.tx.send(i);
        }
    }
}

/// Dials, then reads updates and writes input until either side ends.
async fn serve(
    path: &std::path::Path,
    shared: &Arc<Mutex<Shared>>,
    ctx: &egui::Context,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Input>,
) -> Result<(), String> {
    let (v, reader) = Vnc::dial(path).await.map_err(|e| e.to_string())?;
    if let Ok(mut s) = shared.lock() {
        s.vnc = Some(v.clone());
        s.full = true;
    }
    let (sh, c) = (shared.clone(), ctx.clone());
    let run = reader.run(move |u| {
        if let Ok(mut s) = sh.lock() {
            if u.resized {
                s.full = true;
                s.dirty.clear();
            } else {
                s.dirty.extend_from_slice(&u.dirty);
            }
        }
        c.request_repaint();
    });
    let input = async {
        while let Some(i) = rx.recv().await {
            match i {
                Input::Key(down, k) => v.key(down, k).await,
                Input::Pointer { buttons, x, y } => v.pointer(buttons, x, y).await,
            }
            .map_err(|e| e.to_string())?;
        }
        Ok(()) // the Session was dropped
    };
    tokio::select! {
        r = run => r.map_err(|e| e.to_string()),
        r = input => r,
    }
}

/// One rectangle of the frame as an image, clipped to it.
fn crop(f: &Frame, r: &rg_qemu::Rect) -> Option<([usize; 2], ColorImage)> {
    let (fw, fh) = (f.width as usize, f.height as usize);
    let (x, y) = (r.x as usize, r.y as usize);
    let (w, h) = (
        (r.w as usize).min(fw.saturating_sub(x)),
        (r.h as usize).min(fh.saturating_sub(y)),
    );
    if w == 0 || h == 0 {
        return None;
    }
    let mut px = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        let s = (row * fw + x) * 4;
        px.extend_from_slice(f.pixels.get(s..s + w * 4)?);
    }
    Some(([x, y], ColorImage::from_rgba_premultiplied([w, h], &px)))
}

/// The texture and the input state of the VNC screen.
#[derive(Default)]
pub(crate) struct VncView {
    texture: Option<TextureHandle>,
    /// Updates waiting for the next draw (from the session, or a test).
    pub pending: Vec<PreviewUpdate>,
    pub grabbed: bool,
    mods: Modifiers,
    buttons: u8,
    /// Keys sent down, so their release matches.
    down: Vec<(Key, KeyCode)>,
    hint_since: Option<f64>,
}

impl VncView {
    pub fn has_frame(&self) -> bool {
        self.texture.is_some()
    }

    /// Forgets the frame (another VM, or the VM stopped).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn upload(&mut self, ctx: &egui::Context) {
        for u in std::mem::take(&mut self.pending) {
            match (u, &mut self.texture) {
                (PreviewUpdate::Full(img), Some(t)) if t.size() == img.size => {
                    t.set(img, TextureOptions::LINEAR)
                }
                (PreviewUpdate::Full(img), slot) => {
                    *slot = Some(ctx.load_texture("vnc-screen", img, TextureOptions::LINEAR))
                }
                (PreviewUpdate::Dirty(rects), Some(t)) => {
                    for (pos, img) in rects {
                        t.set_partial(pos, img, TextureOptions::LINEAR);
                    }
                }
                (PreviewUpdate::Dirty(_), None) => {} // wait for a whole frame
            }
        }
    }

    /// Paints the screen into `rect` and returns the guest's input. `waiting` is the text shown
    /// before the first frame.
    pub fn show(&mut self, ui: &mut Ui, rect: Rect, id: Id, waiting: &str) -> Vec<Input> {
        let mut out = Vec::new();
        self.upload(ui.ctx());
        ui.painter().rect_filled(rect, 0, Color32::BLACK);
        let Some((tex, size)) = self.texture.as_ref().map(|t| (t.id(), t.size())) else {
            centered(ui, rect, |ui| {
                ui.add(egui::Spinner::new().size(22.0).color(BACKDROP_TEXT));
                ui.label(egui::RichText::new(waiting).color(BACKDROP_TEXT));
            });
            return out;
        };
        let fb = [size[0] as u32, size[1] as u32];
        let (scale, ox, oy) = fit_rect(fb[0], fb[1], rect.width(), rect.height());
        let image = Rect::from_min_size(
            rect.min + vec2(ox, oy),
            vec2(fb[0] as f32 * scale, fb[1] as f32 * scale),
        );
        let area = ui.interact(rect, id, Sense::click_and_drag());
        area.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "VM screen")
        });
        ui.painter().image(
            tex,
            image,
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        self.keyboard(ui, &area, &mut out);
        self.pointer(ui, &area, rect, fb, &mut out);
        let area = if self.grabbed {
            area
        } else {
            area.on_hover_text("Click to type in macOS; Ctrl+Alt gives the keyboard back")
        };
        if area.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Default);
        }
        self.hint(ui, rect);
        out
    }

    fn keyboard(&mut self, ui: &mut Ui, area: &egui::Response, out: &mut Vec<Input>) {
        let now = ui.input(|i| i.time);
        if area.clicked() || area.drag_started() || area.secondary_clicked() {
            area.request_focus();
            if !self.grabbed {
                self.grabbed = true;
                self.hint_since = Some(now);
            }
        }
        if self.grabbed && !area.has_focus() && !area.clicked() && !area.drag_started() {
            self.release(out);
        }
        if !self.grabbed {
            return;
        }
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                area.id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
        let is_key = |e: &Event| {
            matches!(
                e,
                Event::Text(_)
                    | Event::Key { .. }
                    | Event::Copy
                    | Event::Cut
                    | Event::Paste(_)
                    | Event::Ime(_)
            )
        };
        let (events, final_mods) = ui.input_mut(|i| {
            let all = i.events.clone();
            i.events.retain(|e| !is_key(e));
            (all, i.modifiers)
        });
        for e in events {
            match e {
                Event::ModifiersChanged(m) => {
                    if m.ctrl && m.alt && !m.shift {
                        self.release(out);
                        area.surrender_focus();
                        return;
                    }
                    self.sync_mods(m, out);
                }
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    modifiers,
                    ..
                } => {
                    self.sync_mods(modifiers, out);
                    self.key(physical_key.unwrap_or(key), pressed, out);
                }
                // egui turns Ctrl+C/X/V into these; Ctrl is already down in the guest.
                Event::Copy | Event::Cut | Event::Paste(_) => {
                    let name = match e {
                        Event::Copy => "C",
                        Event::Cut => "X",
                        _ => "V",
                    };
                    if let Some(k) = key_for(name) {
                        out.extend([Input::Key(true, k), Input::Key(false, k)]);
                    }
                }
                _ => {}
            }
        }
        self.sync_mods(final_mods, out);
    }

    /// Lets go of the keyboard: every key and button still down is released in the guest.
    fn release(&mut self, out: &mut Vec<Input>) {
        self.sync_mods(Modifiers::NONE, out);
        for (_, k) in self.down.drain(..) {
            out.push(Input::Key(false, k));
        }
        self.grabbed = false;
        self.hint_since = None;
    }

    fn sync_mods(&mut self, m: Modifiers, out: &mut Vec<Input>) {
        let old = self.mods;
        for (was, now, name) in [
            (old.shift, m.shift, "LeftShift"),
            (old.ctrl, m.ctrl, "LeftControl"),
            (old.alt, m.alt, "LeftAlt"),
        ] {
            if was != now
                && let Some(k) = key_for(name)
            {
                out.push(Input::Key(now, k));
            }
        }
        self.mods = m;
    }

    fn key(&mut self, key: Key, pressed: bool, out: &mut Vec<Input>) {
        if pressed {
            if let Some(k) = key_code(key) {
                self.down.push((key, k));
                out.push(Input::Key(true, k));
            }
        } else if let Some(i) = self.down.iter().position(|(d, _)| *d == key) {
            let (_, k) = self.down.remove(i);
            out.push(Input::Key(false, k));
        }
    }

    fn pointer(
        &mut self,
        ui: &Ui,
        area: &egui::Response,
        rect: Rect,
        fb: [u32; 2],
        out: &mut Vec<Input>,
    ) {
        let at = |p: Pos2| {
            let (x, y) = (p.x - rect.min.x, p.y - rect.min.y);
            to_frame(fb[0], fb[1], rect.width(), rect.height(), x, y)
        };
        let mut moved = None;
        for e in ui.input(|i| i.events.clone()) {
            match e {
                Event::PointerMoved(p) if rect.contains(p) => moved = at(p),
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let bit = match button {
                        PointerButton::Primary => BUTTON_LEFT,
                        PointerButton::Middle => BUTTON_MIDDLE,
                        PointerButton::Secondary => BUTTON_RIGHT,
                        _ => continue,
                    };
                    if pressed && !rect.contains(pos) || !pressed && self.buttons & bit == 0 {
                        continue;
                    }
                    if let Some((x, y)) = moved.take().or_else(|| at(pos)) {
                        self.buttons = if pressed {
                            self.buttons | bit
                        } else {
                            self.buttons & !bit
                        };
                        out.push(Input::Pointer {
                            buttons: self.buttons,
                            x,
                            y,
                        });
                    }
                }
                Event::MouseWheel { unit, delta, .. } if area.hovered() => {
                    let pos = ui.input(|i| i.pointer.hover_pos());
                    let Some((x, y)) = pos.and_then(at) else {
                        continue;
                    };
                    let steps = match unit {
                        MouseWheelUnit::Point => (delta.y / 40.0).round(),
                        _ => delta.y.round(),
                    }
                    .clamp(-10.0, 10.0) as i32;
                    let bit = if steps > 0 {
                        BUTTON_WHEEL_UP
                    } else {
                        BUTTON_WHEEL_DOWN
                    };
                    for _ in 0..steps.unsigned_abs() {
                        out.push(Input::Pointer {
                            buttons: self.buttons | bit,
                            x,
                            y,
                        });
                        out.push(Input::Pointer {
                            buttons: self.buttons,
                            x,
                            y,
                        });
                    }
                }
                _ => {}
            }
        }
        if let Some((x, y)) = moved {
            out.push(Input::Pointer {
                buttons: self.buttons,
                x,
                y,
            });
        }
    }

    /// "Keyboard captured" strip at the top for a few seconds after the grab, fading out.
    fn hint(&self, ui: &Ui, rect: Rect) {
        let (Some(since), true) = (self.hint_since, self.grabbed) else {
            return;
        };
        let age = ui.input(|i| i.time) - since;
        if age >= HINT_SECS {
            return;
        }
        let alpha = ((HINT_SECS - age) / 0.3).min(1.0) as f32;
        ui.ctx().request_repaint_after(Duration::from_millis(30));
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), 24.0));
        let p = ui.painter_at(strip);
        p.rect_filled(strip, 0, Color32::from_black_alpha((200.0 * alpha) as u8));
        p.text(
            strip.center(),
            egui::Align2::CENTER_CENTER,
            "Keyboard captured by macOS: press Ctrl+Alt to release it",
            egui::FontId::proportional(12.0),
            BACKDROP_TEXT.gamma_multiply(alpha),
        );
    }
}

/// Centered content on the black backdrop.
pub(crate) fn centered(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(40.0))
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            ui.add_space((rect.height() / 2.0 - 70.0).max(0.0));
            ui.spacing_mut().item_spacing.y = 8.0;
            add(ui);
        },
    );
}

/// The guest key for a physical egui key (US positions; the guest's layout applies).
fn key_code(key: Key) -> Option<KeyCode> {
    use Key::*;
    let name = match key {
        Enter => "Return",
        Escape => "Escape",
        Tab => "Tab",
        Backspace => "BackSpace",
        Insert => "Insert",
        Delete => "Delete",
        Home => "Home",
        End => "End",
        PageUp => "Prior",
        PageDown => "Next",
        ArrowLeft => "Left",
        ArrowUp => "Up",
        ArrowRight => "Right",
        ArrowDown => "Down",
        Space => "Space",
        Minus => "-",
        Equals => "=",
        OpenBracket => "[",
        CloseBracket => "]",
        Backslash => "\\",
        Semicolon => ";",
        Quote => "'",
        Comma => ",",
        Period => ".",
        Slash => "/",
        Backtick => "`",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        // Letters and digits: egui names them "A"…"Z" and "0"…"9", as key_for does.
        k => k.name(),
    };
    key_for(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_by_position() {
        assert_eq!(key_code(Key::A).map(|k| k.scancode), Some(0x1e));
        assert_eq!(key_code(Key::Num1).map(|k| k.scancode), Some(0x02));
        assert_eq!(key_code(Key::Enter).map(|k| k.scancode), Some(0x1c));
        assert_eq!(key_code(Key::Slash).map(|k| k.scancode), Some(0x35));
    }

    #[test]
    fn crop_clips_to_the_frame() {
        let f = Frame {
            width: 4,
            height: 2,
            pixels: (0..32).collect(),
        };
        let r = rg_qemu::Rect {
            x: 3,
            y: 1,
            w: 5,
            h: 5,
        };
        let (pos, img) = crop(&f, &r).unwrap_or_else(|| unreachable!());
        assert_eq!((pos, img.size), ([3, 1], [1, 1]));
        assert_eq!(img.pixels[0].to_array(), [28, 29, 30, 31]);
    }

    #[test]
    fn ctrl_alt_releases_every_key() {
        let mut v = VncView {
            grabbed: true,
            ..Default::default()
        };
        let mut out = Vec::new();
        v.sync_mods(Modifiers::SHIFT, &mut out);
        v.key(Key::A, true, &mut out);
        out.clear();
        v.release(&mut out);
        assert!(!v.grabbed);
        assert_eq!(out.len(), 2, "{out:?}"); // Shift and A up
        assert!(out.iter().all(|i| matches!(i, Input::Key(false, _))));
    }
}
