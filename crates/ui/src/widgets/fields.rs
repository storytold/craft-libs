//! Numeric value fields: a monospace box with a dimmed unit suffix, scrubbable through its
//! label (see [`super::scrub`]), optionally with a ▾ that opens a pop-up slider. In a
//! right-to-left interface the suffix and the ▾ sit at the field's left (reading-end) edge, and
//! the pop-up opens under that edge; the number's slot, the drag semantics and the value↔x
//! mapping stay physical in both directions.

use std::ops::RangeInclusive;

use egui::{Align2, Color32, FontId, Popup, PopupCloseBehavior, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::direction::{self, HasDirection, is_rtl};
use crate::widgets::scrub::{label_scrub_response, take_label_scrub};

/// Height of a [`value_field`]'s box.
pub const VALUE_FIELD_H: f32 = 24.0;

/// Width of the ▾ that opens a [`popup_value_field`]'s slider, inside the field's box.
pub const POPUP_ARROW_W: f32 = 14.0;

/// Inset of the ▾ from the field's reading-end edge.
pub const POPUP_ARROW_INSET: f32 = 2.0;

/// Width of a [`popup_value_field`]'s pop-up slider.
pub const POPUP_SLIDER_W: f32 = 160.0;

/// Colors, fonts and radii for a value field; the application owns the theme.
#[derive(Clone, Debug)]
pub struct FieldStyle {
    pub fill: Color32,
    pub border: Color32,
    pub radius: f32,
    pub text: Color32,
    pub suffix: Color32,
    /// The number's font.
    pub font: FontId,
    /// The suffix's font.
    pub suffix_font: FontId,
    /// The ▾ arrow's hover fill multiplier.
    pub arrow_hover: f32,
}

impl Default for FieldStyle {
    fn default() -> Self {
        Self {
            fill: Color32::from_gray(40),
            border: Color32::from_gray(70),
            radius: 3.0,
            text: Color32::WHITE,
            suffix: Color32::from_gray(140),
            font: FontId::monospace(12.0),
            suffix_font: FontId::monospace(11.0),
            arrow_hover: 0.6,
        }
    }
}

/// The number's slot inside a field box `rect`: `suffix_w` wide suffix plus `trailing` points of
/// reserved space (a pop-up ▾) are cut from the reading-end edge, so in a right-to-left interface
/// they sit at the left edge. Returns the slot rect.
pub fn field_slot(d: &impl HasDirection, rect: Rect, suffix_w: f32, trailing: f32, row_h: f32) -> Rect {
    let slot_h = (rect.height() - 4.0).max(row_h).min(rect.height() - 2.0);
    let cy = rect.center().y;
    let (slot_l, slot_r) = if is_rtl(d) {
        (rect.left() + 4.0 + suffix_w + trailing, rect.right() - 4.0)
    } else {
        (rect.left() + 4.0, rect.right() - 4.0 - suffix_w - trailing)
    };
    Rect::from_min_max(pos2(slot_l, cy - slot_h / 2.0), pos2(slot_r, cy + slot_h / 2.0))
}

/// Where a suffix `suffix_w` wide anchors inside a field box `rect`, with `trailing` points
/// reserved beyond it: 6 pt from the reading-end edge, which is the right edge left-to-right and
/// the left edge right-to-left.
pub fn suffix_anchor(d: &impl HasDirection, rect: Rect, trailing: f32) -> (f32, Align2) {
    if is_rtl(d) { (rect.left() + 6.0 + trailing, Align2::LEFT_CENTER) } else { (rect.right() - 6.0 - trailing, Align2::RIGHT_CENTER) }
}

/// The ▾'s clickable rect inside a field box `rect`, at its reading-end edge (the right edge
/// left-to-right, the left edge right-to-left).
pub fn popup_arrow_rect(d: &impl HasDirection, rect: Rect) -> Rect {
    if is_rtl(d) {
        Rect::from_min_max(
            pos2(rect.left() + POPUP_ARROW_INSET, rect.top() + 2.0),
            pos2(rect.left() + POPUP_ARROW_W + POPUP_ARROW_INSET, rect.bottom() - 2.0),
        )
    } else {
        Rect::from_min_max(pos2(rect.right() - POPUP_ARROW_W - POPUP_ARROW_INSET, rect.top() + 2.0), rect.max - vec2(POPUP_ARROW_INSET, 2.0))
    }
}

/// A scrubby options caption's expanded hit band over a field box `rect` and its label `label`:
/// the true x-union of the two, cut at `trailing` points before the field's reading-end edge, and
/// never outside `clip`. Under right-to-left with a reserved trailing arrow the band starts at
/// the arrow's inner edge, so the border strip between the field's edge and the arrow stays the
/// field's.
pub fn option_band(d: &impl HasDirection, rect: Rect, label: Rect, trailing: f32, clip: Rect) -> Rect {
    let (l, r) = (rect.left().min(label.left()), rect.right().max(label.right()));
    if label.left() <= rect.right() {
        Rect::from_min_max(pos2(l, rect.top()), pos2(rect.right() - trailing, rect.bottom()))
    } else {
        let l = if trailing > 0.0 && is_rtl(d) { l.max(rect.left() + POPUP_ARROW_INSET) } else { l };
        Rect::from_min_max(pos2(l, rect.top()), pos2(r, rect.bottom()))
    }
    .intersect(clip)
}

/// A monospace numeric field `width` wide with a dimmed `suffix`, scrubbable by dragging its
/// label (drawn on the same row; see [`super::scrub`]). Dragging the number itself scrubs it too.
/// Returns the number's response and the field's box. The value is clamped to `range` only when
/// the range is ordered and the result finite, so a reversed or NaN range never panics.
#[allow(clippy::too_many_arguments)]
pub fn value_field(
    ui: &mut Ui,
    d: &impl HasDirection,
    value: &mut f32,
    range: RangeInclusive<f32>,
    suffix: &str,
    width: f32,
    trailing: f32,
    style: &FieldStyle,
) -> (Response, Rect) {
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, _slot) = ui.allocate_exact_size(vec2(width, VALUE_FIELD_H), Sense::hover());
    let scrub = take_label_scrub(ui, rect).map(|s| {
        if !s.option_area {
            return s;
        }
        let hit = option_band(d, rect, s.rect, trailing, ui.clip_rect());
        let mut resp = ui.interact(hit, s.label, Sense::CLICK | Sense::DRAG);
        if let Some(tip) = s.tip
            && resp.hover_pos().is_some_and(|p| s.rect.contains(p))
        {
            resp = resp.on_hover_text(tip);
        }
        label_scrub_response(ui, &resp, s.rect, s.max_gap, s.name)
    });
    ui.painter().rect_filled(rect, style.radius, style.fill);
    ui.painter().rect_stroke(rect, style.radius, Stroke::new(1.0, style.border), StrokeKind::Inside);
    let suffix_w =
        if suffix.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(suffix.to_owned(), style.suffix_font.clone(), style.suffix).size().x + 3.0 };
    let row_h = ui.painter().layout_no_wrap("0".into(), style.font.clone(), style.text).size().y;
    let field = field_slot(d, rect, suffix_w, trailing, row_h);
    let unit = if hi - lo <= 10.0 { 0.01 } else { 1.0 };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
    let mut resp = {
        let ui = &mut child;
        ui.style_mut().visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
        ui.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        ui.style_mut().visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        ui.style_mut().visuals.widgets.hovered.bg_stroke = Stroke::NONE;
        ui.style_mut().visuals.widgets.hovered.weak_bg_fill = Color32::TRANSPARENT;
        ui.style_mut().override_font_id = Some(style.font.clone());
        ui.spacing_mut().interact_size.y = field.height();
        ui.spacing_mut().button_padding = Vec2::ZERO;
        let layout = egui::Layout::centered_and_justified(ui.layout().main_dir());
        ui.allocate_ui_with_layout(field.size(), layout, |ui| {
            ui.add(egui::DragValue::new(value).range(range.clone()).clamp_existing_to_range(false).speed(0.01))
        })
        .inner
    };
    if let Some(id) = scrub.as_ref().and_then(|s| s.name) {
        resp = resp.labelled_by(id);
    }
    if let Some(s) = scrub {
        if s.steps != 0.0 {
            // Onto the step grid first, so a whole-number field drops its decimals.
            let v = ((*value / unit).round() + s.steps) * unit;
            let v = (v * 1e4).round() / 1e4;
            let v = if lo <= hi { v.clamp(lo, hi) } else { v };
            if v.is_finite() && v != *value {
                *value = v;
                resp.mark_changed();
            }
        }
        use egui::response::Flags;
        resp.flags.set(Flags::DRAG_STARTED, resp.flags.contains(Flags::DRAG_STARTED) || s.started);
        resp.flags.set(Flags::DRAGGED, resp.flags.contains(Flags::DRAGGED) || s.dragging);
        resp.flags.set(Flags::DRAG_STOPPED, resp.flags.contains(Flags::DRAG_STOPPED) || s.stopped);
    }
    if !suffix.is_empty() {
        let (x, anchor) = suffix_anchor(d, rect, trailing);
        ui.painter().text(pos2(x, rect.center().y), anchor, suffix, style.suffix_font.clone(), style.suffix);
    }
    (resp, rect)
}

