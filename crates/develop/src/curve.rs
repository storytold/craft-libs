//! The parametric tone curve model shared by the pipeline renderer and the UI curve editor.
//!
//! Lightroom-style tone curve parametric model: four regions (shadows, darks, lights, highlights)
//! separated by three split points. Each region contributes a cosine-windowed bump, summed and
//! forced monotone.

use crate::color::spline::Lut1;
use craft_develop_model::ToneCurve;

/// The four shape controls of the parametric curve, in graph order: region 0 is the darkest.
pub const PARAM_AMOUNTS: [&str; 4] = ["curve.shadows", "curve.darks", "curve.lights", "curve.highlights"];

/// Resolution the parametric curve is sampled at, matching the renderer's LUT.
pub const PARAM_LUT: usize = 1024;

/// The parametric (region) curve of a [`ToneCurve`]: the four region amounts, each a cosine bump
/// over its window between two splits, summed and then forced monotone.
///
/// This is the renderer's curve, not an approximation of it — both `curve_luts` in the pipeline
/// and the curve editor in the UI use this struct so they can never disagree.
#[derive(Clone, Debug)]
pub struct ParametricCurve {
    /// The curve, sampled and already forced monotone.
    lut: [f32; PARAM_LUT],
    /// The three region boundaries as fractions of the graph (0..1).
    pub splits: [f64; 3],
    /// The four region amounts, in graph order (0 = shadows, 3 = highlights).
    pub amounts: [f64; 4],
}

impl ParametricCurve {
    pub fn new(c: &ToneCurve) -> ParametricCurve {
        let s0 = if c.split_shadows.is_nan() { 0.25 } else { (c.split_shadows / 100.0).clamp(0.0, 1.0) as f32 };
        let s1 = if c.split_mid.is_nan() { 0.50_f32.max(s0) } else { (c.split_mid / 100.0).clamp(s0 as f64, 1.0) as f32 };
        let s2 = if c.split_highlights.is_nan() { 0.75_f32.max(s1) } else { (c.split_highlights / 100.0).clamp(s1 as f64, 1.0) as f32 };
        let clean_amt = |a: f64| if a.is_nan() { 0.0 } else { a.clamp(-100.0, 100.0) };
        let (a0, a1, a2, a3) = (clean_amt(c.shadows), clean_amt(c.darks), clean_amt(c.lights), clean_amt(c.highlights));
        let splits = [s0 as f64, s1 as f64, s2 as f64];
        let amounts = [a0, a1, a2, a3];
        let regions = [(0.0, s0, a0), (s0, s1, a1), (s1, s2, a2), (s2, 1.0, a3)];
        let mut lut = [0.0_f32; PARAM_LUT];
        let mut prev = 0.0_f32;
        for (i, entry) in lut.iter_mut().enumerate() {
            let x = i as f32 / (PARAM_LUT - 1) as f32;
            let mut d = 0.0_f32;
            for (a, b, amt) in regions {
                if amt == 0.0 {
                    continue;
                }
                let (ctr, half) = ((a + b) / 2.0, (b - a) * 0.75 + 0.05);
                if half <= f32::EPSILON {
                    continue;
                }
                let t = ((x - ctr) / half).clamp(-1.0, 1.0);
                d += (amt / 100.0) as f32 * 0.22 * (0.5 + 0.5 * (t * std::f32::consts::PI).cos());
            }
            let y = (x + d * 4.0 * x * (1.0 - x)).clamp(0.0, 1.0);
            prev = prev.max(y);
            *entry = prev;
        }
        ParametricCurve { lut, splits, amounts }
    }

    /// Convert the evaluated parametric curve into a 1024-entry [`Lut1`].
    pub fn to_lut(&self) -> Lut1 {
        Lut1 { v: self.lut.to_vec() }
    }

