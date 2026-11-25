//! [`ProjectStroke`]: one brush stroke as the reference app stores it (`DataStroke`): the pen's path as 2D points with
//! their time, a separate pressure timeline, the camera and the view it was painted in, the symmetry copies and
//! the random seed. Field meanings are in the comments; those marked Inferred come from the corpus, not the reference app.

/// A 4×4 matrix as stored (16 floats; column-major as OpenGL keeps it: the translation is in 12 … 14,
/// Inferred from the view and projection matrices of the corpus).
pub type StoredMat4 = [f32; 16];

/// `DataStrokeKey`: a point of the pen's path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeKey {
    /// `position`: screen position (3D view, normalised device coordinates) or UV (2D view).
    pub position: [f32; 2],
    /// `time`: seconds since the stroke started.
    pub time: f32,
}

/// `DataTimelineKey`: a pen pressure value at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineKey {
    /// `x`: seconds since the stroke started.
    pub time: f32,
    /// `value`: pressure 0 … 1.
    pub value: f32,
}

/// The reference app's `viewType`: where the stroke was painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeView {
    /// 0: the 2D (UV) view; the camera matrices are mostly zero (Inferred).
    Uv,
    /// 1: the 3D view.
    Mesh,
    Other(u64),
}

impl StrokeView {
    pub fn from_value(v: u64) -> StrokeView {
        match v {
            0 => StrokeView::Uv,
            1 => StrokeView::Mesh,
            other => StrokeView::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectStroke {
    pub uid: Option<u64>,
    /// `points`, in order.
    pub points: Vec<StrokeKey>,
    /// `pressure`: empty for strokes made without pressure (a mouse).
    pub pressure: Vec<TimelineKey>,
    /// Entries of `tilt` (never seen filled; their class is unknown).
    pub tilt_keys: usize,
    /// False if a point list could not be read completely (unexpected layout); the points read so far are kept.
    pub complete: bool,
    /// `viewMatrix` (all zero in 2D strokes).
    pub view_matrix: StoredMat4,
    /// `projectionMatrix`.
    pub projection_matrix: StoredMat4,
    /// `viewType`.
    pub view: StrokeView,
    /// `duplicateCount`: symmetry copies (0 = none).
    pub duplicate_count: u32,
    /// `duplicateMatrix`: the step from one copy to the next (a mirror or a rotation, Inferred).
    pub duplicate_matrix: StoredMat4,
    /// `randomSeed`.
    pub random_seed: u32,
    /// `lazyDistance`: the lazy mouse's string (0 = off; 0.08 most common otherwise).
    pub lazy_distance: f32,
}
