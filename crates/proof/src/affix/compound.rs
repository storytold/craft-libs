//! Flag-based compounding, as the hunspell(5) man page describes it and the `hunspell` 1.7.5 CLI
//! answers on synthetic dictionaries (black-box; every rule below has a test pinned to the CLI's
//! answer in the tests below).
//!
//! A compound is two or more parts, each at least `COMPOUNDMIN` characters, each a stem or an
//! affixed form of one whose flags (the stem's, and the continuation classes of its affixes)
//! allow its place: `COMPOUNDFLAG` anywhere, `COMPOUNDBEGIN` first, `COMPOUNDMIDDLE` inside,
//! `COMPOUNDEND` last. (`COMPOUNDFIRST` and `COMPOUNDLAST` are ignored, as the CLI does.)
//!
//! - A prefix may sit on any part but the last and a suffix only on the last, unless the affix
//!   carries `COMPOUNDPERMITFLAG`. Two suffixes in a row (continuation classes) only on the last.
//! - An affix carrying `ONLYINCOMPOUND` is only valid inside a compound: a prefix anywhere, a
//!   suffix anywhere but on the last part (the CLI's answers; the word alone is no word).
//! - `COMPOUNDFORBIDFLAG` on a stem or a suffix: that part may only be last; on a prefix: no part.
//! - A stem with `NEEDAFFIX` is a part only with an affix; `FORBIDDENWORD` stems are no part, and
//!   a forbidden word is never rebuilt from parts.
//! - `COMPOUNDWORDMAX`: most parts (a `COMPOUNDROOT` stem counts as two). With `COMPOUNDSYLLABLE`
//!   and `LANG hu`, more parts are allowed while the word has at most that many vowels; with
//!   `COMPOUNDSYLLABLE` and another language the CLI applies no limit, and neither does this.
//! - `CHECKCOMPOUNDDUP`: the last two parts may not be the same stem (`foofoo`; `foofoobar` is
//!   fine, as in the CLI).
//! - `CHECKCOMPOUNDCASE`: no capital on either side of a boundary, unless the other side is `-`.
//! - `CHECKCOMPOUNDTRIPLE`: no three equal letters across a boundary (`schiff|fahrt`).
//!   `SIMPLIFIEDTRIPLE`: a part ending in a double letter may share its last letter with the next
//!   part (`schiffahrt` = `schiff` + `fahrt`), but with `CHECKCOMPOUNDTRIPLE` not leave three
//!   equal letters (`busssen`).
//! - `CHECKCOMPOUNDPATTERN end[/flag] begin[/flag]`: no boundary where the part before ends with
//!   `end` and the part after begins with `begin` (and they carry the flags); `0` for `end` means
//!   a part without a suffix. The optional replacement field is refused.
//! - `CHECKCOMPOUNDREP`: no compound that a `REP` replacement turns into a word (not a compound),
//!   checked on every two neighbouring parts and on every run of parts that ends the word.
//! - A compound of three or more parts is rejected when one slip turns it into a word (not a
//!   compound): see [`AffixDict::near_word`]. The CLI's answers, with or without
//!   `CHECKCOMPOUNDREP`.
//! - `FORCEUCASE` on the last part's stem: a word with no capital is wrong.
//! - Case: `KEEPCASE` holds for the first part only; a capitalized stem may be a part, and a word
//!   with capitals inside may start with a capital (`FooQux`).
//! - `ONLYINCOMPOUND` homonyms beside others keep their own flags (compound parts only).
//! - `MAXCPDSUGS 0`: compounds are never suggested.
//!
//! Not read: `SYLLABLENUM` (Hungarian), which the man page doesn't describe; it is ignored.
//!
//! The work per word is bounded: words up to [`MAX_WORD_BYTES`](super::MAX_WORD_BYTES), at most
//! [`MAX_PARTS`] parts, each part's analyses computed once, and at most [`MAX_WORK`] steps (rules
//! tried and splits followed) per word; past that the word is no compound.

use super::{AffixDict, Excluded, FlagKey, FlagMode, Rule, has};

mod analysis;
mod near;
mod parse;
#[cfg(test)]
mod tests;

pub(crate) use analysis::Pieces;
use analysis::*;
pub(in crate::affix) use parse::{Compounding, Raw, is_directive};

/// Most parts a compound may have, whatever `COMPOUNDWORDMAX` says.
pub(super) const MAX_PARTS: usize = 16;
/// Most steps one compound check may take (affix rules tried, parts looked up, splits followed).
pub(super) const MAX_WORK: usize = 1 << 16;
/// Most steps the compound search of one suggestion candidate may take.
pub(crate) const MAX_SUGGEST_WORK: usize = 1 << 13;
/// Most steps the compound searches of one suggestion call may take in all; the candidates
/// left are checked as simple words only, so a long garbled word's suggestions stay quick.
pub(crate) const MAX_SUGGEST_COMPOUND_WORK: usize = 1 << 17;
