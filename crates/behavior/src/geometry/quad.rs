//! Box geometry of transform boxes: where the box goes when a part of it is dragged, and the
//! options bar's reading of it. Pure functions in document units; the overlays map them to the
//! screen.
//!
//! The rules are PhotoCraft's (Photoshop CC): a corner scales proportionally about the opposite
//! corner in the box's own frame, ⇧ frees the proportions, ⌥ scales about the reference point,
//! ⌘ distorts a corner or skews an edge, ⌘⌥⇧ on a corner adds perspective, a drag outside the box
//! rotates about the reference point. [`BoxRules`] picks Edit › Transform's mode, the legacy (or
//! DesignCraft and VectorCraft) scaling rule and what the box's content refuses.

use craft_keymap::Mods;
use kurbo::{Affine, Point, Rect, Vec2};

use super::lock::{Directions, lock_direction};
use crate::anchor::Anchor9;

/// The skew fields' reach in degrees: a skew of ±90° would flatten the box (PhotoCraft's).
pub const SKEW_LIMIT: f64 = 89.0;

/// A transform box: its four corners clockwise from the top left (as the box was before any
/// transform) and its reference point, in document space.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BoxState {
    /// The four corners, clockwise from the top left.
    pub quad: [Point; 4],
    /// The reference point.
    pub pivot: Point,
}

/// A part of a box under the pointer or picked from the keyboard. Edge `k` joins corners `k` and
/// `k + 1`: 0 top, 1 right, 2 bottom, 3 left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BoxPart {
    /// Corner `k`, clockwise from the top left.
    Corner(u8),
    /// Edge `k`, joining corners `k` and `k + 1`.
    Edge(u8),
    /// The reference point.
    Pivot,
    /// Inside the box.
    Inside,
    /// Outside the box.
    Outside,
}

impl BoxPart {
    /// The number a part is keyed by in a canvas control's part keys: 0 inside or outside, 1 to 4 the corners, 5 to
    /// 8 the edges, 9 the pivot.
    pub const fn index(self) -> u64 {
        match self {
            BoxPart::Inside | BoxPart::Outside => 0,
            BoxPart::Corner(i) => 1 + (i % 4) as u64,
            BoxPart::Edge(i) => 5 + (i % 4) as u64,
            BoxPart::Pivot => 9,
        }
    }

    /// The part keyed by `index`: the inverse of [`index`](Self::index), with 0 the inside.
    pub const fn from_index(index: u64) -> Option<BoxPart> {
        match index {
            0 => Some(BoxPart::Inside),
            1..=4 => Some(BoxPart::Corner((index - 1) as u8)),
            5..=8 => Some(BoxPart::Edge((index - 5) as u8)),
            9 => Some(BoxPart::Pivot),
            _ => None,
        }
    }
}

/// Edit › Transform's modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransformMode {
    /// Free Transform: every gesture by its keys.
    #[default]
    Free,
    /// Corners and edges scale and the inside moves, as in Free Transform, but the outside does nothing and ⌘ adds
    /// nothing to a corner or an edge (measured on the reference: no turn from outside, no distort).
    Scale,
    /// Corners and edges turn the box about the reference point.
    Rotate,
    /// A corner moves alone along one of its edges; an edge slides along itself. No rotating.
    Skew,
    /// A corner moves alone; edges and the outside do nothing.
    Distort,
    /// A corner moves along one axis and its partner on that side the opposite way; an edge slides along itself, as in
    /// Skew (measured on the reference). No rotating.
    Perspective,
}

/// What a plain corner or edge drag does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScaleRule {
    /// PhotoCraft (Photoshop CC 2019 and later): corners and edges keep the proportions, ⇧ frees
    /// them.
    #[default]
    Proportional,
    /// PhotoCraft's Use Legacy Free Transform, DesignCraft and VectorCraft: corners stretch freely
    /// and ⇧ keeps the proportions; edges stretch their one axis.
    Free,
    /// Corners keep their proportions using the larger relative axis change; ⇧ frees them.
    LargerAxis,
    /// A shape or path bounds box: a corner scales by the projection of the drag on the anchor-corner diagonal,
    /// keeping the proportions; an edge stretches its one axis.
    Shape,
    /// A corner keeps the proportions and ⇧ frees them, as [`ScaleRule::Proportional`]; an edge stretches its one
    /// axis, as [`ScaleRule::Shape`]. The aspect link plays no part.
    Corners,
}

/// What the box's content allows. PhotoCraft: editable type refuses distort and perspective
/// (those drags scale, and Skew mode keeps it a parallelogram); Content-Aware Scale only scales
/// and moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BoxAllow {
    /// Rotating is allowed.
    pub rotate: bool,
    /// Skewing is allowed.
    pub skew: bool,
    /// Distorting and perspective are allowed.
    pub distort: bool,
    /// Warping is allowed.
    pub warp: bool,
}

impl BoxAllow {
    /// What the content allows: turning, skewing and distorting.
    pub const fn new(rotate: bool, skew: bool, distort: bool) -> Self {
        BoxAllow { rotate, skew, distort, warp: true }
    }

    /// Whether the content allows warping.
    pub const fn with_warp(mut self, warp: bool) -> Self {
        self.warp = warp;
        self
    }
}

impl Default for BoxAllow {
    fn default() -> Self {
        BoxAllow { rotate: true, skew: true, distort: true, warp: true }
    }
}

/// What a drag of a box part does ([`BoxRules::drag_kind`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoxDrag {
    /// Moves the box (a ⇧ move keeps to eight directions).
    Move,
    /// Moves the reference point.
    Pivot,
    /// Turns the box about its reference point (⇧ snaps the turn to the rotate step).
    Rotate,
    /// Scales the box from a corner or an edge.
    Scale,
    /// Moves one corner freely.
    Distort,
    /// Moves a corner along its dominant axis and its neighbour the opposite way.
    Perspective,
    /// Slides an edge along itself (Skew mode) or with its two corners (⌘ on an edge).
    Skew,
    /// Skew mode on a corner: the corner slides along one of its two edges.
    SlideCorner,
}

/// What a drag of `part` does in `mode` with `mods` already through [`BoxRules::drag_modifiers`].
fn resolve_kind(mode: TransformMode, part: BoxPart, mods: Mods) -> BoxDrag {
    match (mode, part) {
        (TransformMode::Skew, BoxPart::Corner(i)) if i < 4 => BoxDrag::SlideCorner,
        (TransformMode::Rotate, BoxPart::Corner(_) | BoxPart::Edge(_)) => BoxDrag::Rotate,
        (TransformMode::Skew | TransformMode::Perspective, BoxPart::Edge(_)) => BoxDrag::Skew,
        _ => {
            let mods = mode_mods(mode, part, mods);
            match part {
                BoxPart::Inside => BoxDrag::Move,
                BoxPart::Pivot => BoxDrag::Pivot,
                BoxPart::Outside => BoxDrag::Rotate,
                BoxPart::Corner(_) if mods.cmd && mods.alt && mods.shift => BoxDrag::Perspective,
                BoxPart::Corner(_) if mods.cmd => BoxDrag::Distort,
                BoxPart::Edge(_) if mods.cmd => BoxDrag::Skew,
                BoxPart::Corner(_) | BoxPart::Edge(_) => BoxDrag::Scale,
            }
        }
    }
}

/// How drags change a box.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BoxRules {
    /// The transform mode the options bar selects.
    pub mode: TransformMode,
    /// How corner and edge drags scale.
    pub scale: ScaleRule,
    /// The options bar's aspect link. Off, plain corner and edge drags stretch and ⇧ keeps the
    /// proportions, as [`ScaleRule::Free`] does.
    pub aspect_linked: bool,
    /// What the content allows.
    pub allow: BoxAllow,
    /// The angle ⇧ snaps a rotation to, in degrees (PhotoCraft 15; DesignCraft and VectorCraft
    /// 45).
    pub rotate_step: f64,
    /// The smallest side a scale drag leaves, in document units; 0 sets no minimum. A positive
    /// minimum keeps a scale drag from flipping the box, and a side already shorter than it at
    /// the press keeps its length as its floor, so a click changes nothing.
    pub min_side: f64,
    /// Which handles the box shows and takes keys and drags for.
    pub handles: BoxHandles,
    /// How far either side of a side's line a press grabs that side, in screen points, along the whole side and as
    /// far past its ends: a paragraph frame whose edges rewrap it. Inside the box the band reaches at most a quarter of
    /// the way across, so a narrow box keeps its middle. 0, the default, leaves each side its band outside the box.
    pub edge_band: f32,
    /// Where a turn reads the pointer in the document, from the pointer's own document point: half a pixel up and
    /// left at 100 %, as the reference application does; a host passes its view's reading
    /// ([`CanvasView::pointer_offset`](crate::CanvasView::pointer_offset): the pointer itself along an axis the canvas
    /// stretches, as at 33.3 %, measured 2026-11-25 on two turns there to 0.0001°).
    pub turn_offset: Vec2,
}

/// Which handles of a box are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoxHandles {
    /// The four corners and the four sides.
    #[default]
    All,
    /// The four corners.
    Corners,
    /// The right and left sides.
    Sides,
}

impl Default for BoxRules {
    fn default() -> Self {
        BoxRules {
            mode: TransformMode::Free,
            scale: ScaleRule::Proportional,
            aspect_linked: true,
            allow: BoxAllow::default(),
            rotate_step: 15.0,
            min_side: 0.0,
            handles: BoxHandles::default(),
            edge_band: 0.0,
            turn_offset: TURN_POINTER_OFFSET,
        }
    }
}

impl BoxRules {
    /// The rules of `mode` with the default scale rule, a linked aspect, everything allowed and
    /// a 15 degree rotation step.
    pub fn in_mode(mode: TransformMode) -> BoxRules {
        BoxRules { mode, ..BoxRules::default() }
    }

    /// The rules with `scale` as the plain corner and edge drag rule.
    pub fn with_scale(mut self, scale: ScaleRule) -> BoxRules {
        self.scale = scale;
        self
    }

    /// The rules with the options bar's aspect link on or off.
    pub fn with_aspect_linked(mut self, linked: bool) -> BoxRules {
        self.aspect_linked = linked;
        self
    }

    /// The rules with `allow` as what the content permits.
    pub fn with_allow(mut self, allow: BoxAllow) -> BoxRules {
        self.allow = allow;
        self
    }

    /// The rules with `side` as the minimum side of a scaled box, in document units.
    pub fn with_min_side(mut self, side: f64) -> BoxRules {
        self.min_side = side;
        self
    }

    /// The rules with `handles` as the handles the box shows.
    pub fn with_handles(mut self, handles: BoxHandles) -> BoxRules {
        self.handles = handles;
        self
    }

    /// The rules with each side grabbed within `points` of its line ([`edge_band`](Self::edge_band)).
    pub fn with_edge_band(mut self, points: f32) -> BoxRules {
        self.edge_band = points;
        self
    }

    /// Whether the box shows `part`: `Corners` shows the four corners and no side, `Sides` the right and left sides
    /// and no corner. The inside, the outside and the reference point always show.
    pub fn shows(&self, part: BoxPart) -> bool {
        match (self.handles, part) {
            (BoxHandles::Corners, BoxPart::Edge(_)) | (BoxHandles::Sides, BoxPart::Corner(_)) => false,
            (BoxHandles::Sides, BoxPart::Edge(k)) => k % 2 == 1,
            _ => true,
        }
    }

    /// The rules with `offset` as where a turn reads the pointer ([`turn_offset`](Self::turn_offset)).
    pub fn with_turn_offset(mut self, offset: Vec2) -> BoxRules {
        self.turn_offset = offset;
        self
    }

    /// The mode drags follow: a mode the content refuses scales as Free Transform does.
    pub fn effective_mode(&self) -> TransformMode {
        let refused = match self.mode {
            TransformMode::Free | TransformMode::Scale => false,
            TransformMode::Rotate => !self.allow.rotate,
            TransformMode::Skew => !self.allow.skew,
            TransformMode::Distort | TransformMode::Perspective => !self.allow.distort,
        };
        if refused { TransformMode::Free } else { self.mode }
    }

