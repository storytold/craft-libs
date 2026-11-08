//! Curve order comparisons, with caching.

use rustc_hash::FxHashMap;

use crate::{
    SegIdx, Segments,
    curve::{self, CurveOrder},
};

/// A cache for curve comparisons, so that each pair of curves needs to be compared at most once.
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

        let stored = match self.inner.entry((i, j)) {
            Entry::Occupied(o) => o.into_mut(),
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
                v.insert(forward)
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
}

#[cfg(test)]
mod tests {
    use crate::{
        SegIdx, Segments,
        curve::{CurveOrder, Order},
    };

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