/// What a [`popup_value_field`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PopupFieldResponse {
    /// The value changed (typed, scrubbed, stepped, or set on the slider).
    pub changed: bool,
    /// While the value is dragged: a number for that drag, the same on every frame of it, for the
    /// caller to coalesce the drag's edits into one history step.
    pub drag: Option<u64>,
}

/// A [`value_field`] with a pop-up slider: a click on the ▾ inside the field's reading-end edge
/// opens a slider below it (above it when there is no room), a click outside closes it. Pressing
/// the ▾ and dragging moves the value straight away, as far as the slider would, and closes the
/// slider on release. `name` names the ▾ for its tooltip and for accessibility.
#[allow(clippy::too_many_arguments)]
pub fn popup_value_field(
    ui: &mut Ui,
    d: &impl HasDirection,
    name: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    suffix: &str,
    width: f32,
    style: &FieldStyle,
    popup_contents: impl FnOnce(&mut Ui, &mut f32, RangeInclusive<f32>) -> Response,
) -> PopupFieldResponse {
    let (lo, hi) = (*range.start(), *range.end());
    let (mut field, rect) = value_field(ui, d, value, range.clone(), suffix, width, POPUP_ARROW_W, style);
    let arrow = popup_arrow_rect(d, rect);
    let resp = ui.interact(arrow, field.id.with("popup-slider"), Sense::click_and_drag());
    let name = name.trim_end_matches([':', '：']).to_string();
    field = field.accessible_name(name.clone());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &name));
    let resp = resp.on_hover_text(&name);
    let drag_key = resp.id.with("drag");
    let gesture = |ui: &Ui, started: bool| {
        if started {
            let n = ui.ctx().cumulative_pass_nr();
            ui.data_mut(|d| d.insert_temp(drag_key, n));
        }
        ui.data(|d| d.get_temp::<u64>(drag_key))
    };
    let mut out = PopupFieldResponse { changed: field.changed(), drag: None };
    let scrub = gesture(ui, field.drag_started());
    if field.changed() && field.dragged() {
        out.drag = scrub;
    }

    let popup_id = Popup::default_response_id(&resp);
    let start_key = resp.id.with("start");
    if resp.drag_started() {
        Popup::open_id(ui.ctx(), popup_id);
        ui.data_mut(|d| d.insert_temp(start_key, *value));
        gesture(ui, true);
    }
    if resp.dragged()
        && let (Some(start), Some(origin), Some(pos)) =
            (ui.data(|d| d.get_temp::<f32>(start_key)), ui.input(|i| i.pointer.press_origin()), resp.interact_pointer_pos())
    {
        // The pointer moves the value as much as it would move the slider's knob (physical x).
        let track = POPUP_SLIDER_W - 14.0;
        let v = start + (pos.x - origin.x) / track * (hi - lo);
        let v = if lo <= hi && v.is_finite() { v.clamp(lo, hi) } else { start };
        if v != *value {
            *value = v;
            out.changed = true;
        }
        out.drag = gesture(ui, false);
    }
    if resp.drag_stopped() {
        Popup::close_id(ui.ctx(), popup_id);
    }

    // The popup opens under the ▾, which is at the field's reading-end edge.
    let alternatives = [direction::above(d)];
    let popup = Popup::from_toggle_button_response(&resp)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .anchor(rect)
        .align(direction::below(d))
        .align_alternatives(&alternatives);
    let open = popup.is_open();
    if resp.hovered() || resp.dragged() || open {
        ui.painter().rect_filled(arrow, style.radius, style.border.gamma_multiply(style.arrow_hover));
    }
    chevron(ui, arrow, ui.style().interact(&resp));
    popup.show(|ui| {
        ui.set_width(POPUP_SLIDER_W);
        let s = popup_contents(ui, value, range);
        s.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), &name));
        let key = gesture(ui, s.drag_started());
        if s.changed() {
            out.changed = true;
            // A click on the track is one edit of its own; a drag is one edit however long.
            if s.dragged() {
                out.drag = key;
            }
        }
    });
    out
}

