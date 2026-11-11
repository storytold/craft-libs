//! The marks of choice controls: the check mark and the radio dot. `Checkbox`, `RadioGroup` and `MenuItem` paint
//! these shapes, so they look alike in a form and in a menu.

use super::PaintShape;
use egui::{Pos2, Rect, Vec2};

/// The check mark's corner and tip, as shares of the square's side from its top-left.
const TICK: [(f32, f32); 3] = [(0.22, 0.52), (0.42, 0.72), (0.78, 0.30)];

/// The radius of the radio dot, as a share of the mark's side.
const DOT_RADIUS: f32 = 0.25;

/// The check mark in `square`; stroke it.
#[doc(hidden)]
pub fn check(square: Rect) -> PaintShape {
    let at = |(x, y): (f32, f32)| -> Pos2 { square.min + Vec2::new(x, y) * square.size() };
    PaintShape::Path(TICK.map(at).to_vec())
}

/// The radio dot at `center` in a mark `side` points wide; fill it.
#[doc(hidden)]
pub fn dot(center: Pos2, side: f32) -> PaintShape {
    PaintShape::Circle(center, side * DOT_RADIUS)
}
