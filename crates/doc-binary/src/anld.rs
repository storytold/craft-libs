//! Word 6.0/95 auto-numbering: the `ANLD` and `nLvlAnm` paragraph sprms and the sections'
//! `OLST`, turned into the numbering model.
//!
//! Word 6.0/95 has no list tables. A numbered paragraph carries (or takes from its style) a
//! numbering level, `nLvlAnm`, and the description of its number, an `ANLD`; a section carries
//! the nine levels of its heading numbering, an `OLST`. The layouts and the rules below were read
//! off files Word 95 wrote from RTF `\pn` numbering, against the numbers Word 95 itself shows (in
//! its RTF save's `\pntext`) and Word 2016's reading of them.
//!
//! - **`nLvlAnm`:** 1-9 a heading level of the section's numbering, 10 numbered, 11 bulleted, 12
//!   "skip numbering" (no number, and the count goes on past it), 0 none.
//! - **When the count goes on:** a numbered paragraph continues the last numbered one when both
//!   have the same format and the same text before and after the number, and nothing broke the
//!   run between them. A plain paragraph, a heading, or entering or leaving a table breaks it; a
//!   bullet, a skipped paragraph or a section break doesn't. The start of a paragraph that
//!   continues is ignored. In a table each column counts on its own, down the rows.
//! - **The number's own formatting** (font, size, colour, bold) can change within a run: Word 95
//!   keeps counting and shows each paragraph's. Word 2016 restarts there; this follows Word 95.
//! - **Headings:** one list per distinct `OLST`, so the count carries across sections. A level
//!   with `fPrev` shows the levels above it first, as Word 95 does (`I.A.1.`; Word 2016 shows
//!   `A.1.`).

use std::collections::HashMap;

use craft_wordmodel::numbering::{AbstractNum, Level, Num, Numbering};
use craft_wordmodel::props::{Align, CharProps};
use craft_wordmodel::section::NumFormat;

use crate::fmt;
use crate::sprm;
use crate::table::ParaOut;

/// Paragraph sprm: the numbering level (1 byte).
const NLVL_ANM: u8 = 0x0D;
/// Paragraph sprm: the number's description (a count byte, then an [`Anlv`] and its text).
const ANLD: u8 = 0x0C;
/// Section sprm: the heading numbering (a count byte, then nine [`Anlv`]s and their text).
pub(crate) const OLST: u8 = 0x85;
/// An `ANLV`'s size: nfc, the counts of text before and after, three flag bytes, then the font,
/// size, start, indent and space as u16s.
const ANLV_LEN: usize = 16;
/// Lists made at most (the rest of the paragraphs are left unnumbered).
const MAX_NUMS: usize = 2_000;
/// Style chains followed at most this deep.
const MAX_STYLE_DEPTH: usize = 16;

/// One level's number, as stored ([`ANLV_LEN`] bytes, then its text in the `ANLD` or `OLST`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Anlv {
    /// 0 decimal, 1 upper roman, 2 lower roman, 3 upper letter, 4 lower letter, 5 ordinal, 6
    /// cardinal text, 7 ordinal text; bullets have 0xFF.
    nfc: u8,
    /// The 8-bit text before and after the number (a bullet's character is its text before).
    before: Vec<u8>,
    after: Vec<u8>,
    /// Bits 0-1 the alignment, 2 `fPrev`, 3 `fHang`, 4-7 whether the number sets bold, italic,
    /// small caps and caps.
    flags: u8,
    /// Bit 0 sets strike, 1 sets the underline; bits 3-6 the bold, italic, small caps and caps
    /// values, 7 the strike value.
    flags2: u8,
    /// Bits 0-2 the underline, 3-7 the colour (`ico`).
    kul_ico: u8,
    /// The font's index; bit 15 set: the number has no font of its own (0x8001).
    ftc: u16,
    /// Size in half-points; 0: the paragraph's.
    hps: u16,
    start: u16,
    /// The space the number takes, in twips (with `fHang`, a hanging indent).
    indent: i16,
}

