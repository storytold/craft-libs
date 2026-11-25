//! A hand-painted thin-track slider with a round knob. A plain slider mirrors its knob, fill
//! and drag map from the track's reading-start edge in a right-to-left interface; a gradient
//! strip is a colour legend, so its spectrum, value, drag and knob stay physical in every
//! direction and the knob keeps pointing at its own colour. The wheel stays physical
//! (up = higher).

use std::ops::RangeInclusive;

use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::direction::{HasDirection, is_rtl};
use crate::widgets::scrub::scrub_label_within;

/// Colors and sizes for [`slider`]'s painting; the application owns the theme.
#[derive(Clone, Copy, Debug)]
pub struct SliderStyle {
    /// The empty track.
    pub track: Color32,
    /// The filled part of the track.
    pub fill: Color32,
    pub knob: Color32,
    pub knob_shadow: Color32,
    /// An outline around the knob, if the theme wants one.
    pub knob_outline: Option<Stroke>,
    /// The ring painted around the knob while hovered or dragged.
    pub hover_ring: Stroke,
    /// The knob's radius.
    pub knob_radius: f32,
}

impl Default for SliderStyle {
    fn default() -> Self {
        Self {
            track: Color32::from_gray(60),
            fill: Color32::from_gray(160),
            knob: Color32::WHITE,
            knob_shadow: Color32::from_black_alpha(90),
            knob_outline: None,
            hover_ring: Stroke::new(2.0, Color32::from_gray(120)),
            knob_radius: 7.0,
        }
    }
}

/// Gestures on a slider row beyond click and drag. The wheel takes ten steps with Shift.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowGestures {
    /// A double-click on the slider (knob or track) sets this value.
    pub reset: Option<f32>,
    /// Each wheel notch over the slider moves the value by this step (up = higher; ×10 with
    /// Shift).
    pub wheel_step: Option<f32>,
    /// A curved track: the value at fraction f of the track is `lo + (hi - lo) * f^k`, so small
    /// values get fine steps. Typed, scrubbed and stepped values stay linear; only the track
    /// maps through the curve. The fine end is the track's reading-start edge, which is its
    /// right edge in a right-to-left interface.
    pub curve: Option<f32>,
}

/// The value at fraction `f` of a track over `lo..=hi`, through `curve` (see [`RowGestures`]).
pub fn curve_value(f: f32, lo: f32, hi: f32, curve: Option<f32>) -> f32 {
    match curve {
        Some(k) if k.is_finite() && k > 0.0 => lo + f.clamp(0.0, 1.0).powf(k) * (hi - lo),
        _ => lo + f * (hi - lo),
    }
}

/// Where `v` sits on a track over `lo..=hi`, as a fraction: the inverse of [`curve_value`].
pub fn curve_fraction(v: f32, lo: f32, hi: f32, curve: Option<f32>) -> f32 {
    let f = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    match curve {
        // An unset (NaN) value draws the knob at the start rather than nowhere.
        Some(k) if k.is_finite() && k > 0.0 => {
            if f.is_finite() {
                f.powf(1.0 / k)
            } else {
                0.0
            }
        }
        _ => f,
    }
}

/// The x of fraction `f` along `track`: from its left edge, or from its right edge when `rtl`,
/// where the knob and the fill lead from the reading start.
pub fn track_x(track: Rect, f: f32, rtl: bool) -> f32 {
    if rtl { track.right() - f * track.width() } else { track.left() + f * track.width() }
}

/// The fraction of `track` at `x`: the inverse of [`track_x`], mapping a dragged pointer.
pub fn track_fraction(track: Rect, x: f32, rtl: bool) -> f32 {
    if rtl { (track.right() - x) / track.width() } else { (x - track.left()) / track.width() }
}

