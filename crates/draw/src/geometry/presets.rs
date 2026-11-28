//! [`ShapePreset`]: the kinds of the shape menu, with the geometry or outline each draws.
//!
//! Upstream's list, order and keys. Kind-backed presets draw through [`ShapeGeometry::to_path`]
//! and connectors through [`connector::route`](super::connector::route), so no preset keeps a
//! second outline of a shape the crate already draws.

use super::connector::{RouteEnd, Routing, route};
use super::shape::ShapeGeometry;
use kurbo::{Affine, BezPath, Point, Rect, Vec2};

/// The side of the square box the outlines are drawn in before they are scaled into the caller's rectangle.
const BOX: f64 = 20.0;

/// The corner radius of the Rounded Rectangle as a share of the shorter side (3 of the 10 high preview box).
const ROUND_SHARE: f64 = 0.3;

/// The Star's inner radius over its outer radius (3.3 of 8).
const STAR_INNER: f64 = 3.3 / 8.0;

/// The error bound, in box units, when curves of the kind-backed presets are approximated.
const BOX_TOLERANCE: f64 = 0.02;

/// The rows of a text box's lines, in box units.
const TEXT_LINES: [f64; 3] = [8.0, 10.0, 12.0];

/// How far an arrowhead's wings reach back from the tip and to each side, in box units.
const HEAD_BACK: f64 = 4.0;
const HEAD_SIDE: f64 = 2.5;

/// The points of the Heart's contour.
const HEART_POINTS: usize = 32;

/// One entry of the shape menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ui", derive(craft_ui::contract::MemoProps))]
#[non_exhaustive]
pub enum ShapePreset {
    /// A rectangle.
    Rectangle,
    /// A rectangle with rounded corners.
    RoundedRectangle,
    /// An ellipse.
    Ellipse,
    /// A triangle.
    Triangle,
    /// A diamond.
    Diamond,
    /// A line.
    Line,
    /// A line with an arrowhead.
    Arrow,
    /// A star.
    Star,
    /// A heart.
    Heart,
    /// A text box.
    TextBox,
    /// A straight connector.
    StraightConnector,
    /// A straight connector with an end arrow.
    StraightArrowConnector,
    /// An elbow connector.
    ElbowConnector,
    /// An elbow connector with an end arrow.
    ElbowArrowConnector,
    /// A curved connector.
    CurvedConnector,
    /// A curved connector with an end arrow.
    CurvedArrowConnector,
}

/// Every preset, in menu order.
pub const PRESETS: &[ShapePreset] = &[
    ShapePreset::Rectangle,
    ShapePreset::RoundedRectangle,
    ShapePreset::Ellipse,
    ShapePreset::Triangle,
    ShapePreset::Diamond,
    ShapePreset::Line,
    ShapePreset::Arrow,
    ShapePreset::Star,
    ShapePreset::Heart,
    ShapePreset::TextBox,
    ShapePreset::StraightConnector,
    ShapePreset::StraightArrowConnector,
    ShapePreset::ElbowConnector,
    ShapePreset::ElbowArrowConnector,
    ShapePreset::CurvedConnector,
    ShapePreset::CurvedArrowConnector,
];

/// What a preset draws in a rectangle: a fill path and stroke paths.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct PresetOutline {
    /// The path to fill, when the preset has an inside.
    pub fill: Option<kurbo::BezPath>,
    /// The paths to stroke.
    pub strokes: Vec<kurbo::BezPath>,
}

