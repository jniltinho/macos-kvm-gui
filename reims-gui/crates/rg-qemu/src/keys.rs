//! Physical keys as the guest gets them over VNC: an X11 keysym (US layout) and the XT set 1
//! scancode, where 0xE0-prefixed keys have bit 7 set, as QEMU's extended key events expect
//! (Go `keys.go`). The host keyboard layout detection (`keyboard.go`) belongs to rg-core.

use crate::{Error, Result};

/// What the VM receives for one physical key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyCode {
    pub keysym: u32,
    pub scancode: u32,
}

const fn k(keysym: u32, scancode: u32) -> KeyCode {
    KeyCode { keysym, scancode }
}

/// The named keys (the Go app's Fyne key names); letters and digits are in [`key_for`].
const NAMED: &[(&str, KeyCode)] = &[
    ("Escape", k(0xff1b, 0x01)),
    ("Return", k(0xff0d, 0x1c)),
    ("Tab", k(0xff09, 0x0f)),
    ("BackSpace", k(0xff08, 0x0e)),
    ("Insert", k(0xff63, 0xd2)),
    ("Delete", k(0xffff, 0xd3)),
    ("Home", k(0xff50, 0xc7)),
    ("End", k(0xff57, 0xcf)),
    ("Prior", k(0xff55, 0xc9)),
    ("Next", k(0xff56, 0xd1)),
    ("Left", k(0xff51, 0xcb)),
    ("Up", k(0xff52, 0xc8)),
    ("Right", k(0xff53, 0xcd)),
    ("Down", k(0xff54, 0xd0)),
    ("F1", k(0xffbe, 0x3b)),
    ("F2", k(0xffbf, 0x3c)),
    ("F3", k(0xffc0, 0x3d)),
    ("F4", k(0xffc1, 0x3e)),
    ("F5", k(0xffc2, 0x3f)),
    ("F6", k(0xffc3, 0x40)),
    ("F7", k(0xffc4, 0x41)),
    ("F8", k(0xffc5, 0x42)),
    ("F9", k(0xffc6, 0x43)),
    ("F10", k(0xffc7, 0x44)),
    ("F11", k(0xffc8, 0x57)),
    ("F12", k(0xffc9, 0x58)),
    ("KP_Enter", k(0xff8d, 0x9c)),
    ("Space", k(0x20, 0x39)),
    ("-", k('-' as u32, 0x0c)),
    ("=", k('=' as u32, 0x0d)),
    ("[", k('[' as u32, 0x1a)),
    ("]", k(']' as u32, 0x1b)),
    ("\\", k('\\' as u32, 0x2b)),
    (";", k(';' as u32, 0x27)),
    ("'", k('\'' as u32, 0x28)),
    (",", k(',' as u32, 0x33)),
    (".", k('.' as u32, 0x34)),
    ("/", k('/' as u32, 0x35)),
    ("`", k('`' as u32, 0x29)),
    ("LeftShift", k(0xffe1, 0x2a)),
    ("RightShift", k(0xffe2, 0x36)),
    ("LeftControl", k(0xffe3, 0x1d)),
    ("RightControl", k(0xffe4, 0x9d)),
    ("LeftAlt", k(0xffe9, 0x38)),   // Option in macOS
    ("RightAlt", k(0xffea, 0xb8)),  // Option in macOS
    ("LeftSuper", k(0xffeb, 0xdb)), // Command in macOS
    ("RightSuper", k(0xffec, 0xdc)),
    ("Menu", k(0xff67, 0xdd)),
    ("CapsLock", k(0xffe5, 0x3a)),
];

/// A–Z's scancodes.
const LETTERS: [u32; 26] = [
    0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19,
    0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
];
/// 0–9's scancodes.
const DIGITS: [u32; 10] = [0x0b, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a];

/// The key for a Go/Fyne key name: "A"–"Z" (lowercase keysym: Shift travels as its own key),
/// "0"–"9", and the names in the table ("Return", "LeftSuper", "F2"...).
pub fn key_for(name: &str) -> Option<KeyCode> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_uppercase() {
            let i = c as usize - 'A' as usize;
            return Some(k(c.to_ascii_lowercase() as u32, LETTERS[i]));
        }
        if c.is_ascii_digit() {
            return Some(k(c as u32, DIGITS[c as usize - '0' as usize]));
        }
    }
    NAMED.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

