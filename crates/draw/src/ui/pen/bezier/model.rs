//! The paths canvas overlays edit and draw: knots, subpaths, the host's selection and the edits
//! the overlays report. The host converts its own path type to these each frame and applies the
//! edits to its own.

use craft_theme::Srgba;
use kurbo::{BezPath, CubicBez, Point, Vec2};

/// One point of a path: its anchor and its two direction handles, in document units. A handle at
/// its anchor is retracted.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Knot {
    /// The anchor point.
    pub anchor: Point,
    /// The handle of the segment arriving at the anchor.
    pub in_ctrl: Point,
    /// The handle of the segment leaving the anchor.
    pub out_ctrl: Point,
    /// A smooth point keeps its handles collinear when one turns (the host's rule).
    pub smooth: bool,
}

impl Knot {
    /// A knot with its anchor, both handles and whether it is smooth.
    pub fn new(anchor: Point, in_ctrl: Point, out_ctrl: Point, smooth: bool) -> Self {
        Self { anchor, in_ctrl, out_ctrl, smooth }
    }

    /// A corner point with both handles retracted.
    pub fn corner(anchor: Point) -> Self {
        Self { anchor, in_ctrl: anchor, out_ctrl: anchor, smooth: false }
    }

    /// A smooth point whose out handle is `out` and whose in handle mirrors it.
    pub fn smooth(anchor: Point, out: Point) -> Self {
        Self { anchor, in_ctrl: anchor - (out - anchor), out_ctrl: out, smooth: true }
    }

    /// The handle on `side`.
    pub fn handle(&self, side: HandleSide) -> Point {
        match side {
            HandleSide::In => self.in_ctrl,
            HandleSide::Out => self.out_ctrl,
        }
    }

    /// Whether the handle on `side` lies on the anchor.
    pub fn retracted(&self, side: HandleSide) -> bool {
        self.handle(side) == self.anchor
    }
}

/// One subpath: its knots in order, open or closed.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct EditSubpath {
    /// The knots in path order.
    pub knots: Vec<Knot>,
    /// Whether the last knot joins back to the first.
    pub closed: bool,
}

impl EditSubpath {
    /// A subpath of `knots`, closed when `closed`.
    pub fn new(knots: Vec<Knot>, closed: bool) -> Self {
        Self { knots, closed }
    }

    /// The number of segments: one between each pair of neighbouring knots, and one from the last
    /// knot back to the first when closed.
    pub fn segment_count(&self) -> usize {
        match self.knots.len() {
            0 | 1 => 0,
            n if self.closed => n,
            n => n - 1,
        }
    }

    /// The knots a segment joins: the segment `i` leaves knot `i` for the next one.
    pub fn segment_ends(&self, i: usize) -> Option<(usize, usize)> {
        (i < self.segment_count()).then(|| (i, (i + 1) % self.knots.len()))
    }

    /// The segment leaving knot `i`, as a cubic in document units.
    pub fn segment(&self, i: usize) -> Option<CubicBez> {
        let (a, b) = self.segment_ends(i)?;
        let (a, b) = (&self.knots[a], &self.knots[b]);
        Some(CubicBez::new(a.anchor, a.out_ctrl, b.in_ctrl, b.anchor))
    }

    /// Whether a segment is a straight line: both its handles retracted.
    pub fn is_straight(&self, i: usize) -> bool {
        self.segment_ends(i).is_some_and(|(a, b)| self.knots[a].retracted(HandleSide::Out) && self.knots[b].retracted(HandleSide::In))
    }

    /// The subpath as a kurbo path.
    pub fn bez_path(&self) -> BezPath {
        let mut path = BezPath::new();
        let Some(first) = self.knots.first() else { return path };
        path.move_to(first.anchor);
        for i in 0..self.segment_count() {
            if let Some(c) = self.segment(i) {
                path.curve_to(c.p1, c.p2, c.p3);
            }
        }
        if self.closed {
            path.close_path();
        }
        path
    }
}

/// A path the overlays show: the host's key for it, its subpaths and its colour.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct EditPath {
    /// The host's key, stable across frames.
    pub key: u64,
    /// The path's subpaths.
    pub subpaths: Vec<EditSubpath>,
    /// The layer colour its outline, anchors and handles are drawn in (DesignCraft, VectorCraft);
    /// the overlay's line colour without one.
    pub color: Option<Srgba>,
}