impl Anlv {
    /// An `ANLV` from its [`ANLV_LEN`] bytes, its text taken from `text` from `*at` on.
    fn parse(b: &[u8], text: &[u8], at: &mut usize) -> Option<Anlv> {
        let b = b.get(..ANLV_LEN)?;
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let mut take = |n: u8| {
            let from = (*at).min(text.len());
            let to = from.saturating_add(usize::from(n)).min(text.len());
            *at = to;
            text[from..to].to_vec()
        };
        let before = take(b[1]);
        let after = take(b[2]);
        Some(Anlv {
            nfc: b[0],
            before,
            after,
            flags: b[3],
            flags2: b[4],
            kul_ico: b[5],
            ftc: u16_at(6),
            hps: u16_at(8),
            start: u16_at(10),
            indent: i16::from_le_bytes([b[12], b[13]]),
        })
    }

    /// The text before and after the number, the key a run is continued by.
    fn key(&self) -> (u8, &[u8], &[u8]) {
        (self.nfc, &self.before, &self.after)
    }

    /// The number's formatting, as the Word 97 sprms it stands for, applied to a fresh
    /// [`CharProps`].
    fn chr(&self, fonts: &[String]) -> CharProps {
        let mut g: Vec<u8> = Vec::new();
        let mut put = |op: u16, v: &[u8]| {
            g.extend_from_slice(&op.to_le_bytes());
            g.extend_from_slice(v);
        };
        for (set, value, op) in [
            (self.flags & 0x10, self.flags2 & 0x08, 0x0835u16), // bold
            (self.flags & 0x20, self.flags2 & 0x10, 0x0836),    // italic
            (self.flags & 0x40, self.flags2 & 0x20, 0x083A),    // small caps
            (self.flags & 0x80, self.flags2 & 0x40, 0x083B),    // caps
            (self.flags2 & 0x01, self.flags2 & 0x80, 0x0837),   // strike
        ] {
            if set != 0 {
                put(op, &[u8::from(value != 0)]);
            }
        }
        if self.flags2 & 0x02 != 0 {
            put(0x2A3E, &[self.kul_ico & 0x07]);
        }
        if self.kul_ico >> 3 != 0 {
            put(0x2A42, &[self.kul_ico >> 3]);
        }
        if self.ftc & 0x8000 == 0 {
            put(0x4A4F, &self.ftc.to_le_bytes());
        }
        if self.hps != 0 {
            put(0x4A43, &self.hps.to_le_bytes());
        }
        let mut chr = CharProps::default();
        let base = chr.clone();
        for prl in sprm::iter(&g) {
            fmt::apply_char(&mut chr, &prl, &base, fonts);
        }
        chr
    }

    /// The model level for this number at zero-based `level`; `above` holds the levels above
    /// it, for `fPrev`. Text decodes in `code_page`.
    fn level(&self, level: usize, above: &[Anlv], bullet: bool, fonts: &[String], code_page: u16) -> Level {
        let text8 = |b: &[u8]| -> String { b.iter().map(|&c| craft_codepage::code_page_byte(code_page, c)).collect() };
        let part = |a: &Anlv, k: usize| format!("{}%{}{}", text8(&a.before), k + 1, text8(&a.after));
        let (format, text) = if bullet {
            // A bullet's character is its text before, kept with the font it names.
            let c: String = self.before.iter().map(|&c| crate::list::bullet_char(u16::from(c))).collect();
            (NumFormat::Bullet, if c.is_empty() { "\u{2022}".to_owned() } else { c })
        } else {
            let mut t = String::new();
            if self.flags & 0x04 != 0 {
                for (k, a) in above.iter().enumerate().take(level) {
                    t.push_str(&part(a, k));
                }
            }
            t.push_str(&part(self, level));
            // The eight formats are MSONFC's first eight; anything else reads as decimal.
            (if self.nfc <= 7 { crate::list::num_format(self.nfc) } else { NumFormat::Decimal }, t)
        };
        let space = (f32::from(self.indent) / 20.0).clamp(0.0, 1000.0);
        Level {
            start: u32::from(self.start),
            format,
            text,
            align: match self.flags & 0x03 {
                1 => Align::Center,
                2 => Align::Right,
                _ => Align::Left,
            },
            indent: space,
            hanging: if self.flags & 0x08 != 0 { space } else { 0.0 },
            chr: self.chr(fonts),
            ..Level::default()
        }
    }
}

/// An `ANLD` operand (a count byte, then an [`Anlv`], 4 flag bytes and 32 bytes of text).
fn parse_anld(v: &[u8]) -> Option<Anlv> {
    let b = v.get(1..)?;
    Anlv::parse(b, b.get(ANLV_LEN + 4..).unwrap_or_default(), &mut 0)
}

