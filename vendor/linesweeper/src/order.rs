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
                let segi = &segments[i];
                let segj = &segments[j];

                let forward =
                    if let (Some(l0), Some(l1)) = (segi.as_kurbo_line(), segj.as_kurbo_line()) {
                        curve::line::intersect_lines(l0, l1, self.tolerance)
                    } else {
                        let c0 = segi.to_kurbo_cubic();
                        let c0_div = segments.horiz_decomp(i);
                        let c1 = segj.to_kurbo_cubic();
                        let c1_div = segments.horiz_decomp(j);
                        curve::intersect_cubics_with_precomputed_decomp(
                            c0,
                            c0_div,
                            c1,
                            c1_div,
                            self.accuracy,
                            self.tolerance,
                        )
                    };
                debug_assert_eq!(
                    forward.iter().last().unwrap().1,
                    segi.end().y.min(segj.end().y)
                );
                let reverse = forward.flip();
                let v = v.insert((forward, reverse));
                if flipped { &mut v.1 } else { &mut v.0 }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{SegIdx, Segments, curve::Order};

    use super::ComparisonCache;

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
