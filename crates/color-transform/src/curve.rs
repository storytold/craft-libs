//! Tone curves: monotone cubic Hermite splines and 1D lookup tables.
//!
//! [`MonotoneCurve`] interpolates its points with the tangents of F. N. Fritsch and R. E.
//! Carlson, "Monotone Piecewise Cubic Interpolation", SIAM Journal on Numerical Analysis 17
//! (1980): between two points the curve does not overshoot, so a curve whose points rise never
//! falls.

use std::fmt;

use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// The most points a [`MonotoneCurve`] keeps; points past it are ignored.
pub const MAX_POINTS: usize = 1024;
/// The largest coordinate a [`MonotoneCurve`] point may have; points farther out are dropped.
pub const MAX_COORDINATE: f64 = 1e6;

/// A curve through points in `0..1 × 0..1` (any finite values work). It is saved as its points.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(into = "Vec<(f64, f64)>")]
pub struct MonotoneCurve {
    xs: Vec<f64>,
    ys: Vec<f64>,
    ms: Vec<f64>,
}

impl From<Vec<(f64, f64)>> for MonotoneCurve {
    fn from(points: Vec<(f64, f64)>) -> MonotoneCurve {
        MonotoneCurve::new(&points)
    }
}

/// Reads a saved list of points, keeping the first [`MAX_POINTS`] and skipping the rest
/// without storing them.
impl<'de> Deserialize<'de> for MonotoneCurve {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<MonotoneCurve, D::Error> {
        struct Points;
        impl<'de> Visitor<'de> for Points {
            type Value = MonotoneCurve;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a list of [x, y] points")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<MonotoneCurve, A::Error> {
                let mut p = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX_POINTS));
                while p.len() < MAX_POINTS {
                    match seq.next_element::<(f64, f64)>()? {
                        Some(q) => p.push(q),
                        None => return Ok(MonotoneCurve::new(&p)),
                    }
                }
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(MonotoneCurve::new(&p))
            }
        }
        d.deserialize_seq(Points)
    }
}

impl From<MonotoneCurve> for Vec<(f64, f64)> {
    fn from(c: MonotoneCurve) -> Vec<(f64, f64)> {
        c.points()
    }
}

