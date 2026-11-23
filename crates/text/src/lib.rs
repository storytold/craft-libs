#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Shared text processing for the Craft apps. It sits below the composer: layout code calls it on
//! plain text and a language before it shapes and places glyphs.
//!
//! [`embed`] is always available and classifies supplied OS/2 `fsType` flags using VectorCraft's
//! outline-embedding compatibility policy. See its documentation for the caller's table-data
//! responsibilities.
//!
//! Feature-gated modules, each behind the cargo feature of the same name:
//! - [`hyphen`] (default): hyphenation points, with bundled US English data and optional
//!   hypher patterns for 47 more languages (Slovak, under the GPL, only with `slovak-gpl`).
//! - [`segment`] (default): line-break opportunities (UAX #14 with kinsoku), and word breaks for
//!   Thai, Lao, Khmer and Burmese with their language features.
//! - [`bidi`] (default): bidirectional text (UAX #9): embedding levels, line reordering and
//!   mirroring.
//! - [`linebreak`] (default): line breaking on a font-independent box/glue/penalty model.
//!
//! Language features (`spanish`, `german`, …, `all-languages`) add data for that language and
//! turn on the modules that use it.

pub mod embed;

#[cfg(feature = "positioned")]
pub mod positioned;

#[cfg(feature = "bidi")]
pub mod bidi;
#[cfg(feature = "hyphen")]
pub mod hyphen;
#[cfg(feature = "linebreak")]
pub mod linebreak;
#[cfg(feature = "segment")]
pub mod segment;

/// Paragraph-wide line composition with stateful line measurement.
pub mod composer;
