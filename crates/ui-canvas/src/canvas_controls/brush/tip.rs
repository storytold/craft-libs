//! The brush tip as the cursor draws it.

use craft_behavior::kurbo::{Affine, Rect, Vec2};
use std::sync::Arc;

/// The unit length of a tip axis.
const UNIT: f64 = 1.0;
/// No length.
const NONE: f64 = 0.0;
/// The steepest pen tilt, in degrees.
const MAX_TILT: f64 = 90.0;
/// The least share of its length a tilted pen leaves a tip axis.
const LEAST_FOOTPRINT: f64 = 0.05;

/// A brush tip: its diameter and the outline the cursor draws. The app builds it from its engine when the brush
/// changes (an aliased tip's cell also when the pointer moves); every length is in document units.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BrushTip {
    /// The brush's diameter (its Size).
    pub diameter: f64,
    /// The tip's outline for the cursor preferences.
    pub outline: BrushTipOutline,
}

impl BrushTip {
    /// A tip of `diameter` drawn as `outline`.
    pub fn new(diameter: f64, outline: BrushTipOutline) -> BrushTip {
        BrushTip { diameter, outline }
    }

    /// A plain round tip of `diameter` whose 50 % contour is at `normal_radius` (the engine's half-coverage radius
    /// for its hardness) and whose full reach is `diameter / 2`.
    pub fn round(diameter: f64, normal_radius: f64) -> BrushTip {
        BrushTip::new(diameter, BrushTipOutline::round(normal_radius, diameter / 2.0))
    }
}

/// The outline of a tip, for Normal Brush Tip (where a dab reaches 50 %) and Full Size Brush Tip (where it reaches
/// 0 %).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BrushTipOutline {
    /// A plain round tip: two circles. Under a skewed pose or a view with a pixel aspect the circle becomes an
    /// ellipse.
    #[non_exhaustive]
    Round {
        /// Radius of the 50 % contour, in document units.
        normal_radius: f64,
        /// Radius where the dab reaches 0 %, in document units.
        full_radius: f64,
    },
    /// A shaped tip (a sampled bitmap, a turned or flattened round tip) traced by the engine, in radius units
    /// (1 is `diameter / 2`) in document orientation, for each preference.
    #[non_exhaustive]
    Traced {
        /// The 50 % contour.
        normal: TipRings,
        /// The 0 % contour.
        full: TipRings,
    },
    /// An aliased tip (the Pencil, the Eraser in Pencil or Block mode): the whole-pixel `cell` its dab fills at the
    /// pointer, drawn on the pixel grid as a disc when `round`, else as the square. The pointer hides and
    /// [`TipCell`](super::TipCell) shows the cell.
    #[non_exhaustive]
    Pixels {
        /// The whole-pixel cell the dab fills, in document units.
        cell: Rect,
        /// Draw the cell as a disc rather than a square.
        round: bool,
    },
}

impl BrushTipOutline {
    /// A round tip with the radii of its 50 % contour and its full reach.
    pub fn round(normal_radius: f64, full_radius: f64) -> BrushTipOutline {
        BrushTipOutline::Round { normal_radius, full_radius }
    }

    /// A traced tip with the rings of its 50 % contour and its full reach.
    pub fn traced(normal: TipRings, full: TipRings) -> BrushTipOutline {
        BrushTipOutline::Traced { normal, full }
    }

    /// An aliased tip filling `cell`, a disc when `round`.
    pub fn pixels(cell: Rect, round: bool) -> BrushTipOutline {
        BrushTipOutline::Pixels { cell, round }
    }
}

/// Closed rings of a traced tip outline, points in radius units `[x, y]` (document orientation, y down), as the
/// engine's tip tracer returns them. Shared: the app keeps one value per brush and the cursor maps it each pass
/// without retracing, whatever the pose or view.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TipRings(Arc<[Vec<[f32; 2]>]>);

impl TipRings {
    /// Rings from the engine's tracer.
    pub fn new(rings: Vec<Vec<[f32; 2]>>) -> TipRings {
        TipRings(rings.into())
    }

    /// The rings.
    pub fn rings(&self) -> &[Vec<[f32; 2]>] {
        &self.0
    }

    /// Whether `self` and `other` are the same shared rings (not merely equal): the app's cache check.
    pub fn same(&self, other: &TipRings) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The tip's own axes in document space for one radius: where the tip's x and y directions point after the brush
/// angle and the pen's pose, before the view. Upright is `(1, 0)` and `(0, 1)`. The cursor maps both through the view
/// ([`CanvasView::map_vec`](craft_behavior::CanvasView::map_vec)), so the pose comes before zoom, rotation and flip
/// and a cached outline is only re-mapped.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TipPose {
    /// The document-space direction of the tip's x axis.
    pub x_axis: Vec2,
    /// The document-space direction of the tip's y axis.
    pub y_axis: Vec2,
}

impl Default for TipPose {
    fn default() -> Self {
        TipPose::upright()
    }
}

impl TipPose {
    /// A pose with the given axes.
    pub const fn new(x_axis: Vec2, y_axis: Vec2) -> TipPose {
        TipPose { x_axis, y_axis }
    }

    /// The pose whose axes are the images of the unit axes under the linear part of `map`.
    pub fn from_linear(map: Affine) -> TipPose {
        let [xx, xy, yx, yy, ..] = map.as_coeffs();
        TipPose::new(Vec2::new(xx, xy), Vec2::new(yx, yy))
    }

    /// No pose: the document's axes.
    pub fn upright() -> TipPose {
        TipPose::from_linear(Affine::IDENTITY)
    }

    /// The pose of a pen: the tip turned clockwise by `degrees` (the app's angle from its shape dynamics: pen tilt
    /// or barrel rotation, plus the barrel rotation with Brush Projection), and with `tilt` (Brush Projection on; the
    /// pen's tilt x and y in degrees) shortened along the tilt direction by `cos(|tilt|)`, at least 0.05, as a tilted
    /// pen's footprint. A zero tilt or a non-finite value leaves that step out.
    pub fn pen(degrees: f64, tilt: Option<[f32; 2]>) -> TipPose {
        let degrees = if degrees.is_finite() { degrees } else { NONE };
        let (sin, cos) = degrees.to_radians().sin_cos();
        let (x, y) = (Vec2::new(cos, -sin), Vec2::new(sin, cos));
        let project = |v: Vec2| match tilt {
            Some([tx, ty]) if tx.is_finite() && ty.is_finite() => {
                let (tx, ty) = (f64::from(tx), f64::from(ty));
                let tilt = tx.hypot(ty).clamp(NONE, MAX_TILT);
                if tilt == NONE {
                    return v;
                }
                let dir = Vec2::new(tx, ty) / tx.hypot(ty);
                v + dir * (v.dot(dir) * (tilt.to_radians().cos().max(LEAST_FOOTPRINT) - UNIT))
            }
            _ => v,
        };
        TipPose::new(project(x), project(y))
    }
}
