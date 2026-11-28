//! `Arrangement`: a custom placement for a body, in logical coordinates.

use emath::{Rect, Vec2};

/// A placement of keyed children measured by the caller. Logical: x runs from the start edge of `room`, so
/// `arrange` mirrors the output under RTL and no arrangement reads the direction.
pub trait Arrangement {
    /// Fills `out` with one rect per entry of `sizes`, in order, inside `room`; returns the size the placement
    /// takes.
    fn place(&self, room: Rect, sizes: &[Vec2], out: &mut Vec<Rect>) -> Vec2;

    /// False for a placement that must not mirror (a free-position board).
    fn mirrors(&self) -> bool {
        true
    }
}
