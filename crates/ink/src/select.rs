//! Lasso selection for freehand ink.
//!
//! A lasso is a closed loop the person drags around ink. A stroke is selected when most of its
//! geometry is inside the loop: up to [`LASSO_SAMPLES`] points spread evenly along the stroke are
//! tested with the even–odd rule, and at least 60 % of them must be inside. A stroke that merely
//! crosses the loop (half in, half out) is therefore not selected.
//!
//! The loop itself is bounded: non-finite points are dropped, coordinates are clamped to
//! ±[`MAX_LASSO_COORD`], at most [`MAX_LASSO_INPUT`] input points are read, and a longer loop is
//! thinned evenly to [`MAX_LASSO_POINTS`] points. Fewer than three usable points is no loop.

/// Most points read from a lasso input; the rest are ignored.
pub const MAX_LASSO_INPUT: usize = 20_000;
/// Most points a thinned lasso keeps.
pub const MAX_LASSO_POINTS: usize = 512;
/// Largest coordinate a lasso point may name.
pub const MAX_LASSO_COORD: f64 = 1e7;
/// Points of a stroke tested against the loop: they are spread evenly along it.
pub const LASSO_SAMPLES: usize = 64;
/// The share of samples that must be inside, as a fraction: 6 of 10.
pub const SELECT_FRACTION: (usize, usize) = (6, 10);

/// A closed lasso loop, already sanitised and bounded.
#[derive(Clone, Debug, PartialEq)]
pub struct Lasso {
    polygon: Vec<[f64; 2]>,
}

impl Lasso {
    /// A lasso from a dragged loop, in the caller's coordinates.
    ///
    /// Non-finite points are dropped, coordinates are clamped, and a loop longer than
    /// [`MAX_LASSO_POINTS`] is thinned. `None` when fewer than three usable points remain: there
    /// is no area to test against.
    pub fn new(points: &[[f64; 2]]) -> Option<Lasso> {
        let mut polygon: Vec<[f64; 2]> = points
            .iter()
            .take(MAX_LASSO_INPUT)
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .map(|p| [p[0].clamp(-MAX_LASSO_COORD, MAX_LASSO_COORD), p[1].clamp(-MAX_LASSO_COORD, MAX_LASSO_COORD)])
            .collect();
        if polygon.len() > MAX_LASSO_POINTS {
            let step = polygon.len().div_ceil(MAX_LASSO_POINTS);
            polygon = polygon.into_iter().step_by(step).collect();
        }
        if polygon.len() < 3 {
            return None;
        }
        Some(Lasso { polygon })
    }

    /// The loop's points.
    pub fn polygon(&self) -> &[[f64; 2]] {
        &self.polygon
    }

    /// Left, top, width and height of the loop.
    pub fn bounds(&self) -> [f64; 4] {
        let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for [x, y] in &self.polygon {
            bounds[0] = bounds[0].min(*x);
            bounds[1] = bounds[1].min(*y);
            bounds[2] = bounds[2].max(*x);
            bounds[3] = bounds[3].max(*y);
        }
        [bounds[0], bounds[1], bounds[2] - bounds[0], bounds[3] - bounds[1]]
    }

    /// Whether a point is inside the loop, by the even–odd rule. A non-finite point is never
    /// inside.
    pub fn contains(&self, point: [f64; 2]) -> bool {
        let [x, y] = point;
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        let n = self.polygon.len();
        let mut inside = false;
        for i in 0..n {
            let (Some(a), Some(b)) = (self.polygon.get(i), self.polygon.get((i + n - 1) % n)) else { continue };
            if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
        }
        inside
    }