/// Whole notches in `wheel` (unit, delta, Shift held), a notch with Shift counting
/// `shift_notches`; fractions carry over in egui memory under `acc_id`. Direction-blind: wheel
/// up is always positive.
pub fn count_notches(ui: &Ui, acc_id: egui::Id, wheel: &[(egui::MouseWheelUnit, Vec2, bool)], shift_notches: f32) -> f32 {
    let mut acc: f32 = ui.data(|d| d.get_temp(acc_id)).unwrap_or(0.0);
    let per_line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let per_line = if per_line.is_finite() && per_line > 0.0 { per_line } else { 40.0 };
    let mut notches = 0.0;
    for &(unit, delta, shift) in wheel {
        // Some systems turn Shift+wheel into a horizontal scroll.
        let d = if delta.y != 0.0 { delta.y } else { delta.x };
        acc += match unit {
            egui::MouseWheelUnit::Point => d / per_line,
            _ => d,
        };
        let whole = acc.trunc();
        acc -= whole;
        notches += if shift { whole * shift_notches } else { whole };
    }
    if !acc.is_finite() {
        acc = 0.0;
    }
    ui.data_mut(|d| d.insert_temp(acc_id, acc));
    if notches.is_finite() { notches } else { 0.0 }
}

/// [`count_notches`] of widget `id` while `hovered`, a notch with Shift counting
/// `shift_notches`.
pub fn wheel_notches_of(ui: &Ui, id: egui::Id, hovered: bool, shift_notches: f32) -> f32 {
    if !hovered {
        return 0.0;
    }
    let wheel: Vec<(egui::MouseWheelUnit, Vec2, bool)> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, modifiers, .. } => Some((*unit, *delta, modifiers.shift)),
                _ => None,
            })
            .collect()
    });
    count_notches(ui, id.with("wheel-acc"), &wheel, shift_notches)
}

/// Windows' default double-click time (Photoshop's; egui's own default is 0.3 s).
pub const DOUBLE_CLICK_S: f64 = 0.5;

/// Finger jitter between taps easily exceeds egui's 6 px `max_click_dist`; 64 px is about
/// two-thirds of an inch at 96 DPI (roughly a fingertip pad).
const TOUCH_MAX_CLICK_DIST: f32 = 64.0;

/// True when this frame's pointer work came from a touch contact (still down, or a Touch event
/// including End on the click frame).
fn pointer_from_touch(ui: &Ui) -> bool {
    ui.input(|i| i.any_touches() || i.events.iter().any(|e| matches!(e, egui::Event::Touch { .. })))
}

/// Whether this click on `resp` completes a double-click / double-tap, counted the Windows way
/// (Photoshop): a click soon after a lone click near the same spot; the click after a double
/// starts over. Call on every click for that widget so the first tap is remembered.
pub fn second_click(ui: &Ui, resp: &Response) -> bool {
    if !resp.clicked() {
        return false;
    }
    let key = resp.id.with("last-click");
    // Prefer the response's hit pos: raw `pointer.interact_pos` can be unset on a release-only frame.
    let pos = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.interact_pos()));
    let time = ui.input(|i| i.time);
    let touch = pointer_from_touch(ui);
    let mouse_dist = ui.ctx().options(|o| o.input_options.max_click_dist);
    let last: Option<(f64, Pos2, bool)> = ui.data(|d| d.get_temp(key));
    // Looser distance if either tap in the pair was touch (End clears `any_touches` but leaves Touch in `events`).
    let dist = if touch || last.is_some_and(|(_, _, t)| t) { TOUCH_MAX_CLICK_DIST.max(mouse_dist) } else { mouse_dist };
    let double = match (last, pos) {
        (Some((t, p, _)), Some(q)) => time - t < DOUBLE_CLICK_S && p.distance(q) < dist,
        _ => false,
    };
    ui.data_mut(|d| {
        if let (false, Some(q)) = (double, pos) {
            d.insert_temp(key, (time, q, touch));
        } else {
            d.remove::<(f64, Pos2, bool)>(key);
        }
    });
    double
}

