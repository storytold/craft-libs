//! A camera's white-balance presets at stated colour temperatures, as data.
//!
//! Some cameras record the multipliers they would apply for each of their white-balance presets, with the
//! temperature the camera gives the preset (Canon CR2: the maker note's `ColorData`). Others record only the
//! camera's neutral multipliers for CIE Illuminant A and D65 (Fujifilm RAF, raw IFD tag `0xf00c`):
//! [`KelvinPresets::from_neutral_pair`] keeps that pair. For a raw without colour matrices they are the camera's own
//! Kelvin scale. This crate reads and orders them; placing a white on the curve
//! through them (temperature and tint) is the developer's arithmetic.

use serde::{Deserialize, Serialize};

/// Most points a curve keeps (Canon writes about seven at a stated temperature).
const MAX_POINTS: usize = 32;

/// A camera's multipliers at stated temperatures (at least three), ascending, green = 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KelvinPresets {
    points: Vec<(f32, [f32; 3])>,
    /// Multipliers (green = 1) for Illuminant A and D65, when that pair is all the file records.
    #[serde(default)]
    pair: Option<[[f32; 3]; 2]>,
}

fn usable(m: [f32; 3]) -> bool {
    m.iter().all(|v| *v > 0.0 && v.is_finite())
}

fn log_point(m: [f32; 3]) -> [f64; 2] {
    [(m[0] as f64 / m[1] as f64).ln(), (m[2] as f64 / m[1] as f64).ln()]
}

impl KelvinPresets {
    /// `points` as (Kelvin, multipliers), ascending in Kelvin; `None` unless they make a curve: three to 32 of
    /// them, positive multipliers, red rising and blue falling from one preset to the next.
    pub fn new(points: impl IntoIterator<Item = (f32, [f32; 3])>) -> Option<KelvinPresets> {
        let points: Vec<_> = points.into_iter().take(MAX_POINTS + 1).collect();
        let ok = (3..=MAX_POINTS).contains(&points.len())
            && points.iter().all(|(k, m)| *k > 0.0 && k.is_finite() && usable(*m))
            && points.windows(2).all(|w| {
                let (a, b) = (log_point(w[0].1), log_point(w[1].1));
                w[0].0 < w[1].0 && b[0] > a[0] && b[1] < a[1]
            });
        ok.then(|| KelvinPresets { points: points.into_iter().map(|(k, m)| (k, [m[0] / m[1], 1.0, m[2] / m[1]])).collect(), pair: None })
    }

    /// A curve from presets in any order, as a camera lists them by name (Canon's Daylight, Shade, Cloudy, …):
    /// sorted by Kelvin, a repeated temperature kept once, unusable points and any that would turn the curve back
    /// (red not rising or blue not falling from the warmer neighbour kept) dropped; then [`Self::new`].
    pub fn from_unordered(points: impl IntoIterator<Item = (f32, [f32; 3])>) -> Option<KelvinPresets> {
        let mut pts: Vec<_> = points.into_iter().take(MAX_POINTS).filter(|(k, m)| *k > 0.0 && k.is_finite() && usable(*m)).collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut kept: Vec<(f32, [f32; 3])> = Vec::with_capacity(pts.len());
        for p in pts {
            let fits = kept.last().is_none_or(|q| {
                let (a, b) = (log_point(q.1), log_point(p.1));
                q.0 < p.0 && b[0] > a[0] && b[1] < a[1]
            });
            if fits {
                kept.push(p);
            }
        }
        KelvinPresets::new(kept)
    }

    /// The camera's neutral multipliers for Illuminant A and for D65; `None` unless both are positive and A is
    /// the warmer (more blue gain, less red: red rising and blue falling from A to D65, as on every preset curve).
    /// The temperatures belong to the illuminants, not the file, so placing points between them is the developer's
    /// colour arithmetic; [`Self::points`] is then empty.
    pub fn from_neutral_pair(a: [f32; 3], d65: [f32; 3]) -> Option<KelvinPresets> {
        if !usable(a) || !usable(d65) {
            return None;
        }
        let (pa, pd) = (log_point(a), log_point(d65));
        let unit = |m: [f32; 3]| [m[0] / m[1], 1.0, m[2] / m[1]];
        (pd[0] > pa[0] && pd[1] < pa[1]).then(|| KelvinPresets { points: Vec::new(), pair: Some([unit(a), unit(d65)]) })
    }

