//! Gradients: the flat stop list of a `g.k` value.

use serde_json::json;

use crate::{Json, nums, round6};

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
