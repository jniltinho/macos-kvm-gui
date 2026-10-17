//! Building blocks: toolbar buttons, section bands, key/value grids, item cards, buttons, combo
//! boxes, slider rows, the progress line, modals and the keyboard focus ring.
//!
//! Patterns adapted from rustXterm `crates/ui/src/widgets.rs`, `dialogs.rs` and `theme.rs`
//! (focus ring) (MIT) and PhotoCraft `crates/ui-egui/src/widgets.rs` (cards, dialog buttons)
//! (MIT OR Apache-2.0); the look is the Go app's `internal/gui/vbox.go` and `install.go`.

use egui::{
    Align, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Id, Layout, Margin, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType, emath::easing, vec2,
};

use crate::icons;
use crate::theme::{BOLD, Tokens};

/// Hover and press transitions: 150 ms ease-out (none in tests, so snapshots are stable).
pub const ANIM: f32 = if cfg!(test) { 0.0 } else { 0.15 };

/// 0..1 following `on`, eased out.
pub fn anim(ui: &Ui, id: Id, on: bool) -> f32 {
    ui.ctx()
        .animate_bool_with_time_and_easing(id, on, ANIM, easing::cubic_out)
}

pub fn bold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(FontFamily::Name(BOLD.into()))
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// The tokens stored for this frame (see [`set_tokens`]).
pub fn tokens(ui: &Ui) -> Tokens {
    ui.ctx()
        .data(|d| d.get_temp(Id::new("rg-tokens")))
        .unwrap_or_else(Tokens::light)
}

pub fn set_tokens(ctx: &egui::Context, t: Tokens) {
    ctx.data_mut(|d| d.insert_temp(Id::new("rg-tokens"), t));
}

/// A VirtualBox toolbar button: a 32 px icon over its label, a soft hover fill.
pub fn tool_button(ui: &mut Ui, icon: &str, label: &str, enabled: bool, tip: &str) -> Response {
    let t = tokens(ui);
    let font = FontId::proportional(13.0);
    let text_w = ui.fonts_mut(|f| {
        f.layout_no_wrap(label.into(), font.clone(), t.text)
            .size()
            .x
    });
    let size = vec2((text_w + 14.0).max(50.0), 56.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    let hover = anim(ui, resp.id, enabled && resp.hovered());
    let press = anim(
        ui,
        resp.id.with("press"),
        enabled && resp.is_pointer_button_down_on(),
    );
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if hover > 0.0 {
            p.rect_filled(rect, 4, t.hover.gamma_multiply(hover));
        }
        if press > 0.0 {
            p.rect_filled(rect, 4, t.pressed.gamma_multiply(press));
        }
        let icon_rect = Rect::from_center_size(
            rect.center_top() + vec2(0.0, 4.0 + 16.0 + press),
            vec2(32.0, 32.0),
        );
        let tint = if enabled {
            Color32::WHITE
        } else {
            Color32::from_white_alpha(90)
        };
        egui::Image::new(icons::app(icon))
            .tint(tint)
            .paint_at(ui, icon_rect);
        let color = if enabled { t.text } else { t.text_disabled };
        p.text(
            rect.center_bottom() - vec2(0.0, 4.0),
            Align2::CENTER_BOTTOM,
            label,
            font,
            color,
        );
    }
    if enabled {
        resp.on_hover_text(tip)
    } else {
        resp.on_disabled_hover_text(tip)
    }
}

/// The tinted band at the top of a Details or Settings section: an icon and a bold title.
pub fn section_band(ui: &mut Ui, icon: impl Into<egui::Image<'static>>, title: &str) {
    let t = tokens(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 0, t.header);
    let ir = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(16.0, 16.0));
    icon.into()
        .fit_to_exact_size(vec2(16.0, 16.0))
        .paint_at(ui, ir);
    p.text(
        rect.left_center() + vec2(28.0, 0.0),
        Align2::LEFT_CENTER,
        title,
        bold_font(13.0),
        t.text,
    );
}

