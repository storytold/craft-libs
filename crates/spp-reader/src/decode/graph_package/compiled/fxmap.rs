//! Pattern programs of fx-map nodes (opcodes 0x08 grayscale, 0x09 colour): a tree of records the engine walks
//! to place pattern instances.
//!
//! Record tag: bits 0–3 the kind (0xb iterate, 0x9 switch, 0x8 quadrant), bit 4 an inline random-seed word,
//! bit 5 a random-seed expression, bit 6 / 7 kind-specific (below), bits 8–31 kind-specific.
//! - **Iterate** (0xb): `[seed] [count expression if bit 7, else the count is tag >> 8] [first if bit 6] body`;
//!   `first` (0 = none) is the extra input of an iteration that runs before the repeated body.
//! - **Switch** (0x9): `[seed] [condition expression if bit 7] branch0 branch1` (0 = no branch); the condition picks
//!   branch1 when true. Without an expression, bit 6 is the condition (measured: a switch that always takes branch 1).
//! - **Quadrant** (0x8): `[image word if slot 7 has code 2 or 3] [array ptr] children…`. Bit 6 draws built-in pattern
//!   byte 1; bit 7 draws an image, byte 1 being an index into the node's image inputs; neither draws nothing.
//!   The high half holds 2-bit codes: slot 0 the children (1: four child pointers, 2: one child drawn in all four
//!   quadrants), slots 1–7 parameters, stored in order in the array after the seed. Code 1 is an expression pointer,
//!   code 2 inline words, code 3 an expression pointer followed by inline words.
//!
//! Exceptions: slot 1 has no pointer (code 1 is a flag without data, codes 2 and 3 one inline word). Slot 7 has code 1 a
//! pointer, code 2 no array data, code 3 a pointer. Bit 1 of the image word adds one more expression pointer at the end
//! of the array. Layout checked on 10,451 fx-maps from 1,019 graphs: all 30,720 arrays end exactly on the next object.
//!
//! Slots: 1 pattern variation (one value; probable), 2 pattern offset (float2; probable), 3 branch offset (float2; the
//! only parameter of quadrants that draw nothing), 4 pattern size (float2; probable), 5 rotation in turns (seen against
//! reference images), 6 luminance or colour (probable), 7 and the image word's expression not known yet.

use super::atomic::Value;
use super::expr::Expr;
use super::section::Section;
use crate::decode::DecodeError;

const RECORD: &str = "compiled graph pattern program";
const MAX_DEPTH: usize = 64;
const MAX_RECORDS: usize = 100_000;

/// Inline words per quadrant parameter slot (1–7).
const SLOT_WORDS: [u32; 8] = [0, 1, 2, 2, 2, 1, 1, 1];

/// Record tag bits 0–3: the record kind.
const TAG_KIND: u32 = 0xf;
const KIND_ITERATE: u32 = 0xb;
const KIND_SWITCH: u32 = 0x9;
const KIND_QUADRANT: u32 = 0x8;
/// Bit 4: an inline random-seed word follows.
const TAG_SEED_INLINE: u32 = 0x10;
/// Bit 5: a random-seed expression pointer follows.
const TAG_SEED_EXPR: u32 = 0x20;
/// Bit 6: iterate has a `first` record; switch's constant condition; quadrant draws a built-in pattern.
const TAG_BIT6: u32 = 0x40;
/// Bit 7: iterate/switch have a count/condition expression; quadrant draws an image input.
const TAG_BIT7: u32 = 0x80;
/// Bits 8–31: an iterate's constant count; bits 8–15 a quadrant's pattern byte.
const TAG_HIGH_SHIFT: u32 = 8;
/// Bits 16–31 of a quadrant: eight 2-bit slot codes (slot 0 the children, 1–7 parameters).
const TAG_SLOT_CODES_SHIFT: u32 = 16;
/// Bit 1 of a quadrant's image word: one more expression pointer at the end of the parameter array.
const IMAGE_WORD_EXTRA_EXPR: u32 = 2;

