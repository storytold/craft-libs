//! The paths canvas overlays edit and draw: knots, subpaths, the host's selection and the edits
//! the overlays report. The host converts its own path type to these each frame and applies the
//! edits to its own.

use std::collections::HashSet;

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

/// How close a handle lies to its anchor and still counts as retracted, in document units: PhotoCraft's straightness
/// tolerance, so a segment whose handles sit a hair off their anchors is straight for every reader.
const RETRACTED: f64 = 1e-9;

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

    /// Whether the handle on `side` lies on the anchor, within 1e-9 document units.
    pub fn retracted(&self, side: HandleSide) -> bool {
        self.handle(side).distance(self.anchor) < RETRACTED
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

/// A path the overlays show: the host's key for it, its subpaths, its colour and what it allows.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EditPath {
    /// The host's key, stable across frames.
    pub key: u64,
    /// The path's subpaths.
    pub subpaths: Vec<EditSubpath>,
    /// The layer colour its outline, anchors and handles are drawn in (DesignCraft, VectorCraft);
    /// the overlay's line colour without one.
    pub color: Option<Srgba>,
    /// Whether the path takes copies of its subpaths: Direct Selection's ⌥ drag on a segment drags a copy of the
    /// subpath ([`PathEdit::DuplicateSubpath`]) instead of the subpath itself. The host turns it off for a path
    /// whose subpaths are not copied that way (PhotoCraft: the pen draft and Type Paths, where the drag moves the
    /// subpath).
    pub copies: bool,
}

impl Default for EditPath {
    fn default() -> Self {
        Self::new(0, Vec::new())
    }
}

impl EditPath {
    /// The path the host keys `key`, made of `subpaths`, in the overlay's colour, taking copies of its subpaths.
    pub fn new(key: u64, subpaths: Vec<EditSubpath>) -> Self {
        Self { key, subpaths, color: None, copies: true }
    }

    /// The path taking copies of its subpaths ([`copies`](Self::copies)) or not.
    #[must_use]
    pub fn copies(mut self, copies: bool) -> Self {
        self.copies = copies;
        self
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

/// The host's path selection: the path the overlay edits, its selected anchors, a selected
/// segment (whose two handles show) and the subpaths whose anchors show.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PathSelection {
    /// The host key of the path being edited, if any.
    pub path: Option<u64>,
    /// The selected anchors of that path.
    pub anchors: Vec<KnotRef>,
    /// The segment leaving this knot.
    pub segment: Option<KnotRef>,
    /// The subpaths of that path whose anchors and handles show; `None` shows every subpath's. A Delete Anchor
    /// click narrows it to the subpath it acts on (Photoshop's Delete Anchor Point, #12769); the outlines always show.
    pub shown: Option<Vec<usize>>,
    /// Anchors of other paths, by the host key of each path: they draw selected, with their handles under
    /// [`HandlesShown`](super::HandlesShown), and declare no part: the overlay edits only `path`'s.
    pub others: Vec<(u64, Vec<KnotRef>)>,
}

impl PathSelection {
    /// The selection of `anchors` of the path keyed `path`, and of a segment, with every subpath's anchors shown.
    pub fn new(path: Option<u64>, anchors: Vec<KnotRef>, segment: Option<KnotRef>) -> Self {
        Self { path, anchors, segment, shown: None, others: Vec::new() }
    }

    /// The selection with the anchors of only the `shown` subpaths drawn (`None`: every subpath's).
    #[must_use]
    pub fn shown(mut self, shown: Option<Vec<usize>>) -> Self {
        self.shown = shown;
        self
    }

    /// The selection with the anchors `anchors` of the other path keyed `path` drawn selected.
    #[must_use]
    pub fn also(mut self, path: u64, anchors: Vec<KnotRef>) -> Self {
        match self.others.iter_mut().find(|(key, _)| *key == path) {
            Some((_, held)) => held.extend(anchors),
            None => self.others.push((path, anchors)),
        }
        self
    }

    /// Whether `knot` of `path` draws selected: a selected anchor of the path being edited, or one listed in `others`.
    pub fn draws_selected(&self, path: u64, knot: KnotRef) -> bool {
        #[cfg(all(test, feature = "screen-testing"))]
        reads::add(self.others.iter().map(|(key, anchors)| if *key == path { 1 + anchors.len() } else { 1 }).sum());
        self.has_anchor(path, knot) || self.others.iter().any(|(key, anchors)| *key == path && anchors.contains(&knot))
    }

