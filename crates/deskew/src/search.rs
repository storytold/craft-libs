//! The projection-profile angle search shared by both estimators.

use crate::MAX_SKEW_DEGREES;

const COARSE_STEP: f64 = 0.5;
/// The finest angle step searched; also the smallest residual a re-measure can resolve.
pub(crate) const FINE_STEP: f64 = 0.05;

/// The most points scored. Past it points are taken at an even stride, so a dense page costs
/// no more than this.
pub(crate) const MAX_POINTS: usize = 200_000;

/// Added to every projected key. A point split between two bins scores `w² + (1 − w)²`, which
/// is 1 at a whole-pixel key and 2/3 on average over fractions. At 0° every key is whole, so
/// without this shift level lines are favoured by up to a third and a 0.1° tilt measures as
/// zero; at this phase a whole key scores the average, 2/3, like any other angle.
const PHASE: f64 = 0.211_324_865_405_187_1;

/// A weighted point on the grid.
#[derive(Clone, Copy)]
pub(crate) struct Point {
    pub x: f64,
    pub y: f64,
    pub weight: f64,
}

/// The angle whose projection of `points` is most sharply peaked, and how far it stands out:
/// `1 - median / best` over the coarse scores, 0 to 1.
pub(crate) fn search(points: &[Point]) -> Option<(f64, f64)> {
    let stride = points.len().div_ceil(MAX_POINTS).max(1);
    let points: Vec<Point> = points.iter().step_by(stride).copied().collect();
    let steps = (MAX_SKEW_DEGREES / COARSE_STEP).round() as i32;
    let coarse: Vec<(f64, f64)> = (-steps..=steps)
        .map(|i| {
            let a = f64::from(i) * COARSE_STEP;
            (a, profile_score(&points, a))
        })
        .collect();
    let (best_coarse, _) = coarse.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1))?;
    let fine_steps = (COARSE_STEP / FINE_STEP).round() as i32;
    let (angle, best) = (-fine_steps..=fine_steps)
        .map(|i| {
            let a = (best_coarse + f64::from(i) * FINE_STEP).clamp(-MAX_SKEW_DEGREES, MAX_SKEW_DEGREES);
            (a, profile_score(&points, a))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let mut scores: Vec<f64> = coarse.iter().map(|&(_, s)| s).collect();
    scores.sort_by(f64::total_cmp);
    let median = scores.get(scores.len() / 2).copied().unwrap_or(0.0);
    let confidence = if best > 0.0 { (1.0 - median / best).clamp(0.0, 1.0) } else { 0.0 };
    Some(((angle * 100.0).round() / 100.0, confidence))
}

/// Sum of squared bin weights of the points projected perpendicular to a line at
/// `angle_degrees` (y down: a line tilted counter-clockwise as displayed keeps
/// `y·cos + x·sin` constant). Each weight is split linearly between the two nearest bins;
/// rounding to one bin beats against the pixel grid and biases the angle.
fn profile_score(points: &[Point], angle_degrees: f64) -> f64 {
    let (sin, cos) = angle_degrees.to_radians().sin_cos();
    let key = |p: &Point| p.y * cos + p.x * sin + PHASE;
    let lo = points.iter().map(key).fold(f64::INFINITY, f64::min).floor();
    let hi = points.iter().map(key).fold(f64::NEG_INFINITY, f64::max);
    if !lo.is_finite() || !hi.is_finite() || hi < lo {
        return 0.0;
    }
    // The grid's diagonal bounds the range, so this stays small.
    let mut bins = vec![0.0f64; (hi - lo) as usize + 2];
    for p in points {
        let at = key(p) - lo;
        let i = at as usize;
        let w = at - at.floor();
        if let Some(b) = bins.get_mut(i) {
            *b += p.weight * (1.0 - w);
        }
        if let Some(b) = bins.get_mut(i + 1) {
            *b += p.weight * w;
        }
    }
    bins.iter().map(|n| n * n).sum()
}
