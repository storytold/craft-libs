//! Table dimensions and cell rectangles; see docs/pub-format/tables.md.
use crate::{
    Error as PubError, Result,
    read::{Prop, count, number, slice, u32_at},
};

use std::ops::Range;

const MAX_CELLS: usize = 10_000;
fn bad(message: &str) -> PubError {
    PubError::Malformed(format!("table: {message}"))
}
#[derive(Clone, Debug)]
pub struct CellInfo {
    pub row: usize,
    pub last_row: usize,
    pub col: usize,
    pub last_col: usize,
}
#[derive(Clone)]
pub struct TableInfo {
    /// Native heights derived from consecutive stored row endings.
    pub rows: Vec<u32>,
    pub columns: Vec<u32>,
    pub cells: Vec<CellInfo>,
    pub cell_record: u32,
    /// Largest absolute ending-versus-size-sum difference in this table, for diagnostics only.
    pub row_ending_difference: u32,
    /// What the parser approximated because stored dimensions disagreed; reported by `table_frame`.
    pub approximations: Vec<String>,
}
impl TableInfo {
    pub fn parse(p: &[Prop<'_>]) -> Result<Option<Self>> {
        let (Some(rows), Some(cols), Some(cell_record)) = (number(p, 0x66), number(p, 0x67), number(p, 0x6b)) else { return Ok(None) };
        let rows = count(rows, 256)?;
        let cols = count(cols, 256)?;
        if rows == 0 || cols == 0 || rows * cols > MAX_CELLS {
            return Err(bad("grid exceeds limits"));
        }
        let Some(dimensions) = p.iter().find(|p| p.key == 0x6d && p.kind == 0x90) else { return Ok(None) };
        let entries = dimensions.children()?;
        if entries.len() != rows + cols {
            return Err(bad("dimension count mismatch"));
        }
        let mut sizes = Vec::new();
        let mut sum = 0u32;
        let mut column_endings_agree = true;
        let mut row_ends: Vec<u32> = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            if i == cols {
                sum = 0;
            }
            let fields = entry.children()?;
            let size = number(&fields, 2).ok_or_else(|| bad("missing dimension"))?;
            if size == 0 || size > 197_510_400 {
                return Err(bad("dimension out of range"));
            }
            sum = sum.checked_add(size).ok_or_else(|| bad("dimension overflow"))?;
            let ending = number(&fields, 1).ok_or_else(|| bad("missing cumulative dimension"))?;
            if i < cols {
                // The stored sizes lay out the columns; an ending that disagrees is approximated, not fatal.
                column_endings_agree &= ending == sum;
            } else {
                row_ends.push(ending);
            }
            if i + 1 == cols && number(p, 0x68) != Some(sum) || i + 1 == cols + rows && number(p, 0x69) != Some(sum) {
                return Err(bad("size total mismatch"));
            }
            sizes.push(size);
        }
        let column_sizes = sizes.get(..cols).ok_or_else(|| bad("missing columns"))?.to_vec();
        let row_sizes = sizes.get(cols..).ok_or_else(|| bad("missing rows"))?.to_vec();
        let mut approximations = Vec::new();
        if !column_endings_agree {
            approximations.push("column widths come from the stored column sizes; the stored column endings do not add up to them".to_string());
        }
        // Row positions follow the stored endings when they are positive, strictly increasing and in range.
        let mut previous = 0u32;
        let mut heights = Vec::with_capacity(rows);
        let mut endings_usable = true;
        for &ending in &row_ends {
            match ending.checked_sub(previous) {
                Some(h) if ending <= i32::MAX as u32 && ending > previous && h <= 197_510_400 => heights.push(h),
                _ => {
                    endings_usable = false;
                    break;
                }
            }
            previous = ending;
        }
        let (rows_out, row_ending_difference) = if endings_usable {
            let mut running = 0u32;
            let mut difference = 0u32;
            for (ending, size) in row_ends.iter().zip(&row_sizes) {
                running = running.saturating_add(*size);
                difference = difference.max(ending.abs_diff(running));
            }
            (heights, difference)
        } else {
            approximations.push("row heights come from the stored row sizes; the stored row endings are not increasing".to_string());
            (row_sizes, 0)
        };
        Ok(Some(Self { columns: column_sizes, rows: rows_out, cells: Vec::new(), cell_record, row_ending_difference, approximations }))
    }
    pub fn validate(&self) -> Result<()> {
        let mut occupied = vec![false; self.rows.len() * self.columns.len()];
        for c in &self.cells {
            if c.row > c.last_row || c.col > c.last_col || c.last_row >= self.rows.len() || c.last_col >= self.columns.len() {
                return Err(bad("invalid cell rectangle"));
            }
            for r in c.row..=c.last_row {
                for col in c.col..=c.last_col {
                    let slot = occupied.get_mut(r * self.columns.len() + col).ok_or_else(|| bad("cell out of range"))?;
                    if *slot {
                        return Err(bad("overlapping cells"));
                    }
                    *slot = true;
                }
            }
        }
        if occupied.iter().any(|v| !v) {
            return Err(bad("incomplete cell grid"));
        }
        Ok(())
    }
}
pub fn cells(p: &[Prop<'_>]) -> Result<Vec<CellInfo>> {
    let n = count(number(p, 1).ok_or_else(|| bad("missing cell count"))?, MAX_CELLS)?;
    let Some(array) = p.iter().find(|p| p.key == 2 && p.kind == 0xa0) else { return Err(bad("missing cell list")) };
    let entries = array.children()?;
    if entries.len() != n {
        return Err(bad("cell count mismatch"));
    }
    entries
        .iter()
        .map(|entry| {
            let p = entry.children()?;
            Ok(CellInfo {
                row: number(&p, 1).unwrap_or(0) as usize,
                last_row: number(&p, 2).unwrap_or(0) as usize,
                col: number(&p, 3).unwrap_or(0) as usize,
                last_col: number(&p, 4).unwrap_or(0) as usize,
            })
        })
        .collect()
}
pub fn text_ranges(data: &[u8], units: &[u16]) -> Result<Vec<Range<usize>>> {
    let n = count(u32_at(data, 0)?, MAX_CELLS - 1)? + 1;
    slice(data, 12, n * 4)?;
    let mut ranges = Vec::new();
    let mut start = 0;
    for i in 0..n {
        let end = u32_at(data, 12 + 4 * i)? as usize;
        if end < start || end > units.len() {
            return Err(bad("invalid cell text boundary"));
        }
        let last = i + 1 == n;
        if last && end != units.len() || !last && units.get(end) != Some(&13) {
            return Err(bad("cell text separator mismatch"));
        }
        let z = if last && end > start && units.get(end - 1) == Some(&13) { end - 1 } else { end };
        ranges.push(start..z);
        start = end + usize::from(!last);
    }
    Ok(ranges)
}
