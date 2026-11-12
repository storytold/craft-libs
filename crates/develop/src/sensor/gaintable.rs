//! Rendering a DNG `ProfileGainTableMap` / `ProfileGainTableMap2` (DNG 1.7.1, Apple ProRAW's local tone
//! mapping), parsed by [`craft_raw::gaintable`].
//!
//! It is not rendered by default: Lightroom Classic renders Apple ProRAW without the map (see
//! [`super::profile`]), and so does this pipeline. The photo's "camera local tone mapping" option renders it
//! ([`GainTableMapExt::gain`], applied by [`crate::local_tone`]), as the camera itself does. What the fields
//! mean is in [`craft_raw::gaintable`]'s docs: the gain is looked up per pixel in a grid of tables over the
//! active area, at the table input `weights · (R, G, B, min, max)` raised to `gamma`, of linear ProPhoto RGB
//! after the baseline exposure.

pub use craft_raw::gaintable::GainTableMap;
use serde::{Deserialize, Serialize};

/// Applying the map (see the module docs). Index arithmetic saturates and every lookup is
/// checked: the fields are public and the struct is deserializable, so a hand-built or damaged map
/// must not overflow or panic, whatever it holds.
pub trait GainTableMapExt {
    /// The gain at relative active-area position (`x`, `y`) for `p`, a linear ProPhoto (RIMM)
    /// colour after the baseline exposure. 1.0 for a map that can't be evaluated. For many pixels,
    /// [`Self::evaluator`] once and [`GainEval::gain`] per pixel.
    fn gain(&self, p: [f32; 3], x: f64, y: f64) -> f32;

    /// The map ready to evaluate per pixel, or `None` when it can't be (inconsistent sizes).
    fn evaluator(&self) -> Option<GainEval<'_>>;
}

impl GainTableMapExt for GainTableMap {
    fn gain(&self, p: [f32; 3], x: f64, y: f64) -> f32 {
        self.evaluator().map_or(1.0, |e| e.gain(p, x, y))
    }

    fn evaluator(&self) -> Option<GainEval<'_>> {
        let n = self.points_n;
        let tables = self.points_v.saturating_mul(self.points_h);
        if n == 0 || tables == 0 || tables.saturating_mul(n) != self.gains.len() {
            return None;
        }
        let axis = |points: usize, origin: f64, spacing: f64| Axis {
            last: points.saturating_sub(1),
            origin,
            // (a degenerate axis replicates its first table)
            inv: if points > 1 && spacing.is_finite() && spacing > 0.0 { 1.0 / spacing } else { 0.0 },
        };
        Some(GainEval { map: self, v: axis(self.points_v, self.origin_v, self.spacing_v), h: axis(self.points_h, self.origin_h, self.spacing_h) })
    }
}

/// One grid axis, prepared: last table index, origin and inverse spacing (0: one table).
#[derive(Clone, Copy, Debug)]
struct Axis {
    last: usize,
    origin: f64,
    inv: f64,
}

impl Axis {
    /// The cell at relative position `pos`: (first table, second table, weight of the second).
    /// Outside the grid the edge table is replicated.
    #[inline]
    fn cell(self, pos: f64) -> (usize, usize, f32) {
        if self.last == 0 || self.inv == 0.0 {
            return (0, 0, 0.0);
        }
        let f = (pos - self.origin) * self.inv;
        let f = if f.is_nan() { 0.0 } else { f.clamp(0.0, self.last as f64) };
        // (≥ 0: `as` truncates like `floor`, which isn't inlined on baseline x86-64)
        let i0 = (f as usize).min(self.last);
        let i1 = i0.saturating_add(1).min(self.last);
        (i0, i1, (f - i0 as f64) as f32)
    }
}

/// Grid cell along one axis (see [`Axis::cell`]).
#[cfg(test)]
fn cell(pos: f64, origin: f64, spacing: f64, points: usize) -> (usize, usize, f32) {
    let inv = if points > 1 && spacing.is_finite() && spacing > 0.0 { 1.0 / spacing } else { 0.0 };
    Axis { last: points.saturating_sub(1), origin, inv }.cell(pos)
}

/// A [`GainTableMap`] prepared for evaluating many pixels.
#[derive(Clone, Copy, Debug)]
pub struct GainEval<'a> {
    map: &'a GainTableMap,
    v: Axis,
    h: Axis,
}

