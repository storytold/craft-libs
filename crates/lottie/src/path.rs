//! Bezier paths (`{c, v, i, o}`).

use serde_json::json;

use crate::{Json, nums, round6};

/// A path: vertices with in and out tangents relative to their vertex.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bezier {
    pub closed: bool,
    pub vertices: Vec<[f64; 2]>,
    /// One per vertex.
    pub in_tangents: Vec<[f64; 2]>,
    /// One per vertex.
    pub out_tangents: Vec<[f64; 2]>,
}

impl Bezier {
    /// Reads a path, or the first of a one-element array (a key's `s`). `closed` is used when
    /// the path has no `c`: Bodymovin 3.x and early 4.x keep the flag outside the path, as `cl`
    /// on a mask and `closed` on an `sh` item. Tangent lists are cut or padded with zeros to the
    /// number of vertices; a point that is not numbers is `[0, 0]`.
    pub fn from_json(j: &Json, closed: bool) -> Option<Self> {
        let o = match j {
            Json::Array(a) => a.first()?,
            o => o,
        };
        let pts = |k: &str| -> Vec<[f64; 2]> {
            let point = |p: &Json| nums(p).map_or([0.0; 2], |v| [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0)]);
            o.get(k).and_then(Json::as_array).map(|a| a.iter().map(point).collect()).unwrap_or_default()
        };
        let vertices = pts("v");
        let n = vertices.len();
        let fit = |mut v: Vec<[f64; 2]>| {
            v.resize(n, [0.0; 2]);
            v
        };
        Some(Self { in_tangents: fit(pts("i")), out_tangents: fit(pts("o")), vertices, closed: o.get("c").and_then(Json::as_bool).unwrap_or(closed) })
    }

    /// The path as `{c, v, i, o}`, coordinates rounded to six decimals.
    pub fn to_json(&self) -> Json {
        self.to_json_with(|v| json!(round6(v)))
    }

    /// Writes the path with a caller-owned numeric representation (for example,
    /// three-decimal compact exports). Geometry and tangent lengths are unchanged.
    /// The callback is applied to every coordinate; callers own finite-value policy.
    pub fn to_json_with(&self, number: impl Fn(f64) -> Json) -> Json {
        let pts = |v: &[[f64; 2]]| v.iter().map(|[x, y]| Json::Array(vec![number(*x), number(*y)])).collect::<Vec<_>>();
        json!({"c": self.closed, "v": pts(&self.vertices), "i": pts(&self.in_tangents), "o": pts(&self.out_tangents)})
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn caller_number_policy_is_applied_without_prior_rounding() {
        let p = Bezier { closed: true, vertices: vec![[1.23456, 1e303]], in_tangents: vec![[0.0, -2.0]], out_tangents: vec![] };
        let raw = p.to_json_with(|v| json!(v));
        assert_eq!(raw, json!({"c": true, "v": [[1.23456, 1e303]], "i": [[0.0, -2.0]], "o": []}));
        let zero = p.to_json_with(|_| json!(0));
        assert_eq!(zero, json!({"c": true, "v": [[0, 0]], "i": [[0, 0]], "o": []}));
        assert_eq!(Bezier::default().to_json(), json!({"c": false, "v": [], "i": [], "o": []}));
    }
}
