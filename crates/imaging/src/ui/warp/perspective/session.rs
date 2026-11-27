//! [`PerspectiveSession`]: the planes of a Perspective Warp while it is open, and the plane
//! geometry its screens need (linking, snapping, straightening). The warp map itself
//! (PhotoCraft's piecewise homography) stays the app's (LC20, LC21).

use std::sync::Arc;

use kurbo::{Point, Rect};

use craft_behavior::edit_history::Step;
use craft_behavior::geometry::quad::Homography;
use craft_behavior::{EditEvent, EditHistory, Mixed};

/// Corners closer than this in the source (each axis) are one linked corner (PhotoCraft
/// `LINK_EPS`).
pub const LINK_DISTANCE: f64 = 0.5;

/// The smallest plane Layout adds, in document units each way (LC22).
pub const PLANE_MINIMUM: f64 = 4.0;

/// One plane: its corners in Layout (`source`) and in Warp (`target`), clockwise from the top
/// left, in document units (PhotoCraft `Plane { src, dst }`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Plane {
    /// The corners in Layout, clockwise from the top left.
    pub source: [Point; 4],
    /// The corners in Warp, clockwise from the top left.
    pub target: [Point; 4],
}

impl Plane {
    /// A plane with corners `source` in Layout and `target` in Warp.
    pub fn new(source: [Point; 4], target: [Point; 4]) -> Self {
        Plane { source, target }
    }

    /// A plane whose target is its source.
    pub fn identity(corners: [Point; 4]) -> Self {
        Plane { source: corners, target: corners }
    }

    /// The point at plane coordinates `(u, v)` (each 0 to 1 across), through the source corners
    /// or, with `target`, the target corners; the first corner for a degenerate quad.
    pub fn point(&self, u: f64, v: f64, target: bool) -> Point {
        // pc: crates/algo/src/perspective.rs:255 (`plane_point`).
        let corners = if target { &self.target } else { &self.source };
        Homography::unit_to_quad(*corners).map_or(corners[0], |h| h.apply(Point::new(u, v)))
    }
}

/// A plane's corner: the plane's index and the corner's (0 to 3, clockwise from the top left).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PlaneCorner {
    /// The plane's index.
    pub plane: usize,
    /// The corner's index, 0 to 3.
    pub corner: usize,
}

impl PlaneCorner {
    /// Corner `corner` of plane `plane`.
    pub fn new(plane: usize, corner: usize) -> Self {
        PlaneCorner { plane, corner }
    }
}

/// Perspective Warp's two modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlaneMode {
    /// Draw planes and fit their corners to the picture's lines; the picture does not move.
    #[default]
    Layout,
    /// Drag the target corners; the picture follows.
    Warp,
}

/// Warp mode's straightening buttons (Photoshop's Straighten, Level, Both).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Straighten {
    /// Straighten: near-vertical edges become vertical.
    Vertical,
    /// Level: near-horizontal edges become horizontal.
    Horizontal,
    /// Both.
    Both,
}

/// The groups of linked corners: corners within [`LINK_DISTANCE`] of each other in the source,
/// in plane and corner order.
pub fn linked_corners(planes: &[Plane]) -> Vec<Vec<PlaneCorner>> {
    // pc: crates/algo/src/perspective.rs:47 (`linked_corners`).
    let mut groups: Vec<Vec<PlaneCorner>> = Vec::new();
    for (plane, p) in planes.iter().enumerate() {
        for (corner, c) in p.source.iter().enumerate() {
            let group = groups.iter_mut().find(|g| {
                let first = g[0];
                let o = planes[first.plane].source[first.corner];
                (o.x - c.x).abs() <= LINK_DISTANCE && (o.y - c.y).abs() <= LINK_DISTANCE
            });
            match group {
                Some(g) => g.push(PlaneCorner { plane, corner }),
                None => groups.push(vec![PlaneCorner { plane, corner }]),
            }
        }
    }
    groups
}

/// Moves each group of linked corners to one source and one target position, their means.
pub fn unify(planes: &mut [Plane]) {
    // pc: crates/algo/src/perspective.rs:66 (`unify`).
    for group in linked_corners(planes) {
        if group.len() < 2 {
            continue;
        }
        let n = group.len() as f64;
        let mean = |of: fn(&Plane) -> &[Point; 4], planes: &[Plane]| {
            let sum = group.iter().fold(Point::ZERO, |a, c| a + of(&planes[c.plane])[c.corner].to_vec2());
            Point::new(sum.x / n, sum.y / n)
        };
        let source = mean(|p| &p.source, planes);
        let target = mean(|p| &p.target, planes);
        for c in &group {
            planes[c.plane].source[c.corner] = source;
            planes[c.plane].target[c.corner] = target;
        }
    }
}

