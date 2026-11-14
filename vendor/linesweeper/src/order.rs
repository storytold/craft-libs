//! Curve order comparisons, with caching.

use rustc_hash::FxHashMap;

use crate::{
    SegIdx, Segment, Segments,
    curve::{self, CurveOrder, Order},
};

/// A cache for curve comparisons, so that each pair of curves needs to be compared at most once.
///
/// The cache holds the comparisons of one [`Segments`] arena. When it is given
/// a different arena, a clone included, it drops the comparisons it holds and
/// starts again with that arena.
#[derive(Clone, Debug)]
pub struct ComparisonCache {
    // Craft patch (comparison cache size): each pair stores the order of its
    // lower-indexed segment relative to the other. A lookup in the other
    // direction flips it into `flipped`.
    inner: FxHashMap<(SegIdx, SegIdx), CurveOrder>,
    flipped: Option<CurveOrder>,
    // Craft patch (comparison cache size): `evict_ended` scans the cache once
    // it holds this many pairs.
    next_purge: usize,
    accuracy: f64,
    tolerance: f64,
    // Craft patch (work limit): the number of calls to `compare_segments` and
    // `with_order`, cached or not, which `Sweeper` checks against its work
    // limit.
    pub(crate) calls: u64,
    // Craft patch (comparison cache arena): the arena that `inner` describes.
    arena: Option<u64>,
}

// Craft patch (comparison cache size): the smallest cache that `evict_ended`
// scans. Unit tests use a small value so that the sweep tests evict.
const MIN_PURGE: usize = if cfg!(test) { 8 } else { 4096 };

impl ComparisonCache {
    /// Creates a new comparison cache.
    ///
    /// `tolerance` tells us how close two curves can be to be declared "ish", and
    /// `accuracy` tells us how closely we need to evaluate the tolerance. For
    /// example, if `accuracy` is `tolerance / 2.0` then we'll guarantee (up to
    /// some floating-point error) that if the two curves are further than
    /// `1.5 * tolerance` apart then we'll give them a strict order, and if they're
    /// less than `tolerance / 2.0` apart then we'll give then an "ish" order.
    pub fn new(tolerance: f64, accuracy: f64) -> Self {
        ComparisonCache {
            inner: FxHashMap::default(),
            flipped: None,
            next_purge: MIN_PURGE,
            accuracy,
            tolerance,
            calls: 0,
            arena: None,
        }
    }

    /// Creates a new comparison cache.
    ///
    /// `tolerance` tells us how close two curves can be to be declared "ish", and
    /// `accuracy` tells us how closely we need to evaluate the tolerance. For
    /// example, if `accuracy` is `tolerance / 2.0` then we'll guarantee (up to
    /// some floating-point error) that if the two curves are further than
    /// `1.5 * tolerance` apart then we'll give them a strict order, and if they're
    /// less than `tolerance / 2.0` apart then we'll give then an "ish" order.
    pub fn new_without_y_slop(tolerance: f64, accuracy: f64) -> Self {
        ComparisonCache {
            inner: FxHashMap::default(),
            flipped: None,
            next_purge: MIN_PURGE,
            accuracy,
            tolerance,
            calls: 0,
            arena: None,
        }
    }

