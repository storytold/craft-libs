//! The displaced mesh Show Mesh draws.

use craft_behavior::kurbo::Point;

/// The displaced mesh Show Mesh draws, from [`LiquifySource::mesh`](super::LiquifySource::mesh):
/// `columns × rows` nodes in row order, in document pixels.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LiquifyMesh {
    /// Nodes per row.
    pub columns: usize,
    /// Number of rows.
    pub rows: usize,
    /// The `columns × rows` nodes in row order, in document pixels.
    pub points: Vec<Point>,
}

impl LiquifyMesh {
    /// A mesh of `columns × rows` nodes in row order.
    pub fn new(columns: usize, rows: usize, points: Vec<Point>) -> Self {
        Self { columns, rows, points }
    }

    /// The mesh's lines in document pixels (each horizontal and vertical edge between neighbouring
    /// nodes once), for the mesh lines control. A mesh whose `points` don't match `columns × rows`
    /// has no lines.
    pub fn edges(&self) -> Vec<[Point; 2]> {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:1248 `draw_mesh` (PhotoCraft traces the
        // field's iso-lines itself; the app supplies the displaced nodes, so the lines are the
        // grid's edges).
        if self.columns == 0 || self.rows == 0 || self.columns.checked_mul(self.rows) != Some(self.points.len()) {
            return Vec::new();
        }
        let at = |row: usize, column: usize| self.points[row * self.columns + column];
        let mut edges = Vec::with_capacity(2 * self.points.len());
        for row in 0..self.rows {
            for column in 0..self.columns {
                if column + 1 < self.columns {
                    edges.push([at(row, column), at(row, column + 1)]);
                }
                if row + 1 < self.rows {
                    edges.push([at(row, column), at(row + 1, column)]);
                }
            }
        }
        edges
    }
}
