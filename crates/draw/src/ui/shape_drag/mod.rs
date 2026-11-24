//! The Shape tools on the canvas ([`ShapeDragTool`]: Rectangle, Rounded Rectangle, Ellipse,
//! Triangle, Polygon, Star, Line and Custom Shape) and the Create Shape dialog a click opens
//! ([`CreateShapeDialog`]).
//!
//! The app sets the tool's options each pass, shows it on the canvas's `ToolHost` and drains
//! [`ShapeDragTool::take_events`]: a committed [`Draw`](ShapeDragEvent::Draw) creates the
//! shape, a [`Click`](ShapeDragEvent::Click) opens [`CreateShapeDialog`] for the kinds it
//! [supports](CreateShapeDialog::supports).

mod dialog;
mod tool;
mod words;

use std::ops::RangeInclusive;

use craft_behavior::EditEvent;
use craft_keymap::Mods;
use kurbo::Point;
use serde::{Deserialize, Serialize};

use crate::geometry::shape::{BoxRule, ShapeGeometry};
use crate::ui::shape::VectorMode;

pub use dialog::{CreateShapeDialog, CreateShapeOptions, CreatedShape};
pub use tool::ShapeDragTool;
pub use words::{CreateShapeWords, ShapeDragWords};

/// The Shape tools. Photoshop's Rectangle draws a rounded rectangle when the corner radius is over
/// 0; VectorCraft (Illustrator) has its own Rounded Rectangle and Star.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShapeKind {
    /// A rectangle.
    #[default]
    Rectangle,
    /// A rectangle with rounded corners.
    RoundedRectangle,
    /// An ellipse.
    Ellipse,
    /// A triangle.
    Triangle,
    /// A regular polygon.
    Polygon,
    /// A star.
    Star,
    /// A line.
    Line,
    /// The selected custom shape preset: the tool gives its box and the app fits the preset in it.
    Custom,
}

/// How a drag sizes the shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShapeFrame {
    /// The press is one corner of the shape's box, the pointer the other (Photoshop: every kind).
    #[default]
    Box,
    /// The press is the centre, the pointer sets the radius and the turn (Illustrator's Polygon
    /// and Star, [`RadialFrame`](crate::geometry::shape::RadialFrame)).
    Radial,
}

/// What a Shape tool's gear (the Rectangle's, the Triangle's and the Polygon's) keeps a drag to
/// ([`BoxConstraints::constraint`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum BoxConstraint {
    /// The dragged box.
    #[default]
    Unconstrained,
    /// A square: the Rectangle's Square, the Triangle's Equilateral (its triangle in the square's circle), the
    /// Polygon's Symmetric (the regular polygon in the square's circle); ⇧ draws the dragged box instead.
    Square,
    /// The gear's width and height, with the top left at the release.
    FixedSize,
    /// The largest box of the gear's width to height ratio inside the drag, from the press.
    Proportional,
    /// The Polygon's Freeform: the regular polygon centred on the middle of the drag (on the press with From Center),
    /// its first corner at the release ([`RadialFrame::freeform`](crate::geometry::shape::RadialFrame::freeform)), its
    /// circumradius the gear's [`radius`](BoxConstraints::radius) when set. The other kinds draw it as Unconstrained.
    Freeform,
}

/// A Shape tool's gear (the Rectangle's, the Triangle's or the Polygon's, each its own value): a constraint, its sizes,
/// Freeform's radius, and From Center. A Fixed Size or Proportional pair with an empty field (`None`) or a 0 draws as
/// Unconstrained.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct BoxConstraints {
    /// The constraint chosen.
    pub constraint: BoxConstraint,
    /// Fixed Size's width and height in pixels, in [`FIXED`](Self::FIXED); empty to start.
    pub fixed: (Option<f64>, Option<f64>),
    /// Proportional's width and height, in [`RATIO`](Self::RATIO); 1 : 1 to start.
    pub ratio: (Option<f64>, Option<f64>),
    /// Freeform's Radius in pixels, in [`RADIUS`](Self::RADIUS): the polygon's circumradius. Empty (or 0) lets the drag
    /// set it; empty to start.
    pub radius: Option<f64>,
    /// The press is the box's centre; ⌥ draws from the corner instead.
    pub from_center: bool,
}