    /// Whether a drag of `part` changes the box: in Distort only the corners, the inside and the
    /// reference point; in Skew, Scale and Perspective everything but the outside; the outside only where
    /// rotating is allowed; and never a handle the box does not [show](Self::shows). A click outside a refused
    /// zone still applies the box (the overlay's rule).
    pub fn allows(&self, part: BoxPart) -> bool {
        let by_mode = match self.effective_mode() {
            TransformMode::Distort => !matches!(part, BoxPart::Outside | BoxPart::Edge(_)),
            TransformMode::Skew | TransformMode::Scale | TransformMode::Perspective => part != BoxPart::Outside,
            _ => true,
        };
        by_mode && (self.allow.rotate || part != BoxPart::Outside) && self.shows(part)
    }

    /// The modifiers a drag of `part` acts on, from the keys held: ⌘ is dropped on a corner or
    /// edge the content refuses to distort or skew, the legacy scale rule ([`ScaleRule::Free`])
    /// or an unlinked aspect ratio turns ⇧ over on corners and edges, the shape rule
    /// ([`ScaleRule::Shape`]) sets ⇧ by the part alone, and the corners rule ([`ScaleRule::Corners`]) sets ⇧ on
    /// edges only. Mode keys ([`TransformMode::Distort`],
    /// [`TransformMode::Perspective`]) are not added here: they are part of
    /// [`drag_kind`](Self::drag_kind). [`drag`] acts on these modifiers.
    fn drag_modifiers(&self, part: BoxPart, mods: Mods) -> Mods {
        let mut mods = mods;
        // ⌘-drags the content refuses scale like plain drags, ⌥ and ⇧ keeping their meaning.
        match part {
            BoxPart::Corner(_) if !self.allow.distort => mods.cmd = false,
            BoxPart::Edge(_) if !self.allow.skew => mods.cmd = false,
            _ => {}
        }
        // In Scale ⌘ has no meaning of its own on a handle: it scales as the plain drag does, with the aspect link off
        // too, so it is dropped before the scale rule reads the keys.
        if self.effective_mode() == TransformMode::Scale && matches!(part, BoxPart::Corner(_) | BoxPart::Edge(_)) {
            mods.cmd = false;
        }
        match self.scale {
            ScaleRule::Shape => shape_mods(part, mods),
            ScaleRule::Corners => corners_mods(part, mods),
            scale => {
                let legacy = scale == ScaleRule::Free;
                legacy_mods(legacy, part, unlinked_mods(!self.aspect_linked, legacy, part, mods))
            }
        }
    }

    /// What a drag of `part` with `mods` held (as the host has them, before
    /// `drag_modifiers`) does to the box, or `None` where the rules
    /// refuse the part ([`allows`](Self::allows)). [`drag`] acts on this kind.
    pub fn drag_kind(&self, part: BoxPart, mods: Mods) -> Option<BoxDrag> {
        self.allows(part).then(|| resolve_kind(self.effective_mode(), part, self.drag_modifiers(part, mods)))
    }

    /// Whether a drag of `part` with `mods` held (as the host has them) scales a corner or an edge keeping the box's
    /// proportions.
    pub fn keeps_proportions(&self, part: BoxPart, mods: Mods) -> bool {
        matches!(part, BoxPart::Corner(_) | BoxPart::Edge(_))
            && self.drag_kind(part, mods) == Some(BoxDrag::Scale)
            && !self.drag_modifiers(part, mods).shift
    }
}

/// The box after dragging `part` from `from` to `to` (document points) with `mods` held, from the
/// box at the press, `start`. `last` is the box this drag gave last (`start` on its first move):
/// a ⇧ move inside keeps its locked direction until the pointer is clearly nearer another one.
/// ⌘ is `Mods::cmd` (Ctrl off macOS). A part `rules` refuse leaves `last` as it is.
pub fn drag(start: &BoxState, last: &BoxState, part: BoxPart, from: Point, to: Point, mods: Mods, rules: &BoxRules) -> BoxState {
    if !rules.allows(part) {
        return *last;
    }
    let mods = rules.drag_modifiers(part, mods);
    let g = Gesture { part, start: from, quad0: start.quad, pivot0: start.pivot, last: last.quad[0] - start.quad[0] };
    apply_mode_drag(&g, to, rules, mods)
}

/// The [`ScaleRule::Corners`] rule: a corner keeps the keys as given, so it keeps the proportions and ⇧ frees them;
/// an edge stretches its one axis whatever ⇧ and the aspect link say. ⌘ gestures are left alone.
fn corners_mods(part: BoxPart, mut mods: Mods) -> Mods {
    if !mods.cmd && matches!(part, BoxPart::Edge(_)) {
        mods.shift = true;
    }
    mods
}

/// The shape bounds box's rule: a corner keeps the proportions along the diagonal and an edge
/// stretches its one axis, whatever ⇧ and the aspect link say. ⌘ gestures are left alone.
fn shape_mods(part: BoxPart, mut mods: Mods) -> Mods {
    if !mods.cmd {
        match part {
            BoxPart::Corner(_) => mods.shift = false,
            BoxPart::Edge(_) => mods.shift = true,
            _ => {}
        }
    }
    mods
}

/// The legacy rule: corner drags stretch freely and ⇧ keeps the proportions, edge drags stretch
/// their one axis whatever the keys. ⌘ gestures are left alone.
fn legacy_mods(legacy: bool, part: BoxPart, mut mods: Mods) -> Mods {
    if legacy && !mods.cmd {
        match part {
            BoxPart::Corner(_) => mods.shift = !mods.shift,
            BoxPart::Edge(_) => mods.shift = true,
            _ => {}
        }
    }
    mods
}

/// The aspect link turned off: corner and edge drags stretch and ⇧ keeps the proportions. The
/// legacy rule already works that way, so it is left alone there.
fn unlinked_mods(unlinked: bool, legacy: bool, part: BoxPart, mut mods: Mods) -> Mods {
    if unlinked && !legacy && !mods.cmd && matches!(part, BoxPart::Corner(_) | BoxPart::Edge(_)) {
        mods.shift = !mods.shift;
    }
    mods
}

/// Distort and Perspective: a corner drag acts as if that gesture's keys were held.
fn mode_mods(mode: TransformMode, part: BoxPart, mut mods: Mods) -> Mods {
    match (mode, part) {
        (TransformMode::Distort, BoxPart::Corner(_)) => mods.cmd = true,
        (TransformMode::Scale, BoxPart::Corner(_) | BoxPart::Edge(_)) => mods.cmd = false,
        (TransformMode::Perspective, BoxPart::Corner(_)) => {
            mods.cmd = true;
            mods.alt = true;
            mods.shift = true;
        }
        _ => {}
    }
    mods
}

/// One drag: the part, the press and the box there, and the inside move the drag gave last.
#[derive(Clone, Copy)]
struct Gesture {
    part: BoxPart,
    start: Point,
    quad0: [Point; 4],
    pivot0: Point,
    last: Vec2,
}

/// A drag in the rules' effective mode. Content that refuses distorting stays a parallelogram
/// (editable type), so a Skew corner slides its whole edge.
fn apply_mode_drag(g: &Gesture, p: Point, rules: &BoxRules, mut mods: Mods) -> BoxState {
    let (mode, parallel) = (rules.effective_mode(), !rules.allow.distort);
    match (resolve_kind(mode, g.part, mods), g.part) {
        // A corner moves alone along one of its two edges, whichever the drag mostly follows; ⌥
        // moves the opposite corner the other way.
        (BoxDrag::SlideCorner, BoxPart::Corner(i)) => {
            let (i, d, q0) = (usize::from(i), p - g.start, g.quad0);
            let along = |j: usize| {
                let e = q0[i] - q0[j];
                let l = e.hypot();
                let u = if l > 0.0 { e / l } else { Vec2::ZERO };
                u * d.dot(u)
            };
            // The neighbour along the top or bottom edge, and along the left or right one.
            let (hn, vn) = ([1, 0, 3, 2][i], [3, 2, 1, 0][i]);
            let (h, v) = (along(hn), along(vn));
            let (m, partner) = if h.hypot() >= v.hypot() { (h, hn) } else { (v, vn) };
            let moved: &[usize] = if parallel { &[i, partner] } else { &[i] };
            let mut q = q0;
            for (k, corner) in q.iter_mut().enumerate() {
                let sign = if moved.contains(&k) {
                    1.0
                } else if mods.alt && (parallel || k == (i + 2) % 4) {
                    -1.0
                } else {
                    continue;
                };
                *corner = q0[k] + m * sign;
            }
            let quad = keep_convex(q0, q);
            BoxState { quad, pivot: carry_pivot(q0, quad, g.pivot0) }
        }
        // Corners and edges turn the box as the outside does.
        (BoxDrag::Rotate, BoxPart::Corner(_) | BoxPart::Edge(_)) => {
            let g = Gesture { part: BoxPart::Outside, ..*g };
            apply_drag(&g, p, mods, rules)
        }
        // An edge slides only along itself, in Perspective as in Skew.
        (BoxDrag::Skew, BoxPart::Edge(_)) if matches!(mode, TransformMode::Skew | TransformMode::Perspective) => {
            mods.cmd = true;
            mods.shift = true;
            apply_drag(g, p, mods, rules)
        }
        _ => apply_drag(g, p, mode_mods(mode, g.part, mods), rules),
    }
}

fn apply_drag(g: &Gesture, p: Point, mods: Mods, rules: &BoxRules) -> BoxState {
    let d = p - g.start;
    let (q0, c) = (g.quad0, g.pivot0);
    match g.part {
        BoxPart::Inside => {
            let d = if mods.shift { lock_direction(d, Some(g.last), Directions::Eight) } else { d };
            BoxState { quad: q0.map(|q| q + d), pivot: c + d }
        }
        BoxPart::Pivot => BoxState { quad: q0, pivot: p },
        BoxPart::Outside => {
            // Both angles are read where the pointer acts in the document (the rules' turn offset).
            let a0 = (g.start + rules.turn_offset - c).atan2();
            let a1 = (p + rules.turn_offset - c).atan2();
            let mut da = a1 - a0;
            if mods.shift && rules.rotate_step > 0.0 {
                // Snap the turn of this drag; the angle the box started at is kept.
                let step = rules.rotate_step.to_radians();
                da = (da / step).round() * step;
            }
            let turn = Affine::rotate_about(da, c);
            BoxState { quad: q0.map(|q| turn * q), pivot: c }
        }
        BoxPart::Corner(i) if mods.cmd && mods.alt && mods.shift => {
            // Perspective: the corner moves along one of its own two sides, whichever the drag mostly follows, by the
            // drag's run along it, and the corner at that side's other end moves the opposite way (measured on the
            // reference: on a turned box the corner keeps to the box's sides, not the document's axes; on a skewed
            // box, 2026-11-25, the bottom-right corner dragged down went along the slanted right side, (452, 327) to
            // (380, 489) for a drag of (-15, 189), the top-right corner up the same side; across the top edge it
            // would have gone straight down). Nothing stops the two where they meet: the reference lets them pass
            // through each other into an hourglass (2026-11-25: the top-left corner of a 302 × 202 box dragged
            // 343.5 px right put the top corners at x 493.5 and 108.5, the box crossed).
            let i = usize::from(i) % 4;
            // A side collapsed to a point has no direction: the top edge's (or level) stands in, and the line across it.
            let top = q0[1] - q0[0];
            let len = top.hypot();
            let uh = if len.is_finite() && len > 1e-9 { top / len } else { Vec2::new(1.0, 0.0) };
            let nh = Vec2::new(-uh.y, uh.x);
            let run = |j: usize, fallback: Vec2| {
                let e = q0[i] - q0[j];
                let l = e.hypot();
                let u = if l.is_finite() && l > 1e-9 { e / l } else { fallback };
                u * d.dot(u)
            };
            // The neighbour along the top or bottom edge, and along the left or right one.
            let (hn, vn) = ([1, 0, 3, 2][i], [3, 2, 1, 0][i]);
            let (h, v) = (run(hn, uh), run(vn, nh));
            let (m, j) = if h.hypot() >= v.hypot() { (h, hn) } else { (v, vn) };
            let mut q = q0;
            q[i] = q0[i] + m;
            q[j] = q0[j] - m;
            BoxState { quad: q, pivot: carry_pivot(q0, q, c) }
        }
        BoxPart::Corner(i) if mods.cmd => {
            // Distort: the corner moves freely, past the diagonal into a concave box too (the reference application
            // lets it, 2026-11-24: its wireframe follows the pointer; the picture shows the source until the box is
            // convex again, and Enter on a concave box commits nothing).
            let mut q = q0;
            q[usize::from(i) % 4] += d;
            BoxState { quad: q, pivot: carry_pivot(q0, q, c) }
        }
        BoxPart::Edge(i) if mods.cmd => {
            // Skew: the edge's two corners move together (⇧: only along the edge); ⌥: the
            // opposite edge the other way.
            let (a, b) = (usize::from(i) % 4, (usize::from(i) + 1) % 4);
            let mut m = d;
            if mods.shift {
                let e = q0[b] - q0[a];
                let l2 = e.hypot2();
                m = if l2 > 0.0 { e * (d.dot(e) / l2) } else { Vec2::ZERO };
            }
            let mut quad = q0;
            quad[a] += m;
            quad[b] += m;
            if mods.alt {
                quad[(a + 2) % 4] -= m;
                quad[(a + 3) % 4] -= m;
            }
            BoxState { quad, pivot: carry_pivot(q0, quad, c) }
        }
        BoxPart::Corner(_) | BoxPart::Edge(_) => {
            scale_drag(g, p, mods, rules.min_side, rules.scale == ScaleRule::LargerAxis).unwrap_or(BoxState { quad: q0, pivot: c })
        }
    }
}

