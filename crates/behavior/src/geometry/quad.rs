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
    /// Corners and edges turn the box about the reference point.
    Rotate,
    /// A corner moves alone along one of its edges; an edge slides along itself. No rotating.
    Skew,
    /// A corner moves alone; edges and the outside do nothing.
    Distort,
    /// A corner moves along one axis and its partner on that side the opposite way.
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
}

impl BoxAllow {
    /// What the content allows: turning, skewing and distorting.
    pub const fn new(rotate: bool, skew: bool, distort: bool) -> Self {
        BoxAllow { rotate, skew, distort }
    }
}

impl Default for BoxAllow {
    fn default() -> Self {
        BoxAllow { rotate: true, skew: true, distort: true }
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
    /// Turns the box about its reference point (⇧ snaps the angle).
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
        (TransformMode::Skew, BoxPart::Edge(_)) => BoxDrag::Skew,
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
}

impl Default for BoxRules {
    fn default() -> Self {
        BoxRules { mode: TransformMode::Free, scale: ScaleRule::Proportional, aspect_linked: true, allow: BoxAllow::default(), rotate_step: 15.0 }
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

    /// The mode drags follow: a mode the content refuses scales as Free Transform does.
    pub fn effective_mode(&self) -> TransformMode {
        let refused = match self.mode {
            TransformMode::Free => false,
            TransformMode::Rotate => !self.allow.rotate,
            TransformMode::Skew => !self.allow.skew,
            TransformMode::Distort | TransformMode::Perspective => !self.allow.distort,
        };
        if refused { TransformMode::Free } else { self.mode }
    }

    /// Whether a drag of `part` changes the box: in Distort only the corners, the inside and the
    /// reference point; in Skew everything but the outside; the outside only where rotating is
    /// allowed. A click outside a refused zone still applies the box (the overlay's rule).
    pub fn allows(&self, part: BoxPart) -> bool {
        let by_mode = match self.effective_mode() {
            TransformMode::Distort => !matches!(part, BoxPart::Outside | BoxPart::Edge(_)),
            TransformMode::Skew => part != BoxPart::Outside,
            _ => true,
        };
        by_mode && (self.allow.rotate || part != BoxPart::Outside)
    }

    /// The modifiers a drag of `part` acts on, from the keys held: ⌘ is dropped on a corner or
    /// edge the content refuses to distort or skew, and the legacy scale rule
    /// ([`ScaleRule::Free`]) or an unlinked aspect ratio turns ⇧ over on corners and edges. Mode
    /// keys ([`TransformMode::Distort`], [`TransformMode::Perspective`]) are not added here: they
    /// are part of [`drag_kind`](Self::drag_kind). [`drag`] acts on these modifiers.
    fn drag_modifiers(&self, part: BoxPart, mods: Mods) -> Mods {
        let mut mods = mods;
        // ⌘-drags the content refuses scale like plain drags, ⌥ and ⇧ keeping their meaning.
        match part {
            BoxPart::Corner(_) if !self.allow.distort => mods.cmd = false,
            BoxPart::Edge(_) if !self.allow.skew => mods.cmd = false,
            _ => {}
        }
        let legacy = self.scale == ScaleRule::Free;
        legacy_mods(legacy, part, unlinked_mods(!self.aspect_linked, legacy, part, mods))
    }

    /// What a drag of `part` with `mods` held (as the host has them, before
    /// [`drag_modifiers`](Self::drag_modifiers)) does to the box, or `None` where the rules
    /// refuse the part ([`allows`](Self::allows)). [`drag`] acts on this kind.
    pub fn drag_kind(&self, part: BoxPart, mods: Mods) -> Option<BoxDrag> {
        self.allows(part).then(|| resolve_kind(self.effective_mode(), part, self.drag_modifiers(part, mods)))
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
    apply_mode_drag(&g, to, rules.effective_mode(), !rules.allow.distort, mods, rules.rotate_step)
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

/// A drag in `mode`. `parallel`: the content stays a parallelogram (editable type), so a Skew
/// corner slides its whole edge.
fn apply_mode_drag(g: &Gesture, p: Point, mode: TransformMode, parallel: bool, mut mods: Mods, rotate_step: f64) -> BoxState {
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
            apply_drag(&g, p, mods, rotate_step)
        }
        // An edge slides only along itself.
        (BoxDrag::Skew, BoxPart::Edge(_)) if mode == TransformMode::Skew => {
            mods.cmd = true;
            mods.shift = true;
            apply_drag(g, p, mods, rotate_step)
        }
        _ => apply_drag(g, p, mode_mods(mode, g.part, mods), rotate_step),
    }
}

fn apply_drag(g: &Gesture, p: Point, mods: Mods, rotate_step: f64) -> BoxState {
    let d = p - g.start;
    let (q0, c) = (g.quad0, g.pivot0);
    match g.part {
        BoxPart::Inside => {
            let d = if mods.shift { lock_direction(d, Some(g.last), Directions::Eight) } else { d };
            BoxState { quad: q0.map(|q| q + d), pivot: c + d }
        }
        BoxPart::Pivot => BoxState { quad: q0, pivot: p },
        BoxPart::Outside => {
            let a0 = (g.start - c).atan2();
            let a1 = (p - c).atan2();
            let mut da = a1 - a0;
            if mods.shift && rotate_step > 0.0 {
                // Snap the resulting angle, not the turn.
                let base = (q0[1] - q0[0]).atan2();
                let step = rotate_step.to_radians();
                da = ((base + da) / step).round() * step - base;
            }
            let turn = Affine::rotate_about(da, c);
            BoxState { quad: q0.map(|q| turn * q), pivot: c }
        }
        BoxPart::Corner(i) if mods.cmd && mods.alt && mods.shift => {
            // Perspective: the corner moves along the dominant axis and the corner sharing that
            // side moves the opposite way.
            let i = usize::from(i) % 4;
            let horizontal = d.x.abs() >= d.y.abs();
            let j = match (i, horizontal) {
                (0, true) => 1,
                (1, true) => 0,
                (2, true) => 3,
                (3, true) => 2,
                (0, false) => 3,
                (3, false) => 0,
                (1, false) => 2,
                _ => 1,
            };
            let m = if horizontal { Vec2::new(d.x, 0.0) } else { Vec2::new(0.0, d.y) };
            let mut q = q0;
            q[i] = q0[i] + m;
            q[j] = q0[j] - m;
            let quad = keep_convex(q0, q);
            BoxState { quad, pivot: carry_pivot(q0, quad, c) }
        }
        BoxPart::Corner(i) if mods.cmd => {
            // Distort: the corner moves freely, up to where the box would fold over.
            let mut q = q0;
            q[usize::from(i) % 4] += d;
            let quad = keep_convex(q0, q);
            BoxState { quad, pivot: carry_pivot(q0, quad, c) }
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
        BoxPart::Corner(_) | BoxPart::Edge(_) => scale_drag(g, p, mods).unwrap_or(BoxState { quad: q0, pivot: c }),
    }
}

/// A plain corner or edge drag: scaling in the box's own frame (the unit square through the
/// homography), so rotated and skewed boxes scale along their axes. `None` for a collapsed box.
fn scale_drag(g: &Gesture, p: Point, mods: Mods) -> Option<BoxState> {
    let h = Homography::unit_to_quad(g.quad0)?;
    let inv = h.inverse()?;
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
    let corner = matches!(g.part, BoxPart::Corner(_));
    if !corner && !mods.shift {
        // An edge sets its own axis' scale and the other axis follows by the same factor about
        // the middle of the box (⌥: the reference point). Past the opposite edge only the dragged
        // axis flips.
        if mu.0 || mu.1 {
            (av, kv) = (if mods.alt { pv } else { 0.5 }, ku.abs());
        } else {
            (au, ku) = (if mods.alt { pu } else { 0.5 }, kv.abs());
        }
    }
    if corner && !mods.shift {
        let k = ku.abs().max(kv.abs());
        (ku, kv) = (k * ku.signum(), k * kv.signum());
    }
    let r = [au - au * ku, av - av * kv, au + (1.0 - au) * ku, av + (1.0 - av) * kv];
    let at = |a: f64, b: f64| h.apply(Point::new(a, b));
    let quad = [at(r[0], r[1]), at(r[2], r[1]), at(r[2], r[3]), at(r[0], r[3])];
    // ⌥ scales about the reference point, so it stays put; otherwise it rides along.
    let pivot = if mods.alt { g.pivot0 } else { carry_pivot(g.quad0, quad, g.pivot0) };
    Some(BoxState { quad, pivot })
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

/// Distort and Perspective stop a corner where the box would turn concave or fold over itself:
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
/// the box. A box that has collapsed leaves it where it was.
fn carry_pivot(quad0: [Point; 4], quad: [Point; 4], pivot0: Point) -> Point {
    let (Some(h0), Some(h1)) = (Homography::unit_to_quad(quad0), Homography::unit_to_quad(quad)) else { return pivot0 };
    let Some(inv) = h0.inverse() else { return pivot0 };
    let p = h1.apply(inv.apply(pivot0));
    if p.is_finite() { p } else { pivot0 }
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
