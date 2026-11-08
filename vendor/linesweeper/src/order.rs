//! Curve order comparisons, with caching.

use rustc_hash::FxHashMap;

use crate::{
    SegIdx, Segments,
    curve::{self, CurveOrder},
};

/// A cache for curve comparisons, so that each pair of curves needs to be compared at most once.
///
/// The cache holds the comparisons of one [`Segments`] arena. When it is given
/// a different arena, a clone included, it drops the comparisons it holds and
/// starts again with that arena.
#[derive(Clone, Debug)]
pub struct ComparisonCache {
    inner: FxHashMap<(SegIdx, SegIdx), (CurveOrder, CurveOrder)>,
    accuracy: f64,
    tolerance: f64,
    // Craft patch (comparison cache arena): the arena that `inner` describes.
    arena: Option<u64>,
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
            accuracy,
            tolerance,
            arena: None,
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
}