fn named(name: &str) -> KeyCode {
    key_for(name).unwrap_or_default()
}

/// The US keysym of the key with scancode `sc`, or 0.
fn keysym_for_scancode(sc: u32) -> u32 {
    if let Some((_, k)) = NAMED.iter().find(|(_, k)| k.scancode == sc) {
        return k.keysym;
    }
    if let Some(i) = LETTERS.iter().position(|s| *s == sc) {
        return 'a' as u32 + i as u32;
    }
    DIGITS
        .iter()
        .position(|s| *s == sc)
        .map_or(0, |i| '0' as u32 + i as u32)
}

/// Where a w×h frame is drawn inside a bw×bh box, scaled to fit without distortion: the scale
/// and the offset of its top-left corner.
pub fn fit_rect(w: u32, h: u32, bw: f32, bh: f32) -> (f32, f32, f32) {
    if w == 0 || h == 0 || bw <= 0.0 || bh <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let (w, h) = (w as f32, h as f32);
    let scale = (bw / w).min(bh / h);
    (scale, (bw - w * scale) / 2.0, (bh - h * scale) / 2.0)
}

/// A position inside the box in frame coordinates; `None` in the letterbox bars.
pub fn to_frame(w: u32, h: u32, bw: f32, bh: f32, px: f32, py: f32) -> Option<(u32, u32)> {
    let (scale, ox, oy) = fit_rect(w, h, bw, bh);
    if scale == 0.0 {
        return None;
    }
    let (fx, fy) = ((px - ox) / scale, (py - oy) / scale);
    if fx < 0.0 || fy < 0.0 || fx >= w as f32 || fy >= h as f32 {
        return None;
    }
    Some((fx as u32, fy as u32))
}

/// Linux evdev codes that are not simply their XT set 1 scancode (input-event-codes.h).
fn evdev_to_xt(evdev: u32) -> Option<u32> {
    Some(match evdev {
        85 => 0x76,                     // KEY_ZENKAKUHANKAKU
        86 => 0x56,                     // KEY_102ND: left of Z on ISO keyboards ("\|" on ABNT2)
        89 => 0x73,                     // KEY_RO: ABNT2 "/?" (and JIS ro)
        92 => 0x79,                     // KEY_HENKAN
        93 => 0x70,                     // KEY_KATAKANAHIRAGANA
        94 => 0x7b,                     // KEY_MUHENKAN
        96 => 0x9c,                     // KEY_KPENTER
        97 => 0x9d,                     // KEY_RIGHTCTRL
        98 => 0xb5,                     // KEY_KPSLASH
        99 => 0x54,                     // KEY_SYSRQ (QEMU's qnum)
        100 => 0xb8,                    // KEY_RIGHTALT
        102 => 0xc7,                    // KEY_HOME
        103 => 0xc8,                    // KEY_UP
        104 => 0xc9,                    // KEY_PAGEUP
        105 => 0xcb,                    // KEY_LEFT
        106 => 0xcd,                    // KEY_RIGHT
        107 => 0xcf,                    // KEY_END
        108 => 0xd0,                    // KEY_DOWN
        109 => 0xd1,                    // KEY_PAGEDOWN
        110 => 0xd2,                    // KEY_INSERT
        111 => 0xd3,                    // KEY_DELETE
        117 => 0x59,                    // KEY_KPEQUAL
        119 => 0xc6,                    // KEY_PAUSE (QEMU's qnum)
        121 => 0x7e,                    // KEY_KPCOMMA: ABNT2 keypad "."
        124 => 0x7d,                    // KEY_YEN
        125 => 0xdb,                    // KEY_LEFTMETA: Command
        126 => 0xdc,                    // KEY_RIGHTMETA: Command
        127 => 0xdd,                    // KEY_COMPOSE: Menu
        1..=88 if evdev != 84 => evdev, // KEY_ESC=1 ... KEY_F12=88 equal XT set 1
        _ => return None,
    })
}