impl EditPath {
    /// The path the host keys `key`, made of `subpaths`, in the overlay's colour.
    pub fn new(key: u64, subpaths: Vec<EditSubpath>) -> Self {
        Self { key, subpaths, color: None }
    }

    /// The path in its own colour (a layer's), else the overlay's.
    #[must_use]
    pub fn colored(mut self, color: Option<Srgba>) -> Self {
        self.color = color;
        self
    }

    /// The knot `at` refers to.
    pub fn knot(&self, at: KnotRef) -> Option<&Knot> {
        self.subpaths.get(at.subpath)?.knots.get(at.knot)
    }

    /// The number of anchors over all subpaths.
    pub fn anchor_count(&self) -> usize {
        self.subpaths.iter().map(|s| s.knots.len()).sum()
    }

    /// Every knot's reference, in path order.
    pub fn knot_refs(&self) -> impl Iterator<Item = KnotRef> + '_ {
        self.subpaths.iter().enumerate().flat_map(|(subpath, s)| (0..s.knots.len()).map(move |knot| KnotRef { subpath, knot }))
    }
}

/// A knot of a path, or the segment leaving it: its subpath and its index there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub struct KnotRef {
    /// Index of the subpath in [`EditPath::subpaths`].
    pub subpath: usize,
    /// Index of the knot in [`EditSubpath::knots`].
    pub knot: usize,
}

impl KnotRef {
    /// A reference to knot `knot` of subpath `subpath`.
    pub fn new(subpath: usize, knot: usize) -> Self {
        Self { subpath, knot }
    }
}

/// The host's path selection: the path the overlay edits, its selected anchors and a selected
/// segment (whose two handles show).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PathSelection {
    /// The host key of the path being edited, if any.
    pub path: Option<u64>,
    /// The selected anchors of that path.
    pub anchors: Vec<KnotRef>,
    /// The segment leaving this knot.
    pub segment: Option<KnotRef>,
}

impl PathSelection {
    /// The selection of `anchors` of the path keyed `path`, and of a segment.
    pub fn new(path: Option<u64>, anchors: Vec<KnotRef>, segment: Option<KnotRef>) -> Self {
        Self { path, anchors, segment }
    }

    /// Whether `knot` of `path` is a selected anchor.
    pub fn has_anchor(&self, path: u64, knot: KnotRef) -> bool {
        self.path == Some(path) && self.anchors.contains(&knot)
    }
}

/// Which of a knot's two direction handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HandleSide {
    /// The handle of the segment arriving at the anchor.
    In,
    /// The handle of the segment leaving the anchor.
    Out,
}