    /// Compares two segments, returning their order.
    ///
    /// # Panics
    ///
    /// Panics if either segment is horizontal.
    pub fn compare_segments(
        &mut self,
        segments: &Segments,
        i: SegIdx,
        j: SegIdx,
    ) -> &mut CurveOrder {
        use std::collections::hash_map::Entry;

        self.calls = self.calls.saturating_add(1);
        // Craft patch (comparison cache arena): comparisons cached for
        // another arena do not describe this one.
        let arena = segments.arena_id();
        if self.arena != Some(arena) {
            self.inner.clear();
            self.arena = Some(arena);
        }

        let (i, j, flipped) = if i.0 < j.0 {
            (i, j, false)
        } else {
            (j, i, true)
        };

        let stored = match self.inner.entry((i, j)) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => {
                v.insert(compute(segments, i, j, self.tolerance, self.accuracy))
            }
        };
        if flipped {
            self.flipped.insert(stored.flip())
        } else {
            stored
        }
    }

    // Craft patch (comparison cache size): removes the pairs in which a
    // segment ends above `y`. Callers pass heights that never decrease, so
    // such a pair is not compared again; if it were, the comparison would be
    // computed again with the same result. The cache is scanned only after it
    // has doubled since the last scan, so the scans take time proportional to
    // the number of comparisons.
    pub(crate) fn evict_ended(&mut self, segments: &Segments, y: f64) {
        if self.inner.len() < self.next_purge {
            return;
        }
        self.inner
            .retain(|(i, j), _| segments[*i].end().y >= y && segments[*j].end().y >= y);
        if self.inner.len() * 4 < self.inner.capacity() {
            self.inner.shrink_to_fit();
        }
        self.next_purge = self.inner.len().saturating_mul(2).max(MIN_PURGE);
    }

    #[cfg(test)]
    pub(crate) fn cached_pairs(&self) -> usize {
        self.inner.len()
    }

    #[cfg(test)]
    pub(crate) fn never_evict(&mut self) {
        self.next_purge = usize::MAX;
    }

    /// Compares two segments without storing a new result.
    ///
    /// `f` receives the order that [`ComparisonCache::compare_segments`] would return: the stored
    /// one if there is one, otherwise one computed the same way and dropped after `f` returns.
    /// Each call counts as one comparison toward the sweep's work limit, as a call to
    /// [`ComparisonCache::compare_segments`] does.
    pub fn with_order<R>(
        &mut self,
        segments: &Segments,
        i: SegIdx,
        j: SegIdx,
        f: impl FnOnce(&CurveOrder) -> R,
    ) -> R {
        self.calls = self.calls.saturating_add(1);
        let (i, j, flipped) = if i.0 < j.0 {
            (i, j, false)
        } else {
            (j, i, true)
        };

        // Comparisons cached for another arena do not describe this one.
        let stored = (self.arena == Some(segments.arena_id()))
            .then(|| self.inner.get(&(i, j)))
            .flatten();
        if let Some(forward) = stored {
            return if flipped {
                f(&forward.flip())
            } else {
                f(forward)
            };
        }
        let forward = compute(segments, i, j, self.tolerance, self.accuracy);
        if flipped {
            f(&forward.flip())
        } else {
            f(&forward)
        }
    }

    /// Returns the number of stored segment pairs.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner.len()
    }
}

// Craft patch (far-apart comparison): compares two segments, at least one of
// which is not a line. A pair that is far apart skips the full comparison.
fn compare_curves(
    segments: &Segments,
    i: SegIdx,
    j: SegIdx,
    accuracy: f64,
    tolerance: f64,
) -> CurveOrder {
    let margin = far_apart_margin(tolerance, accuracy);
    let Some(ret) = order_if_far_apart(&segments[i], &segments[j], margin) else {
        return full_comparison(segments, i, j, accuracy, tolerance);
    };

    // The full comparison can split the heights of a far-apart pair and report
    // some of them as `Ish`. It covers the same heights and never finds the
    // opposite order.
    #[cfg(any(test, feature = "slow-asserts"))]
    {
        let far: Vec<_> = ret.iter().collect();
        let full: Vec<_> = full_comparison(segments, i, j, accuracy, tolerance)
            .iter()
            .collect();
        assert_eq!(far[0].0, full[0].0);
        assert_eq!(far[0].1, full[full.len() - 1].1);
        assert!(full.iter().all(|entry| entry.2 != far[0].2.flip()));
    }

    ret
}

// Craft patch (far-apart comparison).
fn full_comparison(
    segments: &Segments,
    i: SegIdx,
    j: SegIdx,
    accuracy: f64,
    tolerance: f64,
) -> CurveOrder {
    curve::intersect_cubics_with_precomputed_decomp(
        segments[i].to_kurbo_cubic(),
        segments.horiz_decomp(i),
        segments[j].to_kurbo_cubic(),
        segments.horiz_decomp(j),
        accuracy,
        tolerance,
    )
}

// Craft patch (far-apart comparison).
fn far_apart_margin(tolerance: f64, accuracy: f64) -> f64 {
    4.0 * tolerance.max(accuracy)
}

// Craft patch (far-apart comparison): the order of two segments whose horizontal
// bounds are more than `margin` apart, and which overlap vertically. Control points
// bound each segment, so the segments keep this order at every height they share.
//
// The full comparison does not always give this answer. Its quadratic approximation
// has a large error on a steep curve, and it can report part of such a pair as `Ish`.
fn order_if_far_apart(segi: &Segment, segj: &Segment, margin: f64) -> Option<CurveOrder> {
    let order = if segi.max_x() + margin < segj.min_x() {
        Order::Left
    } else if segj.max_x() + margin < segi.min_x() {
        Order::Right
    } else {
        return None;
    };
    let start = segi.start().y.max(segj.start().y);
    let end = segi.end().y.min(segj.end().y);
    (start <= end).then(|| CurveOrder::constant(start, end, order))
}

