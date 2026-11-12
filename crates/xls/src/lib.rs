#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room reader for Excel 97-2003 workbooks (`.xls`, BIFF8), written from Microsoft's
//! public specifications **[MS-XLS]: Excel Binary File Format (.xls) Structure** (the Workbook
//! stream) and [MS-CFB] (the compound file around it, read by `craft-cfb`).
//!
//! [`read`] gives a [`Workbook`]: the worksheets with their cells (values, cached formula
//! results, and formulas as A1 text, including shared and array formulas), the cell formats
//! (number formats, fonts, fills, borders, alignment, protection, with palette colours
//! resolved), user-defined cell styles, merged cells, column and row records, sheet display
//! flags, calculation settings, defined names, print areas and print titles, data validation
//! rules, cell comments, conditional formats ("cell value is" and "formula is"), pictures with
//! their cell anchors, the AutoFilter range, and the 1904 date flag. Widths, heights and anchors
//! stay in the file's units.
//!
//! Everything else (charts, other shapes, newer conditional formats, hyperlinks,
//! PivotTables, macros) is skipped and named in [`Read::warnings`]. A workbook encrypted with
//! RC4 or RC4 CryptoAPI is read with its password ([`read_with_password`]; without one, Excel's
//! default password is tried and [`Error::Encrypted`] says one is needed). Other encryption and
//! versions before BIFF8 are refused ([`Error::UnsupportedEncryption`], [`Error::Biff5`]). A formula that
//! cannot be put back into text (links to other workbooks, macro commands, unknown tokens)
//! leaves its cell with the saved result and a [`Warning::FormulaKept`].
//!
//! Input is hostile: [`Limits`] caps records, cells and text, the remaining counts have fixed
//! caps, formula nesting is bounded, and nothing indexes by a number from the file.
//!
//! ```
//! # fn main() -> Result<(), craft_xls::Error> {
//! # let bytes = craft_xls::testing::workbook(&[("Sheet1", vec![vec!["a".into(), "2.5".into()]])]);
//! let read = craft_xls::read(&bytes)?;
//! let sheet = &read.workbook.sheets[0];
//! assert_eq!(sheet.name, "Sheet1");
//! assert_eq!(sheet.cells[&craft_xls::CellRef::new(0, 1)].value, craft_xls::Value::Number(2.5));
//! # Ok(())
//! # }
//! ```

mod cond;
mod crypt;
mod drawing;
mod dv;
mod expr;
mod formula;
mod ftab;
mod model;
mod notes;
mod read;
mod records;
mod styles;
#[doc(hidden)]
pub mod testing;

pub use craft_sheet::{DEFAULT_PALETTE, quote_sheet, sheet_needs_quotes};
pub use model::{
    Alignment, BorderLine, BorderStyle, Borders, Calc, CalcMode, Cell, CellError, CellFormat, CellRef, ClientAnchor, Color, Column, ConditionRule,
    ConditionalFormat, DataValidation, DefinedName, DifferentialFormat, DxfFill, ErrorStyle, Fill, Font, Formula, HAlign, Note, Operator, Pattern,
    Picture, Range, Row, Script, Sheet, Underline, VAlign, ValidationType, Value, View, Visibility, Workbook,
};
pub use read::{is_encrypted, is_xls, read, read_with, read_with_password};

/// Why a workbook could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Not a compound file, no Workbook stream, or a damaged stream.
    #[error("not a readable Excel 97-2003 workbook: {0}")]
    Invalid(String),
    /// More than a [`Limits`] value or a fixed cap allows.
    #[error("the workbook is too large: {0}")]
    TooLarge(String),
    /// The workbook is encrypted with a password (it has a FilePass record): read it with
    /// [`read_with_password`].
    #[error("the workbook is password-protected (encrypted)")]
    Encrypted,
    /// The password does not open the workbook.
    #[error("the password is not the workbook's")]
    WrongPassword,
    /// The workbook is encrypted with a scheme that is not read (XOR obfuscation, CryptoAPI with
    /// AES or another cipher or hash, an external provider), named.
    #[error("the workbook uses {0} encryption, which is not read")]
    UnsupportedEncryption(String),
    /// An Excel 5.0/95 workbook (BIFF5), which is not read.
    #[error("Excel 5.0/95 workbooks (BIFF5) are not supported")]
    Biff5,
}