/// Where a turn reads the pointer at 100 %, as the reference application does: half a document pixel up and left of
/// it (measured on four turns of a 302 × 202 box, within 0.005°); the default [`BoxRules::turn_offset`].
const TURN_POINTER_OFFSET: Vec2 = Vec2::new(-0.5, -0.5);

/// A corner's handle, or an edge's: the middle between its two corners (on a box in perspective the reference
/// application keeps that point of a dragged side on the pointer).
fn handle_at(quad: [Point; 4], part: BoxPart) -> Option<Point> {
    match part {
        BoxPart::Corner(i) => quad.get(usize::from(i)).copied(),
        BoxPart::Edge(i) => {
            let i = usize::from(i) % 4;
            Some(quad[i].midpoint(quad[(i + 1) % 4]))
        }
        _ => None,
    }
}

/// A plain corner or edge drag: scaling in the box's own frame (the unit square through the
/// homography), so rotated and skewed boxes scale along their axes. The grabbed handle moves by the drag (a press
/// beside it keeps its offset, as in the reference application). A proportional corner follows
/// the pointer's projection on the line from the anchor to the corner, so a distorted box scales
/// along its own diagonal, or the larger relative axis change when selected; past the anchor the factor turns
/// negative and the box flips. Each scaled side keeps at least `min_side` ([`scale_floor`]). `None` for a collapsed
/// box.
fn scale_drag(g: &Gesture, p: Point, mods: Mods, min_side: f64, larger_axis: bool) -> Option<BoxState> {
    if !mods.shift
        && !mods.alt
        && let Some(kept) = side_run_scale(g, p, min_side)
    {
        return Some(kept);
    }
    if !mods.shift
        && mods.alt
        && let Some(centred) = centred_side_scale(g, p, min_side)
    {
        return Some(centred);
    }
    // An ⌥ corner keeping the proportions scales the box evenly about the reference point in document space, by the
    // pointer's projection on the line from the reference point to the corner (measured on the reference, also on a box
    // in perspective; on a parallelogram this is the frame scaling below).
    if !mods.shift
        && mods.alt
        && !larger_axis
        && let BoxPart::Corner(i) = g.part
        && let Some(&corner) = g.quad0.get(usize::from(i))
    {
        let r = corner - g.pivot0;
        let len2 = r.hypot2();
        if len2 > 1e-12 {
            let k = ((corner + (p - g.start)) - g.pivot0).dot(r) / len2;
            let k = if k.is_finite() { k } else { 1.0 };
            let k = k.max(scale_floor(&g.quad0, 0, min_side).max(scale_floor(&g.quad0, 1, min_side)));
            let quad = g.quad0.map(|q| g.pivot0 + (q - g.pivot0) * k);
            return quad.iter().all(|q| q.is_finite()).then_some(BoxState { quad, pivot: g.pivot0 });
        }
    }
    if mods.shift
        && mods.alt
        && min_side <= 0.0
        && let Some(stretched) = centred_stretch(g, p)
    {
        return Some(stretched);
    }
    let h = Homography::unit_to_quad(g.quad0)?;
    let inv = h.inverse()?;
    // The grabbed handle moves by the drag, so a press beside it keeps its offset from it.
    let p = match handle_at(g.quad0, g.part) {
        Some(at) => at + (p - g.start),
        None => p,
    };
    let Point { x: u, y: v } = inv.apply(p);
    let Point { x: pu, y: pv } = inv.apply(g.pivot0);
    // Which sides of the unit box move: (u = 0, u = 1) and (v = 0, v = 1).
    let (mu, mv) = match g.part {
        BoxPart::Corner(0) => ((true, false), (true, false)),
        BoxPart::Corner(1) => ((false, true), (true, false)),
        BoxPart::Corner(2) => ((false, true), (false, true)),
        BoxPart::Corner(3) => ((true, false), (false, true)),
        BoxPart::Edge(0) => ((false, false), (true, false)),
        BoxPart::Edge(1) => ((false, true), (false, false)),
        BoxPart::Edge(2) => ((false, false), (false, true)),
        _ => ((true, false), (false, false)),
    };
    // Each axis scales by `k` about an anchor: the opposite side, or with ⌥ the reference point.
    // The dragged side follows the pointer.
    let axis = |moving: (bool, bool), at: f64, pivot: f64| -> (f64, f64) {
        if !moving.0 && !moving.1 {
            return (0.5, 1.0);
        }
        let handle = if moving.0 { 0.0 } else { 1.0 };
        let anchor = if mods.alt { pivot } else { 1.0 - handle };
        let d = handle - anchor;
        // A reference point on the dragged side leaves nothing to scale against.
        let k = if d.abs() > 1e-9 { (at - anchor) / d } else { 1.0 };
        (anchor, if k.is_finite() { k } else { 1.0 })
    };
    let (mut au, mut ku) = axis(mu, u, pu);
    let (mut av, mut kv) = axis(mv, v, pv);
    let (lo_u, lo_v) = (scale_floor(&g.quad0, 0, min_side), scale_floor(&g.quad0, 1, min_side));
    let corner = matches!(g.part, BoxPart::Corner(_));
    let moves_u = mu.0 || mu.1;
    if mods.shift {
        // The axes move independently, so each moved one keeps the minimum on its own.
        if moves_u {
            ku = ku.max(lo_u);
        }
        if mv.0 || mv.1 {
            kv = kv.max(lo_v);
        }
    } else if corner {
        // One factor holds both axes to the stricter floor, including on inward drags.
        let (hu, hv) = (if mu.0 { 0.0 } else { 1.0 }, if mv.0 { 0.0 } else { 1.0 });
        let a = h.apply(Point::new(au, av));
        let d = h.apply(Point::new(hu, hv)) - a;
        let len2 = d.hypot2();
        let k = if larger_axis {
            if (ku - 1.0).abs() > (kv - 1.0).abs() { ku } else { kv }
        } else if len2 > 1e-12 {
            let q = inv.apply(a + d * ((p - a).dot(d) / len2));
            if (hu - au).abs() >= (hv - av).abs() { (q.x - au) / (hu - au) } else { (q.y - av) / (hv - av) }
        } else {
            1.0
        };
        let k = if k.is_finite() { k } else { 1.0 };
        let k = k.max(lo_u.max(lo_v));
        (ku, kv) = (k, k);
    } else if moves_u {
        // An edge sets its own axis' scale and the other axis follows by the same factor about
        // the middle of the box (⌥: the reference point). Past the opposite edge only the dragged
        // axis flips. The follower takes the factor too, so both floors bound it.
        ku = ku.max(lo_u.max(lo_v));
        (av, kv) = (if mods.alt { pv } else { 0.5 }, ku.abs());
    } else {
        kv = kv.max(lo_u.max(lo_v));
        (au, ku) = (if mods.alt { pu } else { 0.5 }, kv.abs());
    }
    let r = [au - au * ku, av - av * kv, au + (1.0 - au) * ku, av + (1.0 - av) * kv];
    let at = |a: f64, b: f64| h.apply(Point::new(a, b));
    let quad = [at(r[0], r[1]), at(r[2], r[1]), at(r[2], r[3]), at(r[0], r[3])];
    // ⌥ scales about the reference point, so it stays put; otherwise it rides along.
    let pivot = if mods.alt { g.pivot0 } else { carry_pivot(g.quad0, quad, g.pivot0) };
    Some(BoxState { quad, pivot })
}

/// A proportional edge drag on a skewed box or a box in perspective, as the reference application scales them
/// (measured on its saved results). Along the top edge `u` and its normal `n`: the top edge scales by `k` about its
/// middle, the left side keeps its run along `u` (a skew's offset, a perspective's inset) while its part along `n`
/// scales by `k`, and the right side keeps its px offset from the left one; the opposite edge's middle holds, and the
/// dragged edge's middle follows the pointer along the drag's axis. Where the dragged edge and the opposite one meet
/// (a box in perspective) the dragged edge turns about its middle onto the line through that point, its corners on the
/// edges beside it, then slide along it to second order ([`perspective_push`]); a side beside it left within 1 px of
/// level is levelled ([`level_snap`]). On a plain or turned rectangle the sides have no run along `u` and this is the frame
/// scaling below. Past the opposite edge the factor turns negative and the box flips along both axes, as on the
/// reference (its top read backwards, its height upside down). `None` for a factor of zero or a collapsed box.
fn side_run_scale(g: &Gesture, p: Point, min_side: f64) -> Option<BoxState> {
    let BoxPart::Edge(edge) = g.part else { return None };
    let [tl, tr, br, bl] = g.quad0;
    let u = tr - tl;
    let len = u.hypot();
    if len.is_nan() || len <= 1e-9 {
        return None;
    }
    let uh = u / len;
    let mut nh = Vec2::new(-uh.y, uh.x);
    let (left, right) = (bl - tl, br - tr);
    if left.dot(nh) < 0.0 {
        nh = -nh;
    }
    let (al, height) = (left.dot(uh), left.dot(nh));
    if height.is_nan() || height <= 1e-9 {
        return None;
    }
    let d = p - g.start;
    let k = match edge {
        0 => 1.0 - d.dot(nh) / height,
        1 => 1.0 + d.dot(uh) / len,
        2 => 1.0 + d.dot(nh) / height,
        _ => 1.0 - d.dot(uh) / len,
    };
    if !(k.is_finite() && k.abs() > 1e-9) {
        return None;
    }
    let k = k.max(scale_floor(&g.quad0, 0, min_side).max(scale_floor(&g.quad0, 1, min_side)));
    // The right side keeps its offset from the left one in px (on a parallelogram it has none).
    let (u2, left2) = (u * k, uh * al + nh * (height * k));
    let right2 = left2 + (right - left);
    let shape = [Point::ZERO, Point::ZERO + u2, Point::ZERO + u2 + right2, Point::ZERO + left2];
    // The edge that holds, by its middle: the opposite one.
    let middle = |q: &[Point; 4], a: usize, b: usize| q[a].midpoint(q[b]);
    let (a, b) = match edge {
        0 => (2, 3),
        1 => (3, 0),
        2 => (0, 1),
        _ => (1, 2),
    };
    let shift = middle(&g.quad0, a, b) - middle(&shape, a, b);
    let mut quad = shape.map(|c| c + shift);
    // Where the dragged edge and the opposite one meet (a box in perspective), the dragged edge turns about its
    // middle onto the line through that point; its corners stay on the lines of the edges beside it. Each dragged
    // corner is paired with its neighbour on the opposite edge.
    let ([d0, d1], [o0, o1]) = match edge {
        0 => ([0, 1], [3, 2]),
        1 => ([1, 2], [0, 3]),
        2 => ([3, 2], [0, 1]),
        _ => ([0, 3], [1, 2]),
    };
    let [q0, q1] = [g.quad0[d0], g.quad0[d1]];
    if let Some(v) = crossing(q0, q1 - q0, g.quad0[o0], g.quad0[o1] - g.quad0[o0]) {
        let offset = quad;
        let mid = offset[d0].midpoint(offset[d1]);
        let line = mid - v;
        let c0 = crossing(offset[o0], offset[d0] - offset[o0], v, line);
        let c1 = crossing(offset[o1], offset[d1] - offset[o1], v, line);
        if let (Some(c0), Some(c1)) = (c0, c1) {
            quad[d0] = c0;
            quad[d1] = c1;
            if let Some(pushed) = perspective_push(g.quad0, offset, quad, edge, [d0, d1], v, mid) {
                quad = pushed;
            }
            level_snap(&mut quad, &g.quad0, [(o0, d0), (o1, d1)]);
        }
    }
    quad.iter().all(|c| c.is_finite()).then(|| BoxState { quad, pivot: carry_pivot(g.quad0, quad, g.pivot0) })
}