/// Computes the order of segment `i` relative to segment `j`.
fn compute(segments: &Segments, i: SegIdx, j: SegIdx, tolerance: f64, accuracy: f64) -> CurveOrder {
    let segi = &segments[i];
    let segj = &segments[j];
    // Craft patch (horizontal comparison): horizontal segments are
    // rejected with a message in every build profile.
    assert!(
        !segi.is_horizontal() && !segj.is_horizontal(),
        "compare_segments does not take horizontal segments"
    );

    let forward = if let (Some(l0), Some(l1)) = (segi.as_kurbo_line(), segj.as_kurbo_line()) {
        curve::line::intersect_lines(l0, l1, tolerance)
    } else {
        compare_curves(segments, i, j, accuracy, tolerance)
    };
    debug_assert_eq!(
        forward.iter().last().unwrap().1,
        segi.end().y.min(segj.end().y)
    );
    forward
}

#[cfg(test)]
mod tests {
    use kurbo::BezPath;
    use proptest::prelude::*;

    use crate::{
        SegIdx, Segments,
        curve::{CurveOrder, Order},
    };

    use super::{ComparisonCache, far_apart_margin, full_comparison, order_if_far_apart};

    // Two closed paths with curves that bulge toward each other. The control points of
    // the left curve reach `bulge` and those of the right curve start `gap` after that.
    fn facing_curves(height: f64, bulge: f64, gap: f64) -> Segments {
        let (left, right) = (0.0, bulge + gap);
        let mut a = BezPath::new();
        a.move_to((left, 0.0));
        a.curve_to(
            (bulge, height / 3.0),
            (bulge, 2.0 * height / 3.0),
            (left, height),
        );
        a.line_to((left - 1.0, height / 2.0));
        a.close_path();

        let mut b = BezPath::new();
        b.move_to((right + bulge, 0.0));
        b.curve_to(
            (right, height / 3.0),
            (right, 2.0 * height / 3.0),
            (right + bulge, height),
        );
        b.line_to((right + bulge + 1.0, height / 2.0));
        b.close_path();

        let mut segments = Segments::default();
        segments.add_bez_path(&a).unwrap();
        segments.add_bez_path(&b).unwrap();
        segments
    }

    // A closed path with a y-monotone cubic from `(x0, 0)` to `(x3, height)`. Its other
    // control points are at the heights `a * height` and `b * height`.
    fn steep_curve(xs: [f64; 4], a: f64, b: f64, height: f64) -> BezPath {
        let mut path = BezPath::new();
        path.move_to((xs[0], 0.0));
        path.curve_to((xs[1], a * height), (xs[2], b * height), (xs[3], height));
        path.close_path();
        path
    }