/// Compact "Key:  value" lines; empty values hide their row.
pub fn kv_grid(ui: &mut Ui, id: &str, rows: &[(&str, String)]) {
    Frame::NONE
        .inner_margin(Margin {
            left: 6,
            right: 6,
            top: 4,
            bottom: 6,
        })
        .show(ui, |ui| {
            egui::Grid::new(id)
                .num_columns(2)
                .spacing([14.0, 1.0])
                .min_row_height(18.0)
                .show(ui, |ui| {
                    for (k, v) in rows.iter().filter(|(_, v)| !v.is_empty()) {
                        ui.label(format!("{k}:"));
                        ui.add(egui::Label::new(v.as_str()).truncate());
                        ui.end_row();
                    }
                });
        });
}

/// Italic, muted, wrapped text (the Go app's `note`).
pub fn note(ui: &mut Ui, text: &str) {
    let t = tokens(ui);
    ui.add(egui::Label::new(RichText::new(text).italics().color(t.text_muted)).wrap());
}

/// Small wrapped text (the Go app's `caption`).
pub fn caption(ui: &mut Ui, text: &str) {
    let t = tokens(ui);
    ui.add(egui::Label::new(RichText::new(text).small().color(t.text_muted)).wrap());
}

/// Wrapped text in a state color (warnings in dialogs).
pub fn warning(ui: &mut Ui, text: &str) {
    let t = tokens(ui);
    ui.horizontal_top(|ui| {
        ui.add(icons::lucide("triangle-alert", 14.0, t.warning_fg));
        ui.add(egui::Label::new(RichText::new(text).color(t.warning_fg)).wrap());
    });
}

/// A raised surface: input-colored, 4 px radius, a soft shadow.
pub fn card_frame(t: &Tokens) -> Frame {
    Frame::new()
        .fill(t.input)
        .corner_radius(4)
        .inner_margin(Margin::same(10))
        .shadow(egui::Shadow {
            offset: [0, 1],
            blur: 4,
            spread: 0,
            color: Color32::from_black_alpha(if t.dark { 60 } else { 22 }),
        })
}

/// A static card for notes and controls.
pub fn surface<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = tokens(ui);
    card_frame(&t)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A card's square badge: a number or an icon on a state color.
pub enum Badge<'a> {
    Number(&'a str),
    Icon(&'a str),
}

/// One row of the Install, Host and GPU tabs: badge, title, description, state at the right,
/// an optional detail line, an accent stripe when it needs attention, and its controls.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the Go itemCard's fields"
)]
pub fn item_card(
    ui: &mut Ui,
    badge: Badge<'_>,
    badge_color: Color32,
    title: &str,
    desc: &str,
    state: (&str, Color32),
    detail: &str,
    accent: bool,
    body: impl FnOnce(&mut Ui),
) {
    let t = tokens(ui);
    let resp = card_frame(&t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
            ui.painter().rect_filled(r, 3, badge_color);
            match badge {
                Badge::Number(n) => {
                    ui.painter().text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        n,
                        bold_font(14.0),
                        Color32::WHITE,
                    );
                }
                Badge::Icon(i) => icons::lucide(i, 18.0, Color32::WHITE)
                    .paint_at(ui, Rect::from_center_size(r.center(), vec2(18.0, 18.0))),
            }
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let w = ui.available_width() - 90.0;
                ui.horizontal(|ui| {
                    ui.set_width(ui.available_width());
                    ui.allocate_ui_with_layout(
                        vec2(w.max(40.0), 18.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.add(egui::Label::new(bold(title)).truncate());
                        },
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(bold(state.0).color(state.1));
                    });
                });
                if !desc.is_empty() {
                    caption(ui, desc);
                }
                if !detail.is_empty() {
                    ui.add(egui::Label::new(detail).wrap());
                }
                body(ui);
            });
        });
    });
    if accent {
        let r = resp.response.rect;
        ui.painter().rect_filled(
            Rect::from_min_size(r.min, vec2(3.0, r.height())),
            0,
            t.accent,
        );
    }
}