/// The key for a physical key given as an X11 keycode (evdev code + 8). Layout independent:
/// every key, including ISO and ABNT2 extras, reaches macOS as the key it physically is. The
/// keysym is the US one when known; QEMU uses the scancode.
pub fn key_for_x11(keycode: u32) -> Option<KeyCode> {
    let sc = evdev_to_xt(keycode.checked_sub(8).filter(|e| *e > 0)?)?;
    Some(k(keysym_for_scancode(sc), sc))
}

/// The key for a Linux evdev code (what a Wayland/winit physical key maps to).
pub fn key_for_evdev(evdev: u32) -> Option<KeyCode> {
    key_for_x11(evdev + 8)
}

const KP_DIVIDE: KeyCode = k(0xffaf, 0xb5);
const KP_MULTIPLY: KeyCode = k(0xffaa, 0x37);
const KP_SUBTRACT: KeyCode = k(0xffad, 0x4a);

/// A line the app types into the guest as key chords, then Return. Only keys that give the
/// same character in every Latin layout macOS may use are allowed: letters other than A, M, Q,
/// W, Y and Z (AZERTY and QWERTZ move those), space, and the keypad's "/", "*" and "-".
pub fn guest_keys(line: &str) -> Result<Vec<Vec<KeyCode>>> {
    let shift = named("LeftShift");
    let mut out = Vec::new();
    for r in line.chars() {
        let keypad = match r {
            '/' => Some(KP_DIVIDE),
            '*' => Some(KP_MULTIPLY),
            '-' => Some(KP_SUBTRACT),
            ' ' => Some(named("Space")),
            _ => None,
        };
        if let Some(kc) = keypad {
            out.push(vec![kc]);
            continue;
        }
        let up = r.is_ascii_uppercase();
        let u = r.to_ascii_uppercase();
        if !u.is_ascii_uppercase() || "AMQWYZ".contains(u) {
            return Err(Error::msg(format!(
                "cannot type {r:?} in every keyboard layout"
            )));
        }
        let kc = named(&u.to_string());
        out.push(if up { vec![shift, kc] } else { vec![kc] });
    }
    out.push(vec![named("Return")]);
    Ok(out)
}

/// Opens Utilities › Terminal from the recovery's window through the menu bar: Control+F2
/// focuses it, Right ×4 reaches Utilities, Down opens it, "t" picks Terminal (Big Sur to Tahoe).
pub fn recovery_terminal() -> Vec<Vec<KeyCode>> {
    let (right, down) = (vec![named("Right")], vec![named("Down")]);
    vec![
        vec![named("LeftControl"), named("F2")],
        right.clone(),
        right.clone(),
        right.clone(),
        right,
        down,
        vec![named("T")],
        vec![named("Return")],
    ]
}

/// Names [`parse_chord`] accepts besides the table's own, case-insensitively.
fn alias(low: &str) -> Option<&'static str> {
    Some(match low {
        "shift" => "LeftShift",
        "ctrl" | "control" => "LeftControl",
        "alt" | "option" => "LeftAlt",
        "cmd" | "command" | "super" | "meta" => "LeftSuper",
        "enter" | "return" => "Return",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "space" => "Space",
        "backspace" => "BackSpace",
        "delete" => "Delete",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Prior",
        "pagedown" => "Next",
        "left" => "Left",
        "right" => "Right",
        "up" => "Up",
        "down" => "Down",
        "capslock" => "CapsLock",
        "kp_enter" => "KP_Enter",
        _ => return None,
    })
}