impl MonotoneCurve {
    /// A curve through `points`, sorted by x. Points with a coordinate that is not finite or is
    /// beyond ±[`MAX_COORDINATE`] are dropped, and of points closer than 1e-9 in x the first is
    /// kept. Fewer than two points give the identity.
    pub fn new(points: &[(f64, f64)]) -> MonotoneCurve {
        let ok = |v: f64| v.is_finite() && v.abs() <= MAX_COORDINATE;
        let mut p: Vec<(f64, f64)> = points.iter().take(MAX_POINTS).copied().filter(|(x, y)| ok(*x) && ok(*y)).collect();
        p.sort_by(|a, b| a.0.total_cmp(&b.0));
        p.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
        if p.len() < 2 {
            p = vec![(0.0, 0.0), (1.0, 1.0)];
        }
        let n = p.len();
        let xs: Vec<f64> = p.iter().map(|q| q.0).collect();
        let ys: Vec<f64> = p.iter().map(|q| q.1).collect();
        let d: Vec<f64> = xs.windows(2).zip(ys.windows(2)).map(|(x, y)| (y[1] - y[0]) / (x[1] - x[0])).collect();
        let mut ms = vec![0.0; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            ms[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) / 2.0 };
        }
        // The Fritsch–Carlson limit: tangents inside the circle of radius 3 keep each segment
        // monotone.
        for i in 0..n - 1 {
            if d[i] == 0.0 {
                ms[i] = 0.0;
                ms[i + 1] = 0.0;
                continue;
            }
            let a = ms[i] / d[i];
            let b = ms[i + 1] / d[i];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                ms[i] = t * a * d[i];
                ms[i + 1] = t * b * d[i];
            }
        }
        MonotoneCurve { xs, ys, ms }
    }

    pub fn identity() -> MonotoneCurve {
        MonotoneCurve::new(&[(0.0, 0.0), (1.0, 1.0)])
    }

    /// Whether the curve maps every value in `0..1` to itself. A curve whose points do not
    /// reach 0 and 1 is not the identity, since it clamps outside them.
    pub fn is_identity(&self) -> bool {
        let covers = self.xs.first().is_some_and(|x| *x <= 0.0) && self.xs.last().is_some_and(|x| *x >= 1.0);
        covers && self.xs.iter().zip(&self.ys).all(|(x, y)| x == y) && self.ms.iter().all(|m| *m == 1.0)
    }

    /// The curve at `x`. Below the first point it is the first point's y, above the last the
    /// last's. NaN gives NaN.
    pub fn eval(&self, x: f64) -> f64 {
        if x.is_nan() {
            return x;
        }
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        let i = match self.xs.binary_search_by(|v| v.total_cmp(&x)) {
            Ok(i) => return self.ys[i],
            Err(i) => i - 1,
        };
        hermite(self.xs[i], self.ys[i], self.ms[i], self.xs[i + 1], self.ys[i + 1], self.ms[i + 1], x)
    }

    /// The curve sampled at `n` evenly spaced points over `0..1` (at least 2).
    pub fn to_lut(&self, n: usize) -> Lut1 {
        let n = n.clamp(2, MAX_LUT);
        Lut1 { v: (0..n).map(|i| self.eval(i as f64 / (n - 1) as f64) as f32).collect() }
    }

    /// The points, sorted by x.
    pub fn points(&self) -> Vec<(f64, f64)> {
        self.xs.iter().copied().zip(self.ys.iter().copied()).collect()
    }

    /// Each point as `[x, y, tangent]`, for code that evaluates the same spline elsewhere (a
    /// GPU pass): between points `k` and `k + 1`, with `h = x₁ − x₀` and `t = (x − x₀) / h`,
    /// the curve is `(2t³ − 3t² + 1) y₀ + (t³ − 2t² + t) h m₀ + (−2t³ + 3t²) y₁ + (t³ − t²) h m₁`.
    pub fn knots(&self) -> impl ExactSizeIterator<Item = [f64; 3]> + '_ {
        self.xs.iter().zip(&self.ys).zip(&self.ms).map(|((x, y), m)| [*x, *y, *m])
    }
}

fn hermite(x0: f64, y0: f64, m0: f64, x1: f64, y1: f64, m1: f64, x: f64) -> f64 {
    let h = x1 - x0;
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * m0 + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * m1
}

/// The most entries [`Lut1::from_fn`] and [`MonotoneCurve::to_lut`] build.
pub const MAX_LUT: usize = 1 << 16;

/// A 1D lookup table over `0..1` with linear interpolation. Inputs outside `0..1` clamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut1 {
    pub v: Vec<f32>,
}

impl Lut1 {
    /// The identity with `n` entries (2 to [`MAX_LUT`]).
    pub fn identity(n: usize) -> Lut1 {
        Lut1::from_fn(n, |x| x)
    }

    /// `f` sampled at `n` evenly spaced points over `0..1` (2 to [`MAX_LUT`]).
    pub fn from_fn(n: usize, f: impl Fn(f32) -> f32) -> Lut1 {
        let n = n.clamp(2, MAX_LUT);
        Lut1 { v: (0..n).map(|i| f(i as f32 / (n - 1) as f32)).collect() }
    }

    /// The table at `x`. `v` is public, so a table of 1 entry reads as that constant and an
    /// empty one as the identity.
    #[inline]
    pub fn eval(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let n = self.v.len();
        if n < 2 {
            return self.v.first().copied().unwrap_or(x);
        }
        let f = x * (n - 1) as f32;
        let i = (f as usize).min(n - 2);
        let t = f - i as f32;
        self.v[i] + (self.v[i + 1] - self.v[i]) * t
    }

