//! Canvas ruler layout, tick placement, and coordinate bounds calculations.
//!
//! Provides bounded ruler tick layout and bounds calculations with caller-provided units and origins.

use egui::emath::GuiRounding as _;
use egui::{Pos2, Rect, Vec2, pos2, vec2};

#[cfg(test)]
mod tests;

/// Standard ruler strip thickness in points.
pub const RULER_THICKNESS: f32 = 16.0;

/// Standard ruler label vertical spacing on vertical rulers.
pub const RULER_CHAR_SPACING: f32 = 8.5;

/// Standard line height for ruler labels.
pub const RULER_LINE_HEIGHT: f32 = 9.5;

/// Layout rectangles for the top ruler, left ruler, and the corner where they meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerRects {
    pub top: Rect,
    pub left: Rect,
    pub corner: Rect,
}

/// Compute ruler rectangles given a full canvas area and thickness.
pub fn ruler_rects(full: Rect, thickness: f32) -> RulerRects {
    let top = Rect::from_min_max(pos2(full.left() + thickness, full.top()), pos2(full.right(), full.top() + thickness));
    let left = Rect::from_min_max(pos2(full.left(), full.top() + thickness), pos2(full.left() + thickness, full.bottom()));
    let corner = Rect::from_min_size(full.min, vec2(thickness, thickness));
    RulerRects { top, left, corner }
}

/// Compute canvas full view and inner content rectangles accounting for optional scrollbars and rulers.
pub fn view_rects(outer: Rect, scrollbar_size: f32, ruler_size: f32, show_scrollbars: bool, show_rulers: bool, ppp: f32) -> (Rect, Rect) {
    let full = if show_scrollbars { Rect::from_min_max(outer.min, outer.max - vec2(scrollbar_size, scrollbar_size)) } else { outer };
    let content = if show_rulers { Rect::from_min_max(full.min + vec2(ruler_size, ruler_size), full.max) } else { full };
    (full, content.round_to_pixels(ppp))
}

/// Scale definition for ruler measurements.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerScale {
    /// Points per unit (e.g. 1.0 for points, 72.0 for inches, 72.0/25.4 for mm).
    pub unit_points: f64,
    /// Ruler origin in document points.
    pub origin: Pos2,
    /// Coordinate stretch factors `[x, y]` for non-square aspect ratios (e.g. video pixels).
    pub stretch: [f64; 2],
    /// Whether this unit divides steps in halves (binary fractions, e.g. inches) or decimals.
    pub halves: bool,
}

impl RulerScale {
    /// Standard document ruler scale with uniform 1:1 aspect ratio.
    pub fn document(unit_points: f64, origin: Pos2, halves: bool) -> Self {
        Self { unit_points, origin, stretch: [1.0, 1.0], halves }
    }

    /// Ruler scale with custom stretch factors along each axis.
    pub fn with_stretch(unit_points: f64, origin: Pos2, stretch: [f64; 2], halves: bool) -> Self {
        Self { unit_points, origin, stretch, halves }
    }

    /// Distance of point `p` from `origin` along unit direction `axis`, in points of this scale.
    pub fn along(&self, p: Pos2, axis: Vec2) -> f64 {
        let dx = f64::from(p.x - self.origin.x);
        let dy = f64::from(p.y - self.origin.y);
        let sx = if self.stretch[0] != 0.0 { self.stretch[0] } else { 1.0 };
        let sy = if self.stretch[1] != 0.0 { self.stretch[1] } else { 1.0 };
        dx / sx * f64::from(axis.x) + dy / sy * f64::from(axis.y)
    }

    /// Screen points spanned by one point of this scale along `axis` at `zoom`.
    pub fn zoom_along(&self, zoom: f64, axis: Vec2) -> f64 {
        let sx = if self.stretch[0] != 0.0 { self.stretch[0] } else { 1.0 };
        let sy = if self.stretch[1] != 0.0 { self.stretch[1] } else { 1.0 };
        let denom = f64::from(axis.x * axis.x) / sx + f64::from(axis.y * axis.y) / sy;
        if denom <= 0.0 || !denom.is_finite() { zoom } else { zoom / denom }
    }