/// An `OLST` operand (a count byte, then nine [`Anlv`]s, 4 flag bytes and 64 bytes of text that
/// the levels take from in turn).
pub(crate) fn parse_olst(v: &[u8]) -> Option<Vec<Anlv>> {
    let b = v.get(1..)?;
    let text = b.get(9 * ANLV_LEN + 4..).unwrap_or_default();
    let mut at = 0;
    (0..9).map(|k| Anlv::parse(b.get(k * ANLV_LEN..)?, text, &mut at)).collect()
}

/// A paragraph's or a style's own numbering sprms.
#[derive(Clone, Debug, Default)]
pub(crate) struct Own {
    nlvl: Option<u8>,
    anld: Option<Anlv>,
}

impl Own {
    /// The numbering sprms of a Word 6 paragraph grpprl, as `sprms` frames it.
    pub(crate) fn of<'a>(sprms: impl Iterator<Item = (u8, &'a [u8])>) -> Own {
        let mut own = Own::default();
        for (op, v) in sprms {
            match op {
                NLVL_ANM => own.nlvl = v.first().copied(),
                ANLD => own.anld = parse_anld(v),
                _ => {}
            }
        }
        own
    }

    fn is_empty(&self) -> bool {
        self.nlvl.is_none() && self.anld.is_none()
    }
}

/// The numbering sprms of the paragraph runs (by FC) and the styles (by istd, with their base).
#[derive(Default)]
pub(crate) struct Lists {
    /// `[lo, hi)` FC ranges, ascending and apart.
    runs: Vec<(u32, u32, Own)>,
    styles: Vec<(u16, Own)>,
}

impl Lists {
    /// From the paragraph runs `(lo, hi, own)` (the first of overlapping ones kept, as for their
    /// formatting) and each style's base and own sprms.
    pub(crate) fn new(mut runs: Vec<(u32, u32, Own)>, styles: Vec<(u16, Own)>) -> Lists {
        runs.sort_by_key(|r| r.0);
        let mut kept: Vec<(u32, u32, Own)> = Vec::new();
        for r in runs {
            if r.0 < r.1 && kept.last().is_none_or(|last| r.0 >= last.1) {
                kept.push(r);
            }
        }
        kept.retain(|r| !r.2.is_empty());
        Lists { runs: kept, styles }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.runs.is_empty() && self.styles.iter().all(|s| s.1.is_empty())
    }

    /// The level and number of the paragraph whose mark is at `fc`, in style `istd`: each its own,
    /// else its style's or the nearest base's.
    pub(crate) fn at(&self, fc: u32, istd: u16) -> (u8, Option<&Anlv>) {
        let i = self.runs.partition_point(|r| r.0 <= fc);
        let own = i.checked_sub(1).and_then(|i| self.runs.get(i)).filter(|r| fc < r.1).map(|r| &r.2);
        let (mut nlvl, mut anld) = (own.and_then(|o| o.nlvl), own.and_then(|o| o.anld.as_ref()));
        let mut at = istd;
        for _ in 0..MAX_STYLE_DEPTH {
            if nlvl.is_some() && anld.is_some() {
                break;
            }
            let Some((base, s)) = self.styles.get(usize::from(at)) else { break };
            nlvl = nlvl.or(s.nlvl);
            anld = anld.or(s.anld.as_ref());
            at = *base;
        }
        (nlvl.unwrap_or(0), anld)
    }
}

/// A run of numbered paragraphs that count on together.
struct Run {
    abstract_id: u32,
    key: (u8, Vec<u8>, Vec<u8>),
    /// The num of each number formatting seen in the run (the first is the run's own).
    nums: Vec<(Anlv, u32)>,
}

/// Builds the lists while the paragraphs are numbered in order.
struct Builder<'a> {
    numbering: Numbering,
    fonts: &'a [String],
    code_page: u16,
    bullets: HashMap<Anlv, u32>,
    /// Heading lists by `OLST` (or, for a section without one, `None`), with their levels.
    headings: HashMap<Option<Vec<Anlv>>, (u32, u32, Vec<Anlv>)>,
    /// Heading levels a paragraph's own `ANLD` sets in a section without an `OLST`: the num.
    heading_overrides: HashMap<(u8, Anlv), u32>,
}

