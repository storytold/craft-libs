#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Rich Text Format.
//!
//! Three layers, each usable alone:
//!
//! - [`Tokens`]: the tokens of a file (groups, control words, `\'hh`, text runs, `\bin` data).
//! - [`TextDecoder`] and [`codepage`]: text in the file's code pages, `\uN` with its fallback
//!   characters and surrogate pairs. [`esc`] goes the other way. [`dttm_to_iso`],
//!   [`iso_to_dttm`], [`date_words`] and [`iso_date`] are RTF's two date forms.
//! - [`read`] and [`write`]: a file to and from a small neutral model ([`Document`]):
//!   paragraphs with outline level, alignment, indents and spacing, runs with bold, italic,
//!   underline, strikethrough, superscript and subscript, size, colour and font, and tables.
//!
//! A reader that keeps more than the model holds (styles, lists, fields, notes, pictures) is
//! built on the first two layers.
//!
//! ```
//! use craft_rtf::{Block, read, write};
//!
//! let doc = read(br"{\rtf1\ansi\pard\outlinelevel0 Caf\'e9 {\b \u8364?5}\par}")?;
//! let Some(Block::Para(p)) = doc.blocks.first() else { return Ok(()) };
//! assert_eq!((p.text().as_str(), p.outline_level), ("Café €5", Some(0)));
//! assert!(p.runs[1].fmt.bold);
//! assert_eq!(read(write(&doc, "Helvetica").as_bytes())?, doc);
//! # Ok::<(), craft_rtf::NotRtf>(())
//! ```

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

pub mod codepage;
mod date;
#[cfg(feature = "doc")]
pub mod doc;
mod lex;
mod model;
#[cfg(feature = "outlook")]
pub mod outlook;
mod read;
mod text;
mod write;

pub use date::{date_words, dttm_to_iso, iso_date, iso_to_dttm};
pub use lex::{Token, Tokens, is_rtf};
pub use model::{Align, Block, COLUMN_BREAK, Cell, CharFmt, Document, LINE_BREAK, PAGE_BREAK, Para, Run, Table, VertAlign};
pub use read::{MAX_COLORS, MAX_FONT_NAME, MAX_FONTS, MAX_GROUPS, NotRtf, read};
pub use text::TextDecoder;
pub use write::{esc, esc_font, push_esc, write};
