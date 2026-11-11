//! What a workbook reads into: the file's own tables and cells, with nothing interpreted beyond
//! the format (widths and heights keep the file's units, formulas are text).

use std::collections::BTreeMap;
use std::sync::Arc;

/// A cell position, zero-based. Ordered by row, then column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    pub const fn new(row: u32, col: u32) -> Self {
        CellRef { row, col }
    }

    /// The A1 text (`B3`).
    pub fn a1(self) -> String {
        crate::expr::a1(self.row, self.col)
    }
}

/// A rectangle of cells, both corners included; `first` is the top left one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Range {
    pub first: CellRef,
    pub last: CellRef,
}

impl Range {
    /// The rectangle spanned by two corners, in any order.
    pub fn new(a: CellRef, b: CellRef) -> Self {
        Range { first: CellRef::new(a.row.min(b.row), a.col.min(b.col)), last: CellRef::new(a.row.max(b.row), a.col.max(b.col)) }
    }

    pub fn contains(&self, c: CellRef) -> bool {
        (self.first.row..=self.last.row).contains(&c.row) && (self.first.col..=self.last.col).contains(&c.col)
    }

    pub fn is_single(&self) -> bool {
        self.first == self.last
    }
}

/// The error values a BIFF8 file stores ([MS-XLS] 2.5.10, BErr).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CellError {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    NA,
    GettingData,
}

impl CellError {
    pub fn as_str(&self) -> &'static str {
        match self {
            CellError::Null => "#NULL!",
            CellError::Div0 => "#DIV/0!",
            CellError::Value => "#VALUE!",
            CellError::Ref => "#REF!",
            CellError::Name => "#NAME?",
            CellError::Num => "#NUM!",
            CellError::NA => "#N/A",
            CellError::GettingData => "#GETTING_DATA",
        }
    }
}

/// A cell's stored value (for a formula cell, its saved result).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Empty,
    Number(f64),
    Text(Arc<str>),
    Bool(bool),
    Error(CellError),
}

impl Value {
    /// A number, or `#NUM!` when it is NaN or infinite (Excel never stores those). Negative zero
    /// becomes zero.
    pub fn number(v: f64) -> Value {
        if v.is_finite() { Value::Number(if v == 0.0 { 0.0 } else { v }) } else { Value::Error(CellError::Num) }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }
}

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    /// Hidden, and not offered by Excel's Unhide dialog.
    VeryHidden,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    SingleAccounting,
    DoubleAccounting,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Script {
    #[default]
    Baseline,
    Superscript,
    Subscript,
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
pub enum HAlign {
    #[default]
    General,
    Left,
    Center,
    Right,
    Fill,
    Justify,
    CenterAcross,
    Distributed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VAlign {
    Top,
    Center,
    #[default]
    Bottom,
    Justify,
    Distributed,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BorderStyle {
    #[default]
    None,
    Thin,
    Medium,
    Dashed,
    Dotted,
    Thick,
    Double,
    Hair,
    MediumDashed,
    DashDot,
    MediumDashDot,
    DashDotDot,
    MediumDashDotDot,
    SlantDashDot,
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

/// Fill patterns, named as SpreadsheetML names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pattern {
    #[default]
    None,
    Solid,
    Gray50,
    Gray75,
    Gray25,
    DarkHorizontal,
    DarkVertical,
    DarkDown,
    DarkUp,
    DarkGrid,
    DarkTrellis,
    LightHorizontal,
    LightVertical,
    LightDown,
    LightUp,
    LightGrid,
    LightTrellis,
    Gray125,
    Gray0625,
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
