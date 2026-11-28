//! The shape layer's Properties sections ([`ShapePanel`]) and the Shape tools' options bar
//! ([`ShapeBar`]), each field a gesture-aware [`EditEvent<ShapeChange>`](craft_behavior::EditEvent).

use std::ops::RangeInclusive;

use craft_color_ui::ui::Paint;
use serde::{Deserialize, Serialize};

use crate::stroke::StrokeStyle;

use super::shape_drag::{BoxConstraints, ShapeDragOptions, ShapeKind};

pub(crate) mod arrowheads;
pub(crate) mod bar;
pub(crate) mod gear;
pub(crate) mod mode_select;
pub(crate) mod panel;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub(crate) mod stroke_cells;
mod words;

pub use arrowheads::{ArrowheadsOutput, ArrowheadsPopover};
pub use bar::{ShapeBar, ShapeBarOutput};
pub use gear::{BoxConstraintsOutput, BoxConstraintsPopover};
pub use mode_select::VectorModeSelect;
pub use panel::{ChipSetup, ShapePaint, ShapePanel, ShapePanelOutput};
pub use stroke_cells::{StrokeCells, StrokeCellsOutput};
pub use words::ShapeWords;

/// The drawing mode of the vector tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, craft_ui::contract::PlainData)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum VectorMode {
    /// Draws a shape layer.
    #[default]
    Shape,
    /// Draws a path.
    Path,
    /// Draws filled pixels.
    Pixels,
}

/// A line's arrowheads: which ends carry one and its size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct Arrowheads {
    /// An arrowhead at the start.
    pub start: bool,
    /// An arrowhead at the end.
    pub end: bool,
    /// The head's width in document pixels.
    pub width: f64,
    /// The head's length in document pixels.
    pub length: f64,
    /// The head's concavity in percent of its length, in [`ARROW_CONCAVITY`](Self::ARROW_CONCAVITY); below 0 the
    /// back bulges out.
    pub concavity: f64,
}

impl Arrowheads {
    /// The range of the width and length fields, in document pixels (production and engine units).
    pub const ARROW_SIZE: RangeInclusive<f64> = 1.0..=4096.0;
    /// The range of the concavity field, in whole percent.
    pub const ARROW_CONCAVITY: RangeInclusive<f64> = -50.0..=50.0;

    /// No arrowheads at the default size.
    pub const fn new() -> Self {
        Arrowheads { start: false, end: false, width: 5.0, length: 10.0, concavity: 0.0 }
    }
}

impl Default for Arrowheads {
    /// No arrowheads; width 5, length 10, concavity 0.
    fn default() -> Self {
        Arrowheads::new()
    }
}

/// The range of a radius field, in document pixels.
pub const RADIUS_RANGE: RangeInclusive<f64> = 0.0..=150_000.0;
/// The range of a line weight field, in document pixels.
pub const LINE_WEIGHT_RANGE: RangeInclusive<f64> = 1.0..=10_000.0;

/// The selected shape layer's live shape, as the Shape section shows it: plain data the app
/// converts to and from its document. Sizes in document pixels.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LiveShape {
    /// A rectangle: radii top-left, top-right, bottom-right, bottom-left.
    #[non_exhaustive]
    Rect {
        /// Width in document pixels.
        width: f64,
        /// Height in document pixels.
        height: f64,
        /// The four corner radii in document pixels.
        radii: [f64; 4],
    },
    /// An ellipse.
    #[non_exhaustive]
    Ellipse {
        /// Width in document pixels.
        width: f64,
        /// Height in document pixels.
        height: f64,
    },
    /// A polygon or star: one corner radius for every corner.
    #[non_exhaustive]
    Polygon {
        /// Width in document pixels.
        width: f64,
        /// Height in document pixels.
        height: f64,
        /// The corner radius in document pixels.
        corner_radius: f64,
        /// Number of sides, or points of a star.
        sides: u32,
        /// Inner radius over outer radius; 1 for a plain polygon.
        star_ratio: f64,
    },
    /// A triangle: one corner radius for every corner.
    #[non_exhaustive]
    Triangle {
        /// Width in document pixels.
        width: f64,
        /// Height in document pixels.
        height: f64,
        /// The corner radius in document pixels.
        corner_radius: f64,
    },
    /// A line.
    #[non_exhaustive]
    Line {
        /// The line's weight in document pixels.
        weight: f64,
        /// The line's arrowheads, or `None` for a plain line.
        arrows: Option<Arrowheads>,
    },
}

