//! The panel's controls, drawn in the host's [`Look`]: labels, buttons, option cells, badges,
//! fields, progress and the few glyphs it needs (drawn, so no font has to carry them).

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, CursorIcon, FontFamily, FontId, Galley, Painter, Pos2, Rect, Response, Role, Sense, Stroke, StrokeKind, TextEdit, TextFormat,
    Ui, Vec2, WidgetInfo, pos2, vec2,
};

use crate::panel::look::{Look, Palette};

/// The look and this frame's palette.
#[derive(Clone)]
pub struct Skin {
    pub look: Look,
    pub p: Palette,
}

/// Letter spacing suits Latin, Greek, Cyrillic and CJK; it breaks up joined scripts.
fn spaceable(text: &str) -> bool {
    text.chars().all(|c| (c as u32) < 0x0590 || (c as u32) >= 0x2E80)
}

impl Skin {
    pub fn h(&self) -> f32 {
        self.look.control_height
    }

    /// `text` on one line, upper case and spaced out when the look says so.
    pub fn caps_galley(&self, ui: &Ui, text: &str, size: f32, family: FontFamily, color: Color32, max_width: Option<f32>) -> Arc<Galley> {
        // Small capitals and letter spacing suit Latin, Greek, Cyrillic and CJK; joined scripts
        // get neither, and a size step up to read as well at label size.
        let joined = !spaceable(text);
        let size = if joined { size * 1.15 } else { size };
        let (text, spacing) = if self.look.caps { (text.to_uppercase(), if joined { 0.0 } else { size * 0.14 }) } else { (text.to_owned(), 0.0) };
        let format = TextFormat { font_id: FontId::new(size, family), color, extra_letter_spacing: spacing, ..Default::default() };
        let mut job = LayoutJob::single_section(text, format);
        if let Some(w) = max_width {
            job.wrap = TextWrapping::truncate_at_width(w.max(0.0));
        }
        ui.painter().layout_job(job)
    }

    /// A small label: a section title, a count, a fact.
    pub fn label_galley(&self, ui: &Ui, text: &str, color: Color32) -> Arc<Galley> {
        self.caps_galley(ui, text, if self.look.caps { 11.0 } else { 12.0 }, self.look.label.clone(), color, None)
    }

    pub fn button_galley(&self, ui: &Ui, text: &str, color: Color32) -> Arc<Galley> {
        self.caps_galley(ui, text, if self.look.caps { 11.0 } else { 13.0 }, self.look.button.clone(), color, None)
    }

    pub fn hairline(&self) -> Stroke {
        self.p.hairline()
    }
}

