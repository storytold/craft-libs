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
        let pts = |v: &[[f64; 2]]| v.iter().map(|[x, y]| json!([round6(*x), round6(*y)])).collect::<Vec<_>>();
        json!({"c": self.closed, "v": pts(&self.vertices), "i": pts(&self.in_tangents), "o": pts(&self.out_tangents)})
    }
}