impl LiveShape {
    /// A rectangle of `width` by `height` with the four corner radii `radii` (top left, top right, bottom right,
    /// bottom left).
    pub fn rect(width: f64, height: f64, radii: [f64; 4]) -> Self {
        LiveShape::Rect { width, height, radii }
    }

    /// An ellipse of `width` by `height`.
    pub fn ellipse(width: f64, height: f64) -> Self {
        LiveShape::Ellipse { width, height }
    }

    /// A polygon (a star below a `star_ratio` of 1) of `width` by `height`.
    pub fn polygon(width: f64, height: f64, corner_radius: f64, sides: u32, star_ratio: f64) -> Self {
        LiveShape::Polygon { width, height, corner_radius, sides, star_ratio }
    }

    /// A triangle of `width` by `height` with `corner_radius` at every corner.
    pub fn triangle(width: f64, height: f64, corner_radius: f64) -> Self {
        LiveShape::Triangle { width, height, corner_radius }
    }

    /// A line of `weight`.
    pub fn line(weight: f64) -> Self {
        LiveShape::Line { weight, arrows: None }
    }

    /// A line of `weight` with `arrows`.
    pub fn arrow_line(weight: f64, arrows: Arrowheads) -> Self {
        LiveShape::Line { weight, arrows: Some(arrows) }
    }
}

/// A shape's fill and stroke as the Appearance section and the bar show them: the selected
/// layer's, or the tool defaults when no shape layer is selected (the app passes the right one).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShapeAppearance {
    /// The fill paint.
    pub fill: Paint,
    /// The stroke paint.
    pub stroke: Paint,
    /// Stroke width in pixels; 0 is no stroke.
    pub stroke_width: f64,
    /// The stroke's style (alignment, caps, corners, dashes): the value the Stroke style button
    /// shows and edits. Its `width` is ignored here; [`stroke_width`](Self::stroke_width) is the
    /// width.
    pub stroke_style: StrokeStyle,
    /// The layer is locked (`locks.all` or `.pixels`): every field is disabled.
    pub locked: bool,
}

impl ShapeAppearance {
    /// Fill and stroke with the default stroke style, unlocked.
    pub fn new(fill: Paint, stroke: Paint, stroke_width: f64) -> Self {
        ShapeAppearance { fill, stroke, stroke_width, stroke_style: StrokeStyle::default(), locked: false }
    }

    /// The stroke's style.
    pub fn stroke_style(mut self, style: StrokeStyle) -> Self {
        self.stroke_style = style;
        self
    }

    /// Disables every field.
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }
}