fn keypad(low: &str) -> Option<KeyCode> {
    Some(match low {
        "kp_divide" => KP_DIVIDE,
        "kp_multiply" => KP_MULTIPLY,
        "kp_subtract" => KP_SUBTRACT,
        "kp_add" => k(0xffab, 0x4e),
        "kp_decimal" => k(0xffae, 0x53),
        "kp_0" => k(0xffb0, 0x52),
        "kp_1" => k(0xffb1, 0x4f),
        "kp_2" => k(0xffb2, 0x50),
        "kp_3" => k(0xffb3, 0x51),
        "kp_4" => k(0xffb4, 0x4b),
        "kp_5" => k(0xffb5, 0x4c),
        "kp_6" => k(0xffb6, 0x4d),
        "kp_7" => k(0xffb7, 0x47),
        "kp_8" => k(0xffb8, 0x48),
        "kp_9" => k(0xffb9, 0x49),
        _ => return None,
    })
}

/// "shift+cmd+t" as keys, pressed in that order. Names are physical keys of a US keyboard.
pub fn parse_chord(s: &str) -> Result<Vec<KeyCode>> {
    let mut out = Vec::new();
    for part in s.split('+') {
        let name = part.trim();
        let low = name.to_lowercase();
        if let Some(kc) = keypad(&low) {
            out.push(kc);
            continue;
        }
        let name = if let Some(a) = alias(&low) {
            a.to_string()
        } else if name.chars().count() == 1 {
            name.to_uppercase()
        } else if let Some(rest) = low.strip_prefix('f') {
            format!("F{rest}")
        } else {
            name.to_string()
        };
        let kc =
            key_for(&name).ok_or_else(|| Error::msg(format!("unknown key {part:?} in {s:?}")))?;
        out.push(kc);
    }
    if out.is_empty() {
        return Err(Error::msg("empty key chord"));
    }
    Ok(out)
}

/// The macOS layout id of U.S.
pub const LAYOUT_US: i32 = 0;
/// The macOS layout id of Brazilian ABNT2.
pub const LAYOUT_ABNT2: i32 = 128;

fn row(c: char, plain: &str, shifted: &str, codes: &[u32]) -> Option<(u32, bool)> {
    if let Some(i) = plain.chars().position(|p| p == c) {
        return Some((codes[i], false));
    }
    shifted
        .chars()
        .position(|p| p == c && p != ' ')
        .map(|i| (codes[i], true))
}

/// The physical key, and whether Shift is held, that types `c` with the layout. Dead keys
/// (ABNT2's ´ ` ~ ^ ¨) and ABNT2's "/?" key, which does not reach macOS, are left out.
fn typed(layout: i32, c: char) -> Option<(u32, bool)> {
    match c {
        ' ' => return Some((0x39, false)),
        '\t' => return Some((0x0f, false)),
        'a'..='z' => return Some((LETTERS[c as usize - 'a' as usize], false)),
        'A'..='Z' => return Some((LETTERS[c as usize - 'A' as usize], true)),
        _ => {}
    }
    let digits = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b];
    match layout {
        LAYOUT_US => row(c, "1234567890", "!@#$%^&*()", &digits).or_else(|| {
            let codes = [
                0x0c, 0x0d, 0x1a, 0x1b, 0x2b, 0x27, 0x28, 0x33, 0x34, 0x35, 0x29,
            ];
            row(c, "-=[]\\;',./`", "_+{}|:\"<>?~", &codes)
        }),
        LAYOUT_ABNT2 => row(c, "1234567890", "!@#$% &*()", &digits)
            .or_else(|| {
                let codes = [0x0c, 0x0d, 0x1b, 0x2b, 0x27, 0x29, 0x35, 0x33, 0x34, 0x56];
                row(c, "-=[]ç';,.\\", "_+{}Ç\":<>|", &codes)
            })
            .or((c == '/').then_some((0xb5, false))), // the keypad's
        _ => None,
    }
}

