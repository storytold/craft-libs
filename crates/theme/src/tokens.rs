//! Semantic tokens: the resolved values component tokens derive from. Colours, shape, space,
//! sizes, type roles, elevation and motion, all concrete.

use super::color::Rgba;
use super::components::ComponentTokens;

mod color;
mod size;
mod type_motion;

pub use color::*;
pub use size::*;
pub use type_motion::*;

/// Every resolved token: the semantic layer and the component tokens.
#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    /// Colours.
    pub color: ColorTokens,
    /// Corner radii and edge.
    pub shape: ShapeTokens,
    /// Gaps and padding.
    pub space: SpaceTokens,
    /// Control and chrome sizes.
    pub size: SizeTokens,
    /// Line widths and dashes.
    pub stroke: StrokeTokens,
    /// Type roles.
    pub typography: TypeTokens,
    /// Shadows.
    pub elevation: ElevationTokens,
    /// Animations.
    pub motion: MotionTokens,
    /// The component tokens.
    pub component: ComponentTokens,
}

impl Tokens {
    /// The value of one colour token.
    pub fn color(&self, token: ColorToken) -> Rgba {
        self.color.get(token)
    }
}