/// One field's change. The app turns it into one document command (and, from the options bar with
/// a shape layer selected, into the tool defaults too: storytold/photocraft#3347).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ShapeChange {
    /// The fill paint. The app also sets its foreground colour from a solid fill (PhotoCraft).
    Fill(Paint),
    /// The options bar's Fill check box (tool defaults only).
    FillOn(bool),
    /// The stroke paint. The app also sets its background colour from a solid stroke. `width` is
    /// `Some(`[`ENABLED_STROKE_WIDTH`](Self::ENABLED_STROKE_WIDTH)`)` when the edit turns the
    /// stroke on from width 0 (the app sets the width with the paint, in one command), else `None`.
    #[non_exhaustive]
    Stroke {
        /// The new stroke paint.
        paint: Paint,
        /// The width to set along with the paint.
        width: Option<f64>,
    },
    /// The stroke width in pixels, 0–288. A step that clamps at 0 is no change.
    StrokeWidth(f64),
    /// The Stroke style button's edit (its `EditEvent<StrokeStyle>`, mapped); `width` as in
    /// [`Stroke`](Self::Stroke).
    #[non_exhaustive]
    StrokeStyle {
        /// The new stroke style.
        style: StrokeStyle,
        /// The width to set along with the style.
        width: Option<f64>,
    },
    /// Width and height of a rect, ellipse or polygon.
    #[non_exhaustive]
    Size {
        /// New width in document pixels.
        width: f64,
        /// New height in document pixels.
        height: f64,
    },
    /// A rectangle's four radii (the "Corner radius" field sets all four).
    Radii([f64; 4]),
    /// A polygon's or triangle's corner radius, or the options bar's Radius (the Rectangle's, or the Triangle
    /// tool's own).
    CornerRadius(f64),
    /// A polygon's number of sides.
    Sides(u32),
    /// A star's inner over outer radius.
    StarRatio(f64),
    /// A line's weight.
    Weight(f64),
    /// A line's arrowheads.
    Arrowheads(Arrowheads),
}

impl ShapeChange {
    /// The stroke width a stroke gets when an edit turns it on at width 0 (PhotoCraft's
    /// `enabled_stroke_width`).
    pub const ENABLED_STROKE_WIDTH: f64 = 3.0;
}

/// The Shape tools' persisted options-bar values (Fill on, the Rectangle's and the Triangle's radius, sides, star
/// ratio, weight, stroke width, the Rectangle's gear); the paints are the app's foreground and background.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ShapeToolSettings {
    /// Whether new shapes get a fill.
    pub fill_on: bool,
    /// The Rectangle's corner radius.
    pub corner_radius: f64,
    /// The Triangle's corner radius, one for its three corners: the Triangle tool keeps its own, apart from the
    /// Rectangle's (storytold/photocraft#13355).
    pub triangle_corner_radius: f64,
    /// A polygon's sides or a star's points.
    pub sides: u32,
    /// A star's inner over outer radius.
    pub star_ratio: f64,
    /// The Line's weight.
    pub line_weight: f64,
    /// Stroke width of new shapes, in pixels; 0 is no stroke.
    pub stroke_width: f64,
    /// The Line's arrowheads.
    pub arrowheads: Arrowheads,
    /// The Rectangle's gear ([`BoxConstraintsPopover`]).
    pub rect_box: BoxConstraints,
}

impl Default for ShapeToolSettings {
    /// Fill on, square corners (Rectangle and Triangle), 5 sides, a star ratio of 0.5 (production's Star), weight 3, no stroke, no
    /// arrowheads, an unconstrained Rectangle.
    fn default() -> Self {
        ShapeToolSettings {
            fill_on: true,
            corner_radius: 0.0,
            triangle_corner_radius: 0.0,
            sides: 5,
            star_ratio: 0.5,
            line_weight: 3.0,
            stroke_width: 0.0,
            arrowheads: Arrowheads::default(),
            rect_box: BoxConstraints::default(),
        }
    }
}

impl ShapeToolSettings {
    /// The drag options of `kind` with these values; the gear's constraints apply to the Rectangle only, and a
    /// Triangle takes its own corner radius.
    pub fn drag_options(&self, kind: ShapeKind) -> ShapeDragOptions {
        let radius = if kind == ShapeKind::Triangle { self.triangle_corner_radius } else { self.corner_radius };
        let options = ShapeDragOptions::new(kind).corner_radius(radius).sides(self.sides).star_ratio(self.star_ratio).line_weight(self.line_weight);
        match kind {
            ShapeKind::Rectangle => options.constraints(self.rect_box),
            _ => options,
        }
    }
}