/// Small chevron for dropdown arrows: two line segments, pointing down.
pub fn chevron(ui: &Ui, rect: Rect, visuals: &egui::style::WidgetVisuals) {
    let c = rect.center();
    let s = 3.2;
    let stroke = Stroke::new(1.3, visuals.fg_stroke.color.gamma_multiply(0.8));
    ui.painter().line_segment([c + vec2(-s, -s * 0.5), c + vec2(0.0, s * 0.5)], stroke);
    ui.painter().line_segment([c + vec2(0.0, s * 0.5), c + vec2(s, -s * 0.5)], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use egui::{Pos2, RawInput, vec2};

    fn rect() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), vec2(120.0, VALUE_FIELD_H))
    }

    #[test]
    fn field_slot_reserves_the_suffix_at_the_reading_end() {
        let r = rect();
        let ltr = field_slot(&Direction::Ltr, r, 10.0, 14.0, 12.0);
        assert!((ltr.left() - 104.0).abs() < 1e-4 && (ltr.right() - (r.right() - 4.0 - 10.0 - 14.0)).abs() < 1e-4, "{ltr:?}");
        let rtl = field_slot(&Direction::Rtl, r, 10.0, 14.0, 12.0);
        assert!((rtl.left() - (r.left() + 4.0 + 10.0 + 14.0)).abs() < 1e-4 && (rtl.right() - (r.right() - 4.0)).abs() < 1e-4, "rtl: {rtl:?}");
    }

    #[test]
    fn suffix_anchor_and_arrow_sit_at_the_reading_end() {
        let r = rect();
        let (x, a) = suffix_anchor(&Direction::Ltr, r, 14.0);
        assert!((x - (r.right() - 6.0 - 14.0)).abs() < 1e-4 && a == Align2::RIGHT_CENTER);
        let (x, a) = suffix_anchor(&Direction::Rtl, r, 14.0);
        assert!((x - (r.left() + 6.0 + 14.0)).abs() < 1e-4 && a == Align2::LEFT_CENTER);
        let al = popup_arrow_rect(&Direction::Ltr, r);
        assert!((al.right() - (r.right() - POPUP_ARROW_INSET)).abs() < 1e-4 && (al.width() - POPUP_ARROW_W).abs() < 1e-4, "{al:?}");
        let ar = popup_arrow_rect(&Direction::Rtl, r);
        assert!((ar.left() - (r.left() + POPUP_ARROW_INSET)).abs() < 1e-4 && (ar.width() - POPUP_ARROW_W).abs() < 1e-4, "rtl: {ar:?}");
    }

    #[test]
    fn option_band_spans_the_label_and_field_either_way() {
        let r = rect();
        let clip = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        // Label left of the field (the LTR caption-before-field row): the band spans from the
        // label's left edge to the field's reading-end edge (trailing cut off).
        let label = Rect::from_min_size(pos2(r.left() - 54.0, r.top()), vec2(50.0, r.height()));
        let b = option_band(&Direction::Ltr, r, label, 0.0, clip);
        assert!((b.left() - label.left()).abs() < 1e-4 && (b.right() - r.right()).abs() < 1e-4, "ltr: {b:?}");
        let b = option_band(&Direction::Ltr, r, label, POPUP_ARROW_W, clip);
        assert!((b.right() - (r.right() - POPUP_ARROW_W)).abs() < 1e-4, "ltr trailing: {b:?}");
        // Label right of the field (the RTL mirror): the band spans the true x-union.
        let label = Rect::from_min_size(pos2(r.right() + 4.0, r.top()), vec2(50.0, r.height()));
        let b = option_band(&Direction::Rtl, r, label, 0.0, clip);
        assert!((b.left() - r.left()).abs() < 1e-4 && (b.right() - label.right()).abs() < 1e-4, "rtl: {b:?}");
        // With a reserved arrow under RTL the band starts at the arrow's inner edge.
        let b = option_band(&Direction::Rtl, r, label, POPUP_ARROW_W, clip);
        assert!((b.left() - (r.left() + POPUP_ARROW_INSET)).abs() < 1e-4 && (b.right() - label.right()).abs() < 1e-4, "rtl arrow band: {b:?}");
    }

    /// Render a `value_field` in `dir`; return the field's box and the suffix text's painted rect.
    fn suffix_rect(dir: Direction) -> (Rect, Rect) {
        let ctx = egui::Context::default();
        direction::apply(&ctx, dir);
        let mut value = 50.0f32;
        let mut box_rect = Rect::NOTHING;
        let mut out = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), ..Default::default() }, |ui| {
            let (_, r) = value_field(ui, &dir, &mut value, 0.0..=100.0, "%", 120.0, 0.0, &FieldStyle::default());
            box_rect = r;
        });
        let mut found = Rect::NOTHING;
        for shape in &out.shapes {
            if let egui::Shape::Text(ts) = &shape.shape
                && ts.galley.text() == "%"
            {
                found = ts.visual_bounding_rect();
            }
        }
        out.textures_delta.clear();
        (box_rect, found)
    }

    #[test]
    fn the_suffix_paints_at_the_reading_end() {
        let (box_l, l) = suffix_rect(Direction::Ltr);
        assert!(
            l.is_positive() && (box_l.right() - 6.0 - l.right()).abs() < 2.0,
            "ltr: the suffix ends 6 pt from the right edge: {l:?} in {box_l:?}"
        );
        let (box_r, r) = suffix_rect(Direction::Rtl);
        assert!(
            r.is_positive() && (r.left() - (box_r.left() + 6.0)).abs() < 2.0,
            "rtl: the suffix starts 6 pt from the left edge: {r:?} in {box_r:?}"
        );
    }
}
