#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Excel workbooks: `.xlsx`/`.xlsm` (SpreadsheetML, ECMA-376 Part 1, transitional and strict) and
//! `.xlsb` (the same package with binary parts, [MS-XLSB]). Clean-room, from the specifications
//! and observed files.
//!
//! This is the part layer: a [`Package`] opens the OPC container (`craft-opc`) and reads
//!
//! - the workbook part: sheets, the date system, defined names ([`Workbook`]);
//! - shared strings with their rich-text runs and phonetic readings ([`StringItem`]);
//! - the styles tables as the file has them ([`Styles`]): number formats, fonts, fills, borders,
//!   cell formats, named styles and differential formats, by index;
//! - one worksheet at a time as a stream of [`Row`]s of [`Cell`]s, so a sheet of any length is
//!   read without holding it; what is left of the part (properties, columns, merged cells,
//!   validation, and so on) comes back as a tree ([`SheetPart`]).
//!
//! Parts are read with fixed prefixes ([`ns`]): the SpreadsheetML main namespace has none
//! (`sheetData`), the others have theirs whatever the file used (`r:id`, `x14:sparklineGroups`,
//! `mc:AlternateContent`), so a tree kept from a part can be written back with [`ns::declare`].
//!
//! Nothing is interpreted beyond the format: a cell holds a shared-string index and a format
//! index, a width is in the file's units, a formula is the file's text. Every other part
//! (drawings, charts, pivot caches, macros) is reachable through [`Package::opc`], so a caller
//! can read it or carry it through unchanged.
//!
//! A binary workbook gives the same [`Workbook`], [`StringItem`]s, [`Row`]s and merged cells:
//! saved values only, since formulas there are token streams that are not decoded and styles are
//! not read.
//!
//! Reading SpreadsheetML is lenient: a damaged optional part is skipped and named in
//! [`Package::warnings`]; only an unreadable package or workbook part is an error. Reading a
//! binary workbook is strict: a record that does not parse fails the read, because skipping it
//! would lose data silently. Input is hostile: [`Limits`] bounds the package, the XML and the
//! binary records, cells outside the grid are dropped, and nothing indexes by a number from the
//! file.
//!
//! ```
//! # fn main() -> Result<(), craft_xlsx::Error> {
//! # let bytes = craft_xlsx::testing::workbook(&[("Prices", r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>2.5</v></c></row>"#)], &["Tea"]);
//! let mut package = craft_xlsx::Package::open(&bytes, &craft_xlsx::Limits::default())?;
//! let workbook = package.workbook()?;
//! let strings = package.shared_strings()?;
//! let mut values = Vec::new();
//! package.sheet(&workbook.sheets[0], |row| {
//!     values.extend(row.cells.iter().map(|c| c.value.resolve(&strings, workbook.date_1904)));
//!     Ok(())
//! })?;
//! assert_eq!(values, [craft_xlsx::Value::text("Tea"), craft_xlsx::Value::Number(2.5)]);
//! # Ok(())
//! # }
//! ```

mod document;
mod drawing;
#[macro_use]
mod features;
pub mod ns;
mod package;
mod pivot;
mod richdata;
mod sheet;
mod strings;
mod styles;
#[doc(hidden)]
pub mod testing;
mod workbook;
mod write;
mod xlsb;

