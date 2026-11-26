//! Curve order comparisons, with caching.

use rustc_hash::FxHashMap;

use crate::{
    SegIdx, Segments,
    curve::{self, CurveOrder},
};

/// A cache for curve comparisons, so that each pair of curves needs to be compared at most once.
#[derive(Clone, Debug)]
pub struct ComparisonCache {
    inner: FxHashMap<(SegIdx, SegIdx), (CurveOrder, CurveOrder)>,
    accuracy: f64,
    tolerance: f64,
}

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
            accuracy,
            tolerance,
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
            accuracy,
            tolerance,
        }
    }

    /// Compares two segments, returning their order.
    pub fn compare_segments(
        &mut self,
        segments: &Segments,
        i: SegIdx,
        j: SegIdx,
    ) -> &mut CurveOrder {
        use std::collections::hash_map::Entry;

        let (i, j, flipped) = if i.0 < j.0 {
            (i, j, false)
        } else {
            (j, i, true)
        };

        match self.inner.entry((i, j)) {
            Entry::Occupied(o) => {
                if flipped {
                    &mut o.into_mut().1
                } else {
                    &mut o.into_mut().0
                }
            }
            Entry::Vacant(v) => {
                let forward = compute(segments, i, j, self.tolerance, self.accuracy);
                let reverse = forward.flip();
                let v = v.insert((forward, reverse));
                if flipped { &mut v.1 } else { &mut v.0 }
            }
        }
    }

    /// Compares two segments without storing a new result.
    ///
    /// `f` receives the order that [`ComparisonCache::compare_segments`] would return: the stored
    /// one if there is one, otherwise one computed the same way and dropped after `f` returns.
    pub fn with_order<R>(
        &self,
        segments: &Segments,
        i: SegIdx,
        j: SegIdx,
        f: impl FnOnce(&CurveOrder) -> R,
    ) -> R {
        let (i, j, flipped) = if i.0 < j.0 {
            (i, j, false)
        } else {
            (j, i, true)
        };

        if let Some((forward, reverse)) = self.inner.get(&(i, j)) {
            return f(if flipped { reverse } else { forward });
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

/// Computes the order of segment `i` relative to segment `j`.
fn compute(segments: &Segments, i: SegIdx, j: SegIdx, tolerance: f64, accuracy: f64) -> CurveOrder {
    let segi = &segments[i];
    let segj = &segments[j];

    let forward = if let (Some(l0), Some(l1)) = (segi.as_kurbo_line(), segj.as_kurbo_line()) {
        curve::line::intersect_lines(l0, l1, tolerance)
    } else {
        let c0 = segi.to_kurbo_cubic();
        let c0_div = segments.horiz_decomp(i);
        let c1 = segj.to_kurbo_cubic();
        let c1_div = segments.horiz_decomp(j);
        curve::intersect_cubics_with_precomputed_decomp(c0, c0_div, c1, c1_div, accuracy, tolerance)
    };
    debug_assert_eq!(
        forward.iter().last().unwrap().1,
        segi.end().y.min(segj.end().y)
    );
    forward
}

#[cfg(test)]
mod tests {
    use crate::{SegIdx, Segments, curve::Order};

    use super::ComparisonCache;

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
            let unstored = ComparisonCache::new(tolerance, accuracy);
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
}