    /// Rulers' label step at `zoom` (screen pixels per point).
    pub fn ruler_step(&self, zoom: f64) -> f64 {
        const DECIMAL: [f64; 19] =
            [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0];
        const BINARY: [f64; 15] = [1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 10000.0];
        let steps: &[f64] = if self.halves { &BINARY } else { &DECIMAL };
        let points = if self.unit_points > 0.0 { self.unit_points } else { 1.0 };
        steps.iter().copied().find(|s| s * points * zoom >= 50.0).unwrap_or(10000.0)
    }

    /// Unlabelled minor ticks between two labels `step` apart.
    pub fn ruler_minor(&self, step: f64) -> f64 {
        if self.halves { step / 8.0 } else { step / 10.0 }
    }

    /// `v` (points) on the nearest ruler tick at `zoom`.
    pub fn snap_to_tick(&self, v: f64, zoom: f64) -> f64 {
        let step = self.ruler_step(zoom);
        let tick = self.ruler_minor(step) * self.unit_points;
        if tick <= 0.0 || !tick.is_finite() {
            return v;
        }
        (v / tick).round() * tick
    }
}

/// A formatted ruler label for value `v` given major step `step`.
pub fn ruler_label(v: f64, step: f64) -> String {
    let exact = |d: u32| (step * 10f64.powi(d as i32)).fract().abs() < 1e-9;
    let decimals = (0..=7).find(|d| exact(*d)).unwrap_or(7) as usize;
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') { "0".into() } else { s.into() }
}

/// Height in points of a vertical label spelled down a ruler with `chars` characters.
pub fn vertical_label_height(chars: usize) -> f32 {
    chars.saturating_sub(1) as f32 * RULER_CHAR_SPACING + RULER_LINE_HEIGHT
}

/// One placed tick on a ruler.
#[derive(Clone, Debug, PartialEq)]
pub struct RulerTick {
    /// Screen points distance from the ruler's start position.
    pub distance: f32,
    /// Length of the tick mark in screen points.
    pub length: f32,
    /// Optional label string for major ticks.
    pub label: Option<String>,
    /// Whether this is a major (labeled) tick.
    pub is_major: bool,
    /// Value in scale units.
    pub value: f64,
}

/// Compute bounded ruler tick placement across a given segment.
pub fn layout_ticks<F>(from_doc: Pos2, to_doc: Pos2, axis: Vec2, zoom: f64, scale: &RulerScale, ruler_thickness: f32, mut draw: F)
where
    F: FnMut(RulerTick),
{
    if !zoom.is_finite() || zoom <= 0.0 || !scale.unit_points.is_finite() || scale.unit_points <= 0.0 {
        return;
    }
    let zoom_axis = scale.zoom_along(zoom, axis);
    let per = scale.unit_points;
    let step = scale.ruler_step(zoom_axis);
    let minor = scale.ruler_minor(step);
    if minor <= 0.0 || !minor.is_finite() {
        return;
    }

    let a = scale.along(from_doc, axis);
    let b = scale.along(to_doc, axis);
    let (min_val, max_val) = if a <= b { (a, b) } else { (b, a) };

    let near = |v: f64, every: f64| ((v / every).round() * every - v).abs() < minor * 0.01;

    let mut v = (min_val / per / minor).floor() * minor;
    let half_step = step / 2.0;

    let mut count = 0;
    const MAX_TICKS: usize = 10_000;

    while v * per <= max_val + minor * 0.01 && count < MAX_TICKS {
        count += 1;
        let is_major = near(v, step);
        let is_half = !is_major && near(v, half_step);
        let (len, label) = if is_major {
            (ruler_thickness, Some(ruler_label(v, step)))
        } else if is_half {
            (ruler_thickness * 0.45, None)
        } else {
            (ruler_thickness * 0.25, None)
        };

        let distance = ((v * per - a) * zoom_axis) as f32;
        draw(RulerTick { distance, length: len, label, is_major, value: v });
        v += minor;
    }
}

/// Clip line segment `(a, b)` to bounding rectangle `bounds`.
pub fn clip_segment(a: Pos2, b: Pos2, bounds: Rect) -> Option<(Pos2, Pos2)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (run, room) in [(-d.x, a.x - bounds.left()), (d.x, bounds.right() - a.x), (-d.y, a.y - bounds.top()), (d.y, bounds.bottom() - a.y)] {
        if run == 0.0 {
            if room < 0.0 {
                return None;
            }
        } else if run < 0.0 {
            t0 = t0.max(room / run);
        } else {
            t1 = t1.min(room / run);
        }
    }
    (t0 <= t1).then(|| (a + d * t0, a + d * t1))
}