/// Paints `galley` at the reading start of `rect`, centred vertically.
pub fn paint_start(ui: &Ui, rect: Rect, galley: Arc<Galley>, color: Color32) {
    let x = if craft_ui::direction::is_rtl(ui) { rect.right() - galley.size().x } else { rect.left() };
    ui.painter().galley_with_override_text_color(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// Paints `galley` at the reading end of `rect`, centred vertically.
pub fn paint_end(ui: &Ui, rect: Rect, galley: Arc<Galley>, color: Color32) {
    let x = if craft_ui::direction::is_rtl(ui) { rect.left() } else { rect.right() - galley.size().x };
    ui.painter().galley_with_override_text_color(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
}

pub fn paint_centred(ui: &Ui, rect: Rect, galley: Arc<Galley>, color: Color32) {
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
}

/// A small label.
pub fn label(ui: &mut Ui, s: &Skin, text: &str, color: Color32) -> Response {
    let galley = s.label_galley(ui, text, color);
    // Joined scripts can draw a pixel or two past their measured width: a little room each side.
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(4.0, 0.0), Sense::hover());
    paint_start(ui, rect.shrink2(vec2(2.0, 0.0)), galley, color);
    resp.widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
    resp
}

/// A focus ring inside `rect`, while `resp` has the keyboard focus.
pub fn focus_ring(ui: &Ui, s: &Skin, rect: Rect, resp: &Response) {
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, s.look.corner, Stroke::new(2.0, s.p.accent_ink), StrokeKind::Inside);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Solid, in the inverted colours.
    Primary,
    /// Outlined.
    Secondary,
    /// Outlined in the error colour.
    Danger,
}

/// A button `height` tall and at least `min_width` wide.
pub fn button(ui: &mut Ui, s: &Skin, text: &str, kind: Kind, height: f32, min_width: f32) -> Response {
    let p = s.p;
    let probe = s.button_galley(ui, text, p.ink);
    let size = vec2((probe.size().x + 28.0).max(min_width), height);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    let hovered = resp.hovered() && enabled;
    let (fill, stroke, color) = match kind {
        Kind::Primary => (p.invert_bg.gamma_multiply(if hovered { 0.82 } else { 1.0 }), Stroke::NONE, p.invert_fg),
        Kind::Secondary if hovered => (p.invert_bg, Stroke::NONE, p.invert_fg),
        Kind::Secondary => (Color32::TRANSPARENT, Stroke::new(1.0, p.line_strong), p.ink),
        Kind::Danger if hovered => (p.bad, Stroke::NONE, Color32::WHITE),
        Kind::Danger => (Color32::TRANSPARENT, Stroke::new(1.0, p.bad.gamma_multiply(0.7)), p.bad),
    };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    ui.painter().rect(rect, s.look.corner, fade(fill), Stroke::new(stroke.width, fade(stroke.color)), StrokeKind::Inside);
    let galley = s.button_galley(ui, text, fade(color));
    paint_centred(ui, rect, galley, fade(color));
    focus_ring(ui, s, rect.expand(2.0), &resp);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, text));
    if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp }
}

/// A text-only action in the accent colour: "Clear filters", "Retry".
pub fn link(ui: &mut Ui, s: &Skin, text: &str) -> Response {
    let galley = s.label_galley(ui, text, s.p.accent_ink);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(4.0, 6.0), Sense::click());
    let color = if resp.hovered() { s.p.ink_strong } else { s.p.accent_ink };
    paint_start(ui, rect.shrink2(vec2(2.0, 0.0)), galley, color);
    if resp.hovered() {
        let y = rect.bottom() - 1.5;
        ui.painter().hline(rect.x_range(), y, Stroke::new(1.0, color));
    }
    focus_ring(ui, s, rect.expand(3.0), &resp);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, text));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// One option in a group of cells: its value, its label and an optional count.
pub struct Cell<'a, T> {
    pub value: T,
    pub label: &'a str,
    pub count: Option<usize>,
}

fn count_text(n: usize) -> String {
    crate::panel::i18n::number(n as u64)
}

/// Options stacked in a column of hairline cells the width of `ui`; the chosen one inverted.
/// Returns whether the choice changed.
pub fn cells_column<T: PartialEq + Clone>(ui: &mut Ui, s: &Skin, value: &mut T, options: &[Cell<'_, T>]) -> bool {
    let p = s.p;
    let mut changed = false;
    let width = ui.available_width();
    let h = s.h() - 2.0;
    let mut rects = Vec::with_capacity(options.len());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for cell in options {
            let on = *value == cell.value;
            let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
            let (fill, ink, faint) = match (on, resp.hovered()) {
                (true, _) => (p.invert_bg, p.invert_fg, p.invert_fg.gamma_multiply(0.6)),
                (false, true) => (p.sunken, p.ink_strong, p.muted),
                (false, false) => (Color32::TRANSPARENT, p.ink, p.faint),
            };
            ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, fill);
            let inner = rect.shrink2(vec2(12.0, 0.0));
            let galley = s.label_galley(ui, cell.label, ink);
            paint_start(ui, inner, galley, ink);
            if let Some(n) = cell.count {
                let g = s.label_galley(ui, &count_text(n), faint);
                paint_end(ui, inner, g, faint);
            }
            focus_ring(ui, s, rect.shrink(1.0), &resp);
            resp.widget_info(|| {
                let mut info = WidgetInfo::selected(Role::RadioButton, true, on, cell.label);
                info.value = cell.count.map(|n| n as f64);
                info
            });
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !on {
                *value = cell.value.clone();
                changed = true;
            }
            rects.push(rect);
        }
    });
    if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
        let painter = ui.painter();
        painter.rect_stroke(first.union(*last), s.look.corner, s.hairline(), StrokeKind::Inside);
        for r in rects.iter().skip(1) {
            painter.hline(r.x_range(), r.top(), s.hairline());
        }
    }
    changed
}