impl Builder<'_> {
    fn full(&self) -> bool {
        self.numbering.nums.len() >= MAX_NUMS
    }

    /// A new list of `levels`: its abstract id and num.
    fn add(&mut self, levels: Vec<Level>) -> (u32, u32) {
        let id = self.numbering.abstracts.len() as u32 + 1;
        self.numbering.abstracts.push(AbstractNum { id, levels, ..Default::default() });
        (id, self.num(id, Vec::new()))
    }

    fn num(&mut self, abstract_id: u32, level_overrides: Vec<(u8, Level)>) -> u32 {
        let id = self.numbering.nums.len() as u32 + 1;
        self.numbering.nums.push(Num { id, abstract_id, level_overrides, ..Default::default() });
        id
    }

    /// The num for a numbered paragraph with `anld`, continuing `run` when it may.
    fn numbered(&mut self, run: &mut Option<Run>, anld: &Anlv) -> u32 {
        let level = |b: &Builder, a: &Anlv| a.level(0, &[], false, b.fonts, b.code_page);
        if let Some(r) = run.as_mut().filter(|r| (r.key.0, r.key.1.as_slice(), r.key.2.as_slice()) == anld.key()) {
            if let Some((_, n)) = r.nums.iter().find(|(a, _)| a == anld) {
                return *n;
            }
            // Other formatting for the number: its own num on the run's list, which shares its
            // count.
            let l = level(self, anld);
            let n = self.num(r.abstract_id, vec![(0, l)]);
            r.nums.push((anld.clone(), n));
            return n;
        }
        let l = level(self, anld);
        let (abstract_id, n) = self.add(vec![l]);
        let (nfc, before, after) = anld.key();
        *run = Some(Run { abstract_id, key: (nfc, before.to_vec(), after.to_vec()), nums: vec![(anld.clone(), n)] });
        n
    }

    fn bullet(&mut self, anld: &Anlv) -> u32 {
        if let Some(&n) = self.bullets.get(anld) {
            return n;
        }
        let l = anld.level(0, &[], true, self.fonts, self.code_page);
        let (_, n) = self.add(vec![l]);
        self.bullets.insert(anld.clone(), n);
        n
    }

    /// The num for a heading at zero-based `level` in a section with `olst`; `own` is the
    /// paragraph's `ANLD`, its level where the section has no `OLST`.
    fn heading(&mut self, olst: Option<&Vec<Anlv>>, level: u8, own: Option<&Anlv>) -> u32 {
        let key = olst.cloned();
        let (abstract_id, num, levels) = match self.headings.get(&key) {
            Some((a, n, l)) => (*a, *n, l.clone()),
            None => {
                let raw: Vec<Anlv> = olst
                    .cloned()
                    .unwrap_or_else(|| (0..9).map(|_| Anlv { nfc: 0, after: b".".to_vec(), start: 1, flags: 0x08, ..Default::default() }).collect());
                let levels: Vec<Level> = raw.iter().enumerate().map(|(k, a)| a.level(k, &raw, false, self.fonts, self.code_page)).collect();
                let (a, n) = self.add(levels);
                self.headings.insert(key, (a, n, raw.clone()));
                (a, n, raw)
            }
        };
        match (olst, own) {
            (None, Some(a)) => {
                if let Some(&n) = self.heading_overrides.get(&(level, a.clone())) {
                    return n;
                }
                let mut above = levels;
                if let Some(slot) = above.get_mut(usize::from(level)) {
                    *slot = a.clone();
                }
                let l = a.level(usize::from(level), &above, false, self.fonts, self.code_page);
                let n = self.num(abstract_id, vec![(level, l)]);
                self.heading_overrides.insert((level, a.clone()), n);
                n
            }
            _ => num,
        }
    }
}

