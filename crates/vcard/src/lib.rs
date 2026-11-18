#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! vCard: contacts as vCard text (`.vcf` files, CardDAV address data).
//!
//! [`parse`] reads vCard 2.1, 3.0 and 4.0, one card or many in a text ([`parse_bytes`] in a file,
//! each value in its own `CHARSET`); [`write()`] writes vCard 3.0, the version every address book
//! reads. Both work on [`Contact`], this crate's own plain model of a card.
//!
//! Input is hostile. Nothing here panics on any text: line lengths and counts are capped, and what
//! makes no sense is skipped.

mod key;
#[cfg(test)]
mod key_tests;
mod lines;
mod model;
mod read;
#[cfg(test)]
mod tests;
mod text;
mod write;

pub use model::{Contact, ContactKey, KeyKind, Labeled, PostalAddress};
pub use read::{parse, parse_bytes};
pub use write::write;