/// Text as key chords for the macOS layout id (rg-core's `RecoveryKeyboard`); never adds
/// Return. Layouts without a table, and characters a table lacks, are refused.
pub fn type_keys(text: &str, layout: i32) -> Result<Vec<Vec<KeyCode>>> {
    if layout != LAYOUT_US && layout != LAYOUT_ABNT2 {
        return Err(Error::msg(format!(
            "typing text is supported only with the U.S. and Brazilian ABNT2 layouts (this VM uses layout {layout})"
        )));
    }
    let shift = named("LeftShift");
    text.chars()
        .map(|r| {
            let Some((sc, up)) = typed(layout, r) else {
                if r == '\n' || r == '\r' {
                    return Err(Error::msg(
                        "text cannot contain a line break: send the Return key with send_keys",
                    ));
                }
                return Err(Error::msg(format!(
                    "cannot type {r:?} with this keyboard layout"
                )));
            };
            let kc = k(r as u32, sc);
            Ok(if up { vec![shift, kc] } else { vec![kc] })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_table() {
        for (name, want) in [
            ("A", k('a' as u32, 0x1e)),
            ("Z", k('z' as u32, 0x2c)),
            ("1", k('1' as u32, 0x02)),
            ("0", k('0' as u32, 0x0b)),
            ("LeftSuper", k(0xffeb, 0xdb)),
            ("LeftAlt", k(0xffe9, 0x38)),
            ("Up", k(0xff52, 0xc8)),
            ("BackSpace", k(0xff08, 0x0e)),
        ] {
            assert_eq!(key_for(name), Some(want), "{name}");
        }
    }

    #[test]
    fn fit_letterboxes() {
        // A 1920x1080 frame in a 960x700 box: half size with 80 px bars above and below.
        assert_eq!(fit_rect(1920, 1080, 960.0, 700.0), (0.5, 0.0, 80.0));
        assert_eq!(
            to_frame(1920, 1080, 960.0, 700.0, 480.0, 350.0),
            Some((960, 540))
        );
        assert_eq!(
            to_frame(1920, 1080, 960.0, 700.0, 480.0, 40.0),
            None,
            "letterbox bar"
        );
    }

    #[test]
    fn x11_keycodes_are_physical_keys() {
        for (keycode, sc) in [
            (9, 0x01),
            (38, 0x1e),
            (47, 0x27),
            (94, 0x56),
            (97, 0x73),
            (113, 0xcb),
            (133, 0xdb),
            (129, 0x7e),
        ] {
            assert_eq!(
                key_for_x11(keycode).map(|k| k.scancode),
                Some(sc),
                "keycode {keycode}"
            );
        }
        assert_eq!(
            key_for_x11(38).map(|k| k.keysym),
            Some('a' as u32),
            "US keysym kept"
        );
        assert_eq!(key_for_x11(8), None, "keycode 8 is not a key");
    }

    #[test]
    fn guest_keys_refuse_layout_dependent_letters() {
        assert!(guest_keys("ditto -x").is_ok_and(|c| c.len() == 9));
        assert!(guest_keys("Quit").is_err());
        let c = guest_keys("B").unwrap_or_default();
        assert_eq!(c[0], vec![named("LeftShift"), named("B")]);
    }

    #[test]
    fn parse_chord_names() {
        let c = parse_chord("shift+cmd+t").unwrap_or_default();
        assert_eq!(c, vec![named("LeftShift"), named("LeftSuper"), named("T")]);
        assert_eq!(parse_chord("f2").ok(), Some(vec![named("F2")]));
        assert_eq!(parse_chord("KP_Divide").ok(), Some(vec![KP_DIVIDE]));
        assert!(parse_chord("hyper+x").is_err());
    }

    #[test]
    fn type_keys_by_layout() {
        let us = type_keys("a:/", LAYOUT_US).unwrap_or_default();
        assert_eq!(us[1], vec![named("LeftShift"), k(':' as u32, 0x27)]);
        assert_eq!(us[2], vec![k('/' as u32, 0x35)]);
        let br = type_keys("ç/", LAYOUT_ABNT2).unwrap_or_default();
        assert_eq!(
            br,
            vec![vec![k('ç' as u32, 0x27)], vec![k('/' as u32, 0xb5)]]
        );
        assert!(type_keys("^", LAYOUT_ABNT2).is_err(), "dead key");
        assert!(type_keys("a\n", LAYOUT_US).is_err());
        assert!(type_keys("a", 3).is_err());
    }

    #[test]
    fn recovery_terminal_starts_with_control_f2() {
        assert_eq!(
            recovery_terminal()[0],
            vec![named("LeftControl"), named("F2")]
        );
    }
}