/// A button with a Lucide icon; `primary` is the accent-filled default action.
pub fn button(ui: &mut Ui, icon: &str, label: &str, primary: bool, enabled: bool) -> Response {
    let t = tokens(ui);
    let fg = match (primary, enabled) {
        (_, false) => t.text_disabled,
        (true, true) => Color32::WHITE,
        (false, true) => t.text,
    };
    let text = if primary {
        bold(label).color(fg)
    } else {
        RichText::new(label).color(fg)
    };
    let mut b =
        egui::Button::image_and_text(icons::lucide(icon, 15.0, fg), text).min_size(vec2(0.0, 30.0));
    if primary && enabled {
        b = b.fill(t.accent).stroke(Stroke::NONE);
    } else {
        b = b.fill(t.button);
    }
    ui.add_enabled(enabled, b)
}

/// A combo box of `options` (value, label) that sets `value`; true when the user changed it.
pub fn combo<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    let current = options
        .iter()
        .find(|(v, _)| v == value)
        .map_or("", |(_, l)| *l);
    let mut changed = false;
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(width)
        .show_ui(ui, |ui| {
            for (v, l) in options {
                if ui.selectable_label(v == value, *l).clicked() && v != value {
                    *value = v.clone();
                    changed = true;
                }
            }
        });
    if r.inner.is_some() {
        // In a modal, a click whose press and release land in one frame (an idle window, a
        // touchpad tap) opens the popup and then raises the modal over it at the end of that
        // frame: raise the popup (egui's `ComboBox::widget_to_popup_id`) and draw once more.
        let layer = egui::LayerId::new(egui::Order::Foreground, r.response.id.with("popup"));
        let ctx = ui.ctx();
        let top = ctx.memory(|m| {
            m.layer_ids()
                .filter(|l| l.order == egui::Order::Foreground)
                .last()
        });
        if top != Some(layer) {
            ctx.move_to_top(layer);
            ctx.request_repaint();
        }
    }
    changed
}

/// A slider with a value box and the range below it (VirtualBox's memory and CPU rows).
pub fn slider_row(ui: &mut Ui, value: &mut u32, min: u32, max: u32, step: u32, unit: &str) {
    let t = tokens(ui);
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 150.0).max(120.0);
        ui.vertical(|ui| {
            ui.spacing_mut().slider_width = w;
            ui.add(
                egui::Slider::new(value, min..=max)
                    .step_by(f64::from(step))
                    .show_value(false),
            );
            ui.horizontal(|ui| {
                ui.set_width(w);
                ui.label(RichText::new(amount(min, unit)).small().color(t.text_muted));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(amount(max, unit)).small().color(t.text_muted));
                });
            });
        });
        ui.add(
            egui::DragValue::new(value)
                .range(min..=max)
                .speed(f64::from(step)),
        );
        ui.label(unit);
    });
}

/// "1 CPU" or "24 CPUs"; units such as MB do not change.
pub fn amount(n: u32, unit: &str) -> String {
    match unit.strip_suffix('s') {
        Some(one) if n == 1 => format!("{n} {one}"),
        _ => format!("{n} {unit}"),
    }
}

/// A thin progress bar, its fill animated toward `value`.
pub fn progress_line(ui: &mut Ui, id: Id, value: f32) {
    let t = tokens(ui);
    let v = ui
        .ctx()
        .animate_value_with_time(id, value.clamp(0.0, 1.0), ANIM);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    ui.painter().rect_filled(r, 3, t.header);
    ui.painter().rect_filled(
        Rect::from_min_size(r.min, vec2(r.width() * v, r.height())),
        3,
        t.success,
    );
}

/// A monospace command with a Copy button.
pub fn command(ui: &mut Ui, text: &str) {
    let t = tokens(ui);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if button(ui, "copy", "Copy", false, true)
            .on_hover_text("Copy to the clipboard")
            .clicked()
        {
            ui.ctx().copy_text(text.to_owned());
        }
        Frame::new()
            .fill(t.input)
            .stroke(Stroke::new(1.0, t.border))
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 14.0);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(text).monospace())
                            .selectable(true)
                            .wrap(),
                    );
                });
            });
    });
}