impl GainEval<'_> {
    /// [`GainTableMapExt::gain`].
    #[inline]
    pub fn gain(&self, p: [f32; 3], x: f64, y: f64) -> f32 {
        let m = self.map;
        let (v0, v1, fv) = self.v.cell(y);
        let (h0, h1, fh) = self.h.cell(x);
        // one fractional index for the four tables (≥ 0, so `as` truncates)
        let n = m.points_n;
        let last = n - 1;
        let idx = (input(m, p) * n as f32).min(last as f32);
        let i0 = (idx as usize).min(last);
        let i1 = i0.saturating_add(1).min(last);
        let t = idx - i0 as f32;
        // (v ≤ points_v − 1, h ≤ points_h − 1 and i ≤ n − 1, with points_v·points_h·n = gains.len()
        // checked by `evaluator`: none of this overflows, and `get` still guards each read)
        let at = |v: usize, h: usize| {
            let table = m.gains.get((v * m.points_h + h) * n..).unwrap_or(&[]);
            let g0 = table.get(i0).copied().unwrap_or(1.0);
            let g1 = table.get(i1).copied().unwrap_or(1.0);
            g0 + (g1 - g0) * t
        };
        let (a, b, c, d) = (at(v0, h0), at(v0, h1), at(v1, h0), at(v1, h1));
        let top = a + (b - a) * fh;
        let bottom = c + (d - c) * fh;
        let g = top + (bottom - top) * fv;
        if g.is_finite() { g.max(0.0) } else { 1.0 }
    }
}

/// The table input for `p` (linear ProPhoto, after the baseline exposure).
#[inline]
fn input(m: &GainTableMap, p: [f32; 3]) -> f32 {
    let w = &m.weights;
    let lo = p[0].min(p[1]).min(p[2]);
    let hi = p[0].max(p[1]).max(p[2]);
    let x = w[0] * p[0] + w[1] * p[1] + w[2] * p[2] + w[3] * lo + w[4] * hi;
    let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
    if m.gamma == 1.0 { x } else { x.powf(m.gamma) }
}

/// Where the pixels of a developed source sit in the raw's active area, for [`GainTableMapExt::gain`].
/// The source is the default crop of the active area (`rect`, relative to the active area),
/// then EXIF-`orientation`ed. Resolution independent.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourcePlacement {
    /// x, y, width, height of the developed crop, relative to the active area.
    pub rect: [f64; 4],
    /// The EXIF orientation applied to the developed crop.
    pub orientation: craft_geom::Orientation,
}

