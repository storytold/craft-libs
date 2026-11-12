#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// The code came from an app whose lints allow this.
#![allow(clippy::type_complexity)]
//! Markdown: a CommonMark subset plus GitHub tables and strikethrough.
//!
//! Import: ATX and setext headings, paragraphs with soft/hard breaks, block quotes (→ Quote
//! style), fenced and indented code (→ one Code paragraph per line), thematic breaks (→ a
//! bottom-border paragraph), bullet and ordered lists (nested by indentation), GFM pipe tables,
//! and inlines: emphasis/strong (the CommonMark delimiter algorithm), `~~strike~~`, code spans,
//! inline links, autolinks, images (`data:` URIs become pictures, others their alt text),
//! backslash escapes, entities, HTML comments (`<!-- ... -->`), `<br>` and `<a id>` anchors.
//!
//! Export writes the same constructs; pictures become `![alt](media-name)` placeholders.

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

use std::collections::HashMap;
use std::sync::Arc;

use craft_wordmodel::table::{MAX_COLS, MAX_ROWS};
use craft_wordmodel::{Align, Document};

use craft_flow::{self as model, Cell, FBlock, FTable, Fmt, Inline, Kind, ListInfo, Para, make_img};

mod blocks;
mod export;
mod inlines;

pub use craft_flow::MAX_TABLE_CELLS;

/// The text of an entity body (`amp`, `#38`, `#x26`).
fn decode_entity(body: &str) -> Option<String> {
    craft_html::decode_entity(body).map(String::from)
}

pub use blocks::{Refs, import, import_on, parse};
pub use export::escape;
pub use export::export;
#[cfg(test)]
use export::{escape_line_start, inlines_md, table_md, write_blocks};
pub use inlines::parse_inlines;

#[cfg(test)]
use blocks::split_row;
#[cfg(test)]
mod tests;
