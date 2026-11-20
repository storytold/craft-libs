//! Numbers the coordinate formulas are generic over: `f64` (the reference evaluation) and [`Interval`]
//! (bounds on the `f64` results over a whole range of inputs, e.g. a block of output pixels).
//!
//! Formulas written once over [`Real`] serve both. For `f64` every operator is the plain IEEE operation, so a
//! generic formula evaluated with `f64` is bit-identical to the same formula written with `f64` directly, as long
//! as it keeps the operation order (`a * k` instead of `k * a` is fine: IEEE `+` and `×` are commutative).

use std::ops::{Add, Div, Mul, Sub};

/// A scalar the geometry formulas can be evaluated with. Mixed operations take a parameter (`f64`) on the right.
pub trait Real:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Add<f64, Output = Self>
    + Sub<f64, Output = Self>
    + Mul<f64, Output = Self>
    + Div<f64, Output = Self>
{
    /// `if |v| < tiny { tiny } else { v }`: the guard of a denominator against zero ([`crate::Homography::apply`]).
    fn clamp_tiny(self, tiny: f64) -> Self;
    /// The square root of a non-negative value (a negative one gives NaN, or for an [`Interval`] its lower bound
    /// is taken as 0, since the value is a sum of squares).
    fn sqrt_nonneg(self) -> Self;
    /// `self` limited to `lo..=hi` (`lo <= hi`).
    fn clamp_range(self, lo: f64, hi: f64) -> Self;
}

impl Real for f64 {
    fn clamp_tiny(self, tiny: f64) -> f64 {
        if self.abs() < tiny { tiny } else { self }
    }
    fn sqrt_nonneg(self) -> f64 {
        self.sqrt()
    }
    fn clamp_range(self, lo: f64, hi: f64) -> f64 {
        if lo <= hi { self.clamp(lo, hi) } else { self }
    }
}

/// A closed interval `[lo, hi]` with outward rounding, or *undecided* (both NaN).
///
/// Invariant: if the `f64` operands of an operation lie in the operand intervals, the `f64` result (rounded to
/// nearest) lies in the result interval. Each bound is computed in `f64` and then moved one ulp outward
/// (`next_down` / `next_up`); since rounding to nearest is monotonic and errs by less than one ulp, the moved
/// bound is on the far side of every rounded result. By induction a formula evaluated with intervals bounds the
/// *`f64` evaluation* of the same formula (not only the exact real value) for every input in the input intervals.
///
/// A result that can't be bounded becomes undecided and stays so: division by an interval that contains zero,
/// [`Real::clamp_tiny`] when some value could be clamped, and any infinite or NaN bound. Undecided intervals fail
/// every comparison of their bounds, so callers must test the bounds with conditions that are false for NaN.
#[derive(Clone, Copy, Debug)]
pub struct Interval {
    lo: f64,
    hi: f64,
}

impl Interval {
    const UNDECIDED: Interval = Interval { lo: f64::NAN, hi: f64::NAN };

    /// `[lo, hi]` exactly (the bounds are `f64` values already); undecided unless finite with `lo <= hi`.
    pub fn new(lo: f64, hi: f64) -> Interval {
        if lo.is_finite() && hi.is_finite() && lo <= hi { Interval { lo, hi } } else { Interval::UNDECIDED }
    }

    /// The lower bound (NaN if undecided).
    pub fn lo(self) -> f64 {
        self.lo
    }

    /// The upper bound (NaN if undecided).
    pub fn hi(self) -> f64 {
        self.hi
    }

    /// Whether the bounds are usable (not undecided).
    pub fn is_decided(self) -> bool {
        self.lo <= self.hi
    }

    /// Bounds computed in round-to-nearest, moved one ulp outward.
    fn outward(lo: f64, hi: f64) -> Interval {
        Interval::new(lo.next_down(), hi.next_up())
    }

    /// The hull of four products or quotients, moved outward. NaN in any of them makes it undecided.
    fn hull(v: [f64; 4]) -> Interval {
        if v.iter().any(|v| !v.is_finite()) {
            return Interval::UNDECIDED;
        }
        let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Interval::outward(lo, hi)
    }
}

impl From<f64> for Interval {
    fn from(v: f64) -> Interval {
        Interval::new(v, v)
    }
}

impl Add for Interval {
    type Output = Interval;
    fn add(self, o: Interval) -> Interval {
        Interval::outward(self.lo + o.lo, self.hi + o.hi)
    }
}

impl Sub for Interval {
    type Output = Interval;
    fn sub(self, o: Interval) -> Interval {
        Interval::outward(self.lo - o.hi, self.hi - o.lo)
    }
}

impl Mul for Interval {
    type Output = Interval;
    fn mul(self, o: Interval) -> Interval {
        Interval::hull([self.lo * o.lo, self.lo * o.hi, self.hi * o.lo, self.hi * o.hi])
    }
}

impl Div for Interval {
    type Output = Interval;
    /// Undecided when the divisor may be zero (or is undecided).
    fn div(self, o: Interval) -> Interval {
        if !(o.lo > 0.0 || o.hi < 0.0) {
            return Interval::UNDECIDED;
        }
        Interval::hull([self.lo / o.lo, self.lo / o.hi, self.hi / o.lo, self.hi / o.hi])
    }
}