impl BoxConstraints {
    /// The range of a Fixed Size field, in whole pixels.
    pub const FIXED: RangeInclusive<f64> = 0.0..=300_000.0;
    /// The range of a Proportional field.
    pub const RATIO: RangeInclusive<f64> = 0.0..=99_999.0;
    /// The range of Freeform's Radius field, in whole pixels.
    pub const RADIUS: RangeInclusive<f64> = 0.0..=300_000.0;

    /// The rule of a box drag under these constraints with ⇧ (`shift`) and ⌥ (`alt`) held as they are now: ⇧ turns
    /// Square on, or off when it is chosen, and ⌥ does the same to From Center; Fixed Size and Proportional take their
    /// pair when both fields hold a value. The box lands on whole pixels; the drag sets
    /// [`keep_fractions`](BoxRule::keep_fractions) from its own options.
    pub fn rule(&self, shift: bool, alt: bool) -> BoxRule {
        let pair = |(w, h): (Option<f64>, Option<f64>)| w.zip(h);
        BoxRule {
            square: shift != (self.constraint == BoxConstraint::Square),
            from_centre: alt != self.from_center,
            fixed: (self.constraint == BoxConstraint::FixedSize).then(|| pair(self.fixed)).flatten(),
            ratio: (self.constraint == BoxConstraint::Proportional).then(|| pair(self.ratio)).flatten(),
            keep_fractions: false,
        }
    }
}

impl Default for BoxConstraints {
    /// Unconstrained, Fixed Size empty, Proportional 1 : 1, Freeform's Radius empty, From Center off.
    fn default() -> Self {
        BoxConstraints {
            constraint: BoxConstraint::Unconstrained,
            fixed: (None, None),
            ratio: (Some(1.0), Some(1.0)),
            radius: None,
            from_center: false,
        }
    }
}

/// The tool options a drag reads each pass: the kind from the selected tool, the rest from the
/// options bar (PhotoCraft's `ToolOptions`). [`ShapeToolSettings::drag_options`](super::shape::ShapeToolSettings::drag_options)
/// builds it from the stored settings.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShapeDragOptions {
    /// Which shape the drag draws.
    pub kind: ShapeKind,
    /// The Rectangle's corner radius; over 0 draws four equal radii.
    pub corner_radius: f64,
    /// The polygon's sides or the star's points, in [`SIDES`](crate::geometry::shape::SIDES).
    pub sides: u32,
    /// The star's inner over outer radius, in [`STAR_RATIO`](crate::geometry::shape::STAR_RATIO).
    pub star_ratio: f64,
    /// The Line's weight, at least 1.
    pub line_weight: f64,
    /// The gear's constraints for a box drag; ⇧ and ⌥ turn its Square and From Center off.
    pub constraints: BoxConstraints,
    /// What the drag makes: a Rectangle in Shape mode ends a click within 1 screen point, every other drag within
    /// the press slop.
    pub mode: VectorMode,
    /// A box drag lands on whole pixels (Photoshop's Align Edges, on by default); off, the box keeps the drag's
    /// fractions. Lines and the radial frame are not boxes and ignore it.
    pub whole_pixels: bool,
}

impl ShapeDragOptions {
    /// The defaults drawing `kind`.
    pub fn new(kind: ShapeKind) -> Self {
        ShapeDragOptions { kind, ..ShapeDragOptions::default() }
    }

    /// The Rectangle's corner radius.
    #[must_use]
    pub fn corner_radius(mut self, radius: f64) -> Self {
        self.corner_radius = radius;
        self
    }

    /// The polygon's sides or the star's points.
    #[must_use]
    pub fn sides(mut self, sides: u32) -> Self {
        self.sides = sides;
        self
    }

    /// The star's inner over outer radius.
    #[must_use]
    pub fn star_ratio(mut self, ratio: f64) -> Self {
        self.star_ratio = ratio;
        self
    }

