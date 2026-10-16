//! The recovery's keyboard layout (NVRAM `prev-lang:kbd`): English with the host's layout, since
//! keys reach macOS as the physical keys they are (port of `keyboard.go`).

use std::process::Command;

/// XKB layouts (and `layout(variant)`) to macOS keyboard layout IDs, from OpenCore's dump of
/// Apple's AppleKeyboardLayouts bundle. Unknown layouts fall back to U.S. (0).
const MAC_KEYBOARD_IDS: [(&str, i32); 22] = [
    ("us", 0),
    ("us(dvorak)", 16300),
    ("gb", 250),
    ("de", 3),
    ("fr", 60),
    ("es", 87),
    ("it", 223),
    ("pt", 10),
    ("br", 128),
    ("br(abnt2)", 128),
    ("latam", 89),
    ("ch", 19),
    ("ch(fr)", 18),
    ("ca(fr)", -19336),
    ("ca", -19336),
    ("ru", 19458),
    ("tr", -36),
    ("se", 224),
    ("no", 12),
    ("dk", 9),
    ("fi", 17),
    ("pl", 30788),
];

fn lookup(key: &str) -> Option<i32> {
    MAC_KEYBOARD_IDS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|&(_, id)| id)
}

/// The macOS layout ID for an XKB layout and variant (0, U.S., when unknown).
pub fn mac_keyboard_id(layout: &str, variant: &str) -> i32 {
    (!variant.is_empty())
        .then(|| lookup(&format!("{layout}({variant})")))
        .flatten()
        .or_else(|| lookup(layout))
        .unwrap_or(0)
}

/// The `prev-lang:kbd` value: English with the host's keyboard layout, e.g. `en-US:128`.
pub fn recovery_keyboard() -> String {
    let (layout, variant) = host_keyboard();
    format!("en-US:{}", mac_keyboard_id(&layout, &variant))
}

/// Whether the app can type into the guest: not on Dvorak or Russian, where letters move.
pub fn guest_typing_ok() -> bool {
    let (layout, variant) = host_keyboard();
    let id = mac_keyboard_id(&layout, &variant);
    Some(id) != lookup("us(dvorak)") && Some(id) != lookup("ru")
}

/// The host's keyboard layout and variant: /etc/default/keyboard (Debian, Ubuntu), then
/// localectl. Only the first of several layouts counts.
pub fn host_keyboard() -> (String, String) {
    let file = std::fs::read_to_string("/etc/default/keyboard").unwrap_or_default();
    let (mut layout, mut variant) = parse_keyboard(&file, '=', "XKBLAYOUT", "XKBVARIANT");
    if layout.is_empty()
        && let Ok(out) = Command::new("localectl").arg("status").output()
    {
        (layout, variant) = parse_keyboard(
            &String::from_utf8_lossy(&out.stdout),
            ':',
            "X11 Layout",
            "X11 Variant",
        );
    }
    let first = |s: &str| s.split(',').next().unwrap_or_default().trim().to_owned();
    (first(&layout), first(&variant))
}

fn parse_keyboard(text: &str, sep: char, layout_key: &str, variant_key: &str) -> (String, String) {
    let (mut layout, mut variant) = (String::new(), String::new());
    for line in text.lines() {
        let Some((k, v)) = line.trim().split_once(sep) else {
            continue;
        };
        let v = if sep == '=' {
            v.trim_matches(['"', '\''])
        } else {
            v.trim()
        };
        match k.trim() {
            k if k == layout_key => layout = v.into(),
            k if k == variant_key => variant = v.into(),
            _ => {}
        }
    }
    (layout, variant)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_layout_ids() {
        assert_eq!(mac_keyboard_id("br", ""), 128);
        assert_eq!(mac_keyboard_id("us", "dvorak"), 16300);
        assert_eq!(mac_keyboard_id("xx", ""), 0);
        assert_eq!(mac_keyboard_id("de", "nodeadkeys"), 3);
        assert!(recovery_keyboard().starts_with("en-US:"));
    }

    #[test]
    fn parses_debian_and_localectl() {
        let deb = "XKBMODEL=\"pc105\"\nXKBLAYOUT=\"br,us\"\nXKBVARIANT=\"abnt2\"\n";
        assert_eq!(
            parse_keyboard(deb, '=', "XKBLAYOUT", "XKBVARIANT"),
            ("br,us".into(), "abnt2".into())
        );
        let lc = "   System Locale: LANG=en_US.UTF-8\n       X11 Layout: de\n      X11 Variant: nodeadkeys\n";
        assert_eq!(
            parse_keyboard(lc, ':', "X11 Layout", "X11 Variant"),
            ("de".into(), "nodeadkeys".into())
        );
    }
}
