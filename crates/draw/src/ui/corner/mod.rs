//! The corner widgets that round a shape's corners on the canvas (F09.02): [`CornerWidgets`].
//!
//! The app turns each selected live shape (a rectangle, a polygon, a star, VectorCraft's corner
//! anchors) into a [`CornerTarget`], shows the widgets, and applies the [`CornerEdit`] they
//! report. The geometry is [`fillet`](crate::geometry::fillet::fillet)'s, so a widget sits on the
//! arc the app's engine draws.

mod geometry;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod style;
mod widgets;
mod words;

use kurbo::Point;

pub use style::CornerStyle;
pub use widgets::{CornerWidgets, CornerWidgetsOutput};
pub use words::CornerWords;

/// How a corner is cut. Only shown and cycled with [`CornerWidgets::kinds`] (VectorCraft).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CornerKind {
    /// A circular arc tangent to both edges.
    #[default]
    Round,
    /// An arc bulging into the shape.
    InvertedRound,
    /// A straight cut between the two tangent points.
    Chamfer,
}

impl CornerKind {
    /// The kind ⌥-click turns this one into: Round, Inverted Round, Chamfer, then Round again.
    pub fn next(self) -> CornerKind {
        // vectorcraft: crates/tools/src/corners.rs:5-6 (⌥-click cycles round, inverted round, chamfer).
        match self {
            CornerKind::Round => CornerKind::InvertedRound,
            CornerKind::InvertedRound => CornerKind::Chamfer,
            CornerKind::Chamfer => CornerKind::Round,
        }
    }
}

/// One corner of a target, in document units: its vertex, its neighbours along the outline (the
/// sharp polygon's, not the rounded one's), its radius now and its kind.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CornerPoint {
    /// The corner's vertex.
    pub vertex: Point,
    /// The previous vertex along the outline.
    pub prev: Point,
    /// The next vertex along the outline.
    pub next: Point,
    /// The current radius, in document units.
    pub radius: f64,
    /// How the corner is cut.
    pub kind: CornerKind,
}

/// A shape whose corners the widgets round: the host's key, its corners in outline order, and
/// whether each corner takes its own radius (`individual`: rectangles, so ⌥ changes one corner)
/// or every corner shares one (polygons, stars: ⌥ is ignored).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CornerTarget {
    /// The app's id for the object.
    pub key: u64,
    /// The object's corners, in outline order.
    pub corners: Vec<CornerPoint>,
    /// Whether each corner has its own radius (else all share one).
    pub individual: bool,
}

/// A corner of a target: the target's key and the corner's index in
/// [`CornerTarget::corners`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct CornerRef {
    /// The [`CornerTarget::key`] of the object.
    pub target: u64,
    /// Index into [`CornerTarget::corners`].
    pub corner: usize,
}

/// A target's radii after an edit: one per corner, in the target's order (equal for a target
/// without `individual`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CornerRadii {
    /// The [`CornerTarget::key`] of the object.
    pub key: u64,
    /// The new radii, in document units, one per corner.
    pub radii: Vec<f64>,
}

/// The operation the widgets report: the new radii of every shown target that changed. The host applies all of them
/// as one history step.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CornerEdit {
    /// The new radii per object.
    pub targets: Vec<CornerRadii>,
}

impl CornerPoint {
    /// A corner at `vertex` between `prev` and `next` with `radius`, cut as `kind`.
    pub fn new(vertex: Point, prev: Point, next: Point, radius: f64, kind: CornerKind) -> Self {
        Self { vertex, prev, next, radius, kind }
    }
}

impl CornerTarget {
    /// The target `key` with `corners` in outline order; `individual` gives each corner its own radius.
    pub fn new(key: u64, corners: Vec<CornerPoint>, individual: bool) -> Self {
        Self { key, corners, individual }
    }
}

impl CornerRef {
    /// Corner `corner` of the target `target`.
    pub fn new(target: u64, corner: usize) -> Self {
        Self { target, corner }
    }
}
