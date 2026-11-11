//! The sheets of a print-ready PDF, read leniently, and the incremental update that gives them
//! one paper shape. Windows prints a job with one PrintTicket, so with one paper size and
//! orientation, and draws each page at the size the PDF gives it. A job whose sheets differ is
//! brought to one shape here, by appending new versions of the pages that need it: `/Rotate`
//! turns a sheet of the other orientation a quarter turn, and for shrink-to-fit a page gets the
//! paper's size with its content scaled down (never up) and centred. The original bytes are
//! kept as they are, so a job that needs nothing is the caller's PDF unchanged.
//!
//! Only what that takes is read: the cross-reference sections (tables and streams, object
//! streams, FlateDecode with PNG predictors), the page tree and each page's boxes and rotation.
//! Every length, count and depth is bounded, and anything unreadable is an [`Error`], never a
//! panic: the caller's PDF is input like any other.

use crate::Error;

/// Nesting of arrays and dictionaries, and of the page tree.
const MAX_DEPTH: usize = 64;
const MAX_PAGES: usize = 100_000;
/// The most a cross-reference or object stream may inflate to.
const MAX_STREAM: usize = 16 << 20;
// A short compressed file can otherwise expand into millions of hash-map entries.
// These are job-wide budgets, including older xref sections and cached object streams.
const MAX_RECORDS: usize = 500_000;
const MAX_DECODED: usize = 32 << 20;
const MAX_NODES: usize = 250_000;
const MAX_TOKENS: usize = 16 << 20;
const MAX_OBJECT_NODES: usize = 32_000;
const MAX_OBJECT_TOKENS: usize = 1 << 20;
const MAX_SECTIONS: usize = 256;
/// The most the page objects of repeated copies ([`Repeat`]) may take, in bytes.
const MAX_REPEATED: usize = 64 << 20;
/// A page without a MediaBox (invalid, but readers accept it) is US Letter.
const DEFAULT_MEDIA: [f64; 4] = [0.0, 0.0, 612.0, 792.0];
/// Sheet sizes this close, in points (0.18 mm), are the same size.
pub(crate) const SAME: f64 = 0.5;

fn unreadable(detail: &str) -> Error {
    Error::Spool(format!("the print-ready PDF could not be read for printing: {detail}"))
}

mod object;
mod plan;
mod read;
mod update;
mod xref;

pub(crate) use plan::{Edit, Fit, plan};
pub(crate) use read::{Sheet, Sheets, read};
#[cfg(windows)]
pub(crate) use update::Repeat;

use object::*;
use xref::*;

#[cfg(test)]
#[path = "tests/pdf_bounds.rs"]
mod tests;