impl ShapePreset {
    /// The preset's stable key (upstream's).
    pub const fn key(self) -> &'static str {
        match self {
            ShapePreset::Rectangle => "rectangle",
            ShapePreset::RoundedRectangle => "roundedRectangle",
            ShapePreset::Ellipse => "ellipse",
            ShapePreset::Triangle => "triangle",
            ShapePreset::Diamond => "diamond",
            ShapePreset::Line => "line",
            ShapePreset::Arrow => "arrow",
            ShapePreset::Star => "star",
            ShapePreset::Heart => "heart",
            ShapePreset::TextBox => "textBox",
            ShapePreset::StraightConnector => "straightConnector",
            ShapePreset::StraightArrowConnector => "straightArrowConnector",
            ShapePreset::ElbowConnector => "elbowConnector",
            ShapePreset::ElbowArrowConnector => "elbowArrowConnector",
            ShapePreset::CurvedConnector => "curvedConnector",
            ShapePreset::CurvedArrowConnector => "curvedArrowConnector",
        }
    }

    /// Whether the preset is a connector.
    pub const fn is_connector(self) -> bool {
        self.routing().is_some()
    }

    /// Whether the preset draws an arrowhead at its end.
    pub const fn has_end_arrow(self) -> bool {
        matches!(
            self,
            ShapePreset::Arrow | ShapePreset::StraightArrowConnector | ShapePreset::ElbowArrowConnector | ShapePreset::CurvedArrowConnector
        )
    }

    /// The kind-backed geometry in `rect`; `None` for the other presets.
    pub fn geometry(self, rect: kurbo::Rect) -> Option<ShapeGeometry> {
        Some(match self {
            ShapePreset::Rectangle => ShapeGeometry::Rect { rect, radii: [0.0; 4] },
            ShapePreset::RoundedRectangle => {
                let radius = ROUND_SHARE * rect.width().abs().min(rect.height().abs());
                ShapeGeometry::Rect { rect, radii: [radius; 4] }
            }
            ShapePreset::Ellipse => ShapeGeometry::Ellipse { rect },
            ShapePreset::Triangle => ShapeGeometry::Polygon { rect, sides: 3, star_ratio: 1.0 },
            ShapePreset::Diamond => ShapeGeometry::Polygon { rect, sides: 4, star_ratio: 1.0 },
            ShapePreset::Star => ShapeGeometry::Polygon { rect, sides: 5, star_ratio: STAR_INNER },
            _ => return None,
        })
    }

    /// How a connector preset routes; `None` for the others.
    pub const fn routing(self) -> Option<Routing> {
        match self {
            ShapePreset::StraightConnector | ShapePreset::StraightArrowConnector => Some(Routing::Straight),
            ShapePreset::ElbowConnector | ShapePreset::ElbowArrowConnector => Some(Routing::Elbow),
            ShapePreset::CurvedConnector | ShapePreset::CurvedArrowConnector => Some(Routing::Curved),
            _ => None,
        }
    }

    /// The paths the preset draws in `rect`.
    pub fn outline(self, rect: kurbo::Rect) -> PresetOutline {
        let (width, height) = (rect.width(), rect.height());
        if !(rect.is_finite() && width > 0.0 && height > 0.0) {
            return PresetOutline::default();
        }
        // The box is centred in the rectangle and scaled by its shorter side, so a preset keeps its proportions.
        let scale = width.min(height) / BOX;
        let fit = Affine::translate(rect.center().to_vec2() - Vec2::new(BOX / 2.0, BOX / 2.0) * scale) * Affine::scale(scale);
        let mut outline = self.box_outline();
        if let Some(fill) = outline.fill.as_mut() {
            fill.apply_affine(fit);
        }
        for stroke in &mut outline.strokes {
            stroke.apply_affine(fit);
        }
        outline
    }

    /// The rectangle of the box the kind-backed preset is drawn in.
    fn box_rect(self) -> Rect {
        match self {
            ShapePreset::Rectangle | ShapePreset::RoundedRectangle | ShapePreset::TextBox => Rect::new(3.0, 5.0, 17.0, 15.0),
            ShapePreset::Ellipse => Rect::new(2.5, 4.5, 17.5, 15.5),
            ShapePreset::Triangle => Rect::new(2.0, 2.5, 18.0, 18.5),
            ShapePreset::Diamond => Rect::new(3.0, 3.0, 17.0, 17.0),
            ShapePreset::Star => Rect::new(2.0, 2.8, 18.0, 18.8),
            _ => Rect::new(0.0, 0.0, BOX, BOX),
        }
    }

    /// The paths in the `BOX` x `BOX` box.
    fn box_outline(self) -> PresetOutline {
        // A text box is a rectangle with lines of text in it.
        let body = if self == ShapePreset::TextBox { ShapePreset::Rectangle } else { self };
        if let Some(geometry) = body.geometry(self.box_rect()) {
            let path = geometry.to_path(BOX_TOLERANCE);
            let mut strokes = vec![path.clone()];
            if self == ShapePreset::TextBox {
                strokes.extend(TEXT_LINES.iter().map(|&y| polyline(&[Point::new(6.0, y), Point::new(14.0, y)], false)));
            }
            return PresetOutline { fill: Some(path), strokes };
        }
        if self == ShapePreset::Heart {
            let path = heart();
            return PresetOutline { fill: Some(path.clone()), strokes: vec![path] };
        }
        let (from, to) =
            if self.is_connector() { (Point::new(4.0, 15.0), Point::new(16.0, 5.0)) } else { (Point::new(4.0, 16.0), Point::new(16.0, 4.0)) };
        let routing = self.routing().unwrap_or(Routing::Straight);
        let Some(points) = route(RouteEnd::Point(from), RouteEnd::Point(to), routing) else {
            return PresetOutline::default();
        };
        // A curved route is its two ends with the cubic's two control points between them.
        let line = match (routing, points.as_slice()) {
            (Routing::Curved, [a, b, c, d]) => {
                let mut path = BezPath::new();
                path.move_to(*a);
                path.curve_to(*b, *c, *d);
                path
            }
            _ => polyline(&points, false),
        };
        let mut strokes = vec![line];
        if self.has_end_arrow() {
            strokes.extend(arrow_head(&points));
        }
        PresetOutline { fill: None, strokes }
    }
}

/// The path through `points`, closed when `closed`.
fn polyline(points: &[Point], closed: bool) -> BezPath {
    let mut path = BezPath::new();
    if let Some((first, rest)) = points.split_first() {
        path.move_to(*first);
        for point in rest {
            path.line_to(*point);
        }
        if closed {
            path.close_path();
        }
    }
    path
}

/// The Heart's closed contour, star-shaped about the box centre.
fn heart() -> BezPath {
    let points: Vec<Point> = (0..HEART_POINTS)
        .map(|i| {
            let t = i as f64 / HEART_POINTS as f64 * std::f64::consts::TAU;
            Point::new(
                10.0 + 16.0 * t.sin().powi(3) * 0.44,
                9.6 - (13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos()) * 0.44,
            )
        })
        .collect();
    polyline(&points, true)
}

/// The open arrowhead at the end of a route, along its last segment; `None` for a route that has no length.
fn arrow_head(points: &[Point]) -> Option<BezPath> {
    let tip = *points.last()?;
    let from = points.iter().rev().find(|p| (tip - **p).hypot() > f64::EPSILON)?;
    let direction = (tip - *from).normalize();
    let side = Vec2::new(-direction.y, direction.x);
    let back = tip - direction * HEAD_BACK;
    Some(polyline(&[back + side * HEAD_SIDE, tip, back - side * HEAD_SIDE], false))
}
