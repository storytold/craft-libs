//! [`BezierOverlay`]: paths with their anchors and direction handles on a canvas, edited as PhotoCraft's Direct
//! Selection, Convert Point, Add and Delete Anchor Point and Curvature Pen tools edit them.

mod hit_test;
mod mark;
mod model;
mod ops;
#[cfg(feature = "screen-testing")]
mod stories;
mod style;
mod tools;
mod words;

use craft_behavior::geometry::{Directions, SelectionOperation, lock_direction};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2};
use craft_behavior::{
    Axis, CanvasView, EditCommand, Granularity, HitLevel, HitPriority, HitShape, Intent, NavMove, NavShape, PartKey, PartRef, PointerButton,
    PressPhase, PressStart, Roving, Sense,
};
use craft_keymap::Mods;
use craft_theme::Srgba;
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, Snap, canvas_snap, canvas_space, context_click};
use craft_ui::canvas_controls::{Marquee, MarqueeLook, path_runs};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{DataWidth, FocusKind, FocusRing, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{EditSet, KeyUse};
use craft_ui::style::DataColor;
use craft_ui::style::{Flags, StyleCx};
use egui::{Pos2, Rect, Response};
use kurbo::ParamCurve as _;

use hit_test::{Shown, nearest_on_screen, shown_handles};
pub(crate) use mark::paint_mark;
pub use mark::{AnchorsShown, HitMark, SegmentMark};
use mark::{Fill, paint_handle, segment_path, stroke};
pub use model::{EditPath, EditSubpath, HandleSide, Knot, KnotRef, PathEdit, PathSelection};
pub use style::BezierStyle;
pub use words::BezierWords;

use super::PathOptionsSettings;

/// Above this many anchors in the paths shown, the overlay draws outlines only and declares no part (no anchor in
/// the AccessKit tree either) (VectorCraft's `OVERLAY_MAX_ANCHORS`).
pub const MAX_ANCHORS: usize = 100_000;

/// What the pointer does to the paths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BezierTool {
    /// Direct Selection: select and move anchors, turn handles, bend segments, marquee.
    #[default]
    Direct,
    /// Convert Point (also the Pen with ⌥): a click on an anchor makes a corner, a drag from it pulls out smooth
    /// handles; a handle drag moves that handle alone.
    Convert,
    /// Anchors drawn, not edited (PhotoCraft's Path Selection and Pen tools).
    Anchors,
    /// Add Anchor Point: a click on a segment reports [`insert`](BezierOutput::insert) at the hit's `t`; ⌥ makes it
    /// Delete Anchor Point. Hover: the segment mark with the anchor a click adds ([`HitMark`]).
    AddAnchor,
    /// Delete Anchor Point: a click on an anchor reports [`delete`](BezierOutput::delete) of that anchor; ⌥ makes it
    /// Add Anchor Point. Hover: the anchor mark.
    DeleteAnchor,
    /// The Pen's Auto Add/Delete while no path is being drawn: Add on a segment, Delete on an anchor that is not an
    /// open subpath's end (the pen draft takes those), no ⌥ swap, no marquee. Presses on empty canvas are left to the
    /// pen draft.
    AutoAddDelete,
    /// The Curvature Pen on finished paths: an anchor drags with the curve reflowing ([`PathEdit::MoveCurvature`]);
    /// a segment gets a smooth anchor that drags the same way ([`PathEdit::InsertCurvature`]); a double-click on an
    /// anchor toggles smooth and corner ([`PathEdit::ToggleCurvature`]). Empty canvas is left to the pen draft.
    /// Arrows move the selected anchors, Enter toggles the picked one.
    Curvature,
}

/// Whose direction handles the overlay shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HandlesShown {
    /// Both of each selected anchor, the facing handles of the segments next to it, and the two of a selected
    /// segment (Illustrator's Direct Selection).
    #[default]
    Selected,
    /// Every anchor's (DesignCraft).
    All,
}

/// A part of a path under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum BezierHit {
    /// A shown direction handle's end.
    #[non_exhaustive]
    Handle {
        /// The path's id.
        path: u64,
        /// The anchor the handle belongs to.
        knot: KnotRef,
        /// Which of the anchor's two handles.
        side: HandleSide,
    },
    /// An anchor.
    #[non_exhaustive]
    Anchor {
        /// The path's id.
        path: u64,
        /// The anchor.
        knot: KnotRef,
    },
    /// The segment leaving `segment`, at its parameter `t`.
    #[non_exhaustive]
    Segment {
        /// The path's id.
        path: u64,
        /// The anchor the segment leaves.
        segment: KnotRef,
        /// Position along the segment, 0..=1.
        t: f64,
    },
}

impl BezierHit {
    /// The path the hit is on.
    pub fn path(&self) -> u64 {
        match *self {
            BezierHit::Handle { path, .. } | BezierHit::Anchor { path, .. } | BezierHit::Segment { path, .. } => path,
        }
    }
}

/// The part of an anchor: `ANCHOR.at(index)` with `index` from [`index`].
const ANCHOR: PartKey = PartKey::new("anchor");
/// The part of a direction handle's end into an anchor.
const IN_HANDLE: PartKey = PartKey::new("in_handle");
/// The part of a direction handle's end out of an anchor.
const OUT_HANDLE: PartKey = PartKey::new("out_handle");
/// The part of a segment.
const SEGMENT: PartKey = PartKey::new("segment");
/// The part under the whole canvas that a Direct Selection marquee drags from.
const MARQUEE: PartKey = PartKey::new("marquee");

/// Bits of a part index that hold the knot, then the subpath; the path's index takes the rest.
const KNOT_BITS: u32 = 20;
const SUBPATH_BITS: u32 = 20;

/// The index of the part of knot `knot` of the path at `path`.
const fn index(path: usize, knot: KnotRef) -> u64 {
    ((path as u64) << (KNOT_BITS + SUBPATH_BITS)) | ((knot.subpath as u64) << KNOT_BITS) | knot.knot as u64
}

/// What a press can do on a part.
const PRESS: Sense = Sense::DRAG.union(Sense::CLICK).union(Sense::CONTEXT_CLICK).union(Sense::TAKE_FOCUS);