/// The opposite side and the dragged corners of an ⌥ side drag on a box in perspective, by `k` about the reference
/// point (in the top edge's frame, `uh` along it): the opposite side scales evenly and turns about its middle to a run
/// of `r + (k - 1)(r - r') / 2` along the top edge (`r` its own run, `r'` the dragged side's; on a symmetric box that is
/// the even scale's `k r`); the dragged corners keep the place along `uh` a plain drag gives them
/// ([`side_run_scale`]) and move along `nh` to the opposite side's rows.
fn turned_side_scale(g: &Gesture, p: Point, min_side: f64, uh: Vec2, nh: Vec2, k: f64) -> Option<[Point; 4]> {
    let BoxPart::Edge(edge) = g.part else { return None };
    let (q, c) = (g.quad0, g.pivot0);
    let (al, ar) = ((q[3] - q[0]).dot(uh), (q[2] - q[1]).dot(uh));
    let mut out = q.map(|x| c + (x - c) * k);
    let ([a0, a1], (run, other)) = if edge == 1 { ([0, 3], (al, ar)) } else { ([1, 2], (ar, al)) };
    let (middle, across) = (out[a0].midpoint(out[a1]), (out[a1] - out[a0]).dot(nh));
    let half = (uh * (run + (k - 1.0) * (run - other) / 2.0 + side_turn_lean(q, edge, uh) * (k * k - 1.0)) + nh * across) / 2.0;
    (out[a0], out[a1]) = (middle - half, middle + half);
    let plain = side_run_scale(g, p, min_side)?.quad;
    for i in if edge == 1 { [1, 2] } else { [0, 3] } {
        out[i] = c + uh * (plain[i] - c).dot(uh) + nh * (out[i] - c).dot(nh);
    }
    out.iter().all(|x| x.is_finite()).then_some(out)
}

/// An ⌥ side drag on a box whose top and bottom converge, fitted to the reference's results within 0.45 px where its
/// sides are parallel or meet gently (7.5 px on a box narrowing hard both ways; the frame scaling was hundreds off):
/// [`turned_side_scale`], then the top and bottom turn parallel to the top edge, `k H0` apart about the reference
/// point's row, `H0` the dragged side's height plus the top and bottom's mean run across the frame. `k = 1 + d / r`,
/// `r` the dragged side's middle's distance from the reference point along the top edge. `None` for a box this leaves
/// without height.
fn converging_side_scale(g: &Gesture, p: Point, min_side: f64, uh: Vec2, nh: Vec2, len: f64) -> Option<BoxState> {
    let BoxPart::Edge(edge) = g.part else { return None };
    let c = g.pivot0;
    let v = g.quad0.map(|x| (x - c).dot(nh));
    if edge % 2 == 0 {
        return converging_top_scale(g, p, uh, nh, len, min_side);
    }
    let [tl, tr, br, bl] = g.quad0;
    let sign = if edge == 1 { 1.0 } else { -1.0 };
    let reach = if edge == 1 { reach_to(c, across(tl, tr, bl, br), uh, tr, br) } else { reach_to(c, -across(tl, tr, bl, br), -uh, tl, bl) };
    let k = 1.0 + sign * (p - g.start).dot(uh) / reach;
    let k = side_factor_lean(g.quad0, edge, k);
    // The opposite side and the height follow a factor eased by the box's slant ([`alt_spread`], as for a top or
    // bottom drag), the more as the sides converge ([`top_weight_ratio`]); the dragged side stays where the plain drag
    // puts it (measured on the reference, thirteen boxes).
    let spread = alt_spread(g.quad0);
    let k = if k.abs() > 1e-9 { k - 0.12 * spread * (k - 1.0) / k - 0.3 * spread * (top_weight_ratio(g.quad0) - 1.0) * (k - 1.0) } else { k };
    let dragged = if edge == 1 { v[2] - v[1] } else { v[3] - v[0] };
    let height = k * (dragged + ((v[1] - v[0]) + (v[2] - v[3])) / 2.0);
    if !(reach.is_finite() && reach > 1e-9 && k.is_finite() && k > 0.0 && height.is_finite() && height > 1e-9) {
        return None;
    }
    let mut quad = turned_side_scale(g, p, min_side, uh, nh, k)?;
    // The rows centre on where the scale about the reference point takes the box's middle (its corners' mean), not on
    // the reference point's own row (the two agree for a centred reference point).
    let centre = k * (mean_of(g.quad0) - c).dot(nh);
    for (i, row) in [(0, -height / 2.0), (1, -height / 2.0), (2, height / 2.0), (3, height / 2.0)] {
        quad[i] = c + uh * (quad[i] - c).dot(uh) + nh * (row + centre);
    }
    quad.iter().all(|x| x.is_finite()).then_some(BoxState { quad, pivot: c })
}

/// An ⌥ top or bottom drag on a box whose top and bottom converge, within 0.35 px of the reference on boxes with
/// parallel sides (1.3 px where they differ by 60 %, 1.4 px where the sides converge hard too; 0.23 px on average over
/// 88 drags). The perspective side rule turned on its side, in the top edge's frame about the reference point: the box
/// scales evenly by `k = 1 + d / r` (`r` the dragged edge's middle's distance from the reference point across the
/// frame, eased by [`alt_spread`] and [`top_weight_ratio`]), the
/// opposite edge turns about its middle to a run of `r + (k - 1)(r - r') / 2` across the frame, the dragged corners keep the place across the frame a plain drag
/// gives them ([`side_run_scale`]); then the sides turn across the frame, `k (|top| ± m)` apart (`m` the sides' mean
/// run along the frame, added for the bottom edge and taken off for the top one).
fn converging_top_scale(g: &Gesture, p: Point, uh: Vec2, nh: Vec2, len: f64, min_side: f64) -> Option<BoxState> {
    let BoxPart::Edge(edge) = g.part else { return None };
    let [tl, tr, br, bl] = g.quad0;
    let c = g.pivot0;
    let f = g.quad0.map(|x| (x - c).dot(uh));
    let v = g.quad0.map(|x| (x - c).dot(nh));
    let sign = if edge == 2 { 1.0 } else { -1.0 };
    let reach = if edge == 2 { reach_to(c, across(tl, bl, tr, br), nh, bl, br) } else { reach_to(c, -across(tl, bl, tr, br), -nh, tl, tr) };
    let k = 1.0 + sign * (p - g.start).dot(nh) / reach;
    // The box grows by a factor eased from the pointer's `k`: a bottom drag by less, a top drag by slightly more as
    // the drag grows, by the box's slant (`spread`) and, where the sides converge, powers of the top's and the
    // bottom's weight ratio: 2.2 on a bottom drag's even part and 1 on the part growing with the drag, 3 on a top drag
    // (measured on the reference, nineteen boxes: within 1.1 px on a bottom drag, 0.25 px on average).
    let spread = alt_spread(g.quad0);
    let ratio = top_weight_ratio(g.quad0);
    let k = if edge == 2 {
        1.0 + (k - 1.0) * (1.0 - spread * (0.22 * ratio.powf(2.2) + 0.1 * ratio * (k - 1.0)))
    } else {
        1.0 + (k - 1.0) * (1.0 + 0.111 * spread * ratio * ratio * ratio * (k - 1.0))
    };
    let width = k * (len + sign * ((f[3] - f[0]) + (f[2] - f[1])) / 2.0);
    if !(reach.is_finite() && reach > 1e-9 && k.is_finite() && k > 0.0 && width.is_finite() && width > 1e-9) {
        return None;
    }
    let mut out = g.quad0.map(|x| c + (x - c) * k);
    let (top_run, bottom_run) = (v[1] - v[0], v[2] - v[3]);
    let ([a0, a1], (run, other)) = if edge == 2 { ([0, 1], (top_run, bottom_run)) } else { ([3, 2], (bottom_run, top_run)) };
    let (middle, along) = (out[a0].midpoint(out[a1]), (out[a1] - out[a0]).dot(uh));
    let share = 0.5 + if edge == 2 { -1.0 } else { 1.0 } * top_turn_lean(g.quad0) * (k + 1.0);
    let half = (uh * along + nh * (run + (k - 1.0) * (run - other) * share)) / 2.0;
    (out[a0], out[a1]) = (middle - half, middle + half);
    let plain = side_run_scale(g, p, min_side)?.quad;
    for i in if edge == 2 { [2, 3] } else { [0, 1] } {
        out[i] = c + uh * (out[i] - c).dot(uh) + nh * (plain[i] - c).dot(nh);
    }
    // The columns centre on where the scale takes the box's middle, as the rows do for a side drag.
    let centre = k * (mean_of(g.quad0) - c).dot(uh);
    for (i, column) in [(0, -width / 2.0), (1, width / 2.0), (2, width / 2.0), (3, -width / 2.0)] {
        out[i] = c + uh * (column + centre) + nh * (out[i] - c).dot(nh);
    }
    out.iter().all(|x| x.is_finite()).then_some(BoxState { quad: out, pivot: c })
}

/// The factor of an ⌥ side drag on a box converging both ways, from the pointer's `k`: `k - 1` grows (a left drag) or
/// shrinks (a right one) by `0.28 ln(w_t / w_b) ln(w_r / w_l) (1 + 1.4 (k - 1))`, `w_*` the projective weights of the
/// edges' middles. Measured on the reference, ten boxes, side drags from -80 to +80 px: their factor is then within
/// 1 % under a level top and 2 % under a tilted one. `k` itself where either pair of edges is parallel, and for a degenerate box.
fn side_factor_lean(q: [Point; 4], edge: u8, k: f64) -> f64 {
    let (Some(wt), Some(wb), Some(wl), Some(wr)) =
        (unit_weight(q, 0.5, 0.0), unit_weight(q, 0.5, 1.0), unit_weight(q, 0.0, 0.5), unit_weight(q, 1.0, 0.5))
    else {
        return k;
    };
    let lean = 0.28 * (wt / wb).ln() * (wr / wl).ln() * if edge == 3 { 1.0 } else { -1.0 };
    let out = 1.0 + (k - 1.0) * (1.0 + lean * (1.0 + 1.4 * (k - 1.0)));
    if out.is_finite() { out } else { k }
}

/// `w_t / w_b`, the projective weights of the top's and the bottom's middles: 1 where the sides are parallel, and for
/// a degenerate box.
fn top_weight_ratio(q: [Point; 4]) -> f64 {
    match (unit_weight(q, 0.5, 0.0), unit_weight(q, 0.5, 1.0)) {
        (Some(top), Some(bottom)) if (top / bottom).is_finite() && top / bottom > 0.0 => top / bottom,
        _ => 1.0,
    }
}

/// The projective weight of the unit square's point `(u, v)` in `q`, `None` for a degenerate box.
fn unit_weight(q: [Point; 4], u: f64, v: f64) -> Option<f64> {
    let [a, b, c] = Homography::unit_to_quad(q)?.row(2);
    let w = a * u + b * v + c;
    w.is_finite().then_some(w)
}

/// How far an ⌥ top or bottom drag on a box converging both ways moves the opposite edge's share of the turn off a
/// half, per unit of `k + 1`: `(1 - w_b / w_t) / 4`, `w_t` and `w_b` the projective weights of the top's and the
/// bottom's middles. The share is `1/2 + lean (k + 1)` for a top drag and `1/2 - lean (k + 1)` for a bottom one
/// (measured on the reference, eight boxes, within 0.5 px). Zero where the sides are parallel, and for a degenerate box.
fn top_turn_lean(q: [Point; 4]) -> f64 {
    let lean = match (unit_weight(q, 0.5, 0.0), unit_weight(q, 0.5, 1.0)) {
        (Some(top), Some(bottom)) => (1.0 - bottom / top) / 4.0,
        _ => 0.0,
    };
    if lean.is_finite() { lean } else { 0.0 }
}