/// Makes target edges within 45° of horizontal exactly horizontal and/or those within 45° of
/// vertical exactly vertical (each edge's two corner groups meet at their mean), moving linked
/// corners together. Unifies first.
pub fn straighten(planes: &mut [Plane], how: Straighten) {
    // pc: crates/algo/src/perspective.rs:127 (`straighten`).
    unify(planes);
    let groups = linked_corners(planes);
    let group_of = |plane: usize, corner: usize| groups.iter().position(|g| g.contains(&PlaneCorner { plane, corner })).unwrap_or(0);
    let mut position: Vec<Point> = groups.iter().map(|g| planes[g[0].plane].target[g[0].corner]).collect();
    let edges: Vec<(usize, usize)> =
        (0..planes.len()).flat_map(|p| (0..4).map(move |k| (p, k))).map(|(p, k)| (group_of(p, k), group_of(p, (k + 1) % 4))).collect();
    let level = matches!(how, Straighten::Horizontal | Straighten::Both);
    let upright = matches!(how, Straighten::Vertical | Straighten::Both);
    for (a, b) in edges {
        let (pa, pb) = (position[a], position[b]);
        let (dx, dy) = ((pb.x - pa.x).abs(), (pb.y - pa.y).abs());
        if dy <= dx && level {
            let y = (pa.y + pb.y) / 2.0;
            position[a].y = y;
            position[b].y = y;
        } else if dx < dy && upright {
            let x = (pa.x + pb.x) / 2.0;
            position[a].x = x;
            position[b].x = x;
        }
    }
    for (group, point) in groups.iter().zip(position) {
        for c in group {
            planes[c.plane].target[c.corner] = point;
        }
    }
}

/// An edit of the perspective session, from [`PerspectiveWarp`](super::PerspectiveWarp) or
/// [`PerspectiveBar`](super::PerspectiveBar).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PerspectiveEdit {
    /// The planes as a whole: a corner drag (Begin, Update, Commit, Cancel) or a one-shot change
    /// (Set): a new plane, a removed plane, Remove Planes, an arrow nudge.
    Planes(EditEvent<Vec<Plane>>),
    /// A mode tab. Entering Warp [unifies](unify) linked corners; Warp needs at least one plane.
    Mode(PlaneMode),
    /// A straightening button (Warp mode).
    Straighten(Straighten),
}

impl PerspectiveEdit {
    /// A one-step change of the planes from `before` to `after`, named `label` in the undo list: what the commands
    /// and the bar report.
    pub fn replace(before: &[Plane], after: Vec<Plane>, label: &str) -> PerspectiveEdit {
        PerspectiveEdit::Planes(EditEvent::set(Mixed::Same(before.to_vec()), after, Arc::from(label), None))
    }
}

/// What one undo step holds.
#[derive(Clone, Debug, PartialEq)]
struct PerspectiveDraft {
    planes: Vec<Plane>,
    mode: PlaneMode,
}

/// The planes of an open Perspective Warp, its mode and its undo steps.
///
/// - Changes only through [`apply`](Self::apply) and [`replace`](Self::replace). Replacing the
///   planes from outside (the app's control channel) bumps the [generation](Self::generation), so
///   the overlay ends a corner drag that held the old planes' indices (PhotoCraft bug #952).
/// - One undo step per drag, one-shot change, mode switch or straightening, an arrow nudge included
///   (Photoshop records each).
#[derive(Clone, Debug)]
pub struct PerspectiveSession {
    /// The layer's bounds: Insert adds a plane over their middle half.
    bounds: Rect,
    draft: PerspectiveDraft,
    history: EditHistory<PerspectiveDraft>,
    before: Option<PerspectiveDraft>,
    generation: u64,
}

impl PerspectiveSession {
    /// A session in Layout mode with no planes over the layer's `bounds`.
    pub fn new(bounds: Rect) -> Self {
        let draft = PerspectiveDraft { planes: Vec::new(), mode: PlaneMode::Layout };
        Self { bounds, history: EditHistory::new(&draft), draft, before: None, generation: 0 }
    }

    /// The layer's bounds.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// The planes in their order.
    pub fn planes(&self) -> &[Plane] {
        &self.draft.planes
    }

    /// The current mode.
    pub fn mode(&self) -> PlaneMode {
        self.draft.mode
    }

    /// Changes whenever [`replace`](Self::replace) sets the planes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether a corner drag is running.
    pub fn dragging(&self) -> bool {
        self.before.is_some()
    }

    /// A plane covering the middle half of the bounds: what Insert adds in Layout mode.
    pub fn inserted_plane(&self) -> Plane {
        // new: the keyboard path (Insert) of the transform spec's key table.
        let b = self.bounds;
        let (dx, dy) = (b.width() / 4.0, b.height() / 4.0);
        Plane::identity([
            Point::new(b.x0 + dx, b.y0 + dy),
            Point::new(b.x1 - dx, b.y0 + dy),
            Point::new(b.x1 - dx, b.y1 - dy),
            Point::new(b.x0 + dx, b.y1 - dy),
        ])
    }

    /// The planes to commit; `None` with no planes or when every target is its source.
    pub fn result(&self) -> Option<&[Plane]> {
        // pc: crates/ui-egui/src/tools/perspective_ui.rs:157 (`commit`); crates/algo/src/perspective.rs:232
        // (`is_identity`).
        let planes = &self.draft.planes;
        let identity = planes.iter().all(|p| p.source.iter().zip(&p.target).all(|(a, b)| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9));
        (!planes.is_empty() && !identity).then_some(planes.as_slice())
    }