/// The sub-priority of a handle end over an anchor, which wins over a segment.
const HANDLE_SUB: u8 = 2;
const ANCHOR_SUB: u8 = 1;

/// What a pass of the overlay reports besides the edit.
#[derive(Clone, Debug, Default)]
struct Report {
    change: Option<PathEdit>,
    select: Option<PathSelection>,
    insert: Option<(u64, KnotRef, f64)>,
    delete: Option<(u64, Vec<KnotRef>)>,
    context: Option<(BezierHit, Pos2)>,
    hover: Option<BezierHit>,
}

/// What a press drags, kept between passes.
#[derive(Clone, Debug, Default, PartialEq)]
enum Target {
    /// A press that moves nothing (⇧ on a selected anchor deselects it).
    #[default]
    Nothing,
    /// Anchors with their handles; `from` is the grabbed point.
    Anchors {
        path: u64,
        anchors: Vec<KnotRef>,
        from: Point,
    },
    Handle {
        path: u64,
        knot: KnotRef,
        side: HandleSide,
        from: Point,
    },
    /// A curved segment, grabbed at `t` where it passes `from`.
    Bend {
        path: u64,
        segment: KnotRef,
        t: f64,
        from: Point,
    },
    Convert {
        path: u64,
        knot: KnotRef,
    },
    /// The Curvature tool's drag of one anchor, grabbed at `from`.
    CurvatureMove {
        path: u64,
        knot: KnotRef,
        from: Point,
    },
    /// The Curvature tool's insert into `segment` at `t`, dragged from `from`, the point of the curve at `t`.
    CurvatureInsert {
        path: u64,
        segment: KnotRef,
        t: f64,
        from: Point,
    },
    /// The second click of a double-click on an anchor: a toggle on release, never a drag.
    Toggle {
        path: u64,
        knot: KnotRef,
    },
    /// A press on the empty canvas under Direct Selection: a marquee.
    Marquee,
}

/// A press the overlay owns: where it began, what it drags and the paths it began on.
#[derive(Clone, Debug, Default)]
struct Held {
    origin: Pos2,
    target: Target,
    start: Vec<EditPath>,
    /// Past the click slop: the gesture began.
    began: bool,
}

/// Where a press is in a pass: the view and snap it maps through, the press, the pointer and the modifiers held.
#[derive(Clone, Copy)]
struct Pointing<'a> {
    view: &'a CanvasView,
    snap: Option<&'a Snap>,
    start: PressStart,
    pos: Pos2,
    mods: Mods,
}

/// The kind of a part of the overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Anchor,
    Handle(HandleSide),
    Segment,
}

/// A part the overlay declares in a pass.
#[derive(Clone, Debug)]
struct Item {
    kind: Kind,
    /// The index in the paths.
    path: usize,
    key: u64,
    /// The anchor, or the anchor a handle belongs to, or the knot a segment leaves.
    knot: KnotRef,
    /// An anchor's or handle end's screen position.
    at: Pos2,
    shape: HitShape,
    /// An anchor that is not the first or last of an open subpath.
    interior: bool,
    /// An anchor's place among the path's anchors, from 1, and their number.
    ordinal: (usize, usize),
}

impl Item {
    fn part(&self) -> PartRef {
        let at = index(self.path, self.knot);
        match self.kind {
            Kind::Anchor => ANCHOR.at(at),
            Kind::Handle(HandleSide::In) => IN_HANDLE.at(at),
            Kind::Handle(HandleSide::Out) => OUT_HANDLE.at(at),
            Kind::Segment => SEGMENT.at(at),
        }
    }

    fn priority(&self) -> HitPriority {
        match self.kind {
            Kind::Handle(_) => HitPriority::new(HitLevel::Handle, HANDLE_SUB),
            Kind::Anchor => HitPriority::new(HitLevel::Handle, ANCHOR_SUB),
            Kind::Segment => HitPriority::new(HitLevel::Edge, 0),
        }
    }

    /// The hit this part stands for with the segment parameter 0.
    fn proxy(&self) -> BezierHit {
        match self.kind {
            Kind::Anchor => BezierHit::Anchor { path: self.key, knot: self.knot },
            Kind::Handle(side) => BezierHit::Handle { path: self.key, knot: self.knot, side },
            Kind::Segment => BezierHit::Segment { path: self.key, segment: self.knot, t: 0.0 },
        }
    }

    /// The hit a press or the pointer at the screen point `at` makes on this part.
    fn hit(&self, paths: &[EditPath], view: &CanvasView, at: Pos2) -> BezierHit {
        match (self.kind, self.proxy()) {
            (Kind::Segment, BezierHit::Segment { path, segment, .. }) => {
                let curve = paths.get(self.path).and_then(|p| p.subpaths.get(segment.subpath)).and_then(|s| s.segment(segment.knot));
                let t = curve.map_or(0.0, |curve| nearest_on_screen(view, curve, at).0);
                BezierHit::Segment { path, segment, t }
            }
            (_, hit) => hit,
        }
    }
}

/// The parts a pass declares.
struct Scene {
    items: Vec<Item>,
    /// Over [`MAX_ANCHORS`] or hidden: outlines only.
    limited: bool,
}

