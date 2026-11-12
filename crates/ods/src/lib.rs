#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! OpenDocument spreadsheets (`.ods`, ODF 1.3 Part 3, `table:` elements of Part 3 section 9),
//! on `craft-odf` for the package. Clean-room, from the specification and observed files.
//!
//! [`read`] gives a [`Workbook`]: each `table:table` of the document body as a [`Sheet`] of
//! [`Cell`]s with their value type, value, displayed text, formula and style name as the file
//! has them, repeated rows and columns expanded, and merged cells (`number-columns-spanned`,
//! `number-rows-spanned`) as ranges. [`Cell::value`] gives the value a cell shows, with dates
//! and durations as serial numbers.
//!
//! Formulas are kept as text (`of:=SUM([.A1:.A3])`), not translated. Styles, charts, pictures,
//! annotations and settings are not read. An encrypted package is [`Error::Encrypted`]: decrypt
//! it with `craft_odf::crypt::decrypt` first.
//!
//! The XML is read strictly, as the source reader did: unbalanced markup, a document type
//! declaration or an undefined entity fails the read. Input is hostile: [`Options`] bounds the
//! package, the cells, the merged ranges and the text, repeats are checked against the grid
//! before anything is expanded, and nothing indexes by a number from the file.
//!
//! ```
//! # fn main() -> Result<(), craft_ods::Error> {
//! # let bytes = craft_ods::testing::spreadsheet(r#"<table:table table:name="Prices"><table:table-row><table:table-cell office:value-type="string"><text:p>Tea</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="2.5" table:number-columns-repeated="2"/></table:table-row></table:table>"#);
//! let book = craft_ods::read(&bytes, &craft_ods::Options::default())?;
//! let sheet = &book.sheets[0];
//! assert_eq!(sheet.name, "Prices");
//! assert_eq!(sheet.cells[&craft_ods::CellRef::new(0, 2)].value(), Some(craft_ods::Value::Number(2.5)));
//! # Ok(())
//! # }
//! ```

mod content;
mod model;
#[doc(hidden)]
pub mod testing;

pub use content::read_content;
pub use craft_sheet::{CellError, CellRef, MAX_COLS, MAX_ROWS, Range, Value};
pub use model::{Cell, Sheet, ValueType, Workbook, date_serial, duration_days};

/// Why a spreadsheet could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Not a ZIP package, a damaged entry, or no spreadsheet mimetype.
    #[error("not a readable OpenDocument spreadsheet: {0}")]
    Invalid(String),
    /// `content.xml` cannot be read or is not a spreadsheet document.
    #[error("the spreadsheet contains invalid XML: {0}")]
    Xml(String),
    /// The package is password-protected.
    #[error("the spreadsheet is password-protected (encrypted)")]
    Encrypted,
    /// More than [`Options`] or a fixed cap allows.
    #[error("the spreadsheet is too large: {0}")]
    TooLarge(String),
}

impl From<craft_odf::Error> for Error {
    fn from(e: craft_odf::Error) -> Self {
        if e.is_limit() { Error::TooLarge(e.to_string()) } else { Error::Invalid(e.to_string()) }
    }
}

/// What a read refuses to exceed, and which sheets it keeps. The defaults suit a spreadsheet
/// application; a caller that only wants a table of values can lower them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// The ZIP package.
    pub package: craft_odf::Limits,
    /// Cells with content (a value, text or a formula), repeats expanded, over all sheets.
    pub max_cells: u64,
    /// Merged ranges, repeats expanded, over all sheets.
    pub max_merges: u64,
    /// Bytes of cell text, repeats expanded, over all sheets.
    pub max_text: u64,
    /// Bytes of one cell's text, and of one attribute value.
    pub max_cell_text: usize,
    /// Keep the cells of this sheet only (by position); the others are listed by name, empty.
    pub only_sheet: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            package: craft_odf::Limits::DEFAULT,
            max_cells: 1_000_000,
            max_merges: 100_000,
            max_text: 64 << 20,
            max_cell_text: 1 << 20,
            only_sheet: None,
        }
    }
}

/// Is this a package whose mimetype is an OpenDocument spreadsheet (or spreadsheet template)?
pub fn is_ods(bytes: &[u8]) -> bool {
    craft_odf::Package::open(bytes, &craft_odf::Limits::DEFAULT)
        .ok()
        .and_then(|mut p| p.mimetype())
        .is_some_and(|m| m == craft_odf::mime::SPREADSHEET || m == craft_odf::mime::SPREADSHEET_TEMPLATE)
}

/// Reads a spreadsheet package (`.ods`, `.ots`).
pub fn read(bytes: &[u8], options: &Options) -> Result<Workbook, Error> {
    let mut package = craft_odf::Package::open(bytes, &options.package)?;
    match package.mimetype().as_deref() {
        Some(craft_odf::mime::SPREADSHEET | craft_odf::mime::SPREADSHEET_TEMPLATE) => {}
        _ => return Err(Error::Invalid("missing or invalid ODS mimetype".into())),
    }
    if package.is_encrypted() {
        return Err(Error::Encrypted);
    }
    let content = package.read("content.xml")?.ok_or_else(|| Error::Invalid("content.xml is missing".into()))?;
    read_content(&content, options)
}
