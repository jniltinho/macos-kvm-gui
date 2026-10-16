//! Theme tokens and the egui style built from them.
//!
//! Token-to-`Visuals` mapping adapted from rustXterm `crates/ui/src/theme.rs` (MIT) and the
//! shadow/radius tokens from PhotoCraft `crates/ui-egui/src/theme.rs` (MIT OR Apache-2.0). The
//! light palette is the Go app's (`internal/gui/theme.go`, measured from VirtualBox on KDE).

use std::sync::Arc;

use egui::epaint::Shadow;
use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    Style, TextStyle, Visuals, vec2,
};

/// Bold UI family name (Noto Sans Bold).
pub const BOLD: &str = "Noto Sans Bold";

/// Light, dark, or the desktop's choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    System,
}

impl ThemeMode {
    pub const ALL: [(Self, &'static str); 3] = [
        (Self::Dark, "Dark"),
        (Self::Light, "Light"),
        (Self::System, "System"),
    ];

    /// The value stored in `settings.yaml` (`theme:`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }
}

/// Every color the app paints with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Window, toolbar, menus, dialogs.
    pub bg: Color32,
    /// The VM list.
    pub list_bg: Color32,
    /// Section bands.
    pub header: Color32,
    pub text: Color32,
    pub text_disabled: Color32,
    pub placeholder: Color32,
    /// Inputs and cards.
    pub input: Color32,
    pub border: Color32,
    pub button: Color32,
    pub separator: Color32,
    pub menu: Color32,
    /// Dialogs.
    pub overlay: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub focus: Color32,
    pub accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    /// Text colors for states and links, each at least 4.5:1 on `bg`, `input` and `header`.
    pub success_fg: Color32,
    pub warning_fg: Color32,
    pub error_fg: Color32,
    pub accent_fg: Color32,
    /// Captions and notes.
    pub text_muted: Color32,
    /// Neutral badge (waiting, not eligible).
    pub neutral: Color32,
    /// The Preview's frame.
    pub frame: Color32,
    /// Modal corner radius.
    pub radius_modal: u8,
    /// Menus and popups.
    pub radius_popup: u8,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Tokens {
    /// The Go app's light map (VirtualBox's Breeze colors).
    pub const fn light() -> Self {
        Self {
            dark: false,
            bg: rgb(0xEBEFF4),
            list_bg: rgb(0xF9FAFB),
            header: rgb(0xDCE0E5),
            text: rgb(0x232629),
            text_disabled: rgb(0xA0A4A8),
            placeholder: rgb(0x8C9094),
            input: rgb(0xFFFFFF),
            border: rgb(0xBCC0C4),
            button: rgb(0xF6FAFC),
            separator: rgb(0xC2C5C9),
            menu: rgb(0xFCFCFC),
            overlay: rgb(0xFCFCFC),
            selection: rgb(0x41B6F4),
            hover: Color32::from_rgba_premultiplied(0x0C, 0x23, 0x2F, 0x33),
            pressed: Color32::from_rgba_premultiplied(0x18, 0x45, 0x5D, 0x66),
            focus: rgb(0x3DAEE9),
            accent: rgb(0x0A84FF),
            success: rgb(0x27AE60),
            warning: rgb(0xF67400),
            error: rgb(0xDA4453),
            success_fg: rgb(0x166B3B),
            warning_fg: rgb(0x9A4800),
            error_fg: rgb(0xB02A38),
            accent_fg: rgb(0x0058C7),
            text_muted: rgb(0x52565A),
            neutral: rgb(0xA0A4A8),
            frame: rgb(0x8C9094),
            radius_modal: 8,
            radius_popup: 4,
        }
    }

    /// Fyne's dark theme with the macOS accent, as the Go app shows it.
    pub const fn dark() -> Self {
        Self {
            dark: true,
            bg: rgb(0x171718),
            list_bg: rgb(0x171718),
            header: rgb(0x262628),
            text: rgb(0xF3F3F3),
            text_disabled: rgb(0x6A6A6C),
            placeholder: rgb(0x8A8A8C),
            input: rgb(0x202023),
            border: rgb(0x3C3C3F),
            button: rgb(0x2B2B2D),
            separator: rgb(0x2E2E30),
            menu: rgb(0x28292E),
            overlay: rgb(0x1E1F22),
            selection: rgb(0x15314F),
            hover: Color32::from_rgba_premultiplied(0x01, 0x0D, 0x1A, 0x1A),
            pressed: Color32::from_rgba_premultiplied(0x02, 0x1A, 0x33, 0x33),
            focus: rgb(0x0A84FF),
            accent: rgb(0x0A84FF),
            success: rgb(0x2FB36A),
            warning: rgb(0xF67400),
            error: rgb(0xE8434F),
            success_fg: rgb(0x5AD68F),
            warning_fg: rgb(0xFF9A3D),
            error_fg: rgb(0xFF7A84),
            accent_fg: rgb(0x5AAEFF),
            text_muted: rgb(0xA8A8AC),
            neutral: rgb(0x5C5C5E),
            frame: rgb(0x2E2E30),
            radius_modal: 8,
            radius_popup: 4,
        }
    }

    /// The modal's stronger shadow (dialogs float above the window).
    pub fn modal_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 2,
            color: Color32::from_black_alpha(0x59),
        }
    }

    /// Menus and popups: a lighter shadow.
    pub fn popup_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 3],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(0x40),
        }
    }
}