/// Paths with anchors and direction handles over a canvas, edited by the pointer and the keyboard as PhotoCraft's
/// Direct Selection and Convert Point tools edit them. The value is the paths: a drag proposes them with the gesture's
/// edit applied and Escape restores the start; [`BezierOutput::change`] carries the gesture's [`PathEdit`] from its
/// start, for a host with a path type of its own. Selection is the host's: it passes it and applies
/// [`BezierOutput::select`].
///
/// - Look: outlines in the path's colour, flattened on the screen; a selected segment in the segment width; anchors
///   hollow squares, selected ones filled; direction handles a line and a dot, for the anchors
///   [`handles`](Self::handles) names, never retracted ones. Above [`MAX_ANCHORS`] anchors, outlines only.
/// - Pointer: shown handles first, then anchors, then segments; the nearest of each kind, of equal distances the path
///   drawn last. A press on a part stays with the overlay; one elsewhere reaches the host's tool, except under Direct
///   Selection, which takes it for a marquee. [`BezierTool`] says what a press does. Past the click slop a press
///   drags; Escape cancels the drag.
/// - Keyboard: one Tab stop. Page Down and Page Up pick the next and previous part (anchors in path order, each
///   followed by its shown handles, in then out), Home and End the first and last; picking an anchor selects it alone.
///   ⇧ Page Down and ⇧ Page Up add the next and previous anchor; ⌘A selects every anchor of the path. The arrows move
///   the selected anchors or the picked handle (⌥ alone); Delete and Backspace report
///   [`delete`](BezierOutput::delete); Escape deselects.
/// - The anchor tools ([`BezierTool::AddAnchor`], [`BezierTool::DeleteAnchor`], [`BezierTool::AutoAddDelete`]) and
///   [`BezierTool::Curvature`] take only a press that hits what they edit, and draw the [`HitMark`] of what a press
///   would take under the pointer. [`segment_mark`](Self::segment_mark), [`anchors`](Self::anchors) and
///   [`path_options`](Self::path_options) change the look; [`hit_test`](Self::hit_test) answers the press priority
///   without a pass.
/// - The units and the zero point of the AccessKit values come from the canvas, and snapping of a dragged point is
///   the canvas's [`Snap`].
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct BezierOverlay<'a> {
    #[common]
    common: Common,
    paths: Bind<'a, Vec<EditPath>>,
    selection: &'a PathSelection,
    /// What the pointer does.
    #[prop]
    tool: BezierTool,
    /// Whose direction handles show.
    #[prop]
    handles: HandlesShown,
    /// Whether anchors and handles show.
    #[prop]
    anchors: AnchorsShown,
    /// How a clicked segment shows.
    #[prop]
    segment_mark: SegmentMark,
    /// The user's Path Options: a colour replaces the overlay line colour for paths without a colour of their own, a
    /// thickness replaces the line width (and the selected segment grows by the same amount).
    #[prop]
    path_options: Option<&'a PathOptionsSettings>,
    report: Report,
    marquee: Option<DocRect>,
    hover_mark: Option<BezierHit>,
}

impl<'a> BezierOverlay<'a> {
    /// An overlay named `label` over the host's `paths` (drawn in order, the last on top) and `selection`.
    pub fn new(label: impl Into<Label>, paths: Bind<'a, Vec<EditPath>>, selection: &'a PathSelection) -> Self {
        BezierOverlay {
            common: Common::new(),
            paths,
            selection,
            tool: BezierTool::default(),
            handles: HandlesShown::default(),
            anchors: AnchorsShown::default(),
            segment_mark: SegmentMark::default(),
            path_options: None,
            report: Report::default(),
            marquee: None,
            hover_mark: None,
        }
        .label(label)
    }

    /// The parts a pass declares for `paths`: the anchors and shown handles inside `rect` grown by `reach`, and the
    /// segments that tool takes and that cross it.
    fn scene(&self, paths: &[EditPath], view: &CanvasView, rect: Rect, reach: f32) -> Scene {
        let limited = self.anchors == AnchorsShown::Hidden || paths.iter().map(EditPath::anchor_count).sum::<usize>() > MAX_ANCHORS;
        let mut items = Vec::new();
        if limited {
            return Scene { items, limited };
        }
        let near = rect.expand(reach);
        let shown = shown_handles(paths, self.selection, self.handles, view);
        let mut handles = shown.iter().peekable();
        for (at, path) in paths.iter().enumerate() {
            let count = path.anchor_count();
            for (n, knot_ref) in path.knot_refs().enumerate() {
                let Some(knot) = path.knot(knot_ref) else { continue };
                let anchor = view.to_screen(knot.anchor);
                let sub = &path.subpaths[knot_ref.subpath];
                let interior = sub.closed || (knot_ref.knot != 0 && knot_ref.knot + 1 != sub.knots.len());
                if near.contains(anchor) {
                    items.push(Item {
                        kind: Kind::Anchor,
                        path: at,
                        key: path.key,
                        knot: knot_ref,
                        at: anchor,
                        shape: HitShape::circle(anchor, reach),
                        interior,
                        ordinal: (n + 1, count),
                    });
                }
                while let Some(&Shown { side, at: end, .. }) = handles.next_if(|s| s.path == at && s.knot == knot_ref) {
                    if near.contains(end) {
                        items.push(Item {
                            kind: Kind::Handle(side),
                            path: at,
                            key: path.key,
                            knot: knot_ref,
                            at: end,
                            shape: HitShape::circle(end, reach),
                            interior,
                            ordinal: (n + 1, count),
                        });
                    }
                }
            }
        }
        if self.tool.takes(&BezierHit::Segment { path: 0, segment: KnotRef::new(0, 0), t: 0.0 }, true) {
            for (at, path) in paths.iter().enumerate() {
                for (s, sub) in path.subpaths.iter().enumerate() {
                    for i in 0..sub.segment_count() {
                        let Some(curve) = sub.segment(i) else { continue };
                        let corners = [curve.p0, curve.p1, curve.p2, curve.p3].map(|p| view.to_screen(p));
                        if !Rect::from_points(&corners).intersects(near) {
                            continue;
                        }
                        let Some((points, _)) = path_runs(view, &segment_path(curve)).into_iter().next() else { continue };
                        // The pen draft owns the ends of an open subpath: a press within reach of one is not a segment's.
                        let ends = self.tool == BezierTool::AutoAddDelete && !sub.closed;
                        let start = if ends && i == 0 { reach } else { 0.0 };
                        let end = if ends && i + 1 == sub.segment_count() { reach } else { 0.0 };
                        let Some(points) = trim_polyline(points, start, end) else { continue };
                        items.push(Item {
                            kind: Kind::Segment,
                            path: at,
                            key: path.key,
                            knot: KnotRef::new(s, i),
                            at: corners[0],
                            shape: HitShape::polyline(points, false),
                            interior: true,
                            ordinal: (0, 0),
                        });
                    }
                }
            }
        }
        Scene { items, limited }
    }

    /// Whether the press on `item` is the tool's.
    fn takes(&self, item: &Item) -> bool {
        self.tool.takes(&item.proxy(), item.interior)
    }

    fn path(paths: &[EditPath], key: u64) -> Option<&EditPath> {
        paths.iter().find(|p| p.key == key)
    }

