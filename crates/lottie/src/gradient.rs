//! Gradients: the flat stop list of a `g.k` value.

use serde_json::json;

use crate::{Json, nums, round6};

/// Raw gradient wire values from borrowed or mapped stops, without intermediate allocation.
///
/// Yields color groups (`position, r, g, b`) then every opacity pair (`position, alpha`).
/// Input order and all values, including non-finite values and opaque alpha, are preserved.
/// Sanitization, rounding, and input/output size budgets remain the caller's responsibility.
pub fn flat_gradient_values(
    colors: impl IntoIterator<Item = (f64, [f64; 3])>,
    opacities: impl IntoIterator<Item = (f64, f64)>,
) -> impl Iterator<Item = f64> {
    colors.into_iter().flat_map(|(p, [r, g, b])| [p, r, g, b]).chain(opacities.into_iter().flat_map(|(p, a)| [p, a]))
}

/// A gradient: colour stops and opacity stops, each at a position in 0..=1.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gradient {
    /// `(position, [r, g, b])`.
    pub colors: Vec<(f64, [f64; 3])>,
    /// `(position, alpha)`.
    pub opacities: Vec<(f64, f64)>,
}

impl Gradient {
    /// Reads the flat list: `ncolors` (the `g.p` of the item) groups of `position, r, g, b`,
    /// then pairs of `position, alpha`. `ncolors` is held to the groups the list has; a list
    /// without alpha pairs is opaque (`(0, 1)` and `(1, 1)`).
    pub fn from_flat(j: &Json, ncolors: usize) -> Option<Self> {
        let v = nums(j)?;
        let n = ncolors.min(v.len() / 4);
        let (head, rest) = v.split_at_checked(n * 4)?;
        let colors = head.as_chunks::<4>().0.iter().map(|[p, r, g, b]| (*p, [*r, *g, *b])).collect();
        let mut opacities: Vec<(f64, f64)> = rest.as_chunks::<2>().0.iter().map(|[p, a]| (*p, *a)).collect();
        if opacities.is_empty() {
            opacities = vec![(0.0, 1.0), (1.0, 1.0)];
        }
        Some(Self { colors, opacities })
    }

    /// Raw color groups (`position, r, g, b`), followed by all opacity pairs
    /// (`position, alpha`), in input order. No rounding, clamping, or opacity elision.
    ///
    /// This allocation-free view lets callers fit or interpolate equal-sized animated
    /// values before applying their own numeric policy. In particular, opaque alpha
    /// stops remain present so an opaque frame can interpolate to a translucent one.
    /// Non-finite values are preserved; callers must sanitize them before JSON output.
    pub fn flat_values(&self) -> impl Iterator<Item = f64> + '_ {
        flat_gradient_values(self.colors.iter().copied(), self.opacities.iter().copied())
    }

    /// The flat list, rounded to six decimals. The alpha pairs are left out when every stop is
    /// opaque.
    pub fn to_flat(&self) -> Json {
        let mut out = vec![];
        for (p, [r, g, b]) in &self.colors {
            out.extend([round6(*p), round6(*r), round6(*g), round6(*b)]);
        }
        if !self.opacities.iter().all(|(_, a)| (*a - 1.0).abs() < 1e-6) {
            for (p, a) in &self.opacities {
                out.extend([round6(*p), round6(*a)]);
            }
        }
        json!(out)
    }
}

#[cfg(test)]
mod raw_tests {
    use super::*;

    #[test]
    fn raw_values_keep_order_precision_and_opaque_alpha() {
        let g = Gradient { colors: vec![(0.123456789, [0.2, 0.3, 0.4]), (1.0, [0.5, 0.6, 0.7])], opacities: vec![(0.0, 1.0), (1.0, 1.0)] };
        assert_eq!(g.flat_values().collect::<Vec<_>>(), vec![0.123456789, 0.2, 0.3, 0.4, 1.0, 0.5, 0.6, 0.7, 0.0, 1.0, 1.0, 1.0]);
        assert_eq!(g.to_flat(), json!([0.123457, 0.2, 0.3, 0.4, 1.0, 0.5, 0.6, 0.7]));
    }

    #[test]
    fn raw_values_keep_nonfinite_signed_zero_and_unpaired_lists() {
        let g = Gradient { colors: vec![], opacities: vec![(-0.0, f64::NAN), (f64::INFINITY, f64::NEG_INFINITY)] };
        let bits: Vec<_> = g.flat_values().map(f64::to_bits).collect();
        assert_eq!(bits, [-0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY].map(f64::to_bits));
        assert_eq!(Gradient::default().flat_values().count(), 0);
        let g = Gradient { colors: vec![(1e303, [1.0, 0.0, 0.0])], opacities: vec![] };
        assert_eq!(g.flat_values().collect::<Vec<_>>(), [1e303, 1.0, 0.0, 0.0]);
    }
}
