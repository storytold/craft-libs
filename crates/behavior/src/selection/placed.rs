//! A selection by position in one display order, read once per pass.

use std::ops::Range;

/// The selected cells by position in one display order: the rows and columns of each range, and the cells taken out
/// of them. A key-based selection places its ends in the order once (`SelectionModel::placed`); the reads after that
/// are by position and scan no keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlacedSelection {
    /// The row positions and column positions of each range.
    ranges: Vec<(Range<usize>, Range<u32>)>,
    /// The cells taken out of the ranges, as (row position, column), sorted.
    removed: Vec<(usize, u32)>,
}

impl PlacedSelection {
    /// The selected rows `spans`, every column of each.
    pub fn of_rows(spans: impl IntoIterator<Item = Range<usize>>) -> Self {
        PlacedSelection { ranges: spans.into_iter().filter(|span| !span.is_empty()).map(|span| (span, 0..u32::MAX)).collect(), removed: Vec::new() }
    }

    /// The ranges `ranges` (row positions, column positions) without the cells `removed`.
    pub fn of_cells(ranges: Vec<(Range<usize>, Range<u32>)>, mut removed: Vec<(usize, u32)>) -> Self {
        removed.sort_unstable();
        removed.dedup();
        PlacedSelection { ranges, removed }
    }

    /// Whether the row at position `at` has a selected cell.
    pub fn holds_row(&self, at: usize) -> bool {
        self.ranges.iter().any(|(rows, columns)| {
            rows.contains(&at) && {
                let out = self.removed_in(at).filter(|column| columns.contains(column)).count();
                u64::from(columns.end.saturating_sub(columns.start)) > out as u64
            }
        })
    }

    /// Whether the cell at row position `at` and column position `column` is selected.
    pub fn holds_cell(&self, at: usize, column: u32) -> bool {
        self.ranges.iter().any(|(rows, columns)| rows.contains(&at) && columns.contains(&column))
            && self.removed.binary_search(&(at, column)).is_err()
    }

    /// The positions of the rows that have a selected cell, ascending.
    pub fn rows(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self.ranges.iter().flat_map(|(rows, _)| rows.clone()).collect();
        rows.sort_unstable();
        rows.dedup();
        rows.retain(|&at| self.holds_row(at));
        rows
    }

    /// The columns removed from row `at`, ascending.
    fn removed_in(&self, at: usize) -> impl Iterator<Item = u32> + '_ {
        let from = self.removed.partition_point(|&(row, _)| row < at);
        self.removed[from..].iter().take_while(move |&&(row, _)| row == at).map(|&(_, column)| column)
    }
}