/// What a read skipped or repaired. Each distinct warning is reported once.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Warning {
    #[error("some cell formatting records were shorter than expected; the formatting present was kept")]
    ShortFormatting,
    #[error("some number formats were shorter than their declared length; the characters present were kept")]
    ShortNumberFormat,
    #[error("chart sheets were skipped")]
    ChartSheet,
    #[error("Excel 4.0 macro sheets were skipped")]
    MacroSheet,
    #[error("VBA modules and other non-worksheet sheets were skipped")]
    OtherSheet,
    #[error("dialog sheets and unreadable sheets were skipped")]
    DialogSheet,
    #[error("some cells refer to missing shared strings and were left empty")]
    MissingSharedString,
    #[error("what-if data tables were not read; their values were kept")]
    DataTable,
    /// Some comment or drawing object records were shorter than they say and were skipped.
    #[error("some comment or object records were damaged and skipped")]
    DamagedObjects,
    /// Charts embedded in a worksheet are not read.
    #[error("charts were not read")]
    Charts,
    /// Pictures in a format that is not read (PICT, TIFF), damaged ones, ones past the limits,
    /// and shapes that name a missing picture.
    #[error("some pictures were damaged, in a format that is not read (PICT, TIFF), or past the limits, and were skipped")]
    PicturesSkipped,
    /// Some CF records could not be read: a rule type other than "cell value is" and "formula
    /// is", a formula that could not be decoded, a damaged record, or more than the reader keeps.
    #[error("some conditional formatting rules could not be read and were skipped")]
    ConditionalFormattingSkipped,
    /// CF12 records (Excel 2007 data bars, colour scales, icon sets) are not read.
    #[error("newer conditional formats (data bars, colour scales, icon sets) were not read")]
    ConditionalFormatting12,
    /// Some Dv records could not be read: Excel would refuse them (an unknown alert style or
    /// operator, a formula or range list past the record), no range was left, a formula could not
    /// be decoded, or the sheet had more rules than the reader keeps.
    #[error("some data validation rules could not be read and were skipped")]
    DataValidationSkipped,
    /// Dv records with no DVal record before them, or past the number of rules it gives: Excel
    /// does not read them either.
    #[error("some data validation records were ignored, as Excel ignores them: no DVal record before them, or past the number of rules it gives")]
    DataValidationIgnored,
    #[error("hyperlinks were not read")]
    Hyperlinks,
    #[error("PivotTables were not read; their values were kept")]
    PivotTables,
    /// Cells beyond the BIFF8 grid (65,536 rows, 256 columns) in one sheet.
    #[error("{0} cells outside the grid were skipped")]
    CellsOutsideGrid(usize),
    /// Some formulas use the named feature; their saved results were kept as values.
    #[error("some formulas use {0}; their saved results were kept as values")]
    FormulaKept(&'static str),
    /// Some defined names use the named feature and were skipped.
    #[error("some defined names use {0} and were skipped")]
    NameSkipped(&'static str),
    #[error("macro names were skipped")]
    MacroNames,
}

/// A workbook and what reading it skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub workbook: Workbook,
    pub warnings: Vec<Warning>,
}

/// Caps on what one workbook may hold. The defaults suit a spreadsheet application; a caller
/// that only wants a table of values can lower them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Records read from the whole Workbook stream (a full 65,536 × 256 sheet has fewer than the
    /// default). One allowance for every sheet: many sheets pointing at one huge substream do
    /// not each get their own.
    pub max_records: usize,
    /// Stored cells in the workbook.
    pub max_cells: usize,
    /// Bytes of text (strings, number formats, expanded shared formulas) in the workbook.
    pub max_text: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_records: 40_000_000, max_cells: 4_000_000, max_text: 256 * 1024 * 1024 }
    }
}