    /// Applies one edit. `Mode(Warp)` with no planes is refused; entering Warp unifies linked
    /// corners; a Layout edit moves source and target together, a Warp edit the target only (the
    /// layout is kept).
    pub fn apply(&mut self, edit: PerspectiveEdit) {
        // pc: crates/ui-egui/src/tools/perspective_ui.rs:165 (`set_mode`), :108 (`set_corner`),
        // :310 (`options_bar`: Warp needs a plane).
        match edit {
            PerspectiveEdit::Planes(event) => self.planes_event(event),
            PerspectiveEdit::Mode(mode) => {
                if self.dragging() || mode == self.draft.mode || (mode == PlaneMode::Warp && self.draft.planes.is_empty()) {
                    return;
                }
                let before = self.draft.clone();
                self.draft.mode = mode;
                if mode == PlaneMode::Warp {
                    unify(&mut self.draft.planes);
                }
                self.record_step(before, "Mode");
            }
            PerspectiveEdit::Straighten(how) => {
                if self.dragging() || self.draft.planes.is_empty() {
                    return;
                }
                let before = self.draft.clone();
                straighten(&mut self.draft.planes, how);
                self.record_step(before, "Straighten");
            }
        }
    }

    /// Sets the planes from outside the widgets (the app's control channel), ends any drag and
    /// bumps the generation. One undo step.
    pub fn replace(&mut self, planes: Vec<Plane>) {
        // pc: crates/ui-egui/src/tools/perspective_ui.rs:190 (#952).
        if let Some(before) = self.before.take() {
            self.draft = before;
        }
        let before = self.draft.clone();
        self.draft.planes = planes;
        if self.draft.planes.is_empty() {
            self.draft.mode = PlaneMode::Layout;
        }
        self.generation += 1;
        self.record_step(before, "Planes");
    }

    /// Edit › Undo while the session is open (the app routes it here whatever
    /// [`can_undo`](Self::can_undo) says, and the session reads no undo key): steps back one undo
    /// step; false with nothing to undo.
    ///
    /// The app cancels a running drag first (`ToolHost::cancel` with `CancelReason::UndoDuringGesture`)
    /// and then calls this. A call during a drag steps nothing and returns false.
    pub fn undo(&mut self) -> bool {
        self.step(Step::Undo)
    }

    /// Steps forward one undone step, routed as [`undo`](Self::undo) is.
    pub fn redo(&mut self) -> bool {
        self.step(Step::Redo)
    }

    /// Whether an edit can be undone.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether an undone edit can be redone.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Records the change from `before` to the draft as one step named `name`.
    fn record_step(&mut self, before: PerspectiveDraft, name: &str) {
        let event = EditEvent::set(Mixed::Same(before), self.draft.clone(), Arc::from(name), None);
        self.history.record(&event);
    }

    /// One planes event. Begin takes the draft as `before`; Cancel restores it. In Warp mode the
    /// sources are kept (the layout is kept) when the plane count is unchanged. A Set is a step of
    /// its own; so is a drag, from its Begin to its Commit.
    fn planes_event(&mut self, event: EditEvent<Vec<Plane>>) {
        match event {
            EditEvent::Begin { id, label, doc_key, .. } => {
                let before = self.draft.clone();
                self.history.record(&EditEvent::begin(id, Mixed::Same(before.clone()), label, doc_key));
                self.before = Some(before);
            }
            EditEvent::Update { value, .. } => self.set_planes(value),
            EditEvent::Commit { id, after, .. } => {
                self.set_planes(after);
                if let Some(before) = self.before.take() {
                    self.history.record(&EditEvent::commit(id, Mixed::Same(before), self.draft.clone()));
                }
            }
            EditEvent::Cancel { id, reason, .. } => {
                self.history.record(&EditEvent::cancel(id, reason, None));
                if let Some(before) = self.before.take() {
                    self.draft = before;
                }
            }
            EditEvent::Set { after, label, doc_key, .. } => {
                if self.dragging() {
                    return;
                }
                let before = self.draft.clone();
                self.set_planes(after);
                if self.draft.planes.is_empty() {
                    self.draft.mode = PlaneMode::Layout;
                }
                self.history.record(&EditEvent::set(Mixed::Same(before), self.draft.clone(), label, doc_key));
            }
            _ => {}
        }
    }

    fn set_planes(&mut self, mut planes: Vec<Plane>) {
        if self.draft.mode == PlaneMode::Warp && planes.len() == self.draft.planes.len() {
            for (new, old) in planes.iter_mut().zip(&self.draft.planes) {
                new.source = old.source;
            }
        }
        self.draft.planes = planes;
    }

    /// Undo or redo one step; refused while a drag runs (the app cancels the drag first).
    fn step(&mut self, step: Step) -> bool {
        if self.dragging() {
            return false;
        }
        self.history.step(&mut self.draft, step)
    }
}