    fn intervals(order: &crate::curve::CurveOrder) -> Vec<(f64, f64, Order)> {
        order.iter().collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2_000))]

        // Steep S-shaped and asymmetric curves, with heights that include the
        // tolerance. Every pair goes through `compare_curves`, which checks in test
        // builds that the full comparison covers the same heights and never finds the
        // opposite order. The full comparison finds `Ish` in only some of these pairs.
        #[test]
        fn far_apart_steep_curves_never_reverse(
            (tolerance, accuracy) in prop_oneof![
                Just((0.1f64, 0.05f64)),
                Just((0.4, 0.05)),
                Just((1e-6, 5e-7)),
            ],
            height in prop_oneof![Just(1.0f64), 1.0f64..4.0, 4.0f64..100.0],
            widths in (-1.0f64..3.0, -1.0f64..3.0),
            shapes in (0usize..3, 0usize..3),
            x_controls in (0.0f64..1.0, 0.0f64..1.0, 0.0f64..1.0, 0.0f64..1.0),
            y_controls in (0.0f64..0.5, 0.5f64..1.0, 0.0f64..0.5, 0.5f64..1.0),
            gap in 1.0001f64..1.5,
            swap: bool,
        ) {
            let height = height * tolerance;
            let shape = |index: usize, x: f64, width: f64, controls: (f64, f64)| match index {
                0 => [x, x + width, x, x + width],
                1 => [x + width, x, x + width, x],
                _ => [x, x + width * controls.0, x + width * controls.1, x + width],
            };
            let w0 = tolerance * 10f64.powf(widths.0);
            let w1 = tolerance * 10f64.powf(widths.1);
            let right = w0 + gap * far_apart_margin(tolerance, accuracy);
            let mut a = steep_curve(
                shape(shapes.0, 0.0, w0, (x_controls.0, x_controls.1)),
                y_controls.0,
                y_controls.1,
                height,
            );
            let mut b = steep_curve(
                shape(shapes.1, right, w1, (x_controls.2, x_controls.3)),
                y_controls.2,
                y_controls.3,
                height,
            );
            if swap {
                std::mem::swap(&mut a, &mut b);
            }
            let mut segments = Segments::default();
            segments.add_bez_path(&a).unwrap();
            segments.add_bez_path(&b).unwrap();

            let mut cmp = ComparisonCache::new(tolerance, accuracy);
            for i in segments.indices() {
                for j in segments.indices() {
                    if i != j {
                        cmp.compare_segments(&segments, i, j);
                    }
                }
            }
        }
    }

    proptest! {
        // Every pair goes through `compare_curves`. The quadratic approximation of
        // these curves has no error to speak of, so the full comparison gives a pair
        // beyond the margin the same single order. The gaps include the margin of each
        // tolerance and accuracy below.
        #[test]
        fn far_apart_curves_compare_like_the_full_comparison(
            height in 0.001f64..10.0,
            bulge in prop_oneof![Just(0.001f64), 0.001f64..0.5, 0.5f64..5.0],
            gap in prop_oneof![
                0.0f64..0.2,
                0.39f64..0.41,
                1.59f64..1.61,
                3.99f64..4.01,
                1.0f64..100.0,
            ],
        ) {
            let segments = facing_curves(height, bulge, gap);
            let eps = 0.1;
            for (tolerance, accuracy) in [(eps, eps / 2.0), (4.0 * eps, eps / 2.0)] {
                let mut cmp = ComparisonCache::new(tolerance, accuracy);
                for i in segments.indices() {
                    for j in segments.indices() {
                        if i != j {
                            let compared = intervals(cmp.compare_segments(&segments, i, j));
                            let margin = far_apart_margin(tolerance, accuracy);
                            if order_if_far_apart(&segments[i], &segments[j], margin).is_some() {
                                let full = full_comparison(&segments, i, j, accuracy, tolerance);
                                prop_assert_eq!(compared, intervals(&full));
                            }
                        }
                    }
                }
            }
        }
    }

    // The full comparison reports part of this pair as `Ish`, so a far-apart answer
    // shows that `compare_curves` did not use it.
    #[test]
    fn far_apart_steep_curves_skip_the_full_comparison() {
        let height = 1.7356316808658495e-6;
        let a = steep_curve(
            [
                0.0,
                2.487438508389841e-5,
                7.989789511893805e-5,
                2.5251761348508994e-4,
            ],
            4.1143632201699014e-7 / height,
            1.3944624441448228e-6 / height,
            height,
        );
        let b = steep_curve(
            [
                2.5693176647313107e-4,
                2.7157585226714214e-4,
                2.9262618025363465e-4,
                4.220260171548132e-4,
            ],
            1.1589752235482678e-6 / height,
            1.6934571328972848e-6 / height,
            height,
        );
        let mut segments = Segments::default();
        segments.add_bez_path(&a).unwrap();
        let curve_b = SegIdx(segments.len());
        segments.add_bez_path(&b).unwrap();
        let (tolerance, accuracy) = (1e-6, 5e-7);

        let full = full_comparison(&segments, SegIdx(0), curve_b, accuracy, tolerance);
        assert!(intervals(&full).iter().any(|entry| entry.2 == Order::Ish));

        let mut cmp = ComparisonCache::new(tolerance, accuracy);
        assert_eq!(
            intervals(cmp.compare_segments(&segments, SegIdx(0), curve_b)),
            vec![(0.0, height, Order::Left)]
        );
    }

    #[test]
    fn far_apart_order() {
        let mut segments = Segments::default();
        segments.add_points([(0.0, 0.0), (1.0, 2.0)]);
        segments.add_points([(2.0, 1.0), (3.0, 3.0)]);
        segments.add_points([(1.5, 2.0), (1.5, 4.0)]);
        segments.add_points([(9.0, 3.0), (9.0, 4.0)]);
        let (a, b, c, d) = (
            &segments[SegIdx(0)],
            &segments[SegIdx(1)],
            &segments[SegIdx(2)],
            &segments[SegIdx(3)],
        );
        let intervals =
            |o: Option<crate::curve::CurveOrder>| o.map(|o| o.iter().collect::<Vec<_>>());

        // The horizontal gap between `a` and `b` is 1.
        assert_eq!(
            intervals(order_if_far_apart(a, b, 0.8)),
            Some(vec![(1.0, 2.0, Order::Left)])
        );
        assert_eq!(
            intervals(order_if_far_apart(b, a, 0.8)),
            Some(vec![(1.0, 2.0, Order::Right)])
        );
        assert_eq!(intervals(order_if_far_apart(a, b, 1.0)), None);

        // `a` and `c` meet only at the height 2.
        assert_eq!(
            intervals(order_if_far_apart(a, c, 0.4)),
            Some(vec![(2.0, 2.0, Order::Left)])
        );

        // `a` and `d` have no height in common.
        assert_eq!(intervals(order_if_far_apart(a, d, 0.4)), None);
    }

    #[test]
    fn unstored_order_matches_stored_order() {
        let mut segments = Segments::default();
        let ends = [
            (-3.0, 5.0),
            (-1e-7, 5.0),
            (0.0, 5.0),
            (1e-7, 4.0),
            (2.5e-7, 7.0),
            (2.0, 1e-3),
            (3.0, 5.0),
            (1e6, 1e6),
        ];
        for &end in &ends {
            segments.add_points([(0.0, 0.0), end]);
        }
        segments
            .add_bez_path(&kurbo::BezPath::from_svg("M0 0 Q 1 1 0 2 Z").unwrap())
            .unwrap();

        for (tolerance, accuracy) in [(1e-6, 5e-7), (4e-6, 5e-7), (0.25, 0.125)] {
            let mut stored = ComparisonCache::new(tolerance, accuracy);
            let mut unstored = ComparisonCache::new(tolerance, accuracy);
            for i in segments.indices() {
                for j in segments.indices() {
                    if i == j {
                        continue;
                    }
                    let computed =
                        unstored.with_order(&segments, i, j, |c| c.iter().collect::<Vec<_>>());
                    let expected = stored
                        .compare_segments(&segments, i, j)
                        .iter()
                        .collect::<Vec<_>>();
                    assert_eq!(computed, expected);
                    // A stored result is passed on as it is.
                    let reused =
                        stored.with_order(&segments, i, j, |c| c.iter().collect::<Vec<_>>());
                    assert_eq!(reused, expected);
                }
            }
            assert_eq!(unstored.len(), 0);
        }
    }

    #[test]
    fn slop_regression() {
        let mut segments = Segments::default();
        let eps = 0.1;
        segments.add_points([(-0.5, -0.5), (-0.5, 0.5)]);
        segments.add_points([(0.0, 0.0), (0.0, 1.0)]);
        let mut cmp_cache = ComparisonCache::new(eps, eps / 2.0);
        assert_eq!(
            cmp_cache
                .compare_segments(&segments, SegIdx(0), SegIdx(1))
                .iter()
                .next()
                .unwrap()
                .2,
            Order::Left
        );
    }

    #[test]
    #[should_panic(expected = "compare_segments does not take horizontal segments")]
    fn horizontal_and_cubic() {
        let mut segments = Segments::default();
        segments.add_points([(-1.0, 1.0), (2.0, 1.0)]);
        let mut path = kurbo::BezPath::new();
        path.move_to((0.0, 0.0));
        path.curve_to((1.0, 0.5), (-1.0, 1.5), (0.0, 2.0));
        segments.add_non_closed_bez_path(&path).unwrap();
        let mut cmp_cache = ComparisonCache::new(0.1, 0.05);
        cmp_cache.compare_segments(&segments, SegIdx(0), SegIdx(1));
    }

    #[test]
    #[should_panic(expected = "compare_segments does not take horizontal segments")]
    fn horizontal_and_line() {
        let mut segments = Segments::default();
        segments.add_points([(-1.0, 1.0), (2.0, 1.0)]);
        segments.add_points([(0.0, 0.0), (0.0, 2.0)]);
        let mut cmp_cache = ComparisonCache::new(0.1, 0.05);
        cmp_cache.compare_segments(&segments, SegIdx(1), SegIdx(0));
    }

    fn first_order(cmp_cache: &mut ComparisonCache, segments: &Segments) -> Order {
        cmp_cache
            .compare_segments(segments, SegIdx(0), SegIdx(1))
            .iter()
            .next()
            .unwrap()
            .2
    }

    #[test]
    fn other_arena_is_compared_again() {
        let mut left_first = Segments::default();
        left_first.add_points([(0.0, 0.0), (0.0, 1.0)]);
        left_first.add_points([(1.0, 0.0), (1.0, 1.0)]);
        let mut right_first = Segments::default();
        right_first.add_points([(1.0, 0.0), (1.0, 1.0)]);
        right_first.add_points([(0.0, 0.0), (0.0, 1.0)]);

        let mut cmp_cache = ComparisonCache::new(0.1, 0.05);
        assert_eq!(first_order(&mut cmp_cache, &left_first), Order::Left);
        assert_eq!(first_order(&mut cmp_cache, &right_first), Order::Right);
        assert_eq!(first_order(&mut cmp_cache, &left_first), Order::Left);
    }

    #[test]
    fn diverged_clone_is_compared_again() {
        let mut original = Segments::default();
        original.add_points([(0.0, 0.0), (0.0, 1.0)]);
        let mut copy = original.clone();
        original.add_points([(1.0, 0.0), (1.0, 1.0)]);
        copy.add_points([(-1.0, 0.0), (-1.0, 1.0)]);

        let mut cmp_cache = ComparisonCache::new(0.1, 0.05);
        assert_eq!(first_order(&mut cmp_cache, &original), Order::Left);
        assert_eq!(first_order(&mut cmp_cache, &copy), Order::Right);
    }

    #[test]
    fn appending_keeps_cached_comparisons() {
        let mut segments = Segments::default();
        segments.add_points([(0.0, 0.0), (0.0, 1.0)]);
        segments.add_points([(1.0, 0.0), (1.0, 1.0)]);
        let mut cmp_cache = ComparisonCache::new(0.1, 0.05);
        assert_eq!(first_order(&mut cmp_cache, &segments), Order::Left);

        segments.add_points([(2.0, 0.0), (2.0, 1.0)]);
        cmp_cache.compare_segments(&segments, SegIdx(1), SegIdx(2));
        assert_eq!(cmp_cache.inner.len(), 2);
    }

    // Two cubics that cross each other twice, and a vertical line that
    // crosses both.
    fn crossing_segments() -> Segments {
        let mut segments = Segments::default();
        let mut path = kurbo::BezPath::new();
        path.move_to((0.0, 0.0));
        path.curve_to((3.0, 1.0), (-3.0, 2.0), (0.0, 3.0));
        segments.add_non_closed_bez_path(&path).unwrap();
        let mut path = kurbo::BezPath::new();
        path.move_to((0.5, 0.0));
        path.curve_to((-2.5, 1.0), (3.5, 2.0), (0.5, 3.0));
        segments.add_non_closed_bez_path(&path).unwrap();
        segments.add_points([(0.2, -0.5), (0.3, 3.5)]);
        segments
    }

    #[test]
    fn one_order_per_pair() {
        let segments = crossing_segments();
        let mut cmp_cache = ComparisonCache::new(0.1, 0.05);
        for i in segments.indices() {
            for j in segments.indices() {
                if i != j {
                    cmp_cache.compare_segments(&segments, i, j);
                }
            }
        }
        let n = segments.len();
        assert_eq!(cmp_cache.inner.len(), n * (n - 1) / 2);
        for stored in cmp_cache.inner.values() {
            assert_eq!(
                std::mem::size_of_val(stored),
                std::mem::size_of::<CurveOrder>()
            );
        }
    }

    #[test]
    fn reversed_lookup_flips_the_stored_order() {
        let segments = crossing_segments();
        let mut forward_first = ComparisonCache::new(0.1, 0.05);
        let mut reverse_first = ComparisonCache::new(0.1, 0.05);
        let entries = |order: &CurveOrder| order.iter().collect::<Vec<_>>();
        for i in segments.indices() {
            for j in segments.indices() {
                if i.0 >= j.0 {
                    continue;
                }
                let reverse_before = entries(reverse_first.compare_segments(&segments, j, i));
                let forward = forward_first.compare_segments(&segments, i, j).clone();
                let reverse_after = forward_first.compare_segments(&segments, j, i).clone();
                let forward_again = entries(forward_first.compare_segments(&segments, i, j));
                let flipped: Vec<_> = entries(&forward)
                    .into_iter()
                    .map(|(y0, y1, order)| (y0, y1, order.flip()))
                    .collect();

                assert_eq!(entries(&reverse_after), flipped);
                assert_eq!(reverse_before, flipped);
                assert_eq!(forward_again, entries(&forward));
                assert_eq!(reverse_after.metrics, forward.metrics);
            }
        }
    }
}
