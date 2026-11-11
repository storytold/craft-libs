//! The nine points of a box, shared by the transform box rules and the reference point picker.

use serde::{Deserialize, Serialize};

/// The nine points of a box: its corners, edge midpoints and centre, row by row from the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Anchor9 {
    /// The top left corner.
    TopLeft,
    /// The middle of the top edge.
    Top,
    /// The top right corner.
    TopRight,
    /// The middle of the left edge.
    Left,
    /// The centre.
    Centre,
    /// The middle of the right edge.
    Right,
    /// The bottom left corner.
    BottomLeft,
    /// The middle of the bottom edge.
    Bottom,
    /// The bottom right corner.
    BottomRight,
}

impl Anchor9 {
    /// Every point, row by row from the top left.
    pub const ALL: [Anchor9; 9] = [
        Anchor9::TopLeft,
        Anchor9::Top,
        Anchor9::TopRight,
        Anchor9::Left,
        Anchor9::Centre,
        Anchor9::Right,
        Anchor9::BottomLeft,
        Anchor9::Bottom,
        Anchor9::BottomRight,
    ];

    /// Where the point sits in the box: 0 to 1 across from the left edge and down from the top.
    pub fn uv(self) -> (f64, f64) {
        let (column, row) = self.cell();
        (column as f64 / 2.0, row as f64 / 2.0)
    }

    /// The point's column and row in the 3 by 3 grid, counted from the top left.
    pub fn cell(self) -> (usize, usize) {
        let index = Anchor9::ALL.iter().position(|point| *point == self).unwrap_or(4);
        (index % 3, index / 3)
    }
}