/// What an ⌥ side drag on a box converging both ways adds to the opposite side's run along `uh`, per unit of `k² - 1`:
/// `± ln(w_r / w_l) / 4` times the two sides' difference of runs across their mean direction (`w_l`, `w_r` the
/// projective weights of the left and right sides' middles; `+` for a right drag). Measured on the reference, eight
/// boxes. Zero where the sides are parallel or the top and bottom are, and for a degenerate box.
fn side_turn_lean(q: [Point; 4], edge: u8, uh: Vec2) -> f64 {
    let [tl, tr, br, bl] = q;
    let (left, right) = (bl - tl, br - tr);
    let (ll, rl) = (left.hypot(), right.hypot());
    let (Some(wl), Some(wr)) = (unit_weight(q, 0.0, 0.5), unit_weight(q, 1.0, 0.5)) else { return 0.0 };
    if !(ll > 1e-9 && rl > 1e-9) {
        return 0.0;
    }
    let d = left / ll + right / rl;
    let mut n = Vec2::new(d.y, -d.x) / d.hypot();
    if n.dot(uh) < 0.0 {
        n = -n;
    }
    // The opposite side's run less the dragged one's, across the sides' mean direction.
    let runs = if edge == 1 { left.dot(n) - right.dot(n) } else { right.dot(n) - left.dot(n) };
    let lean = if edge == 1 { 1.0 } else { -1.0 } * (wr / wl).ln() / 4.0 * runs;
    if lean.is_finite() { lean } else { 0.0 }
}

/// How much an ⌥ top or bottom drag eases its factor: `2 (W / H)² s_t (s_t - s_b)`, `s_t` and `s_b` the top's and the
/// bottom's slopes against the normal of the sides' mean direction, `W` the top's width along that normal and `H` the
/// longer side. Zero where the top is square to the sides, whatever the bottom does (as on the reference); for a
/// symmetric box `(1 - r)²`, `r` the shorter side over the longer. Zero for a degenerate box.
fn alt_spread(q: [Point; 4]) -> f64 {
    let [tl, tr, br, bl] = q;
    let (left, right) = (bl - tl, br - tr);
    let (ll, rl) = (left.hypot(), right.hypot());
    if !(ll > 1e-9 && rl > 1e-9) {
        return 0.0;
    }
    let d = left / ll + right / rl;
    let d = d / d.hypot();
    let mut n = Vec2::new(d.y, -d.x);
    let (top, bottom) = (tr - tl, br - bl);
    if top.dot(n) < 0.0 {
        n = -n;
    }
    let (wt, wb) = (top.dot(n), bottom.dot(n));
    if !(wt.abs() > 1e-9 && wb.abs() > 1e-9) {
        return 0.0;
    }
    let (st, sb) = (top.dot(d) / wt, bottom.dot(d) / wb);
    let x = 2.0 * (wt / ll.max(rl)) * (wt / ll.max(rl)) * st * (st - sb);
    if x.is_finite() { x } else { 0.0 }
}

/// An ⌥⇧ drag (free stretch about the reference point) as the reference does it, also on a box in perspective:
/// - a corner scales the box about the reference point along the top edge's direction and across it, each by where
///   the pointer is against where the corner was;
/// - an edge goes where a ⇧ drag puts it (onto the box's line through the pointer, its corners on the edges beside
///   it), and the opposite edge onto the line through the pointer mirrored about the reference point.
///
/// `None` for a collapsed box or a reference point on the dragged part's axis. Boxes with a minimum side (a text frame)
/// keep the frame scaling, which holds that minimum.
fn centred_stretch(g: &Gesture, p: Point) -> Option<BoxState> {
    let (q, c) = (g.quad0, g.pivot0);
    let d = p - g.start;
    let quad = match g.part {
        BoxPart::Corner(i) => {
            let corner = *q.get(usize::from(i))?;
            let top = q[1] - q[0];
            let len = top.hypot();
            if !(len.is_finite() && len > 1e-9) {
                return None;
            }
            let uh = top / len;
            let nh = Vec2::new(-uh.y, uh.x);
            let (r, to) = (corner - c, corner + d - c);
            let (ru, rv) = (r.dot(uh), r.dot(nh));
            if ru.abs() <= 1e-9 || rv.abs() <= 1e-9 {
                return None;
            }
            let (ku, kv) = (to.dot(uh) / ru, to.dot(nh) / rv);
            q.map(|x| {
                let x = x - c;
                c + uh * (x.dot(uh) * ku) + nh * (x.dot(nh) * kv)
            })
        }
        BoxPart::Edge(i) => {
            let i = usize::from(i) % 4;
            let h = Homography::unit_to_quad(q)?;
            let inv = h.inverse()?;
            let at = q[i].midpoint(q[(i + 1) % 4]) + d;
            let (a, b) = (inv.apply(at), inv.apply(c + (c - at)));
            let ([u0, u1], [v0, v1]) = match i {
                0 => ([0.0, 1.0], [a.y, b.y]),
                1 => ([b.x, a.x], [0.0, 1.0]),
                2 => ([0.0, 1.0], [b.y, a.y]),
                _ => ([a.x, b.x], [0.0, 1.0]),
            };
            let at = |u: f64, v: f64| h.apply(Point::new(u, v));
            [at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)]
        }
        _ => return None,
    };
    quad.iter().all(|x| x.is_finite()).then_some(BoxState { quad, pivot: c })
}

