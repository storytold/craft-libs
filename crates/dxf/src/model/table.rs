//! ACAD_TABLE and TABLESTYLE.
//!
//! An ACAD_TABLE's own groups (after `AcDbTable`) give its size, its row heights and column
//! widths, then one run of groups per cell, each started by a `171` (the cell type), row by row.
//! The table is drawn by its anonymous `*T` block (group 2), which a reader can show instead.
//!
//! From CADCraft's `cadcraft-io` (`dxf_read/entities.rs` `acad_table` and `dxf_table.rs`
//! `read_table_style`, 5a6aa9b6).

use super::document::Record;
use super::{Color, Groups, Lineweight, Point};
use crate::Tag;

/// Most rows, and most columns, a table keeps.
pub const MAX_SIDE: usize = 10_000;
/// Most cells a table keeps.
pub const MAX_CELLS: usize = 1_000_000;

/// A cell of an ACAD_TABLE.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TableCell {
    /// Group 171: 1 text, 2 a block.
    pub cell_type: i64,
    /// Groups 2 and 3 in order, then 1: the text as written.
    pub text: String,
    /// Groups 176 (rows) and 175 (columns): the span of a merge that starts here.
    pub span: Option<(u32, u32)>,
    /// Group 173 is not 0: the cell is part of a merge. Writers differ on whether the first cell
    /// of a merge has it.
    pub merged: bool,
    /// Group 140 outside the value block (`301` ... `304`, where 140 is a number): the cell's
    /// own text height.
    pub text_height: Option<f64>,
    /// Group 170: the alignment, 1–9.
    pub alignment: Option<i64>,
    /// Group 7: the cell's text style name.
    pub text_style: Option<String>,
    /// Group 340: the block of a block cell.
    pub block: Option<u64>,
}

/// An ACAD_TABLE.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Table {
    /// Group 2: the anonymous block that draws the table.
    pub block: Option<String>,
    /// Group 342: the TABLESTYLE.
    pub style: Option<u64>,
    /// Group 10.
    pub insert: Point,
    /// Group 11: the horizontal direction.
    pub direction: Point,
    /// Groups 91 and 92: the rows and columns, as the file states them.
    pub rows: usize,
    pub columns: usize,
    /// Groups 141 and 142, as many as there are rows and columns at most.
    pub row_heights: Vec<f64>,
    pub column_widths: Vec<f64>,
    /// The cells there are, row by row: at most `rows * columns` of them.
    pub cells: Vec<TableCell>,
}

impl Table {
    /// The cell at `row`, `column`, when the file has it.
    pub fn cell(&self, row: usize, column: usize) -> Option<&TableCell> {
        self.cells.get(row.checked_mul(self.columns)?.checked_add(column)?)
    }

    /// The typed form of an ACAD_TABLE record. A table larger than [`MAX_SIDE`] a side or
    /// [`MAX_CELLS`] in all, or with no rows or no columns, keeps neither: its size reads as 0
    /// by 0, with no cells, row heights or column widths. A file states its size, and only cells
    /// it holds are kept, so a large declared size costs nothing.
    pub fn read(tags: &[Tag]) -> Table {
        let all = Groups(tags);
        let start = tags.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim() == "AcDbTable")).unwrap_or(0);
        let tail = tags.get(start..).unwrap_or(&[]);
        let first_cell = tail.iter().position(|t| t.code == 171).unwrap_or(tail.len());
        let head = Groups(tail.get(..first_cell).unwrap_or(&[]));
        let side = |code: i32| usize::try_from(head.i_or(code, 0)).ok().filter(|n| *n <= MAX_SIDE).unwrap_or(0);
        let (mut rows, mut columns) = (side(91), side(92));
        let cells_max = rows.saturating_mul(columns);
        if cells_max == 0 || cells_max > MAX_CELLS {
            (rows, columns) = (0, 0);
        }
        let mut t = Table {
            block: all.name(2),
            style: all.handle(342).filter(|h| *h != 0),
            insert: all.point(10),
            direction: all.point_opt(11).unwrap_or([1.0, 0.0, 0.0]),
            rows,
            columns,
            row_heights: head.0.iter().filter(|t| t.code == 141).take(rows).map(Tag::f64).collect(),
            column_widths: head.0.iter().filter(|t| t.code == 142).take(columns).map(Tag::f64).collect(),
            cells: Vec::new(),
        };
        // Cells, row by row, each from a 171 group; extended data ends the last one.
        let body = tail.get(first_cell..).unwrap_or(&[]);
        let body = body.get(..body.iter().position(|t| t.code >= 1000).unwrap_or(body.len())).unwrap_or(&[]);
        let starts: Vec<usize> = body.iter().enumerate().filter(|(_, t)| t.code == 171).map(|(i, _)| i).take(rows * columns).collect();
        for (k, s) in starts.iter().enumerate() {
            let end = starts
                .get(k + 1)
                .copied()
                .unwrap_or_else(|| body.get(s + 1..).and_then(|r| r.iter().position(|t| t.code == 171)).map_or(body.len(), |p| s + 1 + p));
            let cell = body.get(*s..end).unwrap_or(&[]);
            t.cells.push(read_cell(cell));
        }
        t
    }
}