    /// The Line's weight.
    #[must_use]
    pub fn line_weight(mut self, weight: f64) -> Self {
        self.line_weight = weight;
        self
    }

    /// The gear's constraints for a drag (the Rectangle's, the Triangle's or the Polygon's gear; the app passes the
    /// selected tool's).
    #[must_use]
    pub fn constraints(mut self, constraints: BoxConstraints) -> Self {
        self.constraints = constraints;
        self
    }

    /// What the drag makes (the options bar's mode).
    #[must_use]
    pub fn mode(mut self, mode: VectorMode) -> Self {
        self.mode = mode;
        self
    }

    /// Whether a box drag lands on whole pixels (the options bar's Align Edges).
    #[must_use]
    pub fn whole_pixels(mut self, whole: bool) -> Self {
        self.whole_pixels = whole;
        self
    }
}

impl Default for ShapeDragOptions {
    /// Rectangle, square corners, 5 sides, a plain polygon, weight 3, unconstrained, Shape mode, whole pixels
    /// (PhotoCraft's defaults).
    fn default() -> Self {
        ShapeDragOptions {
            kind: ShapeKind::Rectangle,
            corner_radius: 0.0,
            sides: 5,
            star_ratio: 1.0,
            line_weight: 3.0,
            constraints: BoxConstraints::default(),
            mode: VectorMode::Shape,
            whole_pixels: true,
        }
    }
}

/// Which product's drag rules the tool follows. Photoshop's are the default; VectorCraft uses
/// [`ILLUSTRATOR`](Self::ILLUSTRATOR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ShapeDragRules {
    /// Polygon and Star use the [`Radial`](ShapeFrame::Radial) frame.
    pub radial_polygons: bool,
    /// ↑ and ↓ during a Polygon or Star drag add or remove a side or point (3 to 100).
    pub arrow_sides: bool,
    /// Ctrl/Cmd held during a Star drag keeps the inner radius while the outer follows.
    pub hold_inner: bool,
    /// ⌥ during a Star drag of five points or more makes the shoulders straight (and no longer
    /// draws from the centre: the radial frame already does).
    pub straight_shoulders: bool,
}

impl ShapeDragRules {
    /// Photoshop's (and PhotoCraft's): every kind in the Box frame, no arrow or hold rules.
    pub const PHOTOSHOP: ShapeDragRules = ShapeDragRules { radial_polygons: false, arrow_sides: false, hold_inner: false, straight_shoulders: false };
    /// Illustrator's (VectorCraft's): Polygon and Star radial, ↑/↓ change the count, Ctrl/Cmd holds
    /// the inner radius, ⌥ straightens the shoulders.
    pub const ILLUSTRATOR: ShapeDragRules = ShapeDragRules { radial_polygons: true, arrow_sides: true, hold_inner: true, straight_shoulders: true };

    /// The frame `kind` is drawn in under these rules.
    pub fn frame(self, kind: ShapeKind) -> ShapeFrame {
        match kind {
            ShapeKind::Polygon | ShapeKind::Star if self.radial_polygons => ShapeFrame::Radial,
            _ => ShapeFrame::Box,
        }
    }
}

impl Default for ShapeDragRules {
    fn default() -> Self {
        ShapeDragRules::PHOTOSHOP
    }
}

/// What [`ShapeDragTool`] reports, drained with [`ShapeDragTool::take_events`] after
/// `ToolHost::show`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ShapeDragEvent {
    /// The drag: `Begin` once the pointer is past the click slop, `Update` each pass with the
    /// shape it would draw, `Commit` on release, `Cancel` on Esc or a tool switch, or on release when a side is
    /// under 1 px (`Dismissed`, so the app makes nothing).
    Draw(EditEvent<ShapeGeometry>),
    /// A press and release within the click slop, or Enter with the canvas focused (at the view
    /// centre): the app opens [`CreateShapeDialog`] for kinds it [supports](CreateShapeDialog::supports).
    #[non_exhaustive]
    Click {
        /// The click, in document space.
        at: Point,
        /// The tool's shape kind.
        kind: ShapeKind,
        /// Modifier keys held.
        mods: Mods,
    },
}