    /// Whether a stroke is selected: at least 60 % of up to [`LASSO_SAMPLES`] samples spread
    /// evenly along the stroke lie inside the loop.
    ///
    /// `origin` is added to every point, so a host can pass stroke-relative points and where the
    /// stroke sits on its page. An empty or entirely non-finite stroke is not selected.
    pub fn contains_stroke(&self, points: &[[f64; 2]], origin: [f64; 2]) -> bool {
        if points.is_empty() {
            return false;
        }
        let step = points.len().div_ceil(LASSO_SAMPLES).max(1);
        let (mut samples, mut hits) = (0usize, 0usize);
        for point in points.iter().step_by(step) {
            samples += 1;
            if self.contains([origin[0] + point[0], origin[1] + point[1]]) {
                hits += 1;
            }
        }
        let (part, whole) = SELECT_FRACTION;
        samples > 0 && hits.saturating_mul(whole) >= samples.saturating_mul(part)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Lasso {
        Lasso::new(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]).expect("a square")
    }

    #[test]
    fn the_even_odd_rule_handles_a_loop_that_crosses_itself() {
        let lasso = square();
        assert!(lasso.contains([5.0, 5.0]));
        assert!(!lasso.contains([11.0, 5.0]));
        assert!(!lasso.contains([f64::NAN, 5.0]));
        // A bow-tie: the two lobes are inside, the middle crossing is outside.
        let bow = Lasso::new(&[[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]]).expect("a bow tie");
        assert!(bow.contains([2.0, 5.0]));
        assert!(bow.contains([8.0, 5.0]));
        assert!(!bow.contains([5.0, 5.5]));
    }

    #[test]
    fn a_stroke_is_selected_at_sixty_percent_and_not_below() {
        let lasso = square();
        // Ten points along the x axis at y = 5: the first six inside, the rest to the right.
        let mut points = Vec::new();
        for i in 0..10 {
            points.push([i as f64 * 2.0, 5.0]);
        }
        assert!(lasso.contains_stroke(&points[..6], [0.0, 0.0]), "6 of 6");
        let six: Vec<[f64; 2]> = (0..6).map(|i| [i as f64, 5.0]).chain((0..4).map(|i| [20.0 + i as f64, 5.0])).collect();
        assert!(lasso.contains_stroke(&six, [0.0, 0.0]), "6 of 10");
        let five: Vec<[f64; 2]> = (0..5).map(|i| [i as f64, 5.0]).chain((0..5).map(|i| [20.0 + i as f64, 5.0])).collect();
        assert!(!lasso.contains_stroke(&five, [0.0, 0.0]), "5 of 10");
        // The origin moves the stroke with it.
        assert!(!lasso.contains_stroke(&six, [100.0, 0.0]));
        assert!(!lasso.contains_stroke(&[], [0.0, 0.0]));
    }

    #[test]
    fn the_loop_is_bounded_and_thinned() {
        assert_eq!(Lasso::new(&[[0.0, 0.0], [1.0, 1.0]]), None);
        assert_eq!(Lasso::new(&[[f64::NAN, 0.0], [0.0, f64::INFINITY], [1.0, 1.0]]), None);
        let mut many: Vec<[f64; 2]> = (0..10_000).map(|i| [(i % 100) as f64, (i / 100) as f64]).collect();
        many.push([1e300, -1e300]);
        let lasso = Lasso::new(&many).expect("a loop");
        assert!(lasso.polygon().len() <= MAX_LASSO_POINTS);
        assert!(lasso.polygon().iter().flatten().all(|v| v.abs() <= MAX_LASSO_COORD));
        assert!(lasso.polygon().len() >= 3);
    }

    #[test]
    fn sampling_spreads_over_a_long_stroke() {
        // Only the last point is inside; sampling 64 points of 6,400 must still see it.
        let lasso = square();
        let mut points = vec![[50.0, 50.0]; 6_399];
        points.push([5.0, 5.0]);
        assert!(!lasso.contains_stroke(&points, [0.0, 0.0]), "one sample of 64 is below 60 %");
        let mut inside = vec![[5.0, 5.0]; 6_400];
        assert!(lasso.contains_stroke(&inside, [0.0, 0.0]));
        inside[0] = [50.0, 50.0];
        assert!(lasso.contains_stroke(&inside, [0.0, 0.0]));
    }
}