    fn subpath_anchors(paths: &[EditPath], key: u64, subpath: usize) -> Vec<KnotRef> {
        Self::path(paths, key)
            .and_then(|p| p.subpaths.get(subpath))
            .map_or_else(Vec::new, |s| (0..s.knots.len()).map(|k| KnotRef::new(subpath, k)).collect())
    }

    /// The target a press `start` on `hit` with `mods` drags, and the selection it makes.
    fn target(&mut self, hit: BezierHit, start: PressStart, mods: Mods, paths: &[EditPath]) -> Target {
        let knot_at = |path: u64, knot: KnotRef| Self::path(paths, path).and_then(|p| p.knot(knot));
        match (self.tool, hit) {
            (_, BezierHit::Handle { path, knot, side }) if self.tool != BezierTool::Curvature => {
                Target::Handle { path, knot, side, from: knot_at(path, knot).map_or(Point::ZERO, |k| k.handle(side)) }
            }
            (BezierTool::Curvature, BezierHit::Anchor { path, knot }) => {
                self.report.select = Some(PathSelection { path: Some(path), anchors: vec![knot], segment: None });
                if start.clicks == 2 {
                    Target::Toggle { path, knot }
                } else {
                    Target::CurvatureMove { path, knot, from: knot_at(path, knot).map_or(Point::ZERO, |k| k.anchor) }
                }
            }
            (BezierTool::Curvature, BezierHit::Segment { path, segment, t }) => {
                let t = tools::curvature_t(t);
                let from = Self::path(paths, path)
                    .and_then(|p| p.subpaths.get(segment.subpath))
                    .and_then(|s| s.segment(segment.knot))
                    .map_or(Point::ZERO, |c| c.eval(t));
                Target::CurvatureInsert { path, segment, t, from }
            }
            (BezierTool::Convert, BezierHit::Anchor { path, knot }) => Target::Convert { path, knot },
            (_, BezierHit::Anchor { path, knot }) => {
                let from = knot_at(path, knot).map_or(Point::ZERO, |k| k.anchor);
                let selected = self.selection.has_anchor(path, knot);
                let anchors = if mods.alt {
                    Self::subpath_anchors(paths, path, knot.subpath)
                } else if mods.shift && selected {
                    let rest: Vec<KnotRef> = self.selection.anchors.iter().copied().filter(|&k| k != knot).collect();
                    self.report.select = Some(PathSelection { path: Some(path), anchors: rest, segment: None });
                    return Target::Nothing;
                } else if mods.shift && self.selection.path == Some(path) {
                    self.selection.anchors.iter().copied().chain([knot]).collect()
                } else if selected {
                    self.selection.anchors.clone()
                } else {
                    vec![knot]
                };
                if !selected || mods.alt || mods.shift {
                    self.report.select = Some(PathSelection { path: Some(path), anchors: anchors.clone(), segment: None });
                }
                Target::Anchors { path, anchors, from }
            }
            (_, BezierHit::Segment { path, segment, t }) => {
                let sub = Self::path(paths, path).and_then(|p| p.subpaths.get(segment.subpath));
                let from = sub.and_then(|s| s.segment(segment.knot)).map_or(Point::ZERO, |c| c.eval(t));
                if mods.alt {
                    let anchors = Self::subpath_anchors(paths, path, segment.subpath);
                    self.report.select = Some(PathSelection { path: Some(path), anchors: anchors.clone(), segment: None });
                    Target::Anchors { path, anchors, from }
                } else {
                    self.report.select = Some(PathSelection { path: Some(path), anchors: Vec::new(), segment: Some(segment) });
                    match sub.and_then(|s| s.is_straight(segment.knot).then(|| s.segment_ends(segment.knot)).flatten()) {
                        Some((a, b)) => {
                            Target::Anchors { path, anchors: vec![KnotRef::new(segment.subpath, a), KnotRef::new(segment.subpath, b)], from }
                        }
                        None => Target::Bend { path, segment, t, from },
                    }
                }
            }
            (_, BezierHit::Handle { .. }) => Target::Nothing,
        }
    }

    /// The edit of a press at `origin` dragged to `pointer`: the whole move from the press.
    fn edit(view: &CanvasView, snap: Option<&Snap>, tool: BezierTool, target: &Target, origin: Pos2, pointer: Pos2, mods: Mods) -> Option<PathEdit> {
        let delta = view.to_doc(pointer) - view.to_doc(origin);
        let locked = |delta: Vec2| if mods.shift { constrain(Point::ZERO, Point::ZERO + delta).to_vec2() } else { delta };
        let snapped = |p: Point| match snap {
            Some(snap) => Point::new(snap.apply(Axis::Y, p.x, mods), snap.apply(Axis::X, p.y, mods)),
            None => p,
        };
        Some(match target {
            Target::Anchors { path, anchors, from } => {
                PathEdit::MoveAnchors { path: *path, anchors: anchors.clone(), by: snapped(*from + locked(delta)) - *from }
            }
            &Target::Handle { path, knot, side, from } => {
                PathEdit::MoveHandle { path, knot, side, to: snapped(from + delta), alone: tool == BezierTool::Convert || mods.alt }
            }
            &Target::Bend { path, segment, t, from } => PathEdit::BendSegment { path, segment, t, by: snapped(from + delta) - from },
            &Target::Convert { path, knot } => PathEdit::Convert { path, knot, out: Some(snapped(view.to_doc(pointer))) },
            &Target::CurvatureMove { path, knot, from } => {
                PathEdit::MoveCurvature { path, anchors: vec![knot], by: snapped(from + locked(delta)) - from }
            }
            &Target::CurvatureInsert { path, segment, t, from } => {
                PathEdit::InsertCurvature { path, segment, t, by: snapped(from + locked(delta)) - from }
            }
            Target::Nothing | Target::Toggle { .. } | Target::Marquee => return None,
        })
    }

    /// Proposes `paths` with `edit` applied to the paths the press began on.
    fn propose(&mut self, cx: &mut Cx<'_, Self>, start: &[EditPath], edit: PathEdit) {
        let mut next = start.to_vec();
        edit.apply_all(&mut next);
        cx.propose(next);
        self.report.change = Some(edit);
    }