    /// The curve at `x` (0 = black, 1 = white), read off the table (so it never overshoots the ends).
    pub fn at(&self, x: f64) -> f64 {
        let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) as f32 };
        let at = x * (PARAM_LUT - 1) as f32;
        let i = at.floor().clamp(0.0, (PARAM_LUT - 1) as f32) as usize;
        let cur = self.lut.get(i).copied().unwrap_or(0.0);
        let next = self.lut.get(i + 1).copied().unwrap_or(cur);
        let f = at - i as f32;
        (cur + (next - cur) * f) as f64
    }

    /// Where region `region`'s handle sits on the input axis: the middle of its own window, so a
    /// handle never has to cross a split to be grabbed.
    pub fn region_x(&self, region: usize) -> f64 {
        let (a, b) = self.region_window(region);
        (a + b) / 2.0
    }

    /// The window (two splits, as fractions of the graph) region `region` owns.
    pub fn region_window(&self, region: usize) -> (f64, f64) {
        let s0 = self.split_x(0);
        let s1 = self.split_x(1);
        let s2 = self.split_x(2);
        match region {
            0 => (0.0, s0),
            1 => (s0, s1),
            2 => (s1, s2),
            _ => (s2, 1.0),
        }
    }

    /// The half-width of region `region`'s own bump, as fraction of the graph.
    pub fn region_half(&self, region: usize) -> f64 {
        let (a, b) = self.region_window(region);
        (b - a) * 0.75 + 0.05
    }

    /// The cosine window region `region` applies at `x` (1 at its centre, 0 at the window edges).
    pub fn region_window_at(&self, region: usize, x: f64) -> f64 {
        let ctr = self.region_x(region);
        let half = self.region_half(region);
        if half <= f64::EPSILON {
            return 0.0;
        }
        let t = ((x - ctr) / half).clamp(-1.0, 1.0);
        0.5 + 0.5 * (t * std::f64::consts::PI).cos()
    }

    /// The region that owns graph position `x`: the one whose own window contains it. The four
    /// windows tile 0..1, so this is always defined and a drag anywhere on the curve moves exactly
    /// one slider — the region under the pointer.
    pub fn region_at(&self, x: f64) -> usize {
        let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
        let s0 = self.split_x(0);
        let s1 = self.split_x(1);
        let s2 = self.split_x(2);
        if x <= s0 {
            0
        } else if x <= s1 {
            1
        } else if x <= s2 {
            2
        } else {
            3
        }
    }

    /// How much output level region `region`'s bump gains at `x` per unit of its slider: the bump is
    /// linear in the amount, so this is a constant for a given `x`. A drag converts the pointer's
    /// vertical motion into a slider change through it.
    pub fn level_per_unit(&self, region: usize, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        0.22 / 100.0 * self.region_window_at(region, x) * 4.0 * x * (1.0 - x)
    }

    /// The baseline level at `x` from the identity diagonal plus the contributions of all regions
    /// *except* `region`. Used when inverting the model for `region` while holding the other three
    /// regions fixed.
    pub fn base_level_without(&self, region: usize, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let mut d = 0.0;
        for i in 0..4 {
            if i == region {
                continue;
            }
            let amt = self.amounts.get(i).copied().unwrap_or(0.0);
            if amt == 0.0 {
                continue;
            }
            d += (amt / 100.0) * 0.22 * self.region_window_at(i, x);
        }
        x + d * 4.0 * x * (1.0 - x)
    }

    /// The curve level region `region` sits at when its slider holds `amount`, at graph position `x`,
    /// holding the other regions fixed at their current settings.
    pub fn handle_level_at(&self, region: usize, amount: f64, x: f64) -> f64 {
        (self.base_level_without(region, x) + amount * self.level_per_unit(region, x)).clamp(0.0, 1.0)
    }

    /// The amount that puts region `region`'s curve at `level` at `x`, with the other three
    /// regions held fixed — the inverse of [`Self::handle_level_at`]. A level the region cannot
    /// reach saturates rather than jumping.
    pub fn amount_at_level(&self, region: usize, level: f64, x: f64) -> f64 {
        let per_unit = self.level_per_unit(region, x);
        if per_unit.abs() <= f64::EPSILON {
            return self.amounts.get(region).copied().unwrap_or(0.0);
        }
        let base = self.base_level_without(region, x);
        ((level.clamp(0.0, 1.0) - base) / per_unit).clamp(-100.0, 100.0).round()
    }

    /// The graph position split `split` (0..=2) sits at, 0..1.
    pub fn split_x(&self, split: usize) -> f64 {
        self.splits.get(split).copied().unwrap_or(0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_finite_or_inverted_splits_do_not_panic_or_poison_curve() {
        let c = ToneCurve {
            split_shadows: 80.0,
            split_mid: 20.0,
            split_highlights: f64::NAN,
            shadows: f64::NAN,
            lights: f64::INFINITY,
            ..Default::default()
        };
        let model = ParametricCurve::new(&c);
        assert!(model.at(0.5).is_finite());
        assert!(model.at(f64::NAN).is_finite());
        assert!(model.region_at(f64::NAN) < 4);
        assert!((model.splits[0] - 0.8).abs() < 1e-6);
        assert!((model.splits[1] - 0.8).abs() < 1e-6);
        assert!((model.splits[2] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn horizontal_inversion_at_x_0_51_with_darks_60_preserves_zero_lights() {
        let c = ToneCurve { darks: 60.0, lights: 0.0, ..Default::default() };
        let model = ParametricCurve::new(&c);
        assert_eq!(model.region_at(0.51), 2, "x=0.51 is inside the Lights region");
        let level = model.at(0.51);
        assert!((level - 0.5619).abs() < 0.001, "extracted model at 0.51 is ~0.5619: {level}");
        // With other regions held fixed, inverting at level ~0.5619 must return Lights = 0, NOT 45!
        let lights = model.amount_at_level(2, level, 0.51);
        assert_eq!(lights, 0.0, "zero vertical delta preserves Lights=0: {lights}");
    }

    #[test]
    fn a_curve_level_and_its_amount_are_inverses_with_other_regions_present() {
        let c = ToneCurve { shadows: 20.0, darks: -30.0, lights: 40.0, highlights: -10.0, ..Default::default() };
        let model = ParametricCurve::new(&c);
        for region in 0..4 {
            let centre = model.region_x(region);
            let (_, upper) = model.region_window(region);
            for x in [centre, centre + (upper - centre) * 0.5] {
                for amount in [-100.0, -40.0, 0.0, 30.0, 100.0] {
                    let level = model.handle_level_at(region, amount, x);
                    let back = model.amount_at_level(region, level, x);
                    assert!((back - amount).abs() <= 1.0, "region {region} at {x}: {amount} -> {level} -> {back}");
                }
            }
        }
    }
}
