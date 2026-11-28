//! A1 notation and the sheet's size limits.

use craft_behavior::{CellAt, CellRange};

/// Rows in a sheet, Excel's 1,048,576.
pub const SHEET_ROWS: u32 = 1_048_576;

/// Columns in a sheet, Excel's 16,384 (A to XFD).
pub const SHEET_COLUMNS: u32 = 16_384;

/// The absolute reference `$B$3` of a cell (point mode inserts these).
pub fn absolute_reference(cell: CellAt<u32>) -> String {
    format!("${}${}", column_letters(cell.column), cell.row + 1)
}

/// `$A$1:$C$4`, or `$A$1` for one cell.
pub fn absolute_range_reference(range: CellRange<u32>) -> String {
    let (top, bottom, left, right) = edges(&range);
    if top == bottom && left == right {
        absolute_reference(CellAt::new(top, left))
    } else {
        format!("{}:{}", absolute_reference(CellAt::new(top, left)), absolute_reference(CellAt::new(bottom, right)))
    }
}

/// `A`, `Z`, `AA`, `XFD` for column 0, 25, 26, 16383.
pub fn column_letters(column: u32) -> String {
    let mut rest = u64::from(column) + 1;
    let mut letters = Vec::new();
    while rest > 0 {
        rest -= 1;
        letters.push(char::from(b'A' + (rest % 26) as u8));
        rest /= 26;
    }
    letters.iter().rev().collect()
}

/// The top row, bottom row, left column and right column of `range`.
pub(crate) fn edges(range: &CellRange<u32>) -> (u32, u32, u32, u32) {
    let rows = range.rows();
    let columns = range.column_span();
    (*rows.start(), *rows.end(), *columns.start(), *columns.end())
}