    /// A one-shot `edit` of `paths`, when it changes them.
    fn set(&mut self, cx: &mut Cx<'_, Self>, paths: &[EditPath], edit: PathEdit) {
        let mut next = paths.to_vec();
        if edit.apply_all(&mut next) {
            cx.set(next);
            self.report.change = Some(edit);
        }
    }

    /// The selection a marquee over `rect` makes with `mods`: the anchors inside it on the frontmost path that has
    /// any, combined with the selection as ⇧, ⌥ and ⇧⌥ say (Photoshop's add, subtract and intersect).
    fn marquee_selection(&self, paths: &[EditPath], rect: DocRect, mods: Mods) -> Option<PathSelection> {
        let inside = paths.iter().rev().find_map(|path| {
            let knots: Vec<KnotRef> = path.knot_refs().filter(|&k| path.knot(k).is_some_and(|k| contains(rect, k.anchor))).collect();
            (!knots.is_empty()).then_some((path.key, knots))
        });
        let operation = SelectionOperation::from_mods(mods);
        let Some((path, knots)) = inside else {
            return (operation == SelectionOperation::New).then(PathSelection::default);
        };
        let current: Vec<KnotRef> = if self.selection.path == Some(path) { self.selection.anchors.clone() } else { Vec::new() };
        let anchors = match operation {
            SelectionOperation::Add => current.iter().copied().chain(knots.iter().copied().filter(|k| !current.contains(k))).collect(),
            SelectionOperation::Subtract => current.into_iter().filter(|k| !knots.contains(k)).collect(),
            SelectionOperation::Intersect => current.into_iter().filter(|k| knots.contains(k)).collect(),
            _ => knots,
        };
        Some(PathSelection { path: Some(path), anchors, segment: None })
    }