    /// `self ∘ inner`: `inner` applies first. An empty `inner` is the identity, so the result
    /// is `self`.
    pub fn compose(&self, inner: &Lut1) -> Lut1 {
        if inner.v.is_empty() {
            return self.clone();
        }
        Lut1 { v: inner.v.iter().map(|&x| self.eval(x)).collect() }
    }

    pub fn is_identity(&self) -> bool {
        let n = self.v.len();
        n >= 2 && self.v.iter().enumerate().all(|(i, &y)| (y - i as f32 / (n - 1) as f32).abs() < 1e-6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_curve_maps_every_value_to_itself() {
        let c = MonotoneCurve::identity();
        assert!(c.is_identity());
        for i in 0..=10 {
            let x = i as f64 / 10.0;
            assert!((c.eval(x) - x).abs() < 1e-12);
        }
        assert!(c.to_lut(256).is_identity());
        assert!(!MonotoneCurve::new(&[(0.0, 0.0), (0.5, 0.6), (1.0, 1.0)]).is_identity());
        // These points lie on the diagonal, but the curve clamps below 0.2 and above 0.8.
        let inner = MonotoneCurve::new(&[(0.2, 0.2), (0.8, 0.8)]);
        assert_eq!(inner.eval(0.0), 0.2);
        assert!(!inner.is_identity());
        assert!(MonotoneCurve::new(&[(-1.0, -1.0), (2.0, 2.0)]).is_identity());
    }

    #[test]
    fn the_curve_passes_through_its_points_and_rises_with_them() {
        let pts = [(0.0, 0.0), (0.25, 0.15), (0.5, 0.5), (0.75, 0.88), (1.0, 1.0)];
        let c = MonotoneCurve::new(&pts);
        for (x, y) in pts {
            assert!((c.eval(x) - y).abs() < 1e-12);
        }
        let mut prev = -1.0;
        for i in 0..=1000 {
            let y = c.eval(i as f64 / 1000.0);
            assert!(y >= prev - 1e-12);
            prev = y;
        }
    }

    /// Data that rise, then stay level: a natural cubic spline overshoots the level part, and
    /// this curve stays on it.
    #[test]
    fn the_curve_does_not_overshoot_a_level_run() {
        let c = MonotoneCurve::new(&[(0.0, 0.0), (0.4, 0.5), (0.6, 0.5), (1.0, 1.0)]);
        for i in 400..=600 {
            let y = c.eval(i as f64 / 1000.0);
            assert!((y - 0.5).abs() < 1e-9, "{y}");
        }
    }

    /// Steep and shallow neighbors: the unlimited average tangent overshoots, the limited one
    /// keeps the segment within its end values.
    #[test]
    fn steep_steps_stay_monotone() {
        let c = MonotoneCurve::new(&[(0.0, 0.0), (0.1, 0.0), (0.2, 1.0), (1.0, 1.0)]);
        let mut prev = -1.0;
        for i in 0..=1000 {
            let y = c.eval(i as f64 / 1000.0);
            assert!((0.0..=1.0).contains(&y) && y >= prev - 1e-12, "{i} {y}");
            prev = y;
        }
    }

    #[test]
    fn the_knots_evaluate_to_the_same_curve() {
        let c = MonotoneCurve::new(&[(0.0, 0.1), (0.3, 0.2), (0.7, 0.9), (1.0, 0.95)]);
        let k: Vec<[f64; 3]> = c.knots().collect();
        assert_eq!(k.len(), 4);
        for i in 0..=100 {
            let x = i as f64 / 100.0;
            let seg = k.windows(2).find(|w| x >= w[0][0] && x <= w[1][0]).unwrap();
            let y = hermite(seg[0][0], seg[0][1], seg[0][2], seg[1][0], seg[1][1], seg[1][2], x);
            assert!((y - c.eval(x)).abs() < 1e-12, "{x}");
        }
    }

    #[test]
    fn luts_compose() {
        let a = Lut1::from_fn(1024, |x| x * x);
        let b = Lut1::from_fn(1024, f32::sqrt);
        assert!((a.compose(&b).eval(0.3) - 0.3).abs() < 2e-3);
        assert_eq!(a.compose(&Lut1 { v: vec![] }), a);
    }

    #[test]
    fn a_curve_saves_as_its_points_and_a_hostile_file_does_not_break_it() {
        let c = MonotoneCurve::new(&[(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)]);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "[[0.0,0.0],[0.5,0.7],[1.0,1.0]]");
        assert_eq!(serde_json::from_str::<MonotoneCurve>(&json).unwrap(), c);
        // A single point, unsorted points and duplicates load as a working curve.
        assert!(serde_json::from_str::<MonotoneCurve>("[[0.5,0.5]]").unwrap().is_identity());
        let messy: MonotoneCurve = serde_json::from_str("[[1.0,1.0],[0.0,0.0],[0.0,0.3],[0.5,0.2]]").unwrap();
        assert_eq!(messy.points(), vec![(0.0, 0.0), (0.5, 0.2), (1.0, 1.0)]);
        // The fields are not read from a file, so their lengths always agree.
        assert!(serde_json::from_str::<MonotoneCurve>(r#"{"xs":[0.0],"ys":[],"ms":[]}"#).is_err());
        // A long list keeps its first MAX_POINTS points.
        let long: Vec<(f64, f64)> = (0..3000).map(|i| (f64::from(i) / 3000.0, f64::from(i) / 3000.0)).collect();
        let loaded: MonotoneCurve = serde_json::from_str(&serde_json::to_string(&long).unwrap()).unwrap();
        assert_eq!(loaded.points().len(), MAX_POINTS);
        assert_eq!(loaded.points()[MAX_POINTS - 1], long[MAX_POINTS - 1]);
    }

    #[test]
    fn nan_input_and_short_or_huge_tables_do_not_panic() {
        let c = MonotoneCurve::new(&[(0.0, 0.0), (0.5, 0.6), (1.0, 1.0)]);
        assert!(c.eval(f64::NAN).is_nan());
        assert!(c.eval(-f64::NAN).is_nan());
        assert_eq!(c.eval(-3.0), 0.0);
        assert_eq!(c.eval(9.0), 1.0);
        for n in [0, 1] {
            assert_eq!(c.to_lut(n).v.len(), 2, "{n}");
            assert!(Lut1::identity(n).is_identity(), "{n}");
        }
        assert_eq!(Lut1::from_fn(usize::MAX, |x| x).v.len(), MAX_LUT);
        assert_eq!(Lut1 { v: vec![] }.eval(0.3), 0.3);
        assert_eq!(Lut1 { v: vec![] }.eval(-1.0), 0.0);
        assert_eq!(Lut1 { v: vec![0.7] }.eval(0.3), 0.7);
        assert!(!Lut1 { v: vec![0.0] }.is_identity());
        let many: Vec<(f64, f64)> = (0..5000).map(|i| (i as f64, i as f64)).collect();
        assert_eq!(MonotoneCurve::new(&many).points().len(), MAX_POINTS);
        let bad = MonotoneCurve::new(&[(f64::NAN, 0.0), (0.2, f64::INFINITY), (0.5, 0.5)]);
        assert!(bad.is_identity());
        // Differences between these would overflow to infinity and give NaN slopes.
        let far = MonotoneCurve::new(&[(-1e308, -1e308), (0.5, 0.25), (1e308, 1e308)]);
        assert!(far.is_identity());
        let wide = MonotoneCurve::new(&[(-1e6, -1e6), (1e6, 1e6), (0.0, 0.5)]);
        assert!(wide.eval(3.0).is_finite());
    }
}
