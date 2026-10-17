// Adapted from reims-vgpu's crates/reims-ctl/src/view/keys.rs
// (https://github.com/steelbrain/reims-vgpu), GPL-2.0-or-later OR LGPL-3.0-or-later.
// Kept: the input-send-event model, taps, US-keyboard characters and the tablet's position;
// left out: the terminal (crossterm) key and mouse decoding.

//! Keys, buttons and the tablet's position as QMP's `input-send-event` gives them to the guest:
//! a US keyboard's keys by QEMU's qcodes.

use serde_json::{Value, json};

/// Modifiers held around a key or a click.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const CMD: Mods = Mods(1);
    pub const OPT: Mods = Mods(2);
    pub const CTRL: Mods = Mods(4);
    pub const SHIFT: Mods = Mods(8);
    /// Each modifier with its qcode, in the order they go down.
    const EACH: [(Mods, &'static str); 4] = [
        (Mods::CMD, "meta_l"),
        (Mods::OPT, "alt"),
        (Mods::CTRL, "ctrl"),
        (Mods::SHIFT, "shift"),
    ];

    pub fn contains(self, m: Mods) -> bool {
        self.0 & m.0 == m.0
    }

    fn qcodes(self) -> impl DoubleEndedIterator<Item = &'static str> {
        Mods::EACH
            .into_iter()
            .filter(move |(m, _)| self.contains(*m))
            .map(|(_, q)| q)
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, o: Mods) -> Mods {
        Mods(self.0 | o.0)
    }
}

/// One input event for `input-send-event`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    /// A key, by its qcode, going down (true) or up.
    Key(&'static str, bool),
    /// A mouse button, by QMP's name for it (`left`, `right`, `middle`, `wheel-up`...).
    Btn(&'static str, bool),
    /// The tablet's position, each axis 0 to 0x7fff across the display.
    Abs(u16, u16),
}

impl Ev {
    /// The event as `input-send-event` takes it; a position is two, one per axis.
    pub fn json(self, out: &mut Vec<Value>) {
        match self {
            Ev::Key(k, down) => out.push(json!({
                "type": "key",
                "data": { "down": down, "key": { "type": "qcode", "data": k } }
            })),
            Ev::Btn(b, down) => {
                out.push(json!({ "type": "btn", "data": { "down": down, "button": b } }))
            }
            Ev::Abs(x, y) => {
                out.push(json!({ "type": "abs", "data": { "axis": "x", "value": x } }));
                out.push(json!({ "type": "abs", "data": { "axis": "y", "value": y } }));
            }
        }
    }
}

/// The events of a tap of `key` with `mods` held.
pub fn tap(mods: Mods, key: &'static str) -> Vec<Ev> {
    let mut v: Vec<Ev> = mods.qcodes().map(|q| Ev::Key(q, true)).collect();
    v.push(Ev::Key(key, true));
    v.push(Ev::Key(key, false));
    v.extend(mods.qcodes().rev().map(|q| Ev::Key(q, false)));
    v
}

const LETTERS: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];
const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// The key of a US keyboard that types `c`, and whether it takes Shift.
pub fn char_key(c: char) -> Option<(&'static str, bool)> {
    Some(match c {
        'a'..='z' => (LETTERS[c as usize - 'a' as usize], false),
        'A'..='Z' => (LETTERS[c as usize - 'A' as usize], true),
        '0'..='9' => (DIGITS[c as usize - '0' as usize], false),
        ' ' => ("spc", false),
        '\n' => ("ret", false),
        '\t' => ("tab", false),
        '!' => ("1", true),
        '@' => ("2", true),
        '#' => ("3", true),
        '$' => ("4", true),
        '%' => ("5", true),
        '^' => ("6", true),
        '&' => ("7", true),
        '*' => ("8", true),
        '(' => ("9", true),
        ')' => ("0", true),
        '-' => ("minus", false),
        '_' => ("minus", true),
        '=' => ("equal", false),
        '+' => ("equal", true),
        '[' => ("bracket_left", false),
        '{' => ("bracket_left", true),
        ']' => ("bracket_right", false),
        '}' => ("bracket_right", true),
        '\\' => ("backslash", false),
        '|' => ("backslash", true),
        ';' => ("semicolon", false),
        ':' => ("semicolon", true),
        '\'' => ("apostrophe", false),
        '"' => ("apostrophe", true),
        '`' => ("grave_accent", false),
        '~' => ("grave_accent", true),
        ',' => ("comma", false),
        '<' => ("comma", true),
        '.' => ("dot", false),
        '>' => ("dot", true),
        '/' => ("slash", false),
        '?' => ("slash", true),
        _ => return None,
    })
}

/// The tablet's position at a fraction of the way across and down the display.
pub fn abs(fx: f64, fy: f64) -> Ev {
    // Clamped to 0..=32767 first, so the cast cannot truncate.
    let axis = |f: f64| (f.clamp(0.0, 1.0) * 32767.0).round() as u16;
    Ev::Abs(axis(fx), axis(fy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capital_holds_shift_around_the_key() {
        let (k, shift) = char_key('A').unwrap_or(("", false));
        assert_eq!(
            tap(if shift { Mods::SHIFT } else { Mods::NONE }, k),
            vec![
                Ev::Key("shift", true),
                Ev::Key("a", true),
                Ev::Key("a", false),
                Ev::Key("shift", false)
            ]
        );
    }

    #[test]
    fn modifiers_release_in_reverse() {
        let v = tap(Mods::CMD | Mods::SHIFT, "t");
        assert_eq!(v.first(), Some(&Ev::Key("meta_l", true)));
        assert_eq!(v.last(), Some(&Ev::Key("meta_l", false)));
    }

    #[test]
    fn abs_spans_the_tablet_and_gives_two_events() {
        let mut out = Vec::new();
        abs(1.5, 0.5).json(&mut out);
        assert_eq!(out[0]["data"]["value"], 32767);
        assert_eq!(out[1]["data"]["value"], 16384);
    }
}