    /// The pointer on the parts: selection, drags, clicks, the hover mark and the context request.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, scene: &Scene, paths: &[EditPath]) {
        let snap = canvas_snap(&cx.env);
        let view = &space.view;
        let mut hovered: Option<(u8, BezierHit, Mods)> = None;
        for item in &scene.items {
            if !self.takes(item) {
                continue;
            }
            let Some(result) = cx.part_if_declared(item.part()) else { continue };
            let (press, hover) = (result.press, result.hover_pos);
            if let Some(at) = hover {
                let rank = match item.kind {
                    Kind::Handle(_) => 2,
                    Kind::Anchor => 1,
                    Kind::Segment => 0,
                };
                if hovered.is_none_or(|(best, ..)| rank > best) {
                    hovered = Some((rank, item.hit(paths, view, at), press.mods));
                }
            }
            if let Some(at) = context_click(&press) {
                self.report.context = Some((item.hit(paths, view, at), at));
            }
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    let hit = item.hit(paths, view, start.origin);
                    if self.tool.clicks() {
                        tools::anchor_click(self.tool, hit, press.mods, &mut self.report);
                        *cx.press_slot::<Held>() = None;
                    } else {
                        let target = self.target(hit, start, press.mods, paths);
                        *cx.press_slot::<Held>() = Some(Held { origin: start.origin, target, start: paths.to_vec(), began: false });
                    }
                }
                PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                    let first = matches!(press.phase, PressPhase::DragStart { .. });
                    self.drag(cx, Pointing { view, snap: snap.as_ref(), start, pos, mods: press.mods }, Some(item), paths, first);
                }
                PressPhase::Release { start, pos, click, .. } if start.button == PointerButton::Primary => {
                    self.release(cx, Pointing { view, snap: snap.as_ref(), start, pos, mods: press.mods }, Some(item), click, paths);
                }
                _ => {}
            }
        }
        if self.tool == BezierTool::Direct
            && let Some(result) = cx.part_if_declared(MARQUEE)
        {
            let press = result.press;
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    *cx.press_slot::<Held>() = Some(Held { origin: start.origin, target: Target::Marquee, start: Vec::new(), began: false });
                }
                PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                    let first = matches!(press.phase, PressPhase::DragStart { .. });
                    self.drag(cx, Pointing { view, snap: snap.as_ref(), start, pos, mods: press.mods }, None, paths, first);
                }
                PressPhase::Release { start, pos, click, .. } if start.button == PointerButton::Primary => {
                    self.release(cx, Pointing { view, snap: snap.as_ref(), start, pos, mods: press.mods }, None, click, paths);
                }
                _ => {}
            }
        }
        if let Some((_, hit, mods)) = hovered {
            self.report.hover = Some(hit);
            self.hover_mark = tools::hover_mark(self.tool, hit, mods);
        }
    }

    /// One pass of a press dragged to `pos`; `first` on the pass the drag begins.
    fn drag(&mut self, cx: &mut Cx<'_, Self>, at: Pointing<'_>, item: Option<&Item>, paths: &[EditPath], first: bool) {
        let Pointing { view, snap, start, pos, mods } = at;
        let kept = cx.press_slot::<Held>().take().filter(|held| held.origin == start.origin);
        let mut held = match (kept, item) {
            (Some(held), _) => held,
            (None, Some(item)) => {
                let target = self.target(item.hit(paths, view, start.origin), start, mods, paths);
                Held { origin: start.origin, target, start: paths.to_vec(), began: false }
            }
            (None, None) => Held { origin: start.origin, target: Target::Marquee, start: Vec::new(), began: false },
        };
        let moves = !matches!(held.target, Target::Nothing | Target::Toggle { .. });
        if first && moves {
            held.began = true;
            if let Target::CurvatureInsert { path, segment, .. } = held.target {
                self.report.select =
                    Some(PathSelection { path: Some(path), anchors: vec![KnotRef::new(segment.subpath, segment.knot + 1)], segment: None });
            }
            if held.target != Target::Marquee {
                cx.begin();
            }
        }
        if held.began {
            match held.target {
                Target::Marquee => self.marquee = Some(DocRect::from_points(view.to_doc(start.origin), view.to_doc(pos))),
                _ => {
                    if let Some(edit) = Self::edit(view, snap, self.tool, &held.target, start.origin, pos, mods) {
                        self.propose(cx, &held.start, edit);
                    }
                }
            }
        }
        *cx.press_slot::<Held>() = Some(held);
    }

    /// The release of a press at `pos`: a drag commits, a click does what its tool's click does.
    fn release(&mut self, cx: &mut Cx<'_, Self>, at: Pointing<'_>, item: Option<&Item>, click: bool, paths: &[EditPath]) {
        let Pointing { view, snap, start, pos, mods } = at;
        let Some(held) = cx.press_slot::<Held>().take().filter(|held| held.origin == start.origin) else {
            // A press and its drag in one pass: one edit from the press.
            if !click
                && !self.tool.clicks()
                && let Some(item) = item
            {
                let target = self.target(item.hit(paths, view, start.origin), start, mods, paths);
                if let Some(edit) = Self::edit(view, snap, self.tool, &target, start.origin, pos, mods) {
                    self.set(cx, paths, edit);
                }
            }
            return;
        };
        if held.target == Target::Marquee {
            if held.began {
                let rect = DocRect::from_points(view.to_doc(start.origin), view.to_doc(pos));
                self.report.select = self.marquee_selection(paths, rect, mods);
            } else if *self.selection != PathSelection::default() {
                self.report.select = Some(PathSelection::default());
            }
            return;
        }
        if held.began {
            if let Some(edit) = Self::edit(view, snap, self.tool, &held.target, start.origin, pos, mods) {
                self.propose(cx, &held.start, edit);
            }
            cx.commit();
            return;
        }
        match held.target {
            Target::Convert { path, knot } => self.set(cx, &held.start, PathEdit::Convert { path, knot, out: None }),
            Target::Toggle { path, knot } => self.set(cx, &held.start, PathEdit::ToggleCurvature { path, knot }),
            Target::CurvatureInsert { path, segment, t, .. } => {
                self.set(cx, &held.start, PathEdit::InsertCurvature { path, segment, t, by: Vec2::ZERO });
                self.report.select =
                    Some(PathSelection { path: Some(path), anchors: vec![KnotRef::new(segment.subpath, segment.knot + 1)], segment: None });
            }
            _ => {}
        }
    }

    /// The keys of the focused overlay: picking parts, select all, nudges, Delete, Enter and Escape.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, scene: &Scene, paths: &[EditPath]) {
        let keyboard: Vec<&Item> = scene.items.iter().filter(|item| item.kind != Kind::Segment).collect();
        let active = cx.active_part();
        let current: Option<&Item> = active.and_then(|part| keyboard.iter().copied().find(|item| item.part() == part));
        let keyed = self.tool.keyed();
        // Focus arriving at an anchor (Tab) selects it alone, as picking it does. A press holds the pointer's own
        // selection (Shift on a selected anchor takes it out, and the focus the press took must not put it back),
        // so an arrival with the anchor pressed selects nothing.
        if keyed
            && self.report.select.is_none()
            && let Some(arrived) = cx.focus_arrived_by_key()
            && let Some(item) = keyboard.iter().copied().find(|item| item.part() == arrived)
            && item.kind == Kind::Anchor
            && !self.selection.has_anchor(item.key, item.knot)
        {
            self.report.select = Some(PathSelection { path: Some(item.key), anchors: vec![item.knot], segment: None });
        }
        while let Some((mv, mods)) = cx.intents().take_move() {
            let here = current.and_then(|c| keyboard.iter().position(|item| item.part() == c.part()));
            let picked: Option<&Item> = match mv {
                NavMove::PageDown | NavMove::PageUp if mods.extend => {
                    let anchors: Vec<&Item> = keyboard.iter().copied().filter(|item| item.kind == Kind::Anchor).collect();
                    let at = current.and_then(|c| anchors.iter().position(|a| a.path == c.path && a.knot == c.knot));
                    step(anchors.len(), at, mv == NavMove::PageDown).and_then(|i| anchors.get(i).copied())
                }
                NavMove::PageDown | NavMove::PageUp => step(keyboard.len(), here, mv == NavMove::PageDown).and_then(|i| keyboard.get(i).copied()),
                NavMove::Start => keyboard.first().copied(),
                NavMove::End => keyboard.last().copied(),
                _ => None,
            };
            let Some(item) = picked else { continue };
            cx.focus_part(item.part());
            if item.kind == Kind::Anchor {
                let mut anchors = if mods.extend && self.selection.path == Some(item.key) { self.selection.anchors.clone() } else { Vec::new() };
                if !anchors.contains(&item.knot) {
                    anchors.push(item.knot);
                }
                self.report.select = Some(PathSelection { path: Some(item.key), anchors, segment: None });
            }
        }
        while let Some(command) = cx.intents().take_edit() {
            if command == EditCommand::SelectAll
                && keyed
                && let Some(path) = self.focused_path(paths, current)
            {
                self.report.select = Some(PathSelection { path: Some(path.key), anchors: path.knot_refs().collect(), segment: None });
            }
        }
        if keyed && let Some((direction, mods)) = cx.intents().take_nudge() {
            let distance = if mods.shift { NUDGE_FAR } else { NUDGE };
            let by = nudge_vector(&space.view, direction, distance);
            match current {
                Some(Item { kind: Kind::Handle(side), key, knot, .. }) => {
                    if self.tool != BezierTool::Curvature
                        && let Some(k) = Self::path(paths, *key).and_then(|p| p.knot(*knot))
                    {
                        let edit = PathEdit::MoveHandle { path: *key, knot: *knot, side: *side, to: k.handle(*side) + by, alone: mods.alt };
                        self.set(cx, paths, edit);
                    }
                }
                _ => {
                    if let Some(path) = self.selection.path
                        && !self.selection.anchors.is_empty()
                    {
                        let anchors = self.selection.anchors.clone();
                        let edit = if self.tool == BezierTool::Curvature {
                            PathEdit::MoveCurvature { path, anchors, by }
                        } else {
                            PathEdit::MoveAnchors { path, anchors, by }
                        };
                        self.set(cx, paths, edit);
                    }
                }
            }
        }
        if self.tool == BezierTool::Curvature
            && cx.intents().take(Intent::Submit)
            && let Some(Item { kind: Kind::Anchor, key, knot, .. }) = current
        {
            self.set(cx, paths, PathEdit::ToggleCurvature { path: *key, knot: *knot });
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        if (forward || backward)
            && let Some(path) = self.selection.path
            && !self.selection.anchors.is_empty()
        {
            self.report.delete = Some((path, self.selection.anchors.clone()));
        }
        if keyed && cx.gesture().is_none() && cx.intents().take(Intent::Cancel) && *self.selection != PathSelection::default() {
            self.report.select = Some(PathSelection::default());
        }
    }

    /// The path ⌘A selects: the selection's, else the picked part's, else the only one.
    fn focused_path<'p>(&self, paths: &'p [EditPath], part: Option<&Item>) -> Option<&'p EditPath> {
        self.selection
            .path
            .and_then(|key| Self::path(paths, key))
            .or_else(|| part.and_then(|item| paths.get(item.path)))
            .or_else(|| if paths.len() == 1 { paths.first() } else { None })
    }
}