impl SourcePlacement {
    /// Relative active-area position of normalized (0..1) coordinates of the oriented source.
    pub fn active(&self, u: f64, v: f64) -> (f64, f64) {
        let (u, v) = self.orientation.inverse().map(u, v, 1.0, 1.0);
        let [x, y, w, h] = self.rect;
        (x + u * w, y + v * h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(dims: [usize; 3], spacing: [f64; 2], origin: [f64; 2], weights: [f32; 5], gains: Vec<f32>) -> GainTableMap {
        let [points_v, points_h, points_n] = dims;
        let [spacing_v, spacing_h] = spacing;
        let [origin_v, origin_h] = origin;
        GainTableMap { points_v, points_h, points_n, spacing_v, spacing_h, origin_v, origin_h, weights, gamma: 1.0, gains }
    }

    const LUMA: [f32; 5] = [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 0.0, 0.0];

    #[test]
    fn gain_follows_the_table_input_at_input_times_n() {
        // one table of 3 points: index = input × 3 (DNG 1.7.1), clamped to the last point
        let m = map([1, 1, 3], [1.0, 1.0], [0.0, 0.0], LUMA, vec![1.0, 2.0, 3.0]);
        assert_eq!(m.gain([0.0; 3], 0.5, 0.5), 1.0);
        assert!((m.gain([1.0 / 3.0; 3], 0.5, 0.5) - 2.0).abs() < 1e-5, "index 1");
        assert!((m.gain([1.0 / 6.0; 3], 0.5, 0.5) - 1.5).abs() < 1e-5, "index 0.5: linear in the table");
        assert!((m.gain([0.9; 3], 0.5, 0.5) - 3.0).abs() < 1e-5, "index 2.7 clamps to the last point");
        // min/max weights and gamma: input = max(R, G, B)², two points
        let mut m = map([1, 1, 2], [1.0, 1.0], [0.0, 0.0], [0.0, 0.0, 0.0, 0.0, 1.0], vec![1.0, 3.0]);
        m.gamma = 2.0;
        // max 0.5 → 0.25 → index 0.5 → ×2
        assert!((m.gain([0.1, 0.5, 0.2], 0.0, 0.0) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn tables_interpolate_across_the_grid_and_replicate_at_the_edges() {
        // 1 × 2 constant tables at x = 0.25 (×1) and x = 0.75 (×3)
        let m = map([1, 2, 1], [1.0, 0.5], [0.0, 0.25], LUMA, vec![1.0, 3.0]);
        let g = |x: f64| m.gain([0.2; 3], x, 0.5);
        assert!((g(0.25) - 1.0).abs() < 1e-5 && (g(0.5) - 2.0).abs() < 1e-5 && (g(0.75) - 3.0).abs() < 1e-5);
        assert!((g(0.0) - 1.0).abs() < 1e-5 && (g(1.0) - 3.0).abs() < 1e-5);
        assert!((g(-10.0) - 1.0).abs() < 1e-5 && (g(10.0) - 3.0).abs() < 1e-5);
        // vertically too: 2 × 1 at y = 0 (×1) and y = 1 (×2)
        let m = map([2, 1, 1], [1.0, 1.0], [0.0, 0.0], LUMA, vec![1.0, 2.0]);
        assert!((m.gain([0.2; 3], 0.5, 0.25) - 1.25).abs() < 1e-5);
    }

    #[test]
    fn hand_built_maps_never_panic() {
        // review on storytold/lightcraft#273: `(i0 + 1).min(points - 1)` overflowed for points_v = usize::MAX at +inf
        let mut m = map([usize::MAX, 1, 1], [1.0, 1.0], [0.0, 0.0], LUMA, vec![2.0]);
        assert_eq!(m.gain([0.5; 3], 0.5, f64::INFINITY), 1.0, "inconsistent sizes: no gain");
        for (v, h, n, len) in [(usize::MAX, 1, 1, 1), (1, usize::MAX, usize::MAX, 3), (0, 0, 0, 0), (2, 2, 0, 0), (1, 1, 2, 1)] {
            m.points_v = v;
            m.points_h = h;
            m.points_n = n;
            m.gains = vec![2.0; len];
            for spacing in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300] {
                m.spacing_v = spacing;
                m.spacing_h = spacing;
                for p in [[0.5; 3], [f32::NAN; 3], [f32::INFINITY, 0.0, -1.0], [1e30; 3]] {
                    for (x, y) in [(0.5, 0.5), (f64::INFINITY, f64::NEG_INFINITY), (f64::NAN, f64::NAN)] {
                        assert!(m.gain(p, x, y).is_finite());
                    }
                }
            }
        }
        // the grid cell itself saturates (the reviewer's case, reached directly)
        assert_eq!(cell(f64::INFINITY, 0.0, 1.0, usize::MAX), (usize::MAX - 1, usize::MAX - 1, 0.0));
        assert_eq!(cell(f64::NEG_INFINITY, 0.0, 1.0, usize::MAX).0, 0);
        // a consistent map with a huge grid and positions far outside it
        let m = map([1, 1, 2], [1e-300, 1e-300], [0.0, 0.0], [f32::MAX; 5], vec![1.0, 4.0]);
        assert!(m.gain([1e30; 3], f64::INFINITY, -f64::INFINITY).is_finite());
    }

    #[test]
    fn placement_undoes_crop_and_orientation() {
        // the source is the right half of the active area, rotated 90° clockwise
        let p = SourcePlacement { rect: [0.5, 0.0, 0.5, 1.0], orientation: craft_geom::Orientation::Rotate90 };
        // the oriented source's top-left came from the crop's bottom-left
        let (x, y) = p.active(0.0, 0.0);
        assert!((x - 0.5).abs() < 1e-12 && (y - 1.0).abs() < 1e-12, "{x} {y}");
        // its top-right came from the crop's top-left
        let (x, y) = p.active(1.0, 0.0);
        assert!((x - 0.5).abs() < 1e-12 && y.abs() < 1e-12, "{x} {y}");
        let plain = SourcePlacement { rect: [0.0, 0.0, 1.0, 1.0], orientation: craft_geom::Orientation::Normal };
        assert_eq!(plain.active(0.25, 0.75), (0.25, 0.75));
    }
}