/// Number the main text's paragraphs, in order, and return their lists. `own_of` gives a
/// paragraph's numbering level and `ANLD`, `olst_of` the `OLST` of its section;
/// `section_mark` says whether a paragraph is an empty one ending in a section mark (which
/// neither counts nor breaks a run).
pub(crate) fn number<'l>(
    paras: &mut [ParaOut],
    own_of: impl Fn(&ParaOut) -> (u8, Option<&'l Anlv>),
    olst_of: impl Fn(&ParaOut) -> Option<&'l Vec<Anlv>>,
    section_mark: impl Fn(&ParaOut) -> bool,
    fonts: &[String],
    code_page: u16,
) -> Numbering {
    let mut b = Builder {
        numbering: Numbering::default(),
        fonts,
        code_page,
        bullets: HashMap::new(),
        headings: HashMap::new(),
        heading_overrides: HashMap::new(),
    };
    let mut body: Option<Run> = None;
    // In a table: the column, and each column's run.
    let mut in_table = false;
    let mut column = 0usize;
    let mut columns: HashMap<usize, Run> = HashMap::new();
    for p in paras.iter_mut() {
        let row = p.row.as_deref();
        let table = row.is_some_and(|r| r.in_table || r.ttp || r.cell_mark);
        if table != in_table {
            // Entering or leaving a table breaks every run.
            body = None;
            columns.clear();
            column = 0;
            in_table = table;
        }
        if row.is_some_and(|r| r.ttp) {
            column = 0;
            continue;
        }
        let cell_end = row.is_some_and(|r| r.cell_mark);
        if !section_mark(p) && !b.full() {
            let (nlvl, anld) = own_of(p);
            let mut slot = if table { columns.remove(&column) } else { body.take() };
            let num = match (nlvl, anld) {
                (10, Some(a)) => Some((b.numbered(&mut slot, a), 0)),
                (11, Some(a)) => Some((b.bullet(a), 0)),
                (12, _) => None,
                (1..=9, _) => {
                    slot = None;
                    Some((b.heading(olst_of(p), nlvl - 1, anld), nlvl - 1))
                }
                _ => {
                    slot = None;
                    None
                }
            };
            if let Some((num, level)) = num {
                p.para.props.numbering = Some(craft_wordmodel::props::NumRef { num, level });
            }
            match (table, slot) {
                (true, Some(r)) => {
                    columns.insert(column, r);
                }
                (false, s) => body = s,
                _ => {}
            }
        }
        if cell_end {
            column += 1;
        }
    }
    b.numbering
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ANLD operand: nfc, text before and after, flags, font, size, start, indent.
    pub(crate) fn anld(nfc: u8, before: &[u8], after: &[u8], flags: u8, start: u16) -> Vec<u8> {
        let mut v = vec![52, nfc, before.len() as u8, after.len() as u8, flags, 0, 0];
        v.extend_from_slice(&0x8001u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&start.to_le_bytes());
        v.extend_from_slice(&360i16.to_le_bytes());
        v.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mut text = [before, after].concat();
        text.resize(32, 0);
        v.extend_from_slice(&text);
        v
    }

    #[test]
    fn an_anld_reads_its_text_before_and_after() {
        let a = parse_anld(&anld(3, b"(", b")", 0x08, 1)).expect("anld");
        assert_eq!((a.nfc, a.before.as_slice(), a.after.as_slice(), a.start, a.indent), (3, &b"("[..], &b")"[..], 1, 360));
        let l = a.level(0, &[], false, &[], 1252);
        assert_eq!((l.text.as_str(), l.format, l.hanging), ("(%1)", NumFormat::UpperLetter, 18.0));
    }

    #[test]
    fn an_olsts_levels_take_their_text_in_turn() {
        // As Word 95 wrote I. A. 1. a) (1): the texts ". . . ) ( )" in level order.
        let mut v = vec![212];
        for (nfc, before, after, flags) in [(1u8, 0u8, 1u8, 0x08u8), (3, 0, 1, 0x08), (0, 0, 1, 0x0C), (4, 0, 1, 0x08), (0, 1, 1, 0x08)] {
            v.extend_from_slice(&[nfc, before, after, flags, 0, 0, 0x01, 0x80, 0, 0, 1, 0, 0xD0, 0x02, 0, 0]);
        }
        v.resize(1 + 9 * 16 + 4, 0);
        v.extend_from_slice(b"...)()");
        v.resize(213, 0);
        let olst = parse_olst(&v).expect("olst");
        let texts: Vec<String> = olst.iter().take(5).enumerate().map(|(k, a)| a.level(k, &olst, false, &[], 1252).text).collect();
        assert_eq!(texts, ["%1.", "%2.", "%1.%2.%3.", "%4)", "(%5)"]);
    }
}