/// An ⌥ edge drag (both sides move, about the reference point) as the reference does it. A box whose top and bottom
/// converge goes to [`converging_side_scale`] and [`converging_top_scale`]; for the others, in the frame of the top edge (`uh`
/// along it, `nh` across), with `k = 1 + d / r`, `d` the drag along the edge's axis and `r` how far the dragged edge is
/// from the reference point along it (half the box for a centred reference point):
/// - a left or right edge, its side and the opposite one parallel: the side-run rule ([`side_run_scale`]) about the
///   reference point, which keeps its place in the parallelogram;
/// - a left or right edge, the sides meeting (a box in perspective): the opposite side scales evenly about the
///   reference point, `r` measured to the dragged side's middle, and on an asymmetric box turns about its middle
///   (its run along `uh` grows by `(k - 1)` times half the two sides' difference of runs); the dragged corners keep the
///   place along `uh` a plain drag gives them ([`side_run_scale`]) and move along `nh` to the opposite side's rows;
/// - a top or bottom edge: a rectangle in that frame, `k h` high (`h` the left side's part along `nh`) and
///   `k (|top| ± m)` wide, `m` the sides' mean run along `uh`, added for the bottom edge and taken off for the top
///   one, its centre the corners' mean scaled by `k` about the reference point.
fn centred_side_scale(g: &Gesture, p: Point, min_side: f64) -> Option<BoxState> {
    let BoxPart::Edge(edge) = g.part else { return None };
    let q = g.quad0;
    let [tl, tr, br, bl] = q;
    let u = tr - tl;
    let len = u.hypot();
    if !(len.is_finite() && len > 1e-9) {
        return None;
    }
    let uh = u / len;
    let mut nh = Vec2::new(-uh.y, uh.x);
    let (left, right, bottom) = (bl - tl, br - tr, br - bl);
    if left.dot(nh) < 0.0 {
        nh = -nh;
    }
    let (al, ar, h) = (left.dot(uh), right.dot(uh), left.dot(nh));
    if !(h.is_finite() && h > 1e-9) {
        return None;
    }
    if uh.cross(bottom).abs() > 1e-9 * bottom.hypot() {
        let at = |p: Point| converging_side_scale(g, p, min_side, uh, nh, len);
        let scaled = at(p)?;
        if keeps_min_side(&q, &scaled.quad, min_side) {
            return Some(scaled);
        }
        // A host's minimum side: the drag stops where a side would go under it (found along the drag's travel; the
        // fit turns the box's sides, so no one factor bounds them).
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if at(g.start + (p - g.start) * mid).is_some_and(|s| keeps_min_side(&q, &s.quad, min_side)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        return at(g.start + (p - g.start) * lo).filter(|s| keeps_min_side(&q, &s.quad, min_side));
    }
    let c = g.pivot0;
    let mean = Point::new(q.iter().map(|p| p.x).sum::<f64>() / 4.0, q.iter().map(|p| p.y).sum::<f64>() / 4.0);
    let d = p - g.start;
    let floor = scale_floor(&q, 0, min_side).max(scale_floor(&q, 1, min_side));
    // The factor that carries the dragged edge by the drag's part `by` along its axis, from `reach` away from the
    // reference point (half the box for a centred one).
    let factor = |by: f64, reach: f64| {
        let k = 1.0 + by / reach;
        (reach.is_finite() && reach > 1e-9 && k.is_finite() && k > 0.0).then_some(k.max(floor))
    };
    let at = |du: f64, dv: f64, from: Point| from + uh * du + nh * dv;
    let quad = match edge {
        1 | 3 => {
            let sign = if edge == 1 { 1.0 } else { -1.0 };
            if crossing(tl, left, tr, right).is_none() {
                // The reference point's place in the parallelogram: `tl + tau left + sigma u`.
                let (r, den) = (c - tl, left.cross(u));
                if den.abs() <= 1e-9 * left.hypot() * len {
                    return None;
                }
                let (tau, sigma) = (r.cross(u) / den, left.cross(r) / den);
                let reach = if edge == 1 { (1.0 - sigma) * len } else { sigma * len };
                let k = factor(sign * d.dot(uh), reach)?;
                let (u2, left2) = (u * k, uh * al + nh * (h * k));
                let right2 = left2 + (right - left);
                let origin = c - left2 * tau - u2 * sigma;
                [origin, origin + u2, origin + u2 + right2, origin + left2]
            } else {
                let reach =
                    if edge == 1 { reach_to(c, across(tl, tr, bl, br), uh, tr, br) } else { reach_to(c, -across(tl, tr, bl, br), -uh, tl, bl) };
                let k = factor(sign * d.dot(uh), reach)?;
                turned_side_scale(g, p, min_side, uh, nh, k)?
            }
        }
        _ => {
            let sign = if edge == 2 { 1.0 } else { -1.0 };
            let reach = if edge == 2 { (bl - c).dot(nh) } else { (c - tl).dot(nh) };
            let k = factor(sign * d.dot(nh), reach)?;
            let (w, hh) = (k * (len + sign * (al + ar) / 2.0), k * h);
            let centre = c + (mean - c) * k;
            [at(-w / 2.0, -hh / 2.0, centre), at(w / 2.0, -hh / 2.0, centre), at(w / 2.0, hh / 2.0, centre), at(-w / 2.0, hh / 2.0, centre)]
        }
    };
    // ⌥ scales about the reference point, so it stays put.
    quad.iter().all(|c| c.is_finite()).then_some(BoxState { quad, pivot: g.pivot0 })
}

/// The second order of an edge drag on a box in perspective (measured on the reference): the dragged corners slide
/// along the line through the vanishing point, each to where it meets the line from one centre through the
/// offset box's corner.
///
/// In the start box's unit square, `s` runs across the drag from the anchored edge (0) to the dragged one (1) and `t`
/// along them from the side the drag's factor is measured on (the top for a left or right edge, the left side for a
/// top or bottom one). `rho` is how much nearer the vanishing point that side is than the other (the ratio of the
/// map's weights). The centre lies on the row `t = rho² / (1 + rho²)`, where `s_m` is the dragged edge's middle across
/// the start box: for a left or right edge in the start box's square, at `1 / s = 2 (1 + rho²)² / (1 + rho)³ ·
/// (1 / s_m - 1)`; for a top or bottom edge in the square of the box from the first step (`first`), at
/// `1 / s = (1 + rho³) / (1 + rho²) · (1 - s_m)`. For a side, far from the start both drift with `x = 1 / s_m - 1`:
/// the row by `c (1 - c) ln rho (0.75 - 0.25 rho) x` and the factor before `x` by `2 c (1 - c) (ln rho)² x`
/// (`c` the row), fitted on six boxes from rho 0.5 to 2.5 to within 4 px for sides shrunk to 30 % or doubled. With no
/// drag the centre is the far point of that row and leaves the corners where they are. A push that would carry a
/// corner further than twice the dragged edge (where the reference's own corners cross over) keeps the first step.
fn perspective_push(
    start: [Point; 4],
    offset: [Point; 4],
    first: [Point; 4],
    edge: u8,
    dragged: [usize; 2],
    v: Point,
    mid: Point,
) -> Option<[Point; 4]> {
    let centre = push_centre(start, first, edge, mid)?;
    let homog = |p: Point| [p.x, p.y, 1.0];
    let along = cross3(homog(v), homog(mid));
    let mut out = first;
    for i in dragged {
        let p = cross3(cross3(centre, homog(offset[i])), along);
        if p[2].abs() <= 1e-12 {
            return None;
        }
        out[i] = Point::new(p[0] / p[2], p[1] / p[2]);
    }
    // A push past this is a runaway, so the first step stands; bottom drags on boxes in strong perspective move their
    // corners several edge lengths on the reference.
    let reach = first[dragged[0]].distance(first[dragged[1]]) * if edge == 2 { 6.0 } else { 2.0 };
    (out.iter().all(|c| c.is_finite()) && dragged.iter().all(|&i| out[i].distance(first[i]) <= reach)).then_some(out)
}

/// The push's centre (homogeneous) for a drag of `edge` on `start` whose first step put the dragged edge's middle at
/// `mid`; see [`perspective_push`].
fn push_centre(start: [Point; 4], first: [Point; 4], edge: u8, mid: Point) -> Option<[f64; 3]> {
    // (s, t, w) of the drag's frame as (u, v, w) of the unit square.
    let unit = |s: f64, t: f64, w: f64| match edge {
        1 => [s, t, w],
        3 => [w - s, t, w],
        2 => [t, s, w],
        _ => [t, w - s, w],
    };
    let apply = |h: &Homography, p: [f64; 3]| [0, 1, 2].map(|i| h.row(i).iter().zip(p).map(|(a, b)| a * b).sum::<f64>());
    let h = Homography::unit_to_quad(start)?;
    let rho = apply(&h, unit(0.5, 0.0, 1.0))[2] / apply(&h, unit(0.5, 1.0, 1.0))[2];
    if !(rho.is_finite() && rho > 0.0) {
        return None;
    }
    let at = h.inverse()?.apply(mid);
    let across = match edge {
        1 => at.x,
        3 => 1.0 - at.x,
        2 => at.y,
        _ => 1.0 - at.y,
    };
    if !(across.is_finite() && across.abs() > 1e-9) {
        return None;
    }
    let r2 = rho * rho;
    let row = r2 / (1.0 + r2);
    let (x, lean) = (1.0 / across - 1.0, rho.ln());
    let place = 2.0 * (1.0 + r2) * (1.0 + r2) / ((1.0 + rho) * (1.0 + rho) * (1.0 + rho));
    let centre = if edge % 2 == 1 {
        // Far from the start the centre drifts with `x = 1 / s_m - 1`, both ways by how far `rho` is from 1.
        let spread = row * (1.0 - row);
        let drift_row = spread * lean * (0.75 - 0.25 * rho);
        let drift_place = 2.0 * spread * lean * lean;
        // On a box whose top and bottom converge too (after a distort), `side` = ln of the opposite side's weight over
        // the dragged side's: the row is that of `rho e^(side + side² - 0.31 side x)` and 1/s grows by
        // `e^(-0.41 side + 0.87 side² + 1.23 side ln rho + 1.90 side² ln rho)` (fitted on nine such boxes; zero, and
        // the law unchanged, where top and bottom are parallel).
        let side = (apply(&h, unit(0.0, 0.5, 1.0))[2] / apply(&h, unit(1.0, 0.5, 1.0))[2]).ln();
        let side = if side.is_finite() { side } else { 0.0 };
        let turned = rho * (side + side * side - 0.31 * side * x).exp();
        let row = turned * turned / (1.0 + turned * turned);
        let grow = (-0.41 * side + 0.87 * side * side + 1.23 * side * lean + 1.90 * side * side * lean).exp();
        // Past the opposite edge (x < -1, the middle flipped) the factor levels off at place * e^(-ln² rho / 2) and the
        // row drifts further with 1 / s_m = x + 1 (measured on five boxes, rho 0.5 to 1.66).
        // A shrink (x > 0) bends 1/s to `place * x / (1 - kappa x)`, kappa at least the linear law's slope and larger
        // on boxes in strong perspective (measured on six boxes, rho 0.5 to 2.5). Homogeneous `(w, row * lam, lam)`.
        let (w, lam, row) = if x < -1.0 {
            let extra = drift_row * lean * lean * (0.8 - 0.3 * lean);
            (1.0, place * (-lean * lean / 2.0).exp() * x * grow, row + drift_row * x + extra * (x + 1.0))
        } else if x > 0.0 {
            let strong = if lean > 0.0 { 0.3734 - 0.179 * lean } else { 0.4215 - 0.1735 * lean };
            let kappa = (drift_place / place).max(lean * lean * strong);
            (1.0 - kappa * x, place * x * grow, row + drift_row * x)
        } else {
            (1.0, (place + drift_place * x) * x * grow, row + drift_row * x)
        };
        apply(&h, unit(w, row * lam, lam))
    } else if edge == 2 {
        // A bottom drag, in the start box's frame: the centre's row sits at `frac` of the middle row in document
        // units and drifts with `x` by a term odd in ln rho; its 1/s bends away from the side drags' `place * x` as x
        // grows (fitted on the reference with boxes in perspective from rho 0.26 to 2.5).
        let (frac, bend) = if rho <= 1.0 {
            (0.5 - (1.0 - rho) * (1.0 - rho) / 4.0, lean * lean * (0.78 - 0.19 * lean))
        } else {
            let k = 1.0 - 1.0 / rho;
            (0.5 + 0.47 * k * k, lean * lean * (0.80 - 0.12 * lean))
        };
        let t = frac * rho / (1.0 - frac + frac * rho) + 0.25 * lean / (1.0 + 0.25 * lean * lean) * x;
        // On a box whose sides converge too, `sides` = ln of the top's weight over the bottom's: the centre moves
        // toward where the sides meet, its homogeneous parts shifted by terms in `sides` and ln rho, proportional to
        // x (fitted on nine such boxes; zero, and the law unchanged, where the sides are parallel).
        let sides = (apply(&h, unit(0.0, 0.5, 1.0))[2] / apply(&h, unit(1.0, 0.5, 1.0))[2]).ln();
        let sides = if sides.is_finite() { sides } else { 0.0 };
        let terms = [sides, sides * sides, sides * lean, sides * sides * lean];
        let mix = |c: [f64; 4]| c.iter().zip(terms).map(|(a, b)| a * b).sum::<f64>();
        // Homogeneous, so a centre passing through infinity (1/s = 0) stays defined.
        let lam = place * x + mix([-0.8687, -0.0958, 1.793, 3.4711]) * x;
        let along = t * place * x + mix([-0.3803, -0.2691, -1.2324, 0.3666]) * x;
        apply(&h, unit(1.0 - bend * x, along, lam))
    } else {
        let inv_s = (1.0 + r2 * rho) / (1.0 + r2) * (1.0 - across);
        apply(&Homography::unit_to_quad(first)?, unit(1.0, row * inv_s, inv_s))
    };
    Some(centre)
}

/// The reference levels a side beside the dragged edge that a drag in perspective leaves less than 1 px from level (or
/// plumb), if it was level (or plumb) at the press: both its corners go to their mean.
fn level_snap(quad: &mut [Point; 4], start: &[Point; 4], sides: [(usize, usize); 2]) {
    for (a, b) in sides {
        if start[a].y == start[b].y && (quad[b].y - quad[a].y).abs() < 1.0 {
            let y = (quad[a].y + quad[b].y) / 2.0;
            (quad[a].y, quad[b].y) = (y, y);
        }
        if start[a].x == start[b].x && (quad[b].x - quad[a].x).abs() < 1.0 {
            let x = (quad[a].x + quad[b].x) / 2.0;
            (quad[a].x, quad[b].x) = (x, x);
        }
    }
}

/// How far an ⌥ drag's dragged edge is from the reference point `c`, measured along `axis`: the reference goes from
/// `c` along `dir` (the mean direction of the two edges beside the dragged one) to the dragged edge's line through
/// `a` and `b`. For a centred reference point on a box with straight edges beside it, that is the edge's middle; where
/// `dir` runs along the edge, the edge's middle.
fn reach_to(c: Point, dir: Vec2, axis: Vec2, a: Point, b: Point) -> f64 {
    let along = b - a;
    let den = dir.cross(along);
    if den.abs() > 1e-9 * along.hypot() * dir.hypot() { (a - c).cross(along) / den * dir.dot(axis) } else { (a.midpoint(b) - c).dot(axis) }
}

/// The mean direction, as a unit vector, of the edges from `a0` to `a1` and from `b0` to `b1` (each normalised first);
/// zero for a degenerate pair.
fn across(a0: Point, a1: Point, b0: Point, b1: Point) -> Vec2 {
    let (a, b) = (a1 - a0, b1 - b0);
    let (la, lb) = (a.hypot(), b.hypot());
    if !(la > 1e-12 && lb > 1e-12) {
        return Vec2::ZERO;
    }
    let d = a / la + b / lb;
    let l = d.hypot();
    if l > 1e-12 { d / l } else { Vec2::ZERO }
}

/// The mean of a quad's corners.
fn mean_of(q: [Point; 4]) -> Point {
    Point::new((q[0].x + q[1].x + q[2].x + q[3].x) / 4.0, (q[0].y + q[1].y + q[2].y + q[3].y) / 4.0)
}

/// The cross product of two homogeneous points or lines: the line through two points, or where two lines meet.
fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Where the line through `p` along `dp` crosses the one through `q` along `dq`; `None` for (nearly) parallel lines.
fn crossing(p: Point, dp: Vec2, q: Point, dq: Vec2) -> Option<Point> {
    let den = dp.cross(dq);
    if den.abs() <= 1e-9 * dp.hypot() * dq.hypot() {
        return None;
    }
    let at = p + dp * ((q - p).cross(dq) / den);
    at.is_finite().then_some(at)
}

/// The smallest factor a scale drag may give `quad`'s `axis` (0: u, 1: v) so its sides along that
/// axis keep `min_side`: `min_side` over the shorter of the two (for u the edges 0–1 and 3–2, for
/// v 1–2 and 0–3), since the shorter one reaches the minimum first. At most 1, so a side already
/// shorter than the minimum keeps its length and a click changes nothing; −∞ with no minimum.
fn scale_floor(quad: &[Point; 4], axis: usize, min_side: f64) -> f64 {
    if min_side > 0.0 {
        let ((a0, a1), (b0, b1)) = if axis == 0 { ((0, 1), (3, 2)) } else { ((1, 2), (0, 3)) };
        let (a, b) = ((quad[a1] - quad[a0]).hypot(), (quad[b1] - quad[b0]).hypot());
        // A side collapsed to a point can never reach the minimum: the other side stands in.
        let side = if a.min(b) > 1e-12 { a.min(b) } else { a.max(b) };
        if side > 1e-12 {
            return (min_side / side).min(1.0);
        }
    }
    f64::NEG_INFINITY
}

/// Whether every side of `quad` keeps `min_side`, or its length in `quad0` where that was shorter already (the floor
/// [`scale_floor`] gives a side); always with no minimum.
fn keeps_min_side(quad0: &[Point; 4], quad: &[Point; 4], min_side: f64) -> bool {
    min_side <= 0.0
        || (0..4).all(|k| {
            let side = |q: &[Point; 4]| (q[(k + 1) % 4] - q[k]).hypot();
            side(quad) >= min_side.min(side(quad0)) - 1e-9
        })
}

/// The turn at each corner of a quad: all of one sign for a convex quad, mixed for a concave or
/// self-intersecting one.
fn turns(q: &[Point; 4]) -> [f64; 4] {
    std::array::from_fn(|k| (q[(k + 1) % 4] - q[k]).cross(q[(k + 2) % 4] - q[(k + 1) % 4]))
}

/// Whether `q` is convex with the orientation `sign`, every corner turning by more than `min_turn`.
fn convex(q: &[Point; 4], sign: f64, min_turn: f64) -> bool {
    turns(q).iter().all(|t| t * sign > min_turn)
}

/// A Skew corner slide stops the corner where the box would turn concave or fold over itself:
/// the result goes from `from` towards `to` as far as the quad stays convex. A box that is not
/// convex to begin with is left free.
fn keep_convex(from: [Point; 4], to: [Point; 4]) -> [Point; 4] {
    let t0 = turns(&from);
    let sign = t0[0].signum();
    // A thousandth of the starting box's smallest turn, so the stop is short of flat.
    let min_turn = t0.iter().map(|t| t.abs()).fold(f64::INFINITY, f64::min) * 1e-3;
    if !(min_turn.is_finite() && min_turn > 0.0 && convex(&from, sign, 0.0)) || convex(&to, sign, min_turn) {
        return to;
    }
    let lerp = |t: f64| -> [Point; 4] { std::array::from_fn(|k| from[k].lerp(to[k], t)) };
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if convex(&lerp(mid), sign, min_turn) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lerp(lo)
}

/// Where the reference point lands when the box goes from `quad0` to `quad`: the same place within
/// the box, read bilinearly between the corners (the reference's: a centred reference point stays
/// at the corners' mean on a box in perspective; on a parallelogram this is the affine place). A
/// box that has collapsed leaves it where it was.
fn carry_pivot(quad0: [Point; 4], quad: [Point; 4], pivot0: Point) -> Point {
    let (Some(h0), Some(h1)) = (Homography::unit_to_quad(quad0), Homography::unit_to_quad(quad)) else { return pivot0 };
    let Some(inv) = h0.inverse() else { return pivot0 };
    let projective = inv.apply(pivot0);
    // A reference point with no place read between the corners (far outside a box in perspective) keeps its
    // projective place, through the projective map: a drag back to the press then puts it back.
    let p = match bilinear_inverse(quad0, pivot0, projective) {
        Some(at) => bilinear(quad, at),
        None => h1.apply(projective),
    };
    if p.is_finite() {
        p
    } else {
        let p = h1.apply(projective);
        if p.is_finite() { p } else { pivot0 }
    }
}

/// The point at `(u, v)` of the unit square spread bilinearly over `q`'s corners.
fn bilinear(q: [Point; 4], at: Point) -> Point {
    let (u, v) = (at.x, at.y);
    let top = q[0].lerp(q[1], u);
    let bottom = q[3].lerp(q[2], u);
    top.lerp(bottom, v)
}

/// The `(u, v)` that [`bilinear`] takes to `p` on `q`, by Newton's method from `guess`; `None` where it does not
/// settle (a folded box).
fn bilinear_inverse(q: [Point; 4], p: Point, guess: Point) -> Option<Point> {
    let mut at = if guess.is_finite() { guess } else { Point::new(0.5, 0.5) };
    for _ in 0..16 {
        let (u, v) = (at.x, at.y);
        let r = bilinear(q, at) - p;
        if r.hypot() < 1e-10 {
            return Some(at);
        }
        let du = (q[1] - q[0]) * (1.0 - v) + (q[2] - q[3]) * v;
        let dv = (q[3] - q[0]) * (1.0 - u) + (q[2] - q[1]) * u;
        let det = du.cross(dv);
        if !(det.is_finite() && det.abs() > 1e-12) {
            return None;
        }
        at = Point::new(u - r.cross(dv) / det, v - du.cross(r) / det);
        if !at.is_finite() {
            return None;
        }
    }
    (bilinear(q, at).distance(p) < 1e-6).then_some(at)
}

/// `v`, kept off zero with its sign: a divisor for a W or H scale that may be negative.
fn nonzero(v: f64) -> f64 {
    if v.abs() < 1e-9 { 1e-9f64.copysign(v) } else { v }
}

/// A box read as the options bar shows it: the starting rect scaled by `w` and `h`, skewed by
/// `skew_h` and `skew_v` and turned by `angle`, about the reference point. Scales are factors (1
/// is 100 %; negative for a flipped side); angles are degrees, positive clockwise on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Parts {
    /// Horizontal scale factor.
    pub w: f64,
    /// Vertical scale factor.
    pub h: f64,
    /// Rotation in degrees, clockwise on screen.
    pub angle: f64,
    /// The sides' lean.
    pub skew_h: f64,
    /// The top and bottom's lean. Any parallelogram can be read with any V skew short of ±90°:
    /// the host keeps the one typed into the V field.
    pub skew_v: f64,
}

