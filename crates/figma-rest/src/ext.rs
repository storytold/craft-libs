//! Extensions: fields that are not in the REST API, which the readers that map other files onto
//! this shape add so that nothing those files hold is lost. Each is optional; a consumer that
//! doesn't know one ignores it, and the public fields still describe the layer.
//!
//! | Field | On | Holds |
//! |---|---|---|
//! | `vectorNetwork` | a `VECTOR` node | the editable network ([`VectorNetwork`]); `fillGeometry` / `strokeGeometry` stay as the outlines |
//! | `vectorNetworkRef` | a `VECTOR` node | an index into a list of networks handed over beside the JSON, instead of `vectorNetwork` (far smaller for large files) |
//! | `pointCount` | `STAR`, `REGULAR_POLYGON` | the number of points or sides |
//! | `innerRadius` | `STAR` | the inner radius as a fraction of the outer one |
//! | `_letterSpacingPercent` | a text style | letter spacing as a percentage of the font size, where `letterSpacing` is in pixels |
//!
//! Fields starting with `_fig` are private handoffs between a reader and one importer; they are
//! kept like any other unknown field and are not part of this contract.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The name of the vector network field.
pub const VECTOR_NETWORK: &str = "vectorNetwork";
/// The name of the vector network reference field.
pub const VECTOR_NETWORK_REF: &str = "vectorNetworkRef";

/// An editable vector network: points, the curves between them, and the regions they fill.
/// Coordinates are in the node's own space (0, 0 is its top left corner).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorNetwork {
    #[serde(default)]
    pub vertices: Vec<Vertex>,
    #[serde(default)]
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub regions: Vec<Region>,
    /// Fields this model has no name for.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A point of a network.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub x: f64,
    pub y: f64,
    /// The corner radius at this point.
    #[serde(default)]
    pub radius: f64,
    /// Handle mirroring, end cap, join: kept as written.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A curve from vertex `a` to vertex `b`; `ta` and `tb` are the tangents at each end, relative to
/// their vertex (both zero: a straight line).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub a: usize,
    pub b: usize,
    #[serde(default)]
    pub ta: (f64, f64),
    #[serde(default)]
    pub tb: (f64, f64),
}

/// A filled region: loops of segment indices.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub loops: Vec<Vec<usize>>,
    #[serde(default)]
    pub winding: Winding,
    /// Fields this model has no name for.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// How a region's loops fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Winding {
    #[default]
    NonZero,
    EvenOdd,
}

impl VectorNetwork {
    /// Whether every segment names vertices the network has and every loop names segments it
    /// has. A network read from a file is checked before its indices are used.
    pub fn is_consistent(&self) -> bool {
        let (v, s) = (self.vertices.len(), self.segments.len());
        self.segments.iter().all(|seg| seg.a < v && seg.b < v) && self.regions.iter().flat_map(|r| &r.loops).flatten().all(|i| *i < s)
    }
}
