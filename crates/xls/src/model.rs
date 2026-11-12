//! What a workbook reads into: the file's own tables and cells, with nothing interpreted beyond
//! the format (widths and heights keep the file's units, formulas are text).

use std::collections::BTreeMap;

pub use craft_sheet::{BorderStyle, CellError, CellRef, HAlign, Pattern, Range, Script, Underline, VAlign, Value, Visibility};

/// A cell's formula.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formula {
    /// The formula in A1 notation, without the leading `=`. Function names are the English
    /// ones; newer functions come without their `_xlfn.` prefix.
    pub text: String,
    /// The range of a Ctrl+Shift+Enter array formula, on its top left cell.
    pub array: Option<Range>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub value: Value,
    /// Index into [`Workbook::formats`]. `None` for a cell with plain formatting (see
    /// [`Workbook::formats`]) and for a format index the file does not have.
    pub format: Option<u16>,
    pub formula: Option<Formula>,
}

/// A column's ColInfo record ([MS-XLS] 2.4.53).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Column {
    /// Width in 1/256 of the width of the default font's `0`.
    pub width: u16,
    /// The width was set by the user (`fUserSet`).
    pub custom_width: bool,
    pub hidden: bool,
    /// Outline level, 0 to 7.
    pub outline: u8,
    pub collapsed: bool,
    /// The column's default cell format, as [`Cell::format`].
    pub format: Option<u16>,
}

/// A row's Row record ([MS-XLS] 2.4.221).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// Height in twips (1/20 point).
    pub height: u16,
    /// The height was set by the user (`fUnsynced`).
    pub custom_height: bool,
    pub hidden: bool,
    /// Outline level, 0 to 7.
    pub outline: u8,
    pub collapsed: bool,
    /// The row's default cell format (`fGhostDirty`), as [`Cell::format`].
    pub format: Option<u16>,
}

/// The display flags of a sheet's Window2 record ([MS-XLS] 2.4.346).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub show_formulas: bool,
    pub show_gridlines: bool,
    pub show_headings: bool,
    pub show_zeros: bool,
    pub right_to_left: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CalcMode {
    Manual,
    #[default]
    Automatic,
    /// Automatic except for data tables.
    AutomaticExceptTables,
}

/// Calculation settings. Every worksheet repeats them; `None` is a record the sheet lacks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Calc {
    pub mode: Option<CalcMode>,
    pub iterative: Option<bool>,
    pub max_iterations: Option<u16>,
    pub max_change: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub visibility: Visibility,
    /// The stored cells. A blank cell with plain formatting is not stored.
    pub cells: BTreeMap<CellRef, Cell>,
    /// Merged ranges of more than one cell.
    pub merges: Vec<Range>,
    /// Columns that have a ColInfo record, by zero-based index (0 to 255).
    pub columns: BTreeMap<u16, Column>,
    /// Rows that have a Row record, by zero-based index.
    pub rows: BTreeMap<u32, Row>,
    /// The default row height in twips, when the sheet sets one.
    pub default_row_height: Option<u16>,
    /// `None` when the sheet has no usable Window2 record.
    pub view: Option<View>,
    pub calc: Calc,
    /// The sheet's `Print_Area` name as formula text (`Sheet1!$A$1:$C$9`).
    pub print_area: Option<String>,
    /// The sheet's `Print_Titles` name as formula text (`Sheet1!$1:$2`).
    pub print_titles: Option<String>,
}

/// A user-defined name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinedName {
    pub name: String,
    /// Index into [`Workbook::sheets`] for a sheet-level name, `None` for a workbook-level one.
    pub scope: Option<usize>,
    /// What the name refers to, as formula text.
    pub formula: String,
    pub hidden: bool,
}

/// A colour: one from the workbook's palette, already resolved, or the automatic one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    #[default]
    Auto,
    /// `0xRRGGBB`.
    Rgb(u32),
}

/// A Font record ([MS-XLS] 2.4.122).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Font {
    /// `None` when the record has no (or a blank) name.
    pub name: Option<String>,
    /// Size in points, 1 to 409; `None` when the record gives a height of zero.
    pub size: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub underline: Underline,
    pub script: Script,
    pub color: Color,
    /// The font family (`bFamily`) and character set (`bCharSet`); `None` for a font the file
    /// does not describe.
    pub family: Option<u8>,
    pub charset: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Alignment {
    pub h: HAlign,
    pub v: VAlign,
    pub wrap: bool,
    /// Degrees counter-clockwise, -90 to 90; 255 is vertically stacked text.
    pub rotation: i16,
    pub indent: u8,
    pub shrink: bool,
}

/// One border side. A side without a style has the automatic colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BorderLine {
    pub style: BorderStyle,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Borders {
    pub left: BorderLine,
    pub right: BorderLine,
    pub top: BorderLine,
    pub bottom: BorderLine,
    /// The diagonal from the top left corner down.
    pub diag_down: BorderLine,
    /// The diagonal from the bottom left corner up.
    pub diag_up: BorderLine,
}

/// A cell's fill. Without a pattern both colours are automatic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Fill {
    pub pattern: Pattern,
    /// The pattern's colour (the whole fill for [`Pattern::Solid`]).
    pub fg: Color,
    pub bg: Color,
}

/// The formatting of one XF record ([MS-XLS] 2.4.353), with its font, number format and
/// colours resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct CellFormat {
    pub font: Font,
    /// The number format code; `General` when the format is General or unknown.
    pub number_format: String,
    pub locked: bool,
    pub hidden: bool,
    pub align: Alignment,
    pub borders: Borders,
    pub fill: Fill,
}

impl Default for CellFormat {
    fn default() -> Self {
        CellFormat {
            font: Font::default(),
            number_format: "General".into(),
            locked: true,
            hidden: false,
            align: Alignment::default(),
            borders: Borders::default(),
            fill: Fill::default(),
        }
    }
}

/// A workbook read from a BIFF8 file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workbook {
    /// Dates count from 1904-01-01 instead of 1899-12-31.
    pub date_1904: bool,
    pub precision_as_displayed: bool,
    /// The first worksheet's calculation settings, which Excel applies to the workbook.
    pub calc: Calc,
    /// The worksheets, in workbook order. Chart, macro and dialog sheets are left out.
    pub sheets: Vec<Sheet>,
    /// The sheet the workbook was saved on, as an index into `sheets`: `None` when the file has
    /// no Window1 record or names a sheet that was not read. It may be a hidden sheet; Excel
    /// then opens on the first visible one.
    pub active_sheet: Option<usize>,
    pub names: Vec<DefinedName>,
    /// The font of the Normal style: what an unformatted cell shows. `None` when the file has
    /// no font at all.
    pub normal_font: Option<Font>,
    /// One entry per XF record, in file order. A format is *plain* when it is the Normal font
    /// with everything else at [`CellFormat::default`]; cells, rows and columns with a plain
    /// format carry `None` instead of its index.
    pub formats: Vec<CellFormat>,
    /// User-defined cell styles (name, formatting). Built-in styles are left out.
    pub cell_styles: Vec<(String, CellFormat)>,
}