impl Add<f64> for Interval {
    type Output = Interval;
    fn add(self, k: f64) -> Interval {
        Interval::outward(self.lo + k, self.hi + k)
    }
}

impl Sub<f64> for Interval {
    type Output = Interval;
    fn sub(self, k: f64) -> Interval {
        Interval::outward(self.lo - k, self.hi - k)
    }
}

impl Mul<f64> for Interval {
    type Output = Interval;
    fn mul(self, k: f64) -> Interval {
        let (a, b) = (self.lo * k, self.hi * k);
        if k >= 0.0 { Interval::outward(a, b) } else { Interval::outward(b, a) }
    }
}

impl Div<f64> for Interval {
    type Output = Interval;
    /// Undecided when `k` is zero (or NaN).
    fn div(self, k: f64) -> Interval {
        let (a, b) = (self.lo / k, self.hi / k);
        if k > 0.0 {
            Interval::outward(a, b)
        } else if k < 0.0 {
            Interval::outward(b, a)
        } else {
            Interval::UNDECIDED
        }
    }
}

impl Real for Interval {
    /// Unchanged when no value in the interval would be clamped, else undecided.
    fn clamp_tiny(self, tiny: f64) -> Interval {
        if self.lo >= tiny || self.hi <= -tiny { self } else { Interval::UNDECIDED }
    }

    /// `sqrt` is monotonic and correctly rounded, so one ulp outward encloses the `f64` result.
    fn sqrt_nonneg(self) -> Interval {
        if !self.is_decided() || self.hi < 0.0 {
            return Interval::UNDECIDED;
        }
        Interval::outward(self.lo.max(0.0).sqrt(), self.hi.sqrt())
    }

    /// Clamping is monotonic and exact.
    fn clamp_range(self, lo: f64, hi: f64) -> Interval {
        let ordered = lo <= hi;
        if !self.is_decided() || !ordered {
            return Interval::UNDECIDED;
        }
        Interval::new(self.lo.clamp(lo, hi), self.hi.clamp(lo, hi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn contains(i: Interval, v: f64) -> bool {
        i.lo <= v && v <= i.hi
    }

    /// A value inside `[lo, hi]` at fraction `t`.
    fn pick(i: Interval, t: f64) -> f64 {
        (i.lo + (i.hi - i.lo) * t).clamp(i.lo, i.hi)
    }

    proptest! {
        /// Every `f64` result of `a ∘ b` with `a ∈ A`, `b ∈ B` lies in `A ∘ B` (or that is undecided).
        #[test]
        fn operations_enclose_the_f64_results(
            a0 in -1e6f64..1e6, aw in 0.0f64..1e3, b0 in -1e3f64..1e3, bw in 0.0f64..10.0,
            ta in 0.0f64..=1.0, tb in 0.0f64..=1.0, k in -1e3f64..1e3,
        ) {
            let (ia, ib) = (Interval::new(a0, a0 + aw), Interval::new(b0, b0 + bw));
            let (a, b) = (pick(ia, ta), pick(ib, tb));
            prop_assert!(contains(ia + ib, a + b));
            prop_assert!(contains(ia - ib, a - b));
            prop_assert!(contains(ia * ib, a * b));
            let q = ia / ib;
            prop_assert!(!q.is_decided() || contains(q, a / b));
            prop_assert!(contains(ia + k, a + k));
            prop_assert!(contains(ia - k, a - k));
            prop_assert!(contains(ia * k, a * k));
            prop_assert!(k == 0.0 || contains(ia / k, a / k));
            let c = ib.clamp_tiny(1.0);
            prop_assert!(!c.is_decided() || contains(c, b.clamp_tiny(1.0)));
            let sq = (ia * ia).sqrt_nonneg();
            prop_assert!(!sq.is_decided() || contains(sq, (a * a).sqrt()));
            let cl = ib.clamp_range(-1.0, 2.0);
            prop_assert!(contains(cl, b.clamp_range(-1.0, 2.0)));
        }
    }

    #[test]
    fn unbounded_results_are_undecided() {
        let i = Interval::new(-1.0, 2.0);
        assert!(!(i / Interval::new(-1e-9, 1.0)).is_decided());
        assert!(!(i / Interval::new(0.0, 1.0)).is_decided());
        assert!((i / Interval::new(1e-9, 1.0)).is_decided());
        assert!(!(i / 0.0).is_decided());
        assert!(!(Interval::new(f64::MAX, f64::MAX) * 2.0).is_decided());
        assert!(!Interval::new(2.0, 1.0).is_decided());
        assert!(!Interval::new(f64::NAN, 1.0).is_decided());
        // undecided propagates through everything
        let u = Interval::UNDECIDED;
        for r in [u + i, i - u, u * i, i / u, u + 1.0, u * 0.0, u.clamp_tiny(1e-300)] {
            assert!(!r.is_decided());
            assert!(r.lo.is_nan() && r.hi.is_nan(), "undecided bounds are NaN, which fails every comparison");
        }
        // the denominator guard: unchanged only when no value is clamped
        assert!(Interval::new(1e-300, 1.0).clamp_tiny(1e-300).is_decided());
        assert!(Interval::new(-1.0, -1e-300).clamp_tiny(1e-300).is_decided());
        assert!(!Interval::new(-1.0, 1.0).clamp_tiny(1e-300).is_decided());
        assert!(!Interval::new(5e-301, 1.0).clamp_tiny(1e-300).is_decided());
    }
}