/// Options side by side in hairline cells, in reading order; the chosen one inverted.
pub fn cells_row<T: PartialEq + Clone>(ui: &mut Ui, s: &Skin, value: &mut T, options: &[Cell<'_, T>]) -> bool {
    let p = s.p;
    let mut changed = false;
    let rtl = craft_ui::direction::is_rtl(ui) || ui.layout().prefer_right_to_left();
    let mut rects = Vec::with_capacity(options.len());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for cell in options {
            let on = *value == cell.value;
            let (ink, faint) = if on { (p.invert_fg, p.invert_fg.gamma_multiply(0.6)) } else { (p.muted, p.faint) };
            let label = s.label_galley(ui, cell.label, ink);
            let count = cell.count.map(|n| s.label_galley(ui, &count_text(n), faint));
            let w = label.size().x + count.as_ref().map_or(0.0, |g| g.size().x + 8.0) + 28.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, s.h()), Sense::click());
            let fill = match (on, resp.hovered()) {
                (true, _) => p.invert_bg,
                (false, true) => p.sunken,
                (false, false) => Color32::TRANSPARENT,
            };
            let ink = if !on && resp.hovered() { p.ink_strong } else { ink };
            ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, fill);
            let inner = rect.shrink2(vec2(14.0, 0.0));
            let label_w = label.size().x;
            paint_start(ui, inner, label, ink);
            if let Some(g) = count {
                let rest = if rtl { inner.with_max_x(inner.right() - label_w - 8.0) } else { inner.with_min_x(inner.left() + label_w + 8.0) };
                paint_start(ui, rest, g, faint);
            }
            focus_ring(ui, s, rect.shrink(1.0), &resp);
            resp.widget_info(|| {
                let mut info = WidgetInfo::selected(Role::Tab, true, on, cell.label);
                info.value = cell.count.map(|n| n as f64);
                info
            });
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !on {
                *value = cell.value.clone();
                changed = true;
            }
            rects.push(rect);
        }
    });
    if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
        let painter = ui.painter();
        painter.rect_stroke(first.union(*last), s.look.corner, s.hairline(), StrokeKind::Inside);
        for r in rects.iter().skip(1) {
            let x = if rtl { r.right() } else { r.left() };
            painter.vline(x, first.y_range(), s.hairline());
        }
    }
    changed
}

/// A small square status label on the sunken surface, with an optional dot before it.
pub fn badge(ui: &mut Ui, s: &Skin, text: &str, color: Color32, dot: bool) -> Response {
    let galley = s.caps_galley(ui, text, 10.0, s.look.label.clone(), color, None);
    let dot_w = if dot { 12.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 16.0 + dot_w, 22.0), Sense::hover());
    ui.painter().rect(rect, s.look.corner, s.p.sunken, s.hairline(), StrokeKind::Inside);
    let rtl = craft_ui::direction::is_rtl(ui);
    let inner = rect.shrink2(vec2(8.0, 0.0));
    if dot {
        let x = if rtl { inner.right() - 3.0 } else { inner.left() + 3.0 };
        ui.painter().circle_filled(pos2(x, rect.center().y), 3.0, color);
    }
    let text_rect = if rtl { inner.with_max_x(inner.right() - dot_w) } else { inner.with_min_x(inner.left() + dot_w) };
    paint_start(ui, text_rect, galley, color);
    resp.widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
    resp
}