/// The geometry [`slider`] paints, exposed for callers that paint their own chrome.
#[derive(Clone, Copy, Debug)]
pub struct SliderGeom {
    /// The whole widget rect.
    pub rect: Rect,
    /// The track.
    pub track: Rect,
    /// The knob's centre x.
    pub knob_x: f32,
    /// The value fill (none over a gradient strip).
    pub fill: Option<Rect>,
    /// The value's fraction of the track, after `curve`.
    pub fraction: f32,
}

/// Handles the pointer for a slider at `value` and returns its response and geometry; the
/// caller paints from [`SliderGeom`] (see [`paint_slider`]).
pub fn slider_interact(
    ui: &mut Ui,
    d: &impl HasDirection,
    value: &mut f32,
    range: RangeInclusive<f32>,
    gradient: Option<&[Color32]>,
    g: RowGestures,
) -> (Response, SliderGeom) {
    let width = ui.available_width().max(60.0);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 14.0, if gradient.is_some() { 5.0 } else { 3.0 }));
    // A gradient strip is a colour legend: its colours keep their physical order, so the
    // value↔x mapping stays physical with them and the knob keeps pointing at its own colour.
    let strip = matches!(gradient, Some(colors) if colors.len() >= 2);
    let rtl = is_rtl(d) && !strip;
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        let f = track_fraction(track, p.x, rtl).clamp(0.0, 1.0);
        let nv = curve_value(f, lo, hi, g.curve);
        if (nv - *value).abs() > f32::EPSILON {
            *value = nv;
            resp.mark_changed();
        }
    }
    // The double-click's first click has moved the knob to the pointer; the second resets.
    if let Some(reset) = g.reset
        && second_click(ui, &resp)
    {
        *value = reset;
        resp.mark_changed();
    }
    if let Some(step) = g.wheel_step {
        let n = wheel_notches_of(ui, resp.id, resp.hovered(), 10.0);
        if n != 0.0 {
            let nv = (*value + n * step).clamp(lo, hi);
            if nv != *value {
                *value = nv;
                resp.mark_changed();
            }
        }
    }
    let f = curve_fraction(*value, lo, hi, g.curve);
    let knob_x = track_x(track, f, rtl);
    // The fill grows from the reading-start edge: the left in LTR, the right in RTL.
    let fill = if strip {
        None
    } else if rtl {
        Some(Rect::from_min_max(pos2(knob_x, track.min.y), track.max))
    } else {
        Some(Rect::from_min_max(track.min, pos2(knob_x, track.max.y)))
    };
    (resp, SliderGeom { rect, track, knob_x, fill, fraction: f })
}

/// Paints a [`SliderGeom`] with `style`.
pub fn paint_slider(ui: &Ui, geom: &SliderGeom, gradient: Option<&[Color32]>, style: &SliderStyle, resp: &Response) {
    let painter = ui.painter();
    match gradient {
        Some(colors) if colors.len() >= 2 => {
            let track = geom.track;
            let n = colors.len() - 1;
            for (i, w) in colors.windows(2).enumerate() {
                let x0 = track.left() + track.width() * i as f32 / n as f32;
                let x1 = track.left() + track.width() * (i + 1) as f32 / n as f32;
                let mut mesh = egui::Mesh::default();
                let r = Rect::from_min_max(pos2(x0, track.top()), pos2(x1, track.bottom()));
                mesh.colored_vertex(r.left_top(), w[0]);
                mesh.colored_vertex(r.right_top(), w[1]);
                mesh.colored_vertex(r.right_bottom(), w[1]);
                mesh.colored_vertex(r.left_bottom(), w[0]);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(mesh);
            }
        }
        _ => {
            painter.rect_filled(geom.track, 2.0, style.track);
            if let Some(filled) = geom.fill {
                painter.rect_filled(filled, 2.0, style.fill);
            }
        }
    }
    let knob = pos2(geom.knob_x, geom.rect.center().y);
    painter.circle_filled(knob + vec2(0.0, 1.0), style.knob_radius + 0.5, style.knob_shadow);
    painter.circle_filled(knob, style.knob_radius, style.knob);
    if let Some(outline) = style.knob_outline {
        painter.circle_stroke(knob, style.knob_radius, outline);
    }
    if resp.hovered() || resp.dragged() {
        painter.circle_stroke(knob, style.knob_radius + 2.5, style.hover_ring);
    }
}