impl Parts {
    /// The readings: scale factors `w` and `h`, the `angle` in degrees and the two skews in degrees.
    pub const fn new(w: f64, h: f64, angle: f64, skew_h: f64, skew_v: f64) -> Self {
        Parts { w, h, angle, skew_h, skew_v }
    }
}

/// [`Parts`] in radians with the raw signs: `sy` is negative for a box flipped relative to its
/// rotation, whatever the bar shows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    sx: f64,
    sy: f64,
    angle: f64,
    skew_h: f64,
    skew_v: f64,
}

impl Frame {
    fn from_columns([c0, c1]: [Vec2; 2], skew_v: f64) -> Frame {
        // X edge = sx · R(angle) · (1, tan v): its direction is the angle plus v.
        let angle = c0.atan2() - skew_v;
        let sx = c0.hypot() * skew_v.cos();
        // Y edge = sy · R(angle) · (−tan h, 1).
        let g = unturn(c1, angle);
        let skew_h = if g.y.abs() < 1e-12 { 0.0 } else { (-g.x / g.y).atan() };
        Frame { sx, sy: g.y, angle, skew_h, skew_v }
    }

    fn columns(self) -> [Vec2; 2] {
        let turn = |v: Vec2| Vec2::from_angle(self.angle) * v.x + Vec2::from_angle(self.angle + std::f64::consts::FRAC_PI_2) * v.y;
        [turn(Vec2::new(self.sx, self.sx * self.skew_v.tan())), turn(Vec2::new(-self.sy * self.skew_h.tan(), self.sy))]
    }
}

/// `c` turned back by `angle` (radians).
fn unturn(c: Vec2, angle: f64) -> Vec2 {
    let (sn, cs) = angle.sin_cos();
    Vec2::new(c.x * cs + c.y * sn, -c.x * sn + c.y * cs)
}

impl BoxState {
    /// The reference point's place within the box, carried to `rect`: the pivot through the
    /// inverse homography onto the unit square, then onto `rect` (the relative X and Y fields'
    /// origin). A flattened box with no inverse is read through the affine map that fits its
    /// four corners best; an empty `rect` gives its origin.
    pub fn origin_of_pivot(&self, rect: Rect) -> Point {
        if rect.is_zero_area() {
            return rect.origin();
        }
        let unit = Homography::unit_to_quad(self.quad)
            .and_then(|h| h.inverse())
            .map(|inv| inv.apply(self.pivot))
            .filter(|u| u.is_finite())
            .unwrap_or_else(|| self.unit_by_least_squares());
        Point::new(rect.x0 + unit.x * rect.width(), rect.y0 + unit.y * rect.height())
    }

    /// The pivot in the unit square through the least-squares affine fit of the unit square's
    /// corners to the box's, solved with the pseudo-inverse where the fit is singular.
    fn unit_by_least_squares(&self) -> Point {
        let q = self.quad.map(|c| c.to_vec2());
        let centre = (q[0] + q[1] + q[2] + q[3]) / 4.0;
        // The unit corners centred on (0.5, 0.5) are (±0.5, ±0.5), whose scatter is the identity,
        // so the fit's columns are these corner sums.
        let cu = ((q[1] + q[2]) - (q[0] + q[3])) / 2.0;
        let cv = ((q[2] + q[3]) - (q[0] + q[1])) / 2.0;
        let r = self.pivot.to_vec2() - centre;
        let det = cu.cross(cv);
        let norm2 = cu.hypot2() + cv.hypot2();
        let (du, dv) = if det.abs() > 1e-12 * norm2 {
            (r.cross(cv) / det, cu.cross(r) / det)
        } else if norm2 > 1e-24 {
            // Rank one: the pseudo-inverse is the transpose over the squared norm.
            (cu.dot(r) / norm2, cv.dot(r) / norm2)
        } else {
            (0.0, 0.0)
        };
        let unit = Point::new(0.5 + du, 0.5 + dv);
        if unit.is_finite() { unit } else { Point::new(0.5, 0.5) }
    }

    /// A box with the corners `quad`, clockwise from the top left, and the reference point `pivot`.
    pub const fn new(quad: [Point; 4], pivot: Point) -> Self {
        BoxState { quad, pivot }
    }

    /// The box of `r`, the reference point at its centre.
    pub fn from_rect(r: Rect) -> Self {
        BoxState { quad: [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], pivot: r.center() }
    }

    /// The box and its reference point moved by `by`.
    pub fn moved(&self, by: Vec2) -> Self {
        BoxState { quad: self.quad.map(|q| q + by), pivot: self.pivot + by }
    }

    /// Whether the box is still an affine image of its rect (opposite sides parallel), so W and H
    /// describe it; a distorted or perspective box is not.
    pub fn is_parallelogram(&self) -> bool {
        let q = &self.quad;
        let scale = q.iter().flat_map(|p| [p.x, p.y]).fold(1.0f64, |m, v| m.max(v.abs()));
        let d = (q[0].to_vec2() + q[2].to_vec2()) - (q[1].to_vec2() + q[3].to_vec2());
        d.x.abs() <= scale * 1e-9 && d.y.abs() <= scale * 1e-9
    }

    /// The box read as the options bar shows it, `rect` being the box before the transform and
    /// `skew_v` the V skew the host keeps (degrees). A mirrored box reads as a negative W or H,
    /// whichever keeps the angle nearer 0 (Flip Horizontal is W −100 % at 0°); flipped both ways
    /// is a half turn.
    pub fn parts(&self, rect: Rect, skew_v: f64) -> Parts {
        let f = self.frame(rect, skew_v.to_radians());
        let wrap = |a: f64| {
            let a = a.rem_euclid(std::f64::consts::TAU);
            if a > std::f64::consts::PI { a - std::f64::consts::TAU } else { a }
        };
        let turned = wrap(f.angle + std::f64::consts::PI);
        let (w, h, angle) = if f.sy < 0.0 && turned.abs() < wrap(f.angle).abs() { (-f.sx, -f.sy, turned) } else { (f.sx, f.sy, f.angle) };
        Parts { w, h, angle: angle.to_degrees(), skew_h: f.skew_h.to_degrees(), skew_v }
    }

    /// The box with the options bar's fields set to `to`: W and H scale it along its own axes, the
    /// angle turns it and the skews lean it, each about the reference point, which stays put.
    /// `skew_v` is the V skew the box reads with now ([`parts`](Self::parts)); fields equal to the
    /// current reading change nothing. Skews stop at ±[`SKEW_LIMIT`].
    pub fn with_parts(&self, rect: Rect, skew_v: f64, mut to: Parts) -> Self {
        // Mirrored along both axes is a half turn, which is how `parts` reads it.
        if to.w < 0.0 && to.h < 0.0 {
            let turned = to.angle + 180.0;
            (to.w, to.h, to.angle) = (-to.w, -to.h, if turned > 180.0 { turned - 360.0 } else { turned });
        }
        let mut s = *self;
        let now = s.parts(rect, skew_v);
        if to.w != now.w || to.h != now.h {
            s = s.scaled_about_pivot(to.w / nonzero(now.w), to.h / nonzero(now.h));
            // A distorted box scaled through its homography keeps its perspective, so its affine
            // reading moves with the corners. When only W and H were asked to change, that drift
            // is not an angle or skew edit and must not be undone.
            let same_sides = (to.w < 0.0) == (now.w < 0.0) && (to.h < 0.0) == (now.h < 0.0);
            if !self.is_parallelogram() && same_sides && to.angle == now.angle && to.skew_h == now.skew_h && to.skew_v == skew_v {
                return s;
            }
        }
        let now = s.parts(rect, skew_v);
        if to.angle != now.angle {
            s = s.turned_about_pivot((to.angle - now.angle).to_radians());
        }
        let now = s.parts(rect, skew_v);
        if to.skew_h != now.skew_h || to.skew_v != skew_v {
            s = s.skewed_about_pivot(rect, skew_v, to.skew_h, to.skew_v);
        }
        s
    }

