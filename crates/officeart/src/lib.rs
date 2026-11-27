//! Office Drawing records ([MS-ODRAW], public Microsoft Open Specifications), as the Office binary
//! formats store them: Word's `.doc` (`craft-doc-binary`), PowerPoint's `.ppt`
//! (`craft-ppt-binary`) and Excel's `.xls` (`craft-xls`) each hold their shapes and pictures in
//! these records.
//!
//! - [`record`]: the record framing, an 8-byte header and a body clipped to what holds it.
//! - [`rt`]: the record types.
//! - [`props`]: shape property tables, their entries and a merged property set.
//! - [`blip`]: the BLIP store and its pictures, and a decode budget.
//! - [`dib`]: device-independent bitmaps as PNG files.
//! - [`tiff`]: TIFF pictures as PNG files.
//! - [`wmf`]: Windows metafiles with their placeable header put back.
//!
//! Everything here reads hostile bytes: a length past the end is clipped, every read is checked,
//! and sizes and counts are bounded.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

pub mod blip;
pub mod dib;
pub mod props;
pub mod record;
pub mod rt;
#[cfg(feature = "tiff")]
pub mod tiff;
pub mod wmf;
