//! [`GradientTile`]: a square two-dimensional gradient in a swatch or the palette grid.

use super::GradientStyle;

/// How a gradient swatch or palette cell draws its gradient.
///
/// A tile other than [`Ramp`](GradientTile::Ramp) is one `craft_color_ui::ui::smooth_mesh` sampled through
/// [`t_at`](GradientTile::t_at).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum GradientTile {
    /// The ramp chip: the gradient along its length.
    #[default]
    Ramp,
    /// A square tile of `style` at `angle`.
    Style {
        /// How the gradient runs.
        style: GradientStyle,
        /// The angle of the gradient line, in degrees.
        angle: f32,
    },
    /// A square tile whose gradient runs out from `focus`.
    Focus {
        /// The shape of the outline the gradient follows.
        outline: FocusOutline,
        /// The focus rectangle, in the tile's unit square.
        focus: egui::Rect,
    },
}

/// The outline a [`GradientTile::Focus`] gradient follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusOutline {
    /// A circle.
    Circle,
    /// A rectangle.
    Rect,
}

impl GradientTile {
    /// The gradient position, 0 to 1, at `p` in the tile's unit square.
    pub fn t_at(&self, p: egui::Vec2) -> f32 {
        let _ = p;
        todo!("ch.C15")
    }
}