/// A slim bar along the bottom of `rect`: `fraction` done, or a moving segment when unknown.
pub fn progress_bar(ui: &Ui, s: &Skin, rect: Rect, fraction: Option<f32>) {
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::ZERO, s.p.line);
    let rtl = craft_ui::direction::is_rtl(ui);
    match fraction {
        Some(f) => {
            let w = rect.width() * f.clamp(0.0, 1.0);
            let bar = if rtl {
                Rect::from_min_max(pos2(rect.right() - w, rect.top()), rect.max)
            } else {
                Rect::from_min_size(rect.min, vec2(w, rect.height()))
            };
            painter.rect_filled(bar, egui::CornerRadius::ZERO, s.p.accent);
        }
        None => {
            let w = rect.width();
            let x = (ui.input(|i| i.time) * 0.8).fract() as f32 * w;
            for left in [x, x - w] {
                let seg = Rect::from_min_size(pos2(rect.left() + left, rect.top()), vec2(w * 0.3, rect.height())).intersect(rect);
                if seg.is_positive() {
                    painter.rect_filled(seg, egui::CornerRadius::ZERO, s.p.accent);
                }
            }
            ui.ctx().request_repaint();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Search,
    Close,
    Info,
}

/// Draws `glyph` centred on `c`, `size` across.
pub fn glyph(painter: &Painter, glyph: Glyph, c: Pos2, size: f32, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    let r = size / 2.0;
    match glyph {
        Glyph::Search => {
            let centre = c + vec2(-r * 0.2, -r * 0.2);
            painter.circle_stroke(centre, r * 0.6, stroke);
            painter.line_segment([centre + vec2(r * 0.45, r * 0.45), c + vec2(r, r)], stroke);
        }
        Glyph::Close => {
            let k = r * 0.75;
            painter.line_segment([c + vec2(-k, -k), c + vec2(k, k)], stroke);
            painter.line_segment([c + vec2(-k, k), c + vec2(k, -k)], stroke);
        }
        Glyph::Info => {
            painter.circle_stroke(c, r * 0.85, Stroke::new(1.2, color));
            painter.line_segment([c + vec2(0.0, -r * 0.05), c + vec2(0.0, r * 0.45)], Stroke::new(1.4, color));
            painter.circle_filled(c + vec2(0.0, -r * 0.38), 1.0, color);
        }
    }
}

/// A square button with a drawn glyph, outlined, inverting on hover.
pub fn glyph_button(ui: &mut Ui, s: &Skin, g: Glyph, tip: &str, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let hovered = resp.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, s.look.corner, s.p.invert_bg);
    } else {
        painter.rect_stroke(rect, s.look.corner, s.hairline(), StrokeKind::Inside);
    }
    let color = if hovered { s.p.invert_fg } else { s.p.muted };
    glyph(painter, g, rect.center(), 12.0, color);
    focus_ring(ui, s, rect, &resp);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, tip));
    resp.on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand)
}

/// Names a widget for screen readers (and kittest) where it has no label of its own.
pub fn name(ui: &Ui, resp: &Response, label: &str) {
    ui.ctx().accesskit_node_builder(resp.id, |node| node.set_label(label));
}

/// A single-line field `width` wide, named `label`, with a search glyph at its start when
/// `search`.
pub fn field(ui: &mut Ui, s: &Skin, text: &mut String, hint: &str, label: &str, width: f32, search: bool) -> Response {
    let rtl = craft_ui::direction::is_rtl(ui);
    let pad = if search { 34.0 } else { 10.0 };
    let margin = if rtl {
        egui::Margin { left: 10, right: pad as i8, top: 0, bottom: 0 }
    } else {
        egui::Margin { left: pad as i8, right: 10, top: 0, bottom: 0 }
    };
    let edit = TextEdit::singleline(text)
        .hint_text(hint)
        .desired_width(width)
        .min_size(vec2(0.0, s.h()))
        .align(if rtl { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER })
        .margin(margin);
    let resp = ui.add(edit);
    if search {
        let x = if rtl { resp.rect.right() - 18.0 } else { resp.rect.left() + 18.0 };
        glyph(ui.painter(), Glyph::Search, pos2(x, resp.rect.center().y), 13.0, s.p.faint);
    }
    name(ui, &resp, label);
    resp
}

/// A drop-down `width` wide showing `selected`, named `label`.
pub fn select<R>(ui: &mut Ui, id_salt: &str, label: &str, selected: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let inner = select_box(id_salt, selected, width).show_ui(ui, add);
    name(ui, &inner.response, label);
    inner.inner
}

