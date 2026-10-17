//! Spelling mechanism shared by the Craft apps: Hunspell affix dictionaries and where to find
//! them, with no opinion on which languages an app proofs or how it shows the result.
//!
//! - [`affix`]: an affix dictionary ([`affix::AffixDict`]) built from a Hunspell `.aff`/`.dic`
//!   pair, word checks, generated forms, the suggestion tables (`TRY`, `MAP`, `REP`) and a
//!   compact serialized form ([`affix::AffixDict::to_bytes`]) for fast loading.
//! - [`hunspell`]: finding an installed pair per app ([`hunspell::Search`]), decoding it from
//!   UTF-8, ISO 8859-2, KOI8-R or Windows-1251 and loading it.
//! - [`tag`]: canonical BCP 47 tags and primary subtags, for mapping a text's language to a
//!   dictionary name.
//!
//! Per-app policy stays in the apps: the list of proofed languages, suggestion ranking and
//! grammar rules per language, user dictionaries and how marks are drawn. The code comes from
//! WordCraft's `wordcraft-proof` (storytold/craft-libs#30).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod affix;
mod fx;
pub mod hunspell;
pub mod tag;

pub use affix::AffixDict;
