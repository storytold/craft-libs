//! What a workbook reads into: the file's own tables and cells, with nothing interpreted beyond
//! the format (widths and heights keep the file's units, formulas are text).

use std::collections::BTreeMap;
use std::sync::Arc;

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
    /// Data validation rules (Dv records), in file order.
    pub validations: Vec<DataValidation>,
    /// Cell comments (Note records with their text), in file order.
    pub notes: Vec<Note>,
    /// Conditional formats (CF records), in file order: the first has the highest priority.
    pub conditional_formats: Vec<ConditionalFormat>,
    /// Pictures placed on the sheet, in drawing order.
    pub pictures: Vec<Picture>,
    /// The AutoFilter's range: the sheet's `_FilterDatabase` name, when the sheet has an
    /// AutoFilterInfo record (the name can outlive the filter). The criteria are not read.
    pub autofilter: Option<Range>,
}

/// A cell comment (Note, [MS-XLS] 2.4.179, with the TxO text of its object).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub cell: CellRef,
    pub author: String,
    /// Empty when the comment's text object is missing.
    pub text: String,
    /// Shown all the time, not only when the cell is pointed at.
    pub visible: bool,
}

/// Where a picture sits: its top left and bottom right corners as a cell and a fraction of it
/// (OfficeArtClientAnchorSheet, [MS-XLS] 2.5.193): `dx` in 1/1024 of the column's width, `dy`
/// in 1/256 of the row's height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientAnchor {
    pub col1: u16,
    pub dx1: u16,
    pub row1: u16,
    pub dy1: u16,
    pub col2: u16,
    pub dx2: u16,
    pub row2: u16,
    pub dy2: u16,
}

/// A picture: a shape that names a stored picture and has a client anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub anchor: ClientAnchor,
    /// The file: PNG, JPEG, EMF, WMF (with its placeable header put back) or BMP (a DIB with a
    /// file header in front). Shapes that show one picture share its bytes.
    pub data: Arc<Vec<u8>>,
    /// `image/png`, `image/jpeg`, `image/x-emf`, `image/x-wmf` or `image/bmp`.
    pub mime: &'static str,
}

/// What a conditional format tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConditionRule {
    /// "Cell value is": the cell compared with one formula, or two for `Between` and
    /// `NotBetween`.
    CellIs { operator: Operator, formula1: String, formula2: Option<String> },
    /// A formula that is true.
    Expression { formula: String },
}

/// The fill a differential format sets: each part `None` when it leaves the cell's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DxfFill {
    pub pattern: Option<Pattern>,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

/// What a conditional format changes (DXFN, [MS-XLS] 2.5.81): each part `None` when it leaves
/// the cell's. The font's face and size are never changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DifferentialFormat {
    pub number_format: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub strike: Option<bool>,
    pub underline: Option<Underline>,
    pub font_color: Option<Color>,
    pub left: Option<BorderLine>,
    pub right: Option<BorderLine>,
    pub top: Option<BorderLine>,
    pub bottom: Option<BorderLine>,
    pub fill: Option<DxfFill>,
}

/// A conditional format (CondFmt with one of its CF records).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionalFormat {
    pub ranges: Vec<Range>,
    /// Formulas are A1 text without `=`, relative references from the top left cell of the
    /// CondFmt's bounding range.
    pub rule: ConditionRule,
    pub format: DifferentialFormat,
}

/// What a data validation rule allows ([MS-XLS] 2.4.95 `typOperator`'s sibling `valType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValidationType {
    /// Any value; also the undefined types 8 to 15, which Excel opens as "any value".
    #[default]
    Any,
    Whole,
    Decimal,
    List,
    Date,
    Time,
    TextLength,
    Custom,
}

/// How a value is compared with one or two formulas: in data validation (whole, decimal,
/// date, time and text length rules) and in "cell value is" conditional formats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Operator {
    #[default]
    Between,
    NotBetween,
    Equal,
    NotEqual,
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
}

/// The alert shown for a value the rule refuses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ErrorStyle {
    #[default]
    Stop,
    Warning,
    Information,
}

/// A data validation rule (Dv, [MS-XLS] 2.4.95), in the terms SpreadsheetML uses for the same
/// rule (`dataValidation`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataValidation {
    /// The ranges it covers. A range over every BIFF8 row (column) is a whole column (row) of
    /// the 1,048,576 × 16,384 grid, as SpreadsheetML stores it.
    pub ranges: Vec<Range>,
    pub kind: ValidationType,
    /// `Between` for the types without an operator.
    pub operator: Operator,
    /// The first formula as A1 text without `=`, relative references from the top left cell of
    /// all the ranges. A literal list is quoted, its items separated by commas and each `"`
    /// doubled (`"Red,Green"`), as SpreadsheetML's `formula1`. Empty for "any value".
    pub formula1: String,
    /// The upper bound of `Between` and `NotBetween`.
    pub formula2: Option<String>,
    pub allow_blank: bool,
    /// A list shows its drop-down arrow in the cell.
    pub in_cell_dropdown: bool,
    pub prompt_title: String,
    pub prompt: String,
    pub show_prompt: bool,
    pub error_title: String,
    pub error: String,
    pub error_style: ErrorStyle,
    pub show_error: bool,
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