/// The combo box of [`select`], for a menu with a header; name its response with [`name`].
pub fn select_box<'a>(id_salt: &str, selected: &'a str, width: f32) -> craft_ui::ComboBox<'a> {
    craft_ui::ComboBox::from_id_salt(id_salt).selected_text(selected).width(width).height(380.0).truncate()
}

/// An inline note in a sunken, hairline box: an info glyph, then `add`.
pub fn note<R>(ui: &mut Ui, s: &Skin, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(s.p.sunken)
        .stroke(s.hairline())
        .corner_radius(s.look.corner)
        .inner_margin(egui::Margin { left: 14, right: 12, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            craft_ui::direction::horizontal(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(16.0, s.h().min(28.0)), Sense::hover());
                glyph(ui.painter(), Glyph::Info, rect.center(), 14.0, s.p.muted);
                ui.add_space(4.0);
                add(ui)
            })
            .inner
        })
        .inner
}

/// A [`note`] with `add` beside buttons at its reading end: one per label in `buttons`, then a
/// close glyph when `closable`. `button(ui, n)` draws the `n`th, the close glyph last.
pub fn note_with_actions(ui: &mut Ui, s: &Skin, add: impl FnOnce(&mut Ui), buttons: &[&str], mut button: impl FnMut(&mut Ui, usize), closable: bool) {
    let gap = 10.0;
    let actions_w: f32 =
        buttons.iter().map(|b| s.button_galley(ui, b, s.p.ink).size().x + 28.0 + gap).sum::<f32>() + if closable { 28.0 + gap } else { 0.0 };
    note(ui, s, |ui| {
        let text_w = (ui.available_width() - actions_w - 8.0).max(120.0);
        ui.allocate_ui_with_layout(vec2(text_w, 0.0), craft_ui::direction::column(ui), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            add(ui);
        });
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 28.0), craft_ui::direction::trailing(ui), |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if closable {
                button(ui, buttons.len());
            }
            for n in (0..buttons.len()).rev() {
                button(ui, n);
            }
        });
    });
}

/// A slim slider `width` wide for whole numbers in `range`, named `label`: drag or click the
/// track, or step with the arrow keys (Shift for ten). It reads left to right in every interface
/// direction, as sliders do.
pub fn slider(ui: &mut Ui, s: &Skin, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32, label: &str) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, s.h()), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(width - 14.0, 2.0));
    let before = *value;
    if let Some(pos) = resp.interact_pointer_pos() {
        let t = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
        *value = (lo + t * (hi - lo)).round();
    }
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
        });
        let step = ui.input(|i| {
            let n = if i.modifiers.shift { 10.0 } else { 1.0 };
            let up = i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp);
            let down = i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown);
            if up {
                n
            } else if down {
                -n
            } else {
                0.0
            }
        });
        *value = (*value + step).clamp(lo, hi);
    }
    if *value != before {
        resp.mark_changed();
    }
    let t = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0);
    let x = track.left() + t * track.width();
    let painter = ui.painter();
    painter.rect_filled(track, egui::CornerRadius::ZERO, s.p.line_strong);
    painter.rect_filled(Rect::from_min_max(track.min, pos2(x, track.bottom())), egui::CornerRadius::ZERO, s.p.ink);
    let handle = Rect::from_center_size(pos2(x, rect.center().y), vec2(12.0, 12.0));
    let hovered = resp.hovered() || resp.dragged();
    painter.rect(
        handle,
        egui::CornerRadius::ZERO,
        if hovered { s.p.ink_strong } else { s.p.invert_bg },
        Stroke::new(2.0, s.p.bg),
        StrokeKind::Outside,
    );
    if resp.has_focus() {
        painter.rect_stroke(handle.expand(4.0), egui::CornerRadius::ZERO, Stroke::new(2.0, s.p.accent_ink), StrokeKind::Inside);
    }
    resp.widget_info(|| WidgetInfo::slider(true, f64::from(*value), label));
    resp.on_hover_cursor(CursorIcon::ResizeHorizontal)
}