fn read_cell(cell: &[Tag]) -> TableCell {
    let g = Groups(cell);
    let mut text: String = cell.iter().filter(|t| t.code == 2 || t.code == 3).map(Tag::str).collect();
    text.push_str(&g.s(1).unwrap_or_default());
    let span = |code: i32| g.i(code).and_then(|v| u32::try_from(v).ok());
    // The cell's own text height: 140 outside the value block, where it is a number.
    let mut in_value = false;
    let mut text_height = None;
    for t in cell {
        match t.code {
            301 => in_value = true,
            304 => in_value = false,
            140 if !in_value && text_height.is_none() => text_height = Some(t.f64()),
            _ => {}
        }
    }
    TableCell {
        cell_type: g.i_or(171, 1),
        text,
        span: span(176).zip(span(175)),
        merged: g.i_or(173, 0) != 0,
        text_height,
        alignment: g.i(170),
        text_style: g.name(7),
        block: g.handle(340).filter(|h| *h != 0),
    }
}

/// One edge of a table cell style: top, right, bottom, left, inside horizontal, inside vertical.
#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    /// Groups 274 to 279.
    pub lineweight: Option<Lineweight>,
    /// Groups 284 to 289 (shown unless 0).
    pub visible: Option<bool>,
    /// Groups 64 to 69.
    pub color: Option<Color>,
}

/// The groups of one cell type of a TABLESTYLE (data, header, title, in that order).
#[derive(Clone, Debug, PartialEq)]
pub struct CellTypeStyle {
    /// Group 7: the text style name.
    pub text_style: String,
    /// Group 140.
    pub text_height: Option<f64>,
    /// Group 170: the alignment, 1–9.
    pub alignment: Option<i64>,
    /// Group 62.
    pub text_color: Option<Color>,
    /// Group 283 is not 0 and group 63: the fill colour.
    pub fill: Option<Color>,
    pub borders: [Border; 6],
}

/// A TABLESTYLE.
#[derive(Clone, Debug, PartialEq)]
pub struct TableStyle {
    pub handle: Option<u64>,
    /// Group 3.
    pub description: String,
    /// Group 70: the flow direction (0 down, 1 up).
    pub flow: i64,
    /// Groups 40 and 41: the horizontal and vertical cell margins.
    pub margin_h: Option<f64>,
    pub margin_v: Option<f64>,
    /// Groups 280 and 281: the title and the header row are suppressed (not 0).
    pub title_suppressed: bool,
    pub header_suppressed: bool,
    /// Data, header and title, as many as the record has.
    pub cell_types: Vec<CellTypeStyle>,
}

impl TableStyle {
    pub fn read(r: &Record<'_>) -> TableStyle {
        let start = r.tags.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim() == "AcDbTableStyle")).unwrap_or(0);
        let body = r.tags.get(start..).unwrap_or(&[]);
        let body = body.get(..body.iter().position(|t| t.code >= 1000).unwrap_or(body.len())).unwrap_or(&[]);
        // Groups up to the first cell type (code 7) are the style's own.
        let head_end = body.iter().position(|t| t.code == 7).unwrap_or(body.len());
        let head = Groups(body.get(..head_end).unwrap_or(&[]));
        let starts: Vec<usize> = body.iter().enumerate().filter(|(_, t)| t.code == 7).map(|(i, _)| i).take(3).collect();
        let cell_types = starts
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let end = starts
                    .get(k + 1)
                    .copied()
                    .unwrap_or_else(|| body.get(s + 1..).and_then(|x| x.iter().position(|t| t.code == 7)).map_or(body.len(), |p| s + 1 + p));
                let g = Groups(body.get(*s..end).unwrap_or(&[]));
                let border = |k: i32| Border {
                    lineweight: g.i(274 + k).map(Lineweight::from_dxf),
                    visible: g.i(284 + k).map(|v| v != 0),
                    color: g.i(64 + k).map(Color::from_index),
                };
                CellTypeStyle {
                    text_style: g.s(7).unwrap_or_default().trim().to_string(),
                    text_height: g.f(140),
                    alignment: g.i(170),
                    text_color: g.i(62).map(Color::from_index),
                    fill: (g.i_or(283, 0) != 0).then(|| g.i(63).map_or(Color::Index(7), Color::from_index)),
                    borders: [0, 1, 2, 3, 4, 5].map(border),
                }
            })
            .collect();
        TableStyle {
            handle: r.handle(),
            description: head.s(3).unwrap_or_default(),
            flow: head.i_or(70, 0),
            margin_h: head.f(40),
            margin_v: head.f(41),
            title_suppressed: head.i_or(280, 0) != 0,
            header_suppressed: head.i_or(281, 0) != 0,
            cell_types,
        }
    }
}
