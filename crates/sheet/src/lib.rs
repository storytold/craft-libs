#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Spreadsheet vocabulary shared by the spreadsheet format crates (`craft-xls`, `craft-xlsx`,
//! `craft-ods`): cell positions and their A1 text, cell values and error values, sheet-name
//! quoting, the cell formatting enumerations, Excel's built-in number formats and its indexed
//! colour palette.
//!
//! Nothing here is a file format; each format crate re-exports what it uses, so an application
//! gets one `Value` and one `CellRef` whichever file it read.

mod a1;
mod style;
mod tables;
mod value;

pub use a1::{CellRef, MAX_COLS, MAX_ROWS, Range, col_from_letters, col_letters, push_col, quote_sheet, sheet_needs_quotes};
pub use style::{BorderStyle, HAlign, Pattern, Script, Underline, VAlign};
pub use tables::{DEFAULT_PALETTE, builtin_format, builtin_format_id};
pub use value::{CellError, Value, Visibility};
