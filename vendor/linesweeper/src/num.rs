//! Some numerical utilities.

use std::hash::Hash;

/// A wrapper for `f64` that implements `Ord`.
///
/// Unlike the more principled wrappers in the `ordered_float` crate, this one
/// just breaks the `Ord` rules when comparing NaNs -- it doesn't order them,
/// nor does it panic or guard against them on construction. This makes things
/// substantially faster: I measured a 20% improvement to some benchmarks by
/// switching from `OrderedFloat` to `CheapOrderedFloat`.
///
/// Ignoring NaNs should be ok: we check input for NaNs and infinities, and
/// then we make some effort not to introduce them.
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(test, serde(transparent))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheapOrderedFloat(f64);

impl std::ops::Add<CheapOrderedFloat> for CheapOrderedFloat {
    type Output = Self;

    fn add(self, rhs: CheapOrderedFloat) -> Self::Output {
        CheapOrderedFloat(self.0 + rhs.0)
    }
}

impl<'a> std::ops::Add<&'a CheapOrderedFloat> for CheapOrderedFloat {
    type Output = Self;

    fn add(self, rhs: &'a CheapOrderedFloat) -> Self::Output {
        CheapOrderedFloat(self.0 + rhs.0)
    }
}

impl std::ops::Sub<CheapOrderedFloat> for CheapOrderedFloat {
    type Output = Self;

    fn sub(self, rhs: CheapOrderedFloat) -> Self::Output {
        CheapOrderedFloat(self.0 - rhs.0)
    }
}

impl<'a> std::ops::Sub<&'a CheapOrderedFloat> for CheapOrderedFloat {
    type Output = Self;

    fn sub(self, rhs: &'a CheapOrderedFloat) -> Self::Output {
        CheapOrderedFloat(self.0 - rhs.0)
    }
}

impl std::ops::Mul<CheapOrderedFloat> for CheapOrderedFloat {
    type Output = Self;

    fn mul(self, rhs: CheapOrderedFloat) -> Self::Output {
        CheapOrderedFloat(self.0 * rhs.0)
    }
}

impl<'a> std::ops::Mul<&'a CheapOrderedFloat> for CheapOrderedFloat {
    type Output = Self;

    fn mul(self, rhs: &'a CheapOrderedFloat) -> Self::Output {
        CheapOrderedFloat(self.0 * rhs.0)
    }
}

impl std::ops::Div<CheapOrderedFloat> for CheapOrderedFloat {
    type Output = Self;

    fn div(self, rhs: CheapOrderedFloat) -> Self::Output {
        CheapOrderedFloat(self.0 / rhs.0)
    }
}

impl<'a> std::ops::Div<&'a CheapOrderedFloat> for CheapOrderedFloat {
    type Output = Self;

    fn div(self, rhs: &'a CheapOrderedFloat) -> Self::Output {
        CheapOrderedFloat(self.0 / rhs.0)
    }
}

impl std::ops::Neg for CheapOrderedFloat {
    type Output = Self;

    fn neg(self) -> Self::Output {
        CheapOrderedFloat(-self.0)
    }
}

impl Hash for CheapOrderedFloat {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state)
    }
}

impl CheapOrderedFloat {
    /// Retrieve the inner `f64`.
    pub fn into_inner(self) -> f64 {
        self.0
    }
}

// Now comes the fishy stuff.
impl Eq for CheapOrderedFloat {}

impl PartialOrd for CheapOrderedFloat {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CheapOrderedFloat {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.0 < other.0 {
            std::cmp::Ordering::Less
        } else if self.0 > other.0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }
}

impl From<f64> for CheapOrderedFloat {
    fn from(value: f64) -> Self {
        CheapOrderedFloat(value)
    }
}