/// A thin-track slider with a round knob, mirrored in a right-to-left interface (see the
/// module docs).
pub fn slider(
    ui: &mut Ui,
    d: &impl HasDirection,
    value: &mut f32,
    range: RangeInclusive<f32>,
    gradient: Option<&[Color32]>,
    g: RowGestures,
    style: &SliderStyle,
) -> Response {
    let (resp, geom) = slider_interact(ui, d, value, range, gradient, g);
    paint_slider(ui, &geom, gradient, style, &resp);
    resp
}

/// A labelled slider row: `Label ........ [field]` above a full-width slider, in reading order.
/// The caption scrubs the field (across the whole row); `field` draws the value field, pinned
/// to the row's reading end — the row's own direction decides the side, so a row that did not
/// mirror keeps its field at the far end. The returned response is the slider's, with the
/// field's change and drag flags merged in.
#[allow(clippy::too_many_arguments)]
pub fn slider_row(
    ui: &mut Ui,
    d: &impl HasDirection,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    gradient: Option<&[Color32]>,
    g: RowGestures,
    style: &SliderStyle,
    field: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    let mut label_id = None;
    let mut field_resp = None;
    ui.horizontal(|ui| {
        let l = ui.label(label);
        label_id = Some(l.id);
        scrub_label_within(ui, &l, f32::INFINITY);
        let pin = if ui.layout().main_dir() == egui::Direction::RightToLeft {
            egui::Layout::left_to_right(egui::Align::Center)
        } else {
            egui::Layout::right_to_left(egui::Align::Center)
        };
        ui.with_layout(pin, |ui| field_resp = Some(field(ui)));
    });
    let s = slider(ui, d, value, range, gradient, g, style);
    s.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), label));
    let s = if let Some(id) = label_id { s.labelled_by(id) } else { s };
    let mut r = s.clone();
    if let Some(v) = field_resp {
        if v.changed() {
            r.mark_changed();
        }
        // A scrub of the field or its label is one drag, like the slider's, for history.
        use egui::response::Flags;
        r.flags |= v.flags & (Flags::DRAG_STARTED | Flags::DRAGGED | Flags::DRAG_STOPPED);
    }
    ui.add_space(4.0);
    r
}

#[cfg(test)]
mod tests {
    use egui::{Context, Pos2, RawInput, Rect, pos2, vec2};

    use super::*;
    use crate::direction::{Direction, apply};

    fn ctx(dir: Direction) -> Context {
        let ctx = Context::default();
        apply(&ctx, dir);
        ctx
    }