    /// The signed factors `(w, h)` after the side on `axis` (0: W, 1: H) is set to `typed` with W
    /// and H linked, from the factors `now`. The typed side takes `typed` with its sign; the other
    /// scales by the same ratio's magnitude and keeps its sign, so a negative `typed` mirrors the
    /// typed side where scaling both by it would turn the box a half turn.
    pub fn linked_scale(now: (f64, f64), axis: usize, typed: f64) -> (f64, f64) {
        let (side, other) = if axis == 0 { (now.0, now.1) } else { (now.1, now.0) };
        let k = typed / nonzero(side);
        let other = other * k.abs();
        if axis == 0 { (typed, other) } else { (other, typed) }
    }

    /// One of the box's nine points, through the box's own frame (a distorted box's centre is
    /// where its diagonals' frame puts it).
    pub fn point_at(&self, at: Anchor9) -> Point {
        let (u, v) = at.uv();
        if let Some(h) = Homography::unit_to_quad(self.quad) {
            let p = h.apply(Point::new(u, v));
            if p.is_finite() {
                return p;
            }
        }
        // A collapsed box: the corners' averages.
        let q = &self.quad;
        let top = q[0].lerp(q[1], u);
        let bottom = q[3].lerp(q[2], u);
        top.lerp(bottom, v)
    }

    /// The point of the nine the reference point is on, within 0.05 document units; `None` for a
    /// reference point placed elsewhere.
    pub fn anchor_of_pivot(&self) -> Option<Anchor9> {
        Anchor9::ALL.into_iter().find(|&a| {
            let p = self.point_at(a);
            (p.x - self.pivot.x).abs() <= 0.05 && (p.y - self.pivot.y).abs() <= 0.05
        })
    }

    /// The part at `p`: the nearest of the corners, edge midpoints and reference point within
    /// `tol` (on a small box their reaches overlap), else inside or outside the box. In whatever
    /// space the box is in: overlays test a box mapped to the screen.
    pub fn part_at(&self, p: Point, tol: f64) -> BoxPart {
        let q = &self.quad;
        let corners = (0..4u8).map(|i| (q[usize::from(i)], BoxPart::Corner(i)));
        let edges = (0..4u8).map(|i| (q[usize::from(i)].midpoint(q[(usize::from(i) + 1) % 4]), BoxPart::Edge(i)));
        let nearest = corners
            .chain(edges)
            .chain([(self.pivot, BoxPart::Pivot)])
            .map(|(at, part)| (at.distance(p), part))
            .filter(|(d, _)| *d < tol)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        match nearest {
            Some((_, part)) => part,
            None if self.contains(p) => BoxPart::Inside,
            None => BoxPart::Outside,
        }
    }

    /// Whether `p` is inside the box (even-odd, so a folded box has holes).
    pub fn contains(&self, p: Point) -> bool {
        let q = &self.quad;
        let mut inside = false;
        for i in 0..4 {
            let (a, b) = (q[i], q[(i + 3) % 4]);
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
        }
        inside
    }

    /// The direction of the top edge in degrees, clockwise in document space (y down): the rotate
    /// readout. 0 for a collapsed top edge.
    pub fn angle(&self) -> f64 {
        let d = self.quad[1] - self.quad[0];
        if d.is_finite() && d.hypot() > 1e-9 { d.atan2().to_degrees() } else { 0.0 }
    }

    /// The box's columns: where `rect`'s unit X and Y edges went.
    fn columns(&self, rect: Rect) -> Option<[Vec2; 2]> {
        let (w, h) = (rect.width(), rect.height());
        if w.abs() < 1e-9 || h.abs() < 1e-9 {
            return None;
        }
        let q = &self.quad;
        let m = [(q[1] - q[0]) / w, (q[3] - q[0]) / h];
        m.iter().all(|c| c.is_finite()).then_some(m)
    }

    fn frame(&self, rect: Rect, skew_v: f64) -> Frame {
        let none = Frame { sx: 1.0, sy: 1.0, angle: 0.0, skew_h: 0.0, skew_v };
        self.columns(rect).map_or(none, |m| Frame::from_columns(m, skew_v))
    }

    /// Scaled by `kx` and `ky` along the box's own axes about the reference point. A negative
    /// factor mirrors that axis; both negative is a half turn.
    pub fn scaled_about_pivot(&self, kx: f64, ky: f64) -> Self {
        let Some(h) = Homography::unit_to_quad(self.quad) else { return *self };
        let Some(inv) = h.inverse() else { return *self };
        let p = inv.apply(self.pivot);
        let at = |u: f64, v: f64| h.apply(Point::new(p.x + (u - p.x) * kx, p.y + (v - p.y) * ky));
        BoxState { quad: [at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)], pivot: self.pivot }
    }

    /// Turned by `radians` about the reference point.
    pub fn turned_about_pivot(&self, radians: f64) -> Self {
        let turn = Affine::rotate_about(radians, self.pivot);
        BoxState { quad: self.quad.map(|q| turn * q), pivot: self.pivot }
    }

    /// With H and V skews set (degrees), keeping the scale, angle and reference point. `skew_v`
    /// is the V skew the box reads with now.
    fn skewed_about_pivot(&self, rect: Rect, skew_v: f64, to_h: f64, to_v: f64) -> Self {
        let Some(m) = self.columns(rect) else { return *self };
        let (to_h, to_v) = (to_h.clamp(-SKEW_LIMIT, SKEW_LIMIT), to_v.clamp(-SKEW_LIMIT, SKEW_LIMIT));
        let from = Frame::from_columns(m, skew_v.to_radians());
        let to = Frame { skew_h: to_h.to_radians(), skew_v: to_v.to_radians(), ..from }.columns();
        // The map from the old box to the new one: to · m⁻¹.
        let det = m[0].cross(m[1]);
        if det.abs() < 1e-12 {
            return *self;
        }
        let map = |d: Vec2| {
            let (u, v) = (d.cross(m[1]) / det, m[0].cross(d) / det);
            to[0] * u + to[1] * v
        };
        let c = self.pivot;
        let quad = self.quad.map(|q| c + map(q - c));
        if quad.iter().all(|q| q.is_finite()) { BoxState { quad, pivot: c } } else { *self }
    }
}

/// A projective map of the plane: the unit square onto a box, so a box scales along its own axes
/// whatever its shape. Hosts that draw pixels through a box keep their own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography([f64; 9]);

impl Homography {
    /// [`Homography::apply`] for a point on the front side of the horizon, the side most of
    /// `within`'s corners lie on (its centre's side on a two–two split); `None` for a point on
    /// the horizon or behind it. The side is read from `within` rather than from `w > 0` because
    /// [`unit_to_quad`](Self::unit_to_quad) fixes `w = 1` at corner 0, which a dart whose reflex
    /// corner is corner 0 puts behind the horizon.
    pub fn apply_in_front(&self, p: Point, within: Rect) -> Option<Point> {
        let [g, k, i] = self.row(2);
        let w = |q: Point| g * q.x + k * q.y + i;
        let corners =
            [Point::new(within.x0, within.y0), Point::new(within.x1, within.y0), Point::new(within.x1, within.y1), Point::new(within.x0, within.y1)];
        let front = match corners.iter().filter(|&&c| w(c) > 0.0).count() {
            3 | 4 => 1.0,
            0 | 1 => -1.0,
            _ if w(within.center()) >= 0.0 => 1.0,
            _ => -1.0,
        };
        if front * w(p) <= 1e-9 {
            return None;
        }
        let out = self.apply(p);
        out.is_finite().then_some(out)
    }

    /// The map with these rows of (x, y, w) coefficients, applied to (x, y, 1).
    pub fn from_rows(rows: [[f64; 3]; 3]) -> Homography {
        let [[a, b, c], [d, e, f], [g, h, i]] = rows;
        Homography([a, b, c, d, e, f, g, h, i])
    }

    /// Row `i` (0 to 2) of the matrix: the coefficients of x, y and the constant.
    pub fn row(&self, i: usize) -> [f64; 3] {
        [self.0[i * 3], self.0[i * 3 + 1], self.0[i * 3 + 2]]
    }

    /// The map taking (0, 0), (1, 0), (1, 1), (0, 1) to `q`'s corners; `None` for a degenerate
    /// box.
    pub fn unit_to_quad(q: [Point; 4]) -> Option<Homography> {
        let [p0, p1, p2, p3] = q;
        let (sx, sy) = (p0.x - p1.x + p2.x - p3.x, p0.y - p1.y + p2.y - p3.y);
        if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
            return Some(Homography([p1.x - p0.x, p3.x - p0.x, p0.x, p1.y - p0.y, p3.y - p0.y, p0.y, 0.0, 0.0, 1.0]));
        }
        let (dx1, dx2, dy1, dy2) = (p1.x - p2.x, p3.x - p2.x, p1.y - p2.y, p3.y - p2.y);
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-12 {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        Some(Homography([p1.x - p0.x + g * p1.x, p3.x - p0.x + h * p3.x, p0.x, p1.y - p0.y + g * p1.y, p3.y - p0.y + h * p3.y, p0.y, g, h, 1.0]))
    }

    /// `p` of the unit square, carried to the box.
    pub fn apply(&self, p: Point) -> Point {
        let m = &self.0;
        let w = m[6] * p.x + m[7] * p.y + m[8];
        Point::new((m[0] * p.x + m[1] * p.y + m[2]) / w, (m[3] * p.x + m[4] * p.y + m[5]) / w)
    }

    /// The map back from the box to the unit square; `None` when the map is singular.
    pub fn inverse(&self) -> Option<Homography> {
        let m = &self.0;
        let c = [
            m[4] * m[8] - m[5] * m[7],
            m[2] * m[7] - m[1] * m[8],
            m[1] * m[5] - m[2] * m[4],
            m[5] * m[6] - m[3] * m[8],
            m[0] * m[8] - m[2] * m[6],
            m[2] * m[3] - m[0] * m[5],
            m[3] * m[7] - m[4] * m[6],
            m[1] * m[6] - m[0] * m[7],
            m[0] * m[4] - m[1] * m[3],
        ];
        let det = m[0] * c[0] + m[1] * c[3] + m[2] * c[6];
        if det.abs() < 1e-12 {
            return None;
        }
        Some(Homography(c.map(|v| v / det)))
    }
}

/// Corner `i` of `screen`'s unit bisector, pointing into the box; zero for a collapsed corner. The
/// box is in screen points, so an overlay can set a handle off its corner along it.
pub fn bisector(screen: &BoxState, i: usize) -> emath::Vec2 {
    let q = &screen.quad;
    let unit = |to: Point| {
        let d = to - q[i];
        let l = d.hypot();
        if l.is_finite() && l > 1e-9 { d / l } else { Vec2::ZERO }
    };
    let s = unit(q[(i + 1) % 4]) + unit(q[(i + 3) % 4]);
    let l = s.hypot();
    if l.is_finite() && l > 1e-6 { emath::vec2((s.x / l) as f32, (s.y / l) as f32) } else { emath::Vec2::ZERO }
}

/// The closed polygon `corners` moved `by` points outward, each corner along its mitre (kept
/// within four times `by` at a sharp corner). Either winding works.
pub fn outset(corners: &[emath::Pos2; 4], by: f32) -> [emath::Pos2; 4] {
    let area: f32 = (0..4).map(|i| corners[i].x * corners[(i + 1) % 4].y - corners[(i + 1) % 4].x * corners[i].y).sum();
    let side = if area >= 0.0 { 1.0 } else { -1.0 };
    let normal = |a: emath::Pos2, b: emath::Pos2| {
        let d = (b - a).normalized();
        emath::vec2(d.y, -d.x) * side
    };
    std::array::from_fn(|i| {
        let (prev, here, next) = (corners[(i + 3) % 4], corners[i], corners[(i + 1) % 4]);
        let (n1, n2) = (normal(prev, here), normal(here, next));
        let sum = n1 + n2;
        let k = 1.0 + n1.dot(n2);
        let mitre = if k > 1e-3 && sum.is_finite() { sum * (by / k) } else { n1 * by };
        let mitre = if mitre.length() > 4.0 * by { mitre.normalized() * 4.0 * by } else { mitre };
        here + mitre
    })
}
