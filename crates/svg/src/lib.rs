//! SVG: the one place the Craft apps' SVG format code lives.
//!
//! - [`sniff`]: is this an SVG, or a gzip-compressed one (SVGZ)?
//! - [`svgz`]: SVG text from the bytes of a file: SVGZ unpacked within a byte limit, UTF-8,
//!   UTF-16 and ISO 8859-1 decoded; and SVGZ compression.
//! - [`guard`]: checks on the text before any XML reader or SVG converter sees it: how deep the
//!   elements nest, and what the document type's entities expand to.
//! - [`paths`]: the geometry of a document as Bézier paths in millimetres, for cutters, plotters
//!   and CAD import.
//! - [`color`]: CSS colours.
//! - [`write`]: a writer for SVG documents, their path data and numbers.
//! - `tree` (feature `tree`): the full parser. usvg at the one version the Craft apps share,
//!   re-exported, behind an entry point that runs the checks above before it parses.
//!
//! Input is treated as hostile throughout: every size, count and depth is bounded, and a file
//! past a bound is an error, never a panic or an unbounded allocation.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod color;
pub mod guard;
pub mod paths;
pub mod sniff;
pub mod svgz;
#[cfg(feature = "tree")]
pub mod tree;
pub mod write;