pub use craft_drawingml;
pub use craft_sheet::{
    BorderStyle, CellError, CellRef, DEFAULT_PALETTE, HAlign, MAX_COLS, MAX_ROWS, Pattern, Range, Script, Underline, VAlign, Value, Visibility,
    builtin_format, builtin_format_id, quote_sheet, sheet_needs_quotes,
};
pub use document::{Document, DrawingPart, KeptPart, PivotCachePart, PivotTablePart, TablePart, Worksheet, XlsxKind};
pub use drawing::{Anchor, Drawing, Item, Marker, ObjectKind, Placement};
pub use features::{
    Break, CfRule, Cfvo, ColorScale, Comment, Comments, ConditionalFormatting, DataBar, DataValidation, DataValidations, Features, HeaderFooter,
    Hyperlink, IconSet, NoteShape, PageMargins, PageSetup, Pane, Print, PrintOptions, Selection, SheetView, Table, TableColumn, TableStyleInfo,
    notes_vml, parse_sqref, range_ref,
};
pub use package::{Package, is_xlsb};
pub use pivot::{
    CacheField, CacheItem, CacheRecord, CacheSource, CacheValue, DataField, FieldGroup, Location, PageField, PivotCacheDefinition, PivotField,
    PivotItem, PivotTableDefinition, PivotTableStyleInfo, RangePr, SharedItems, WorksheetSource, records_xml,
};
pub use richdata::{CellPicture, MAX_PICTURE, MAX_PICTURES_TOTAL, MAX_RECORDS};
pub use sheet::{Cell, CellFormula, CellValue, Column, FormulaKind, Row, SheetPart, iso_date_serial};
pub use strings::{PhoneticProps, PhoneticRun, StringItem, TextRun, decode_xstring, encode_xstring, rich_text, t_el};
pub use styles::{
    Alignment, Border, BorderEdge, CellStyle, Color, ColorSource, Dxf, Fill, Font, FontScheme, GradientFill, PatternFill, Protection, Styles, Xf,
};
pub use workbook::{DefinedName, PivotCacheEntry, SheetEntry, SheetKind, Workbook};
pub use write::num;

/// Why a workbook could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a ZIP package, or an entry is damaged.
    #[error("the file is not a valid zip package: {0}")]
    Zip(String),
    /// A part that must be read holds XML that cannot be.
    #[error("the file contains invalid XML: {0}")]
    Xml(String),
    /// The package is not a workbook, or a binary part is damaged.
    #[error("the file is not a valid spreadsheet: {0}")]
    Invalid(String),
    /// More than [`Limits`] or a fixed cap allows.
    #[error("the file is too large: {0}")]
    TooLarge(String),
}

impl From<craft_opc::Error> for Error {
    fn from(e: craft_opc::Error) -> Self {
        match e {
            e if e.is_limit() => Error::TooLarge(e.to_string()),
            craft_opc::Error::Zip(m) => Error::Zip(m),
            craft_opc::Error::Xml { .. } => Error::Xml(e.to_string()),
            e => Error::Invalid(e.to_string()),
        }
    }
}

impl From<craft_xml::Error> for Error {
    fn from(e: craft_xml::Error) -> Self {
        if e.is_limit() { Error::TooLarge(e.to_string()) } else { Error::Xml(e.to_string()) }
    }
}

/// What a read refuses to exceed. The defaults suit a spreadsheet application; a caller that
/// only wants a table of values can lower them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The ZIP package: entries, bytes per part and bytes in all.
    pub package: craft_opc::Limits,
    /// One XML part. Rows and shared strings are streamed, so `max_nodes` bounds one row or one
    /// string and what is left of a part, not a sheet's length.
    pub xml: craft_xml::Limits,
    /// Records read from a binary workbook, every part together. Records are skipped or decoded
    /// without being kept, so this bounds parse time, not memory. Excel writes four records per
    /// row besides the cells, and one per formatted empty cell, so a sheet needs many more
    /// records than cells.
    pub binary_records: usize,
    /// Cells with a value read from a binary workbook, over all its sheets; also the most
    /// shared-string items.
    pub binary_cells: usize,
    /// Bytes of decoded text (shared and inline strings) in a binary workbook, over all its parts.
    pub binary_text: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            package: craft_opc::Limits::new(200_000, 512 << 20, 1 << 30),
            xml: craft_xml::Limits { max_input: 512 << 20, ..craft_xml::Limits::DEFAULT },
            // The limits of `craft-xls`.
            binary_records: 40_000_000,
            binary_cells: 4_000_000,
            binary_text: 256 << 20,
        }
    }
}

/// What a read skipped or repaired, each distinct message once and at most 1000 of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Warnings(pub(crate) Vec<String>);

impl Warnings {
    pub(crate) fn warn(&mut self, m: impl Into<String>) {
        let m = m.into();
        if self.0.len() < 1000 && !self.0.contains(&m) {
            self.0.push(m);
        }
    }
}