    /// The number of selected anchors over the edited path and every path in `others`.
    pub fn selected_len(&self) -> usize {
        self.anchors.len() + self.others.iter().map(|(_, anchors)| anchors.len()).sum::<usize>()
    }

    /// Whether `knot` of `path` is a selected anchor.
    pub fn has_anchor(&self, path: u64, knot: KnotRef) -> bool {
        #[cfg(all(test, feature = "screen-testing"))]
        reads::add(if self.path == Some(path) { self.anchors.len() } else { 0 });
        self.path == Some(path) && self.anchors.contains(&knot)
    }

    /// Whether the anchors of `subpath` of `path` show: always for a path other than the selection's.
    pub fn shows(&self, path: u64, subpath: usize) -> bool {
        self.path != Some(path) || self.shown.as_ref().is_none_or(|shown| shown.contains(&subpath))
    }

    /// The selection after the host deleted the anchor `deleted` of a Delete Anchor click, from `path` as the delete
    /// left it, which had `subpaths_before` subpaths: the anchor that followed the deleted one (the first of a
    /// closed subpath after its last), with only that subpath's anchors shown. When the delete removed a subpath,
    /// the indices moved: nothing is selected and every subpath shows.
    pub fn after_delete(path: &EditPath, deleted: KnotRef, subpaths_before: usize) -> PathSelection {
        // pc: crates/ui-egui/src/tools/anchor_tools.rs `select_follower` (#12769).
        let Some(sub) = path.subpaths.get(deleted.subpath).filter(|_| path.subpaths.len() == subpaths_before) else {
            return PathSelection::new(Some(path.key), Vec::new(), None);
        };
        let next = if deleted.knot < sub.knots.len() { Some(deleted.knot) } else { (sub.closed && !sub.knots.is_empty()).then_some(0) };
        let anchors = next.map(|knot| KnotRef::new(deleted.subpath, knot)).into_iter().collect();
        PathSelection::new(Some(path.key), anchors, None).shown(Some(vec![deleted.subpath]))
    }
}

/// The anchors a [`PathSelection`] selects, gathered into sets once so that the overlay asks about each knot it lays
/// out or draws without searching the selection's lists. A path that `others` lists more than once is treated as one
/// path holding every anchor listed for it.
#[derive(Default)]
pub(super) struct SelectedAnchors {
    path: Option<u64>,
    anchors: HashSet<KnotRef>,
    others: HashSet<(u64, KnotRef)>,
    listed: HashSet<u64>,
    others_len: usize,
}

impl SelectedAnchors {
    /// The anchors `sel` selects.
    pub(super) fn of(sel: &PathSelection) -> Self {
        let others_len = sel.others.iter().map(|(_, anchors)| anchors.len()).sum();
        #[cfg(all(test, feature = "screen-testing"))]
        reads::add(sel.anchors.len() + others_len);
        Self {
            path: sel.path,
            anchors: sel.anchors.iter().copied().collect(),
            others: sel.others.iter().flat_map(|(key, anchors)| anchors.iter().map(|knot| (*key, *knot))).collect(),
            listed: sel.others.iter().map(|(key, _)| *key).collect(),
            others_len,
        }
    }

    /// Whether `knot` of `path` is a selected anchor of the path being edited ([`PathSelection::has_anchor`]).
    pub(super) fn has_anchor(&self, path: u64, knot: KnotRef) -> bool {
        self.path == Some(path) && self.anchors.contains(&knot)
    }

    /// Whether `knot` of `path` draws selected ([`PathSelection::draws_selected`]).
    pub(super) fn draws_selected(&self, path: u64, knot: KnotRef) -> bool {
        self.has_anchor(path, knot) || self.others.contains(&(path, knot))
    }

    /// Whether `knot` is one of the anchors `others` lists for `path`.
    pub(super) fn listed_anchor(&self, path: u64, knot: KnotRef) -> bool {
        self.others.contains(&(path, knot))
    }

    /// Whether `others` lists `path`, with or without anchors.
    pub(super) fn lists(&self, path: u64) -> bool {
        self.listed.contains(&path)
    }

    /// The number of anchors `others` lists over every path.
    pub(super) fn others_len(&self) -> usize {
        self.others_len
    }
}

/// The number of entries of selection anchor lists read on this thread, for trials that count the overlay's work.
#[cfg(all(test, feature = "screen-testing"))]
pub(crate) mod reads {
    use std::cell::Cell;

    thread_local! {
        static READS: Cell<usize> = const { Cell::new(0) };
    }

