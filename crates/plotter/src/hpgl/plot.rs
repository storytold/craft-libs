//! HP-GL/2 plotter state types: stroke attributes, line patterns, and fill/cap/join rules.

use kurbo::Point;

/// Plotter units per millimeter (0.025 mm each).
pub const PLU_PER_MM: f64 = 40.0;

/// The hard-clip limits taken for P1 and P2 when the file sets none (plotter units): 11 × 8.5 in.
pub const HARD_CLIP: Point = Point::new(11176.0, 8636.0);

/// How far from the origin art may lie (plotter units, twice the range the format defines).
pub const LIMIT: f64 = 16_777_216.0;

/// Fill rule for polygon mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    EvenOdd,
    NonZero,
}

/// Line cap style (`LA` kind 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Square,
    Triangular,
    Round,
}

/// Line join style (`LA` kind 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    MiterBevel,
    Triangular,
    Round,
    Bevel,
    None,
}

/// A stroke as the pen draws it (lengths in plotter units).
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub color: [u8; 3],
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter: f64,
    pub dash: Option<LinePattern>,
}

/// A line type as dashes (lengths in plotter units).
#[derive(Clone, Debug, PartialEq)]
pub struct LinePattern {
    /// Dash, gap, dash, gap…: an even number of lengths.
    pub pattern: Vec<f64>,
    /// How far into the pattern each line starts.
    pub offset: f64,
}