/// Noto Sans (regular and bold) first, egui's fonts as fallbacks.
pub fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let regular = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
    let bold = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");
    defs.font_data
        .insert("Noto Sans".into(), Arc::new(FontData::from_static(regular)));
    defs.font_data
        .insert(BOLD.into(), Arc::new(FontData::from_static(bold)));
    let mono = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
    defs.font_data.insert(
        "JetBrains Mono".into(),
        Arc::new(FontData::from_static(mono)),
    );
    let prop = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    defs.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Noto Sans".into());
    defs.families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrains Mono".into());
    defs.families.insert(
        FontFamily::Name(BOLD.into()),
        std::iter::once(BOLD.to_owned()).chain(prop).collect(),
    );
    defs
}

/// Install the fonts and the style for `t`.
pub fn apply(ctx: &Context, t: &Tokens) {
    ctx.set_global_style(style(t));
}

/// The full egui style: 13 px text, Fyne-like spacing, square controls, rounded modals.
pub fn style(t: &Tokens) -> Style {
    let mut s = Style {
        visuals: visuals(t),
        ..Style::default()
    };
    s.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
        (
            TextStyle::Heading,
            FontId::new(15.0, FontFamily::Name(BOLD.into())),
        ),
    ]
    .into();
    let sp = &mut s.spacing;
    sp.item_spacing = vec2(8.0, 4.0);
    sp.button_padding = vec2(8.0, 4.0);
    sp.window_margin = Margin::same(12);
    sp.menu_margin = Margin::same(4);
    sp.interact_size = vec2(40.0, 26.0);
    sp.combo_height = 300.0;
    // Snapshots must not depend on time.
    s.animation_time = if cfg!(test) { 0.0 } else { 0.08 };
    s.visuals.text_cursor.blink = !cfg!(test);
    s
}

/// egui visuals from tokens: square controls like the Go app, rounded popups and modals.
pub fn visuals(t: &Tokens) -> Visuals {
    let mut v = if t.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let text = Stroke::new(1.0, t.text);
    let border = Stroke::new(1.0, t.border);
    v.panel_fill = t.bg;
    v.window_fill = t.overlay;
    v.window_stroke = Stroke::new(1.0, t.separator);
    v.window_shadow = t.modal_shadow();
    v.popup_shadow = t.popup_shadow();
    v.window_corner_radius = CornerRadius::same(t.radius_modal);
    v.menu_corner_radius = CornerRadius::same(t.radius_popup);
    v.extreme_bg_color = t.input;
    v.text_edit_bg_color = Some(t.input);
    v.faint_bg_color = t.bg;
    v.code_bg_color = t.input;
    v.hyperlink_color = t.accent;
    v.error_fg_color = t.error;
    v.warn_fg_color = t.warning;
    v.weak_text_color = Some(t.placeholder);
    v.selection.bg_fill = t.selection;
    v.selection.stroke = Stroke::new(1.0, t.text);
    v.override_text_color = None;
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.bg;
    w.noninteractive.weak_bg_fill = t.bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
    w.noninteractive.fg_stroke = text;
    w.inactive.weak_bg_fill = t.button;
    w.inactive.bg_fill = t.input;
    w.inactive.bg_stroke = border;
    w.inactive.fg_stroke = text;
    w.hovered.weak_bg_fill = blend(t.button, t.hover);
    w.hovered.bg_fill = blend(t.input, t.hover);
    w.hovered.bg_stroke = Stroke::new(1.0, t.focus);
    w.hovered.fg_stroke = text;
    w.active.weak_bg_fill = blend(t.button, t.pressed);
    w.active.bg_fill = blend(t.input, t.pressed);
    w.active.bg_stroke = Stroke::new(1.0, t.focus);
    w.active.fg_stroke = text;
    w.open.weak_bg_fill = t.button;
    w.open.bg_fill = t.input;
    w.open.bg_stroke = Stroke::new(1.0, t.focus);
    w.open.fg_stroke = text;
    for w in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.expansion = 0.0;
    }
    v
}

/// `over` (premultiplied, translucent) painted on `base`.
pub fn blend(base: Color32, over: Color32) -> Color32 {
    let a = u16::from(over.a());
    let mix = |b: u8, o: u8| (u16::from(o) + u16::from(b) * (255 - a) / 255).min(255) as u8;
    Color32::from_rgb(
        mix(base.r(), over.r()),
        mix(base.g(), over.g()),
        mix(base.b(), over.b()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_palette_matches_the_go_app() {
        let t = Tokens::light();
        assert_eq!(
            (t.bg, t.header, t.selection),
            (rgb(0xEBEFF4), rgb(0xDCE0E5), rgb(0x41B6F4))
        );
    }

    fn luminance(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    pub(crate) fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_reaches_aa_contrast_on_every_surface() {
        for t in [Tokens::light(), Tokens::dark()] {
            for (bg_name, bg) in [
                ("bg", t.bg),
                ("list", t.list_bg),
                ("header", t.header),
                ("input", t.input),
                ("overlay", t.overlay),
            ] {
                for (name, fg) in [
                    ("text", t.text),
                    ("muted", t.text_muted),
                    ("success", t.success_fg),
                    ("warning", t.warning_fg),
                    ("error", t.error_fg),
                    ("accent", t.accent_fg),
                ] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.5, "{name} on {bg_name} (dark={}): {c:.2}", t.dark);
                }
            }
            assert!(
                contrast(t.text, t.selection) >= 4.5,
                "selected row (dark={})",
                t.dark
            );
            assert!(
                contrast(Color32::WHITE, t.accent) >= 3.0,
                "primary button label (bold)"
            );
        }
    }

    #[test]
    fn modals_are_rounded_with_a_stronger_shadow_than_popups() {
        for t in [Tokens::light(), Tokens::dark()] {
            let v = visuals(&t);
            assert_eq!(v.window_corner_radius, CornerRadius::same(8));
            assert!(v.window_shadow.blur > v.popup_shadow.blur);
            assert_eq!(v.widgets.inactive.corner_radius, CornerRadius::ZERO);
        }
    }
}