/// `points` without the first `start` and the last `end` screen points of its length; `None` when nothing is left.
fn trim_polyline(points: Vec<Pos2>, start: f32, end: f32) -> Option<Vec<Pos2>> {
    if start <= 0.0 && end <= 0.0 {
        return Some(points);
    }
    let total: f32 = points.windows(2).map(|pair| (pair[1] - pair[0]).length()).sum();
    let (low, high) = (start, total - end);
    let mut kept: Vec<Pos2> = Vec::new();
    let mut before = 0.0;
    for pair in points.windows(2) {
        let length = (pair[1] - pair[0]).length();
        let after = before + length;
        if length > 0.0 && after > low && before < high {
            let from = ((low - before) / length).clamp(0.0, 1.0);
            let to = ((high - before) / length).clamp(0.0, 1.0);
            if kept.is_empty() {
                kept.push(pair[0].lerp(pair[1], from));
            }
            kept.push(pair[0].lerp(pair[1], to));
        }
        before = after;
    }
    (kept.len() >= 2).then_some(kept)
}

/// The index `forward` or back of `at` among `len` parts, stopping at the ends.
fn step(len: usize, at: Option<usize>, forward: bool) -> Option<usize> {
    match (at, forward) {
        (Some(i), true) => Some((i + 1).min(len.saturating_sub(1))),
        (Some(i), false) => Some(i.saturating_sub(1)),
        (None, true) => (len > 0).then_some(0),
        (None, false) => len.checked_sub(1),
    }
}

/// The document vector of an arrow nudge: `distance` document units along the screen direction `direction`.
fn nudge_vector(view: &CanvasView, direction: egui::Vec2, distance: f64) -> Vec2 {
    let doc = view.unmap_vec(direction);
    let length = doc.hypot();
    if length > 0.0 { doc / length * distance } else { Vec2::ZERO }
}

/// `to` turned onto the nearest 45° ray from `from` (PhotoCraft's `Edit::constrain`): its projection there.
pub(crate) fn constrain(from: Point, to: Point) -> Point {
    from + lock_direction(to - from, None, Directions::Eight)
}

fn contains(r: DocRect, p: Point) -> bool {
    (r.x0..=r.x1).contains(&p.x) && (r.y0..=r.y1).contains(&p.y)
}

/// What the overlay reports: the edit of the paths and what the overlay asks the host to do.
#[derive(Debug)]
#[must_use = "apply the selection and the anchor edits to the host's paths"]
#[non_exhaustive]
pub struct BezierOutput {
    /// The edit of the paths: a drag is one `Begin` to `Commit`, each value the paths with the move from the press
    /// applied; a key is a `Set`.
    pub edit: EditResponse<Vec<EditPath>>,
    /// The edit the gesture makes, from its start, for a host that applies it to a path type of its own.
    pub change: Option<PathEdit>,
    /// The selection the overlay asks for.
    pub select: Option<PathSelection>,
    /// Insert: add an anchor to the segment leaving a knot of a path, at `t` (PhotoCraft's Add Anchor Point).
    pub insert: Option<(u64, KnotRef, f64)>,
    /// Delete or Backspace: remove these anchors of a path (Delete Anchor Point).
    pub delete: Option<(u64, Vec<KnotRef>)>,
    /// A secondary click on a part of a path, with its screen position, for the host's path menu.
    pub context: Option<(BezierHit, Pos2)>,
    /// The part under the pointer.
    pub hover: Option<BezierHit>,
}

impl Outcome for BezierOutput {
    type Value = Vec<EditPath>;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl Component for BezierOverlay<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse =
        KeyUse::CANVAS.with_moves().with_deletes().with_submits().with_chrome_key(false).with_edits(EditSet::NONE.with(EditCommand::SelectAll));
    type Style = BezierStyle;
    type Output = BezierOutput;