    /// Counts `n` entries read.
    pub(crate) fn add(n: usize) {
        READS.with(|reads| reads.set(reads.get() + n));
    }

    /// The entries read since the last call, and starts the count again.
    pub(crate) fn take() -> usize {
        READS.with(|reads| reads.replace(0))
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
    /// Add an anchor to the segment leaving `segment` at `t`, then move it by `by`.
    #[non_exhaustive]
    InsertAnchor {
        /// The path's host key.
        path: u64,
        /// The anchor the segment leaves.
        segment: KnotRef,
        /// Position along the segment, 0..=1.
        t: f64,
        /// The move of the new anchor, in document units.
        by: Vec2,
    },
    /// Add Anchor's drag from a segment: add an anchor to the segment leaving `segment` at `t` and pull out its
    /// handles, as the Pen does: the out handle at `out` and the in handle opposite it through the anchor, a smooth
    /// knot; once Alt broke the pair, the in handle at `in_ctrl` and a corner knot. `by` moves the new anchor from
    /// the original segment point; both handles are absolute document positions, including that move.
    #[non_exhaustive]
    PullAnchor {
        /// The path's host key.
        path: u64,
        /// The anchor the segment leaves.
        segment: KnotRef,
        /// Position along the segment, 0..=1.
        t: f64,
        /// The move of the new anchor from the original segment point, in document units.
        by: Vec2,
        /// The new anchor's out handle, in document coordinates.
        out: Point,
        /// The new anchor's in handle once Alt broke the pair; `None` mirrors `out`.
        in_ctrl: Option<Point>,
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
    /// Direct Selection's ⌥ drag on a segment: a copy of subpath `subpath` goes last in the path and moves, with
    /// its handles, by `by`; the source stays. Photoshop names the step "Drag Path".
    #[non_exhaustive]
    DuplicateSubpath {
        /// The path's host key.
        path: u64,
        /// The copied subpath.
        subpath: usize,
        /// The copy's move, in document units.
        by: Vec2,
    },
}

/// A request to delete anchors of a path.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AnchorDelete {
    /// The path's host key.
    pub path: u64,
    /// The selected anchors.
    pub anchors: Vec<KnotRef>,
    /// Segments are selected too (the live-shape prompt applies).
    pub segments: bool,
}

impl AnchorDelete {
    /// A delete of `anchors` of the path keyed `path`; `segments` is whether segments are selected too.
    pub fn new(path: u64, anchors: Vec<KnotRef>, segments: bool) -> Self {
        AnchorDelete { path, anchors, segments }
    }
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

    /// The insert of an anchor into the segment leaving `segment` at `t`, moved by `by`.
    pub fn insert_anchor(path: u64, segment: KnotRef, t: f64, by: Vec2) -> Self {
        PathEdit::InsertAnchor { path, segment, t, by }
    }

    /// Add Anchor's insert into the segment leaving `segment` at `t` with the new anchor's handles pulled out to `out`
    /// and mirrored, or with the in handle at `in_ctrl`.
    pub fn pull_anchor(path: u64, segment: KnotRef, t: f64, out: Point, in_ctrl: Option<Point>) -> Self {
        Self::pull_anchor_moved(path, segment, t, Vec2::ZERO, out, in_ctrl)
    }

    /// Add Anchor's insert with its anchor moved by `by` from the original segment point.
    /// `out` and `in_ctrl` are absolute document positions; `None` mirrors `out` through the moved anchor.
    /// Hosts applying edits themselves must apply `by` to the inserted anchor before setting the handles.
    pub fn pull_anchor_moved(path: u64, segment: KnotRef, t: f64, by: Vec2, out: Point, in_ctrl: Option<Point>) -> Self {
        PathEdit::PullAnchor { path, segment, t, by, out, in_ctrl }
    }

    /// The Curvature tool's toggle of `knot` between smooth and corner.
    pub fn toggle_curvature(path: u64, knot: KnotRef) -> Self {
        PathEdit::ToggleCurvature { path, knot }
    }

    /// A copy of `subpath` of the path keyed `path`, appended and moved by `by`.
    pub fn duplicate_subpath(path: u64, subpath: usize, by: Vec2) -> Self {
        PathEdit::DuplicateSubpath { path, subpath, by }
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
            | PathEdit::InsertAnchor { path, .. }
            | PathEdit::PullAnchor { path, .. }
            | PathEdit::ToggleCurvature { path, .. }
            | PathEdit::DuplicateSubpath { path, .. } => path,
        }
    }
}
