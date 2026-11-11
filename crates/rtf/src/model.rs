//! A small neutral model of formatted text: what the reader gives and the writer takes.

/// A line break inside a paragraph (`\line`), in [`Run::text`].
pub const LINE_BREAK: char = '\n';
/// A page break (`\page`), in [`Run::text`].
pub const PAGE_BREAK: char = '\u{000C}';
/// A column break (`\column`), in [`Run::text`].
pub const COLUMN_BREAK: char = '\u{000E}';

/// Character formatting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharFmt {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// `\fsN`: the size in half-points (24 is 12 pt). `None`: the reader's default.
    pub half_points: Option<u32>,
    /// `\cfN` through the colour table, as red, green, blue. `None`: the default colour.
    pub color: Option<[u8; 3]>,
    /// `\fN` through the font table. `None`: the default font.
    pub font: Option<String>,
}

/// Text in one formatting. Tabs are `\t`; breaks are [`LINE_BREAK`], [`PAGE_BREAK`] and
/// [`COLUMN_BREAK`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub fmt: CharFmt,
}

/// A paragraph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Para {
    pub runs: Vec<Run>,
    /// `\outlinelevelN`, 0 to 8: a heading of level N + 1. `None`: body text.
    pub outline_level: Option<u8>,
    /// `\liN`: the left indent in twips (1/1440 inch).
    pub left_indent: i32,
}

impl Para {
    /// The paragraph's text without its formatting.
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

/// A table cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// `\cellxN`: the cell's right edge from the table's left edge, in twips. A merged cell
    /// ends at the right edge of the last column it spans.
    pub right: i32,
    pub paras: Vec<Para>,
}

/// A table: rows of cells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Para(Para),
    Table(Table),
}

/// A document: the body's paragraphs and tables, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// Pictures the reader left out (`\pict`, `\object`).
    pub pictures_left_out: usize,
}

impl Document {
    /// Every paragraph in reading order, a table's cells row by row.
    pub fn paras(&self) -> impl Iterator<Item = &Para> {
        self.blocks.iter().flat_map(|b| -> Box<dyn Iterator<Item = &Para> + '_> {
            match b {
                Block::Para(p) => Box::new(std::iter::once(p)),
                Block::Table(t) => Box::new(t.rows.iter().flatten().flat_map(|c| &c.paras)),
            }
        })
    }
}