    fn frame(c: &Context, add: impl FnMut(&mut Ui)) {
        let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 60.0))), ..Default::default() };
        c.run_ui(raw, add).textures_delta.clear();
    }

    /// The x↔fraction mapping mirrors from the track's right edge in RTL and is unchanged in LTR.
    #[test]
    fn track_mapping_mirrors_in_rtl() {
        let track = Rect::from_min_size(pos2(20.0, 10.0), vec2(200.0, 3.0));
        assert!((track_x(track, 0.25, false) - 70.0).abs() < 1e-4);
        assert!((track_x(track, 0.25, true) - 170.0).abs() < 1e-4);
        assert!((track_fraction(track, 70.0, false) - 0.25).abs() < 1e-4);
        assert!((track_fraction(track, 70.0, true) - 0.75).abs() < 1e-4);
    }

    /// RTL: the fill grows from the track's right edge and the knob rides right of centre for a
    /// small value; LTR is unchanged.
    #[test]
    fn slider_fill_and_knob_mirror_in_rtl() {
        for (dir, rtl) in [(Direction::Ltr, false), (Direction::Rtl, true)] {
            let c = ctx(dir);
            let mut geom = None;
            frame(&c, |ui| {
                let mut v = 25.0;
                let (resp, g) = slider_interact(ui, &dir, &mut v, 0.0..=100.0, None, RowGestures::default());
                paint_slider(ui, &g, None, &SliderStyle::default(), &resp);
                geom = Some(g);
            });
            let g = geom.expect("the slider ran");
            let mid = g.track.center().x;
            match g.fill {
                Some(fill) if rtl => {
                    assert!((fill.right() - g.track.right()).abs() < 1e-4, "rtl: fill ends at the right edge: {fill:?} of {:?}", g.track);
                    assert!((fill.left() - (g.track.right() - 0.25 * g.track.width())).abs() < 1e-4, "25% spans 75%..100%: {fill:?}");
                    assert!(g.knob_x > mid, "rtl: knob right of centre at {} (mid {mid})", g.knob_x);
                }
                Some(fill) => {
                    assert!((fill.left() - g.track.left()).abs() < 1e-4, "ltr: fill starts at the left edge: {fill:?} of {:?}", g.track);
                    assert!((fill.right() - (g.track.left() + 0.25 * g.track.width())).abs() < 1e-4, "25% spans 0%..25%: {fill:?}");
                    assert!(g.knob_x < mid, "ltr: knob left of centre at {} (mid {mid})", g.knob_x);
                }
                None => panic!("a plain slider paints a fill"),
            }
        }
    }

    /// A gradient strip keeps its physical colour order in RTL: the spectrum's vertex x's and
    /// colours are identical in both directions, the knob stays over its own colour, and no
    /// value fill is painted.
    #[test]
    fn slider_gradient_strip_stays_physical_in_rtl() {
        let stops: Vec<Color32> = (0..=12).map(|i| Color32::from_rgb(i as u8 * 20, 255 - i as u8 * 20, 128)).collect();
        let probe = |dir: Direction| -> (Vec<f32>, Vec<Color32>, f32, Option<Rect>) {
            let c = ctx(dir);
            let mut out = None;
            let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 60.0))), ..Default::default() };
            let mut res = c.run_ui(raw, |ui| {
                let mut v = 100.0f32;
                let (resp, g) = slider_interact(ui, &dir, &mut v, 0.0..=360.0, Some(&stops), RowGestures::default());
                paint_slider(ui, &g, Some(&stops), &SliderStyle::default(), &resp);
                out = Some(g);
            });
            res.textures_delta.clear();
            let mut verts = Vec::new();
            let mut colors = Vec::new();
            for clipped in &res.shapes {
                if let egui::Shape::Mesh(m) = &clipped.shape {
                    verts.extend(m.vertices.iter().map(|v| v.pos.x));
                    colors.extend(m.vertices.iter().map(|v| v.color));
                }
            }
            let g = out.expect("ran");
            (verts, colors, g.knob_x, g.fill)
        };
        let (ltr_verts, ltr_colors, knob_ltr, ltr_fill) = probe(Direction::Ltr);
        let (rtl_verts, rtl_colors, knob_rtl, rtl_fill) = probe(Direction::Rtl);
        assert!(ltr_fill.is_none() && rtl_fill.is_none(), "a gradient track paints no value fill");
        assert_eq!(ltr_verts, rtl_verts, "the spectrum's vertex x's are identical in both directions");
        assert_eq!(ltr_colors, rtl_colors, "the spectrum's colours are identical in both directions");
        let lo = ltr_verts.iter().copied().fold(f32::INFINITY, f32::min);
        assert_eq!(ltr_verts.first(), Some(&lo), "the strip is painted left to right");
        assert!((knob_ltr - knob_rtl).abs() < 1e-4, "a gradient slider's knob stays over its colours (LTR {knob_ltr}, RTL {knob_rtl})");
    }

    /// RTL: a drag maps the pointer mirrored — pressing at 75% of the widget width picks ~25%
    /// of the range; the same press in LTR picks ~75%.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn slider_drag_follows_the_pointer_mirrored_in_rtl() {
        use egui_kittest::Harness;

        let press_at = |dir: Direction| -> f32 {
            let mut h = Harness::builder().with_size(vec2(300.0, 60.0)).build_ui_state(
                |ui, s: &mut (f32, Rect)| {
                    apply(ui.ctx(), dir);
                    s.1 = slider(ui, &dir, &mut s.0, 0.0..=100.0, None, RowGestures::default(), &SliderStyle::default()).rect;
                },
                (50.0, Rect::NOTHING),
            );
            h.run();
            let r = h.state().1;
            let at = pos2(r.left() + 0.75 * r.width(), r.center().y);
            h.hover_at(at);
            h.run();
            h.drag_at(at);
            h.run();
            // A real movement, so the drag is under way and the slider reads the pointer.
            let moved = at + vec2(4.0, 0.0);
            h.event(egui::Event::PointerMoved(moved));
            h.run();
            h.drop_at(moved);
            h.run();
            h.state().0
        };
        let ltr = press_at(Direction::Ltr);
        let rtl = press_at(Direction::Rtl);
        assert!(ltr > 70.0 && ltr < 85.0, "LTR: a press at 75% of the width picks ~75% ({ltr})");
        assert!(rtl > 15.0 && rtl < 30.0, "RTL: the same press picks ~25% ({rtl})");
    }

    /// The wheel stays physical in RTL: up steps the value up, Shift ×10.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn slider_wheel_still_steps_upward_in_rtl() {
        use egui_kittest::Harness;

        let mut h = Harness::builder().with_size(vec2(300.0, 80.0)).build_ui_state(
            |ui, s: &mut (f32, Rect)| {
                apply(ui.ctx(), Direction::Rtl);
                s.1 = slider(
                    ui,
                    &Direction::Rtl,
                    &mut s.0,
                    0.0..=100.0,
                    None,
                    RowGestures { wheel_step: Some(1.0), ..Default::default() },
                    &SliderStyle::default(),
                )
                .rect;
            },
            (50.0, Rect::NOTHING),
        );
        h.run();
        let wheel = |h: &mut Harness<'_, (f32, Rect)>, delta: Vec2, mods: egui::Modifiers| {
            let at = h.state().1.center();
            h.event(egui::Event::ModifiersChanged(mods));
            h.hover_at(at);
            h.run();
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers: mods,
            });
            h.run();
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.run();
        };
        wheel(&mut h, vec2(0.0, 1.0), egui::Modifiers::NONE);
        assert_eq!(h.state().0, 51.0, "wheel up steps up in RTL");
        wheel(&mut h, vec2(0.0, 1.0), egui::Modifiers::SHIFT);
        assert_eq!(h.state().0, 61.0, "Shift+wheel is ten steps");
    }

    /// The curve keeps fine steps at the reading-start edge under RTL: `f^k` maps the fraction,
    /// and the mirrored x puts the fine end at the right.
    #[test]
    fn curve_keeps_fine_steps_at_the_reading_start() {
        let track = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 3.0));
        let k = Some(2.0);
        // value 1 of 0..=100 is at fraction sqrt(0.01) = 0.1
        let f = curve_fraction(1.0, 0.0, 100.0, k);
        assert!((f - 0.1).abs() < 1e-4);
        assert!((track_x(track, f, true) - 90.0).abs() < 1e-4, "rtl: the fine end is the right edge");
        assert!((track_x(track, f, false) - 10.0).abs() < 1e-4, "ltr: the fine end is the left edge");
    }
}