/// An edit of a path, the value of the overlays' edit events. A drag reports the whole move from
/// its press: the host applies each one to the path it had at the press, with its own path code
/// or with [`PathEdit::apply`] on an [`EditPath`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PathEdit {
    /// Move anchors with their handles by `by`, in document units.
    #[non_exhaustive]
    MoveAnchors {
        /// The path's host key.
        path: u64,
        /// The anchors moved.
        anchors: Vec<KnotRef>,
        /// The move, in document units.
        by: Vec2,
    },
    /// Move a direction handle to `to`. A smooth knot keeps its other handle collinear, unless
    /// `alone`, which also makes the knot a corner (the host's rule).
    #[non_exhaustive]
    MoveHandle {
        /// The path's host key.
        path: u64,
        /// The anchor the handle belongs to.
        knot: KnotRef,
        /// Which of the anchor's handles.
        side: HandleSide,
        /// The handle's new position.
        to: Point,
        /// Whether the other handle stays put (⌥ held).
        alone: bool,
    },
    /// Bend the segment leaving `segment` so its point at `t` moves by `by`, the nearer handle
    /// more (PhotoCraft's `bend_segment`).
    #[non_exhaustive]
    BendSegment {
        /// The path's host key.
        path: u64,
        /// The anchor the segment leaves.
        segment: KnotRef,
        /// Position along the segment, 0..=1.
        t: f64,
        /// How far the point at `t` moves, in document units.
        by: Vec2,
    },
    /// Convert Point: `None` makes a corner with retracted handles; `Some(out)` a smooth point
    /// whose out handle is at `out` and whose in handle mirrors it.
    #[non_exhaustive]
    Convert {
        /// The path's host key.
        path: u64,
        /// The converted anchor.
        knot: KnotRef,
        /// The out handle of the smooth point, or `None` for a corner.
        out: Option<Point>,
    },
    /// The Curvature tool's drag of anchors: move them with their handles by `by` (the whole move
    /// from the press), then reflow every subpath they are on ([`EditSubpath::reflow`]).
    #[non_exhaustive]
    MoveCurvature {
        /// The path's host key.
        path: u64,
        /// The anchors moved.
        anchors: Vec<KnotRef>,
        /// The move from the press, in document units.
        by: Vec2,
    },
    /// The Curvature tool's press on a segment: insert a smooth anchor into the segment leaving
    /// `segment` at `t` ([`EditSubpath::insert_at`], `t` in
    /// [`CURVATURE_T_RANGE`](crate::geometry::curvature::CURVATURE_T_RANGE)), move it by `by`
    /// (zero for a click), and reflow the subpath. Insert and drag are one undo step.
    #[non_exhaustive]
    InsertCurvature {
        /// The path's host key.
        path: u64,
        /// The anchor the segment leaves.
        segment: KnotRef,
        /// Position along the segment, 0..=1.
        t: f64,
        /// The move of the new anchor, in document units.
        by: Vec2,
    },
    /// The Curvature tool's double-click or Enter on an anchor: a smooth knot becomes a corner
    /// with retracted handles and a corner becomes smooth, then the subpath reflows.
    #[non_exhaustive]
    ToggleCurvature {
        /// The path's host key.
        path: u64,
        /// The toggled anchor.
        knot: KnotRef,
    },
}

impl PathEdit {
    /// Moves `anchors` of the path keyed `path` with their handles by `by`.
    pub fn move_anchors(path: u64, anchors: Vec<KnotRef>, by: Vec2) -> Self {
        PathEdit::MoveAnchors { path, anchors, by }
    }

    /// Moves the `side` handle of `knot` to `to`; `alone` keeps the other handle where it is.
    pub fn move_handle(path: u64, knot: KnotRef, side: HandleSide, to: Point, alone: bool) -> Self {
        PathEdit::MoveHandle { path, knot, side, to, alone }
    }

    /// Bends the segment leaving `segment` so its point at `t` moves by `by`.
    pub fn bend_segment(path: u64, segment: KnotRef, t: f64, by: Vec2) -> Self {
        PathEdit::BendSegment { path, segment, t, by }
    }

    /// Converts `knot` to a corner (`out` is `None`) or to a smooth point whose out handle is at `out`.
    pub fn convert(path: u64, knot: KnotRef, out: Option<Point>) -> Self {
        PathEdit::Convert { path, knot, out }
    }

    /// The Curvature tool's move of `anchors` by `by`, with the curve reflowed.
    pub fn move_curvature(path: u64, anchors: Vec<KnotRef>, by: Vec2) -> Self {
        PathEdit::MoveCurvature { path, anchors, by }
    }

    /// The Curvature tool's insert into the segment leaving `segment` at `t`, moved by `by`.
    pub fn insert_curvature(path: u64, segment: KnotRef, t: f64, by: Vec2) -> Self {
        PathEdit::InsertCurvature { path, segment, t, by }
    }

    /// The Curvature tool's toggle of `knot` between smooth and corner.
    pub fn toggle_curvature(path: u64, knot: KnotRef) -> Self {
        PathEdit::ToggleCurvature { path, knot }
    }

    /// The edit's path.
    pub fn path(&self) -> u64 {
        match *self {
            PathEdit::MoveAnchors { path, .. }
            | PathEdit::MoveHandle { path, .. }
            | PathEdit::BendSegment { path, .. }
            | PathEdit::Convert { path, .. }
            | PathEdit::MoveCurvature { path, .. }
            | PathEdit::InsertCurvature { path, .. }
            | PathEdit::ToggleCurvature { path, .. } => path,
        }
    }
}