/// A form row: the label bold and right-aligned in a fixed column, then the control.
pub fn form_row<R>(ui: &mut Ui, label_w: f32, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            vec2(label_w, 26.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(bold(label));
            },
        );
        add(ui)
    })
    .inner
}

/// What a modal's footer reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Footer {
    None,
    Ok,
    Cancel,
}

/// A modal dialog: backdrop dim, 8 px radius, layered shadow, a bold title, the body and an
/// OK/Cancel footer. Enter is OK (outside multi-line fields), Esc is Cancel.
pub fn modal(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    size: egui::Vec2,
    ok: Option<&str>,
    cancel: Option<&str>,
    body: impl FnOnce(&mut Ui),
) -> Footer {
    let t: Tokens = ctx
        .data(|d| d.get_temp(Id::new("rg-tokens")))
        .unwrap_or_else(Tokens::light);
    let frame = Frame::new()
        .fill(t.overlay)
        .corner_radius(CornerRadius::same(t.radius_modal))
        .stroke(Stroke::new(1.0, t.separator))
        .shadow(t.modal_shadow())
        .inner_margin(Margin::same(16));
    let resp = egui::Modal::new(Id::new(id))
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(if t.dark { 120 } else { 70 }))
        .show(ctx, |ui| {
            ui.set_width(size.x);
            ui.label(bold(title).size(14.0));
            ui.add_space(6.0);
            let body_h = (size.y - 90.0).max(40.0);
            ui.allocate_ui_with_layout(vec2(size.x, body_h), Layout::top_down(Align::Min), |ui| {
                ui.set_min_height(if size.y > 0.0 { body_h } else { 0.0 });
                body(ui);
            });
            ui.add_space(6.0);
            ui.separator();
            let mut out = Footer::None;
            // A row as tall as its buttons: a bare right-to-left layout would take the height
            // left in the modal, which keeps the size of the last (taller) dialog shown under
            // the same id, and float the buttons in the middle of the gap.
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(c) = cancel
                        && button(ui, "x", c, false, true)
                            .on_hover_text("Esc")
                            .clicked()
                    {
                        out = Footer::Cancel;
                    }
                    if let Some(o) = ok
                        && button(ui, "check", o, true, true)
                            .on_hover_text("Enter")
                            .clicked()
                    {
                        out = Footer::Ok;
                    }
                });
            });
            out
        });
    let (enter, esc) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    match resp.inner {
        Footer::None if resp.is_top_modal && esc && !resp.any_popup_open => Footer::Cancel,
        Footer::None if resp.is_top_modal && enter && ok.is_some() && !resp.any_popup_open => {
            Footer::Ok
        }
        f => f,
    }
}

/// Paint the focus ring around the focused widget when focus came from the keyboard (Tab).
/// Adapted from rustXterm's `theme::focus_ring`.
pub fn focus_ring(ctx: &egui::Context, t: &Tokens) {
    #[derive(Clone, Copy, Default)]
    struct State {
        last: Option<Id>,
        keyboard: bool,
    }
    let id = Id::new("rg-focus-ring");
    let mut st: State = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let focused = ctx.memory(|m| m.focused());
    let (tab, pointer) = ctx.input(|i| (i.key_pressed(egui::Key::Tab), i.pointer.any_pressed()));
    if pointer {
        st.keyboard = false;
    } else if tab {
        st.keyboard = true;
    }
    st.last = focused;
    ctx.data_mut(|d| d.insert_temp(id, st));
    let Some(rect) = focused
        .filter(|_| st.keyboard)
        .and_then(|f| ctx.read_response(f))
        .map(|r| r.rect)
    else {
        return;
    };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, id));
    painter.rect_stroke(
        rect.expand(2.0),
        3,
        Stroke::new(2.0, t.focus),
        StrokeKind::Outside,
    );
}