/// Inline words of quadrant parameter slot `k` (0 for slots outside 1–7).
fn slot_words(k: u32) -> u32 {
    usize::try_from(k).ok().and_then(|k| SLOT_WORDS.get(k)).copied().unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Seed {
    Const(u32),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Count {
    Const(u32),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Iterate {
        seed: Option<Seed>,
        count: Count,
        first: Option<Box<Record>>,
        body: Option<Box<Record>>,
    },
    Switch {
        seed: Option<Seed>,
        condition: Count,
        branches: [Option<Box<Record>>; 2],
    },
    Quadrant {
        seed: Option<Seed>,
        pattern: u8,
        draws: bool,
        /// Draws an image input instead of a built-in pattern: `pattern` is then an index into the node's inputs.
        image: bool,
        /// Slot 1 code 1 (meaning not known yet).
        flag1: bool,
        /// The word before the array when slot 7 has code 2 or 3 (meaning not known yet; seen: 2, 4, 6, 8, 10).
        image_word: Option<u32>,
        /// (slot 1–7, value), then (8, the expression bit 1 of `image_word` adds)
        params: Vec<(u8, Value)>,
        /// Four children (top-left, top-right, bottom-left, bottom-right), or one shared by all four.
        children: Children,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Children {
    None,
    Shared(Box<Record>),
    Four([Option<Box<Record>>; 4]),
}

impl Record {
    pub fn parse(s: &Section<'_>, off: u32) -> Result<Record, DecodeError> {
        let mut budget = MAX_RECORDS;
        parse(s, off, 0, &mut budget)
    }

    /// Records in this tree (including this one).
    pub fn count(&self) -> usize {
        1 + match self {
            Record::Iterate { first, body, .. } => first.iter().chain(body).map(|r| r.count()).sum(),
            Record::Switch { branches, .. } => branches.iter().flatten().map(|r| r.count()).sum(),
            Record::Quadrant { children, .. } => match children {
                Children::None => 0,
                Children::Shared(r) => r.count(),
                Children::Four(c) => c.iter().flatten().map(|r| r.count()).sum(),
            },
        }
    }
}

fn opt_child(s: &Section<'_>, ptr: u32, depth: usize, budget: &mut usize) -> Result<Option<Box<Record>>, DecodeError> {
    if ptr == 0 { Ok(None) } else { Ok(Some(Box::new(parse(s, ptr, depth + 1, budget)?))) }
}

fn parse(s: &Section<'_>, off: u32, depth: usize, budget: &mut usize) -> Result<Record, DecodeError> {
    if depth > MAX_DEPTH || *budget == 0 {
        return Err(DecodeError::invalid(RECORD, "pattern program too deep or too large"));
    }
    *budget -= 1;
    let tag = s.u32(off)?;
    let mut at = off + 4;
    let mut next = || -> Result<u32, DecodeError> {
        let w = s.u32(at)?;
        at += 4;
        Ok(w)
    };
    let seed_of = |w: u32, inline: bool| -> Result<Seed, DecodeError> { if inline { Ok(Seed::Const(w)) } else { Ok(Seed::Expr(Expr::parse(s, w)?)) } };
    match tag & TAG_KIND {
        KIND_ITERATE | KIND_SWITCH => {
            let seed = match (tag & TAG_SEED_INLINE != 0, tag & TAG_SEED_EXPR != 0) {
                (true, _) => Some(seed_of(next()?, true)?),
                (_, true) => Some(seed_of(next()?, false)?),
                _ => None,
            };
            let count = if tag & TAG_BIT7 != 0 {
                Count::Expr(Expr::parse(s, next()?)?)
            } else if tag & TAG_KIND == KIND_SWITCH {
                Count::Const(u32::from(tag & TAG_BIT6 != 0))
            } else {
                Count::Const(tag >> TAG_HIGH_SHIFT)
            };
            if tag & TAG_KIND == KIND_ITERATE {
                let first = if tag & TAG_BIT6 != 0 { opt_child(s, next()?, depth, budget)? } else { None };
                let body = opt_child(s, next()?, depth, budget)?;
                Ok(Record::Iterate { seed, count, first, body })
            } else {
                let a = opt_child(s, next()?, depth, budget)?;
                let b = opt_child(s, next()?, depth, budget)?;
                Ok(Record::Switch { seed, condition: count, branches: [a, b] })
            }
        }
        KIND_QUADRANT => {
            let codes = tag >> TAG_SLOT_CODES_SHIFT;
            let code = |k: u32| (codes >> (2 * k)) & 3;
            let image_word = if code(7) >= 2 { Some(next()?) } else { None };
            let has_seed = tag & (TAG_SEED_INLINE | TAG_SEED_EXPR) != 0;
            let has_params = has_seed || (1..8).any(|k| code(k) != 0);
            let mut seed = None;
            let mut params = Vec::new();
            let mut flag1 = false;
            if has_params {
                let mut a = next()?;
                let mut word = || -> Result<u32, DecodeError> {
                    let w = s.u32(a)?;
                    a += 4;
                    Ok(w)
                };
                if tag & TAG_SEED_INLINE != 0 {
                    seed = Some(Seed::Const(word()?));
                } else if tag & TAG_SEED_EXPR != 0 {
                    seed = Some(Seed::Expr(Expr::parse(s, word()?)?));
                }
                for k in 1..8u32 {
                    match (k, code(k)) {
                        (_, 0) | (7, 2) => {}
                        (1, 1) => flag1 = true,
                        (1, c) => {
                            flag1 = c == 3;
                            params.push((1, Value::Const(vec![word()?])));
                        }
                        (7, c) => {
                            debug_assert!(c == 1 || c == 3);
                            params.push((7, Value::Expr(Expr::parse(s, word()?)?)));
                        }
                        (_, 1) => params.push((k as u8, Value::Expr(Expr::parse(s, word()?)?))),
                        (_, 2) => {
                            let words = (0..slot_words(k)).map(|_| word()).collect::<Result<_, _>>()?;
                            params.push((k as u8, Value::Const(words)));
                        }
                        _ => {
                            params.push((k as u8, Value::Expr(Expr::parse(s, word()?)?)));
                            let words = (0..slot_words(k)).map(|_| word()).collect::<Result<_, _>>()?;
                            params.push((k as u8, Value::Const(words)));
                        }
                    }
                }
                if image_word.is_some_and(|w| w & IMAGE_WORD_EXTRA_EXPR != 0) {
                    params.push((8, Value::Expr(Expr::parse(s, word()?)?)));
                }
            }
            let children = match code(0) {
                1 => {
                    let mut c: [Option<Box<Record>>; 4] = [None, None, None, None];
                    for slot in &mut c {
                        let p = next()?;
                        *slot = opt_child(s, p, depth, budget)?;
                    }
                    Children::Four(c)
                }
                2 | 3 => Children::Shared(Box::new(parse(s, next()?, depth + 1, budget)?)),
                _ => Children::None,
            };
            Ok(Record::Quadrant {
                seed,
                pattern: ((tag >> TAG_HIGH_SHIFT) & 0xff) as u8,
                draws: tag & (TAG_BIT6 | TAG_BIT7) != 0,
                image: tag & TAG_BIT7 != 0,
                flag1,
                image_word,
                params,
                children,
            })
        }
        k => Err(DecodeError::invalid(RECORD, format!("unknown record kind {k:#x} (tag {tag:#010x}) at {off:#x}"))),
    }
}
