//! [`StrokePath3d`]: a 3D path stroke as the reference app stores it (`DataStroke3D`, made by the "Paint along path" tool,
//! `Curve` in the toolbar). Black box: read from projects the reference app saved after scripted clicks,
//! one change at a time.
//!
//! - `points` is a cubic Bézier spline in the stored mesh's space: anchor, its out-handle, the next anchor's in-handle,
//!   the next anchor, and so on (`type` 1, 3, 2, 1, ...). Anchors carry the surface normal and a `size` and `opacity`
//!   of 0 … 1 (1 when clicked); handles store zeros there (except on the closing segment).
//! - A closed path (`isClosed`) repeats its first anchor at the end, with the closing segment's handles before it.
//! - Default handles, as placed by clicks (Inferred, not needed to replay: the handles are stored): a third of the
//!   distance to the neighbouring anchor, along the line through both neighbours; at an end, towards the neighbour's
//!   handle.

use super::stroke::StoredMat4;

/// What a key of a path is (`type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKeyKind {
    /// 1: a point the path passes through.
    Anchor,
    /// 2: the handle before an anchor.
    In,
    /// 3: the handle after an anchor.
    Out,
    Other(u64),
}

impl PathKeyKind {
    pub fn from_value(v: u64) -> PathKeyKind {
        match v {
            1 => PathKeyKind::Anchor,
            2 => PathKeyKind::In,
            3 => PathKeyKind::Out,
            other => PathKeyKind::Other(other),
        }
    }
}

/// `DataStroke3DKey`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathKey {
    pub kind: PathKeyKind,
    /// `position`, in the stored mesh's space.
    pub position: [f32; 3],
    /// `normal`: the surface's at an anchor; zero on handles.
    pub normal: [f32; 3],
    /// `size` and `opacity`: 0 … 1, as a pen's pressure would scale them (`pressure`, both, before format 92).
    pub size: f32,
    pub opacity: f32,
}

/// One cubic piece of a path: from `points[0]` to `points[3]` through the handles `points[1]` and `points[2]`, with the
/// anchors' size and opacity at its ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathSegment {
    pub points: [[f32; 3]; 4],
    pub normals: [[f32; 3]; 2],
    pub size: [f32; 2],
    pub opacity: [f32; 2],
}

impl PathSegment {
    /// The point at `t` in 0 … 1.
    pub fn at(&self, t: f32) -> [f32; 3] {
        let [a, b, c, d] = self.points;
        let s = 1.0 - t;
        let (w0, w1, w2, w3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
        let mut out = a.map(|v| v * w0);
        for (p, w) in [(b, w1), (c, w2), (d, w3)] {
            for (o, v) in out.iter_mut().zip(p) {
                *o += v * w;
            }
        }
        out
    }
}

/// `DataStroke3D`.
#[derive(Debug, Clone, PartialEq)]
pub struct StrokePath3d {
    pub uid: Option<u64>,
    /// `label` ("Path 1").
    pub label: String,
    /// `isClosed`.
    pub closed: bool,
    /// `points`, in order.
    pub points: Vec<PathKey>,
    /// False if the point list could not be read completely; the points read so far are kept.
    pub complete: bool,
    /// `randomSeed`.
    pub random_seed: u32,
    /// `duplicateCount` and `duplicateMatrix`: symmetry copies, as on brush strokes.
    pub duplicate_count: u32,
    pub duplicate_matrix: StoredMat4,
}

impl StrokePath3d {
    /// The cubic pieces between consecutive anchors. A missing handle is the anchor itself (a straight end).
    pub fn segments(&self) -> Vec<PathSegment> {
        let mut out = Vec::new();
        let mut last: Option<&PathKey> = None;
        let (mut out_handle, mut in_handle): (Option<[f32; 3]>, Option<[f32; 3]>) = (None, None);
        for key in &self.points {
            match key.kind {
                PathKeyKind::Anchor => {
                    if let Some(a) = last {
                        out.push(PathSegment {
                            points: [a.position, out_handle.unwrap_or(a.position), in_handle.unwrap_or(key.position), key.position],
                            normals: [a.normal, key.normal],
                            size: [a.size, key.size],
                            opacity: [a.opacity, key.opacity],
                        });
                    }
                    last = Some(key);
                    out_handle = None;
                    in_handle = None;
                }
                PathKeyKind::Out => out_handle = Some(key.position),
                PathKeyKind::In => in_handle = Some(key.position),
                PathKeyKind::Other(_) => {}
            }
        }
        out
    }

    /// A straight open path from `from` to `to` on a surface facing `normal`, with the handles a click places (a third
    /// of the way along each end): for tests and synthetic strokes.
    pub fn line(from: [f32; 3], to: [f32; 3], normal: [f32; 3]) -> StrokePath3d {
        let third = |k: f32| {
            let mut out = from;
            for (o, t) in out.iter_mut().zip(to) {
                *o = *o + (t - *o) * k;
            }
            out
        };
        let key = |kind, position, normal, w| PathKey { kind, position, normal, size: w, opacity: w };
        StrokePath3d {
            uid: None,
            label: "Path 1".to_string(),
            closed: false,
            points: vec![
                key(PathKeyKind::Anchor, from, normal, 1.0),
                key(PathKeyKind::Out, third(1.0 / 3.0), [0.0; 3], 0.0),
                key(PathKeyKind::In, third(2.0 / 3.0), [0.0; 3], 0.0),
                key(PathKeyKind::Anchor, to, normal, 1.0),
            ],
            complete: true,
            random_seed: 0,
            duplicate_count: 0,
            duplicate_matrix: [0.0; 16],
        }
    }

    /// The anchors.
    pub fn anchors(&self) -> impl Iterator<Item = &PathKey> + '_ {
        self.points.iter().filter(|k| k.kind == PathKeyKind::Anchor)
    }
}