    /// The presets as (Kelvin, multipliers with green = 1), ascending in Kelvin. Empty for a neutral pair.
    pub fn points(&self) -> &[(f32, [f32; 3])] {
        &self.points
    }

    /// The Illuminant A and D65 multipliers (green = 1) of [`Self::from_neutral_pair`].
    pub fn neutral_pair(&self) -> Option<[[f32; 3]; 2]> {
        self.pair
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn five() -> Vec<(f32, [f32; 3])> {
        [(2500.0, 1.0, 3.2), (3200.0, 1.3, 2.6), (4500.0, 1.8, 1.9), (6000.0, 2.3, 1.5), (8500.0, 2.8, 1.2)]
            .map(|(k, r, b)| (k, [r, 1.0, b]))
            .to_vec()
    }

    #[test]
    fn unusable_presets_give_nothing() {
        assert!(KelvinPresets::new(five()).is_some());
        let mut pts = five();
        pts.swap(1, 2);
        assert_eq!(KelvinPresets::new(pts), None, "out of order");
        let mut pts = five();
        pts[4].1 = pts[3].1;
        assert_eq!(KelvinPresets::new(pts), None, "no step");
        let mut pts = five();
        pts[0].1[0] = 0.0;
        assert_eq!(KelvinPresets::new(pts), None);
        assert_eq!(KelvinPresets::new(five()[..2].to_vec()), None, "too few");
        assert_eq!(KelvinPresets::new((0..33).map(|i| (2000.0 + i as f32, [1.0 + i as f32, 1.0, 40.0 - i as f32]))), None, "too many");
    }

    /// Canon lists its presets by name, not by temperature, with repeats and an off-curve one among them: the
    /// curve takes them in Kelvin order, each temperature once, and leaves out a point that would turn it back.
    #[test]
    fn a_curve_from_presets_in_any_order() {
        let g = |r: f32, b: f32| [r / 1024.0, 1.0, b / 1024.0];
        let p = KelvinPresets::from_unordered([
            (5651.0, g(1418.0, 1357.0)),
            (7505.0, g(1653.0, 1171.0)),
            (6594.0, g(1533.0, 1259.0)),
            (3174.0, g(963.0, 2016.0)),
            (5651.0, g(1418.0, 1357.0)),
            (8000.0, g(1400.0, 1400.0)),
            (8529.0, g(1807.0, 1080.0)),
            (f32::NAN, g(1000.0, 1000.0)),
        ])
        .unwrap();
        assert_eq!(p.points().len(), 5);
        assert!(p.points().windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(KelvinPresets::from_unordered([(5000.0, g(1400.0, 1400.0)), (6000.0, g(1500.0, 1300.0))]), None);
        assert_eq!(KelvinPresets::from_unordered(std::iter::empty()), None);
    }

    #[test]
    fn a_neutral_pair_is_kept_as_stated() {
        let p = KelvinPresets::from_neutral_pair([1.2, 0.8, 2.4], [2.0, 1.0, 1.5]).unwrap();
        assert_eq!(p.neutral_pair(), Some([[1.5, 1.0, 3.0], [2.0, 1.0, 1.5]]));
        assert!(p.points().is_empty());
        assert_eq!(KelvinPresets::from_neutral_pair([2.0, 1.0, 1.5], [1.5, 1.0, 3.0]), None, "D65 warmer than A");
        assert_eq!(KelvinPresets::from_neutral_pair([1.5, 1.0, 3.0], [1.5, 1.0, 3.0]), None, "one point twice");
        assert_eq!(KelvinPresets::from_neutral_pair([0.0, 1.0, 3.0], [2.0, 1.0, 1.5]), None);
        assert_eq!(KelvinPresets::from_neutral_pair([f32::NAN, 1.0, 3.0], [2.0, 1.0, 1.5]), None);
        assert_eq!(
            KelvinPresets::new([(2500.0, [1.0, 1.0, 3.2]), (4500.0, [1.8, 1.0, 1.9]), (8500.0, [2.8, 1.0, 1.2])]).unwrap().neutral_pair(),
            None
        );
    }
}
