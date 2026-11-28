//! [`GradientTile`]: a square two-dimensional gradient in a swatch or the palette grid.

use std::cell::Cell;
use std::f32::consts::TAU;

use craft_color_ui::ui::gradient::{GradientStops, Sampler};
use craft_color_ui::ui::smooth_mesh;
use craft_ui::primitives::{Checker, ContentPainterCx, PaintShape, StrokeWidth};
use craft_ui::style::StyleColor;
use egui::{CornerRadius, Mesh, Rect, Vec2, vec2};

use super::GradientStyle;

/// Vertices along a side of a tile's mesh.
const TILE_STEPS: usize = 17;

/// How a gradient swatch or palette cell draws its gradient.
///
/// A tile other than [`Ramp`](GradientTile::Ramp) is one `craft_color_ui::ui::smooth_mesh` sampled through
/// [`t_at`](GradientTile::t_at). `p` below is a point in the tile's unit square (0 to 1, y down) and `c` its centre.
#[derive(Clone, Copy, Debug, Default, PartialEq, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub enum GradientTile {
    /// The ramp chip: the gradient along its length.
    #[default]
    Ramp,
    /// A square tile of `style` at `angle`.
    #[non_exhaustive]
    Style {
        /// How the gradient runs.
        style: GradientStyle,
        /// The angle of the gradient line, in degrees, in the convention of `GradientFill::angle`: 0 points right and
        /// 90 points up.
        angle: f32,
    },
    /// A square tile whose gradient runs out from `focus`.
    #[non_exhaustive]
    Focus {
        /// The shape of the outline the gradient follows.
        outline: FocusOutline,
        /// The focus rectangle, in the tile's unit square.
        focus: egui::Rect,
    },
}

impl GradientTile {
    /// A square tile of `style` at `angle`.
    pub const fn style(style: GradientStyle, angle: f32) -> Self {
        GradientTile::Style { style, angle }
    }

    /// A square tile whose gradient runs out from `focus`, in the shape of `outline`.
    pub const fn focus(outline: FocusOutline, focus: egui::Rect) -> Self {
        GradientTile::Focus { outline, focus }
    }
}

/// The outline a [`GradientTile::Focus`] gradient follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub enum FocusOutline {
    /// A circle.
    Circle,
    /// A rectangle.
    Rect,
}

impl GradientTile {
    /// The gradient position, 0 to 1, at `p` in the tile's unit square.
    ///
    /// - `Linear`: the line runs through the centre from edge to edge, 0 on the start side and 1 on the far side.
    /// - `Radial`, `Reflected`, `Diamond`: 0 at the centre, 1 at the edge along the angle's direction, in circles,
    ///   mirrored about the centre, or in diamonds.
    /// - `Angle`: the counter-clockwise sweep from the angle's direction, over a full turn.
    /// - `Focus`: 0 at the focus rectangle's centre, 1 at the farthest corner, in circles or in rectangles.
    ///
    /// `Ramp` has no second dimension: its position is `p.x`.
    pub fn t_at(&self, p: Vec2) -> f32 {
        // design: allow[literal-size/vec2](the centre of the tile unit square, a gradient coordinate and not a size)
        let c = vec2(0.5, 0.5);
        let t = match *self {
            GradientTile::Ramp => p.x,
            GradientTile::Style { style, angle } => {
                let rad = angle.to_radians();
                let d = vec2(rad.cos(), -rad.sin());
                let v = p - c;
                let h = (d.x.abs() + d.y.abs()) / 2.0;
                let along = v.dot(d);
                match style {
                    GradientStyle::Linear => 0.5 + along / (2.0 * h),
                    GradientStyle::Radial => v.length() / h,
                    GradientStyle::Reflected => along.abs() / h,
                    GradientStyle::Diamond => (along.abs() + v.dot(vec2(-d.y, d.x)).abs()) / h,
                    GradientStyle::Angle => {
                        let at = (-v.y).atan2(v.x);
                        (at - rad).rem_euclid(TAU) / TAU
                    }
                }
            }
            GradientTile::Focus { outline, focus } => {
                let f = focus.center().to_vec2();
                let corners = [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0, 1.0)];
                let distance = |v: Vec2| match outline {
                    FocusOutline::Circle => v.length(),
                    FocusOutline::Rect => v.x.abs().max(v.y.abs()),
                };
                let far = corners.iter().map(|&corner| distance(corner - f)).fold(0.0, f32::max);
                if far > 0.0 { distance(p - f) / far } else { 0.0 }
            }
        };
        if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Paints the tile over `rect`: the checkerboard where a colour is translucent, the gradient of `stops` through
    /// `sampler`, then a thin `edge` ring. A `Ramp` tile paints nothing here; the ramp chip is the caller's.
    pub(crate) fn paint(
        &self,
        cx: &ContentPainterCx<'_>,
        rect: Rect,
        radius: CornerRadius,
        stops: &GradientStops,
        sampler: &Sampler,
        edge: StyleColor,
    ) {
        if matches!(self, GradientTile::Ramp) || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let translucent = Cell::new(false);
        let at = |fx: f32, fy: f32| {
            let color = sampler(stops, self.t_at(vec2(fx, fy)));
            if color.a < 1.0 {
                translucent.set(true);
            }
            color
        };
        let mut mesh = Mesh::default();
        smooth_mesh(&mut mesh, rect, TILE_STEPS, &at);
        if translucent.get() {
            cx.checker_with(&Checker::new(), rect, radius);
        }
        cx.mesh_in(rect, radius, mesh);
        cx.with_draw(|draw| draw.stroke(&PaintShape::Rect(rect, radius), StrokeWidth::Thin, edge));
    }
}