    fn style(cx: &StyleCx<'_>, _: BezierOverlayStyleKey) -> BezierStyle {
        BezierStyle::of(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, Vec<EditPath>>> {
        Some(self.paths.reborrow())
    }

    /// The anchors and shown handles in view, then the segments the tool takes, and under Direct Selection one part
    /// below them all for the marquee. The anchors and handles are one roving group in keyboard order: one Tab stop,
    /// arriving at the first selected anchor. The Page keys, Home and End are the overlay's own moves.
    fn parts(&mut self, rect: egui::Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let reach = cx.style.reach;
        let Some(scene) = cx.value().map(|paths| self.scene(paths, &space.view, rect, reach)) else { return };
        let focusable = self.tool != BezierTool::Anchors;
        if self.tool == BezierTool::Direct && !scene.limited {
            cx.part(MARQUEE, HitShape::Rect(rect))
                .sense(Sense::DRAG.union(Sense::CLICK).union(Sense::TAKE_FOCUS))
                .priority(HitPriority::new(HitLevel::Background, 0))
                .pointer_only("⌘A selects every anchor and the arrows move the selected anchors");
        }
        let arrival = self.selection.path.zip(self.selection.anchors.first().copied());
        for item in &scene.items {
            let press = self.takes(item);
            let keyboard = item.kind != Kind::Segment && focusable;
            let selected = item.kind == Kind::Anchor && self.selection.has_anchor(item.key, item.knot);
            let part = cx.part(item.part(), item.shape.clone()).priority(item.priority());
            let part = if press { part.sense(PRESS) } else { part };
            let part = if keyboard {
                part.focusable()
            } else if press {
                part.pointer_only("Page Down and Page Up pick the anchors, the arrows move them and Delete removes them")
            } else {
                part
            };
            let part = if selected { part.flags(Flags::SELECTED) } else { part };
            if item.kind == Kind::Anchor && arrival == Some((item.key, item.knot)) {
                part.selected();
            }
        }
        if focusable && scene.items.iter().any(|item| item.kind != Kind::Segment) {
            cx.roving(Roving::new(NavShape::Flat).home_end(false));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let Some(paths) = cx.value().cloned() else { return };
        let scene = self.scene(&paths, &space.view, cx.rect, cx.style.reach);
        self.run_pointer(cx, &space, &scene, &paths);
        self.run_keys(cx, &space, &scene, &paths);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(paths)) = (canvas_space(&cx.env), cx.value()) else { return };
        let (view, style) = (space.view, &cx.style);
        let scene = self.scene(paths, &view, cx.rect, style.reach);
        let options = self.path_options;
        let thickness = options.and_then(|options| options.thickness);
        let option_color =
            options.and_then(|options| options.color).map(|[r, g, b]| Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0));
        let painter = ui.content_painter(cx);
        let draw = ui.draw();
        // The path's thickness is a document width; without one the lines take the theme's widths. A path with a
        // thickness and no colour of its own keeps that width in the theme's line colour.
        let line = |points: Vec<Pos2>, closed: bool, segment: bool, color: Option<Srgba>| {
            let shape = if closed { PaintShape::Polygon(points) } else { PaintShape::Path(points) };
            let named = if segment { StrokeWidth::Thick } else { StrokeWidth::Medium };
            match (color, thickness) {
                (color, Some(thickness)) => {
                    let ink = color.map_or(DataColor::new(style.line.egui()), |color| DataColor::new(color.into()));
                    draw.stroke(&shape, DataWidth::points(thickness), ink);
                    if segment {
                        draw.stroke(&shape, named, ink);
                    }
                }
                (Some(color), None) => draw.stroke(&shape, named, DataColor::new(color.into())),
                (None, None) => draw.stroke(&shape, named, style.line),
            }
        };
        for path in paths {
            let color = path.color.or(option_color);
            for sub in &path.subpaths {
                for (points, closed) in path_runs(&view, &sub.bez_path()) {
                    line(points, closed, false, color);
                }
            }
            if self.segment_mark == SegmentMark::Line
                && let Some(s) = self.selection.segment.filter(|_| self.selection.path == Some(path.key))
                && let Some(curve) = path.subpaths.get(s.subpath).and_then(|sub| sub.segment(s.knot))
            {
                for (points, closed) in path_runs(&view, &segment_path(curve)) {
                    line(points, closed, true, color);
                }
            }
        }
        if scene.limited {
            return;
        }
        for item in &scene.items {
            let path = &paths[item.path];
            match item.kind {
                Kind::Handle(_) => {
                    if let Some(knot) = path.knot(item.knot) {
                        let shape = PaintShape::Path(vec![view.to_screen(knot.anchor), item.at]);
                        stroke(&draw, &shape, StrokeWidth::Thin, path.color, style.line);
                    }
                    let fill = Fill::Style(if cx.part_if_declared(item.part()).is_some() {
                        cx.resolve_part(item.part(), &style.control.fill)
                    } else {
                        cx.resolve(&style.control.fill)
                    });
                    paint_handle(&draw, &style.control, true, item.at, fill, path.color);
                }
                Kind::Anchor => {
                    let fill = if self.selection.has_anchor(path.key, item.knot) {
                        Fill::Path(style.line)
                    } else {
                        Fill::Style(if cx.part_if_declared(item.part()).is_some() {
                            cx.resolve_part(item.part(), &style.anchor.fill)
                        } else {
                            cx.resolve(&style.anchor.fill)
                        })
                    };
                    paint_handle(&draw, &style.anchor, false, item.at, fill, path.color);
                }
                Kind::Segment => {}
            }
        }
        if let Some(hit) = self.hover_mark
            && let Some(path) = Self::path(paths, hit.path())
        {
            paint_mark(&draw, &view, HitMark::new(path, hit), style);
        }
        if cx.focus_visible()
            && let Some(part) = cx.active_part()
            && let Some(item) = scene.items.iter().find(|item| item.part() == part)
        {
            let side = if item.kind == Kind::Anchor { style.anchor.side } else { style.control.side };
            let ring = mark::square(item.at, side);
            painter.painter.extend(FocusRing::shapes(&style.focus, FocusKind::Halo, &ring, painter.pixels_per_point()));
        }
        if let Some(rect) = self.marquee {
            let name = cx.words::<BezierWords>().marquee.as_str().to_owned();
            ui.child("marquee", cx.rect, Marquee::rect(Label::from(name), rect).look(MarqueeLook::Ants));
        }
    }

    /// A graphics object per anchor and shown handle, named "Anchor 3 of 12", "In handle" and "Out handle" and valued
    /// in the canvas's units. None above [`MAX_ANCHORS`].
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(paths)) = (canvas_space(&cx.env), cx.value()) else { return };
        let scene = self.scene(paths, &space.view, cx.rect, cx.style.reach);
        let (words, units, rulers) = (cx.words::<BezierWords>(), cx.env.units(), space.rulers());
        for item in scene.items.iter().filter(|item| item.kind != Kind::Segment) {
            let Some(knot) = paths[item.path].knot(item.knot) else { continue };
            let (point, name) = match item.kind {
                Kind::Handle(HandleSide::In) => (knot.in_ctrl, words.in_handle.as_str().to_owned()),
                Kind::Handle(HandleSide::Out) => (knot.out_ctrl, words.out_handle.as_str().to_owned()),
                _ => (knot.anchor, words.anchor.fill(&[("i", &item.ordinal.0.to_string()), ("n", &item.ordinal.1.to_string())])),
            };
            let value =
                format!("{}, {}", rulers.x.format_value(&units, point.x, rulers.zero.x), rulers.y.format_value(&units, point.y, rulers.zero.y));
            let node = access.node(item.part(), Role::GRAPHICS_OBJECT);
            node.name = Name::Text(name);
            node.value(value);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> BezierOutput {
        let Report { change, select, insert, delete, context, hover } = self.report;
        BezierOutput { edit: cx.edit_response(), change, select, insert, delete, context, hover }
    }
}
