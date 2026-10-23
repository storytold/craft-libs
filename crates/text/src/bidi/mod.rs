//! Bidirectional text (UAX #9) for document layout, on plain text and independent of fonts.
//!
//! The pipeline is the one the algorithm prescribes: [`Paragraph::new`] resolves embedding levels
//! once for the whole logical paragraph (or for each paragraph, in text with paragraph
//! separators); shaping and line breaking happen in logical order; then each line is reordered on
//! its own. [`Paragraph::line_levels`] applies rule L1 and
//! [`Paragraph::visual_runs`] rule L2 to a byte range of the paragraph. For lines built from other
//! units (clusters, glyphs, cells), [`line_levels`] and [`visual_order`] apply the same rules to
//! slices of levels. [`mirror`] gives the character for rule L4; drawing it, or a font's mirrored
//! glyph, is left to the shaper.
//!
//! Levels are `u8` embedding levels: even runs left to right, odd runs right to left.

use std::ops::Range;

use unicode_bidi::BidiClass::{self, AL, AN, B, BN, FSI, LRE, LRI, LRO, PDF, PDI, R, RLE, RLI, RLO, S, WS};
use unicode_bidi::{BidiInfo, Level, bidi_class};

/// A paragraph's base direction (rules P2–P3, or the higher-level protocol HL1 when set).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Direction {
    /// Left to right: paragraph level 0.
    Ltr,
    /// Right to left: paragraph level 1.
    Rtl,
    /// From the first strong character, left to right when there is none.
    Auto,
}

/// A directional override (LRO or RLO … PDF) over a byte range of a paragraph: every character in
/// it is treated as strongly left to right or right to left.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Override {
    pub range: Range<usize>,
    pub rtl: bool,
}

/// A maximal run of one embedding level in a line, as [`Paragraph::visual_runs`] returns it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Run {
    /// Byte range in the paragraph text, in logical order.
    pub range: Range<usize>,
    pub level: u8,
}

impl Run {
    /// Is the run displayed right to left (its characters from the end of `range` to its start)?
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }
}

/// Resolved embedding levels (rules up to I2) for a paragraph's text, one per byte.
///
/// A paragraph separator (`\n`, U+2029 and the other class B characters) inside the text ends a
/// paragraph and belongs to it; the next one starts after it with its own paragraph level, which
/// `Direction::Auto` resolves from that paragraph's own first strong character. Rule L1 uses the
/// level of the paragraph each byte is in.
#[derive(Clone, Debug)]
pub struct Paragraph<'t> {
    text: &'t str,
    levels: Vec<u8>,
    /// Each paragraph's byte range and level, in order and covering the text; never empty.
    paragraphs: Vec<(Range<usize>, u8)>,
}

impl<'t> Paragraph<'t> {
    /// Resolve the levels of `text`. Text with nothing right to left in a left-to-right or `Auto`
    /// paragraph skips the algorithm.
    pub fn new(text: &'t str, base: Direction) -> Self {
        if base != Direction::Rtl && !needs_bidi(text) {
            let paragraphs = split_paragraphs(text).into_iter().map(|r| (r, 0)).collect();
            return Self { text, levels: vec![0; text.len()], paragraphs };
        }
        let (levels, paragraphs) = resolve(text, base);
        Self { text, levels, paragraphs }
    }

    /// Like [`new`](Self::new), with directional overrides. Overrides that are empty, out of
    /// range, off char boundaries or overlapping an earlier one are ignored. An override that
    /// spans a paragraph separator continues in the next paragraph; the separator keeps its
    /// paragraph's level.
    pub fn with_overrides(text: &'t str, base: Direction, overrides: &[Override]) -> Self {
        let mut sorted: Vec<&Override> = overrides.iter().filter(|o| o.range.start < o.range.end && text.get(o.range.clone()).is_some()).collect();
        sorted.sort_by_key(|o| o.range.start);
        // The text with LRO/RLO … PDF around each override, and where each source char went.
        let mut wrapped = String::with_capacity(text.len() + sorted.len() * 6);
        let mut at: Vec<usize> = Vec::with_capacity(text.len());
        let mut copy = |wrapped: &mut String, piece: Range<usize>| {
            if let Some(s) = text.get(piece) {
                for (_, c) in s.char_indices() {
                    at.extend(std::iter::repeat_n(wrapped.len(), c.len_utf8()));
                    wrapped.push(c);
                }
            }
        };
        let mut pos = 0;
        for o in sorted {
            if o.range.start < pos {
                continue;
            }
            copy(&mut wrapped, pos..o.range.start);
            // A paragraph separator ends every embedding (rule X8), so the override is closed
            // before each separator and opened again after it.
            let mut start = o.range.start;
            while start < o.range.end {
                let rest = text.get(start..o.range.end).unwrap_or_default();
                let (end, next) = rest
                    .char_indices()
                    .find(|&(_, c)| bidi_class(c) == B)
                    .map_or((o.range.end, o.range.end), |(i, c)| (start + i, start + i + c.len_utf8()));
                if start < end {
                    wrapped.push(if o.rtl { '\u{202E}' } else { '\u{202D}' });
                    copy(&mut wrapped, start..end);
                    wrapped.push('\u{202C}');
                }
                copy(&mut wrapped, end..next);
                start = next;
            }
            pos = o.range.end;
        }
        if pos == 0 {
            return Self::new(text, base);
        }
        copy(&mut wrapped, pos..text.len());
        let (wrapped_levels, wrapped_paragraphs) = resolve(&wrapped, base);
        // The inserted controls are not separators, so the paragraphs are the same ones; their
        // ranges map back through `at`, which is ascending.
        let back = |w: usize| at.partition_point(|&a| a < w);
        let paragraphs: Vec<(Range<usize>, u8)> = wrapped_paragraphs.iter().map(|(r, l)| (back(r.start)..back(r.end), *l)).collect();
        let levels = at.iter().enumerate().map(|(b, &w)| wrapped_levels.get(w).copied().unwrap_or_else(|| level_in(&paragraphs, b))).collect();
        Self { text, levels, paragraphs }
    }

    pub fn text(&self) -> &'t str {
        self.text
    }

    /// The first paragraph's embedding level: 0 or 1. Text with paragraph separators has one
    /// level per paragraph: see [`paragraphs`](Self::paragraphs).
    pub fn level(&self) -> u8 {
        self.paragraphs.first().map_or(0, |p| p.1)
    }

    /// Is the first paragraph right to left?
    pub fn is_rtl(&self) -> bool {
        self.level() % 2 == 1
    }

    /// Each paragraph's byte range (its separator included) and embedding level, in order. There
    /// is always at least one; empty text has one empty paragraph.
    pub fn paragraphs(&self) -> &[(Range<usize>, u8)] {
        &self.paragraphs
    }

    /// The embedding level of the paragraph containing byte `byte` (the last one's past the end).
    pub fn paragraph_level_at(&self, byte: usize) -> u8 {
        level_in(&self.paragraphs, byte)
    }

    /// Does any line need reordering (is any level above 0)?
    pub fn needs_reordering(&self) -> bool {
        self.levels.iter().any(|l| *l > 0)
    }

    /// The level of each byte of the text.
    pub fn levels(&self) -> &[u8] {
        &self.levels
    }

    pub fn into_levels(self) -> Vec<u8> {
        self.levels
    }

    /// The level of the character at byte `byte` (the last paragraph's level past the end).
    pub fn level_at(&self, byte: usize) -> u8 {
        self.levels.get(byte).copied().unwrap_or_else(|| self.paragraph_level_at(byte))
    }

    /// The level of each character, in order.
    pub fn char_levels(&self) -> Vec<u8> {
        self.text.char_indices().map(|(b, _)| self.level_at(b)).collect()
    }

    /// Rule L1 for the line `line` (a byte range of the text): the level of each of its bytes,
    /// with segment and paragraph separators, whitespace before them and whitespace at the end of
    /// the line at their paragraph's level; zero-width joiners and tag characters keep the level of
    /// the char before them. Empty when `line` is not a char-boundary range of the text. A line
    /// should lie within one paragraph; one that crosses a separator takes each byte's own
    /// paragraph level here and is reordered as a whole by [`visual_runs`](Self::visual_runs).
    pub fn line_levels(&self, line: Range<usize>) -> Vec<u8> {
        let Some(s) = self.text.get(line.clone()) else { return Vec::new() };
        let mut out: Vec<u8> = line.clone().map(|b| self.level_at(b)).collect();
        let mut trailing = true;
        for (b, c) in s.char_indices().rev() {
            if l1_reset(bidi_class(c), &mut trailing)
                && let Some(bytes) = out.get_mut(b..b + c.len_utf8())
            {
                bytes.fill(self.paragraph_level_at(line.start + b));
            }
        }
        let mut prev = None;
        for (b, c) in s.char_indices() {
            if joins_previous(c)
                && let Some(l) = prev
                && let Some(bytes) = out.get_mut(b..b + c.len_utf8())
            {
                bytes.fill(l);
            }
            prev = out.get(b).copied();
        }
        out
    }

    /// Rules L1 and L2 for the line `line`: its runs of one level, left to right as displayed.
    /// Empty when `line` is not a char-boundary range of the text.
    pub fn visual_runs(&self, line: Range<usize>) -> Vec<Run> {
        let levels = self.line_levels(line.clone());
        let Some(s) = self.text.get(line.clone()) else { return Vec::new() };
        let mut runs: Vec<Run> = Vec::new();
        for (b, c) in s.char_indices() {
            let level = levels.get(b).copied().unwrap_or_else(|| self.paragraph_level_at(line.start + b));
            let end = line.start + b + c.len_utf8();
            match runs.last_mut() {
                Some(run) if run.level == level => run.range.end = end,
                _ => runs.push(Run { range: line.start + b..end, level }),
            }
        }
        let run_levels: Vec<u8> = runs.iter().map(|r| r.level).collect();
        visual_order(&run_levels).into_iter().filter_map(|i| runs.get(i).cloned()).collect()
    }
}

/// Rule L1 over a line's characters `chars` and their resolved `levels`: segment and paragraph
/// separators, any whitespace before them and whitespace at the end of the line take `para_level`;
/// zero-width joiners and tag characters keep the level of the char before them. Missing levels are
/// taken as `para_level`.
pub fn line_levels(chars: &[char], levels: &[u8], para_level: u8) -> Vec<u8> {
    let mut out = levels.to_vec();
    out.resize(chars.len(), para_level);
    let mut trailing = true;
    for (i, c) in chars.iter().enumerate().rev() {
        if l1_reset(bidi_class(*c), &mut trailing)
            && let Some(l) = out.get_mut(i)
        {
            *l = para_level;
        }
    }
    for i in 1..out.len() {
        if chars.get(i).is_some_and(|&c| joins_previous(c))
            && let Some(&l) = out.get(i - 1)
            && let Some(cur) = out.get_mut(i)
        {
            *cur = l;
        }
    }
    out
}

/// Zero-width joiners and tag characters extend the grapheme cluster of the char before them
/// (UAX #29), so they take its level. Rule L1 counts them as whitespace and would otherwise give a
/// trailing one the paragraph level, splitting its cluster across two runs.
fn joins_previous(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}' | '\u{E0020}'..='\u{E007F}')
}

/// Rule L2: the display order, left to right, of items with these levels (indices into `levels`).
/// From the highest level down to the lowest odd one, every maximal run at that level or higher is
/// reversed.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(min_odd) = levels.iter().copied().filter(|l| l % 2 == 1).min() else { return order };
    let max = levels.iter().copied().max().unwrap_or(0);
    let level_at = |order: &[usize], i: usize| order.get(i).and_then(|&o| levels.get(o)).copied().unwrap_or(0);
    for lvl in (min_odd..=max).rev() {
        let mut i = 0;
        while i < order.len() {
            if level_at(&order, i) < lvl {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < order.len() && level_at(&order, j) >= lvl {
                j += 1;
            }
            if let Some(run) = order.get_mut(i..j) {
                run.reverse();
            }
            i = j;
        }
    }
    order
}

/// The mirrored counterpart of `c` (Bidi_Mirroring_Glyph), for a character at an odd level (rule
/// L4): `(` for `)`, `»` for `«`. `None` when it has none; some Bidi_Mirrored characters (`∑`)
/// need a mirrored glyph from the font instead.
pub fn mirror(c: char) -> Option<char> {
    unicode_bidi_mirroring::get_mirrored(c)
}

/// Does anything in the text need the full algorithm (strong right-to-left characters, Arabic
/// numbers or explicit embeddings)?
fn needs_bidi(text: &str) -> bool {
    text.chars().any(|c| matches!(bidi_class(c), R | AL | AN | LRE | RLE | LRO | RLO | PDF | LRI | RLI | FSI | PDI))
}

/// Levels per byte of `text`, and each paragraph's byte range and level.
fn resolve(text: &str, base: Direction) -> (Vec<u8>, Vec<(Range<usize>, u8)>) {
    let default = match base {
        Direction::Ltr => Some(Level::ltr()),
        Direction::Rtl => Some(Level::rtl()),
        Direction::Auto => None,
    };
    let info = BidiInfo::new(text, default);
    let mut paragraphs: Vec<(Range<usize>, u8)> = info.paragraphs.iter().map(|p| (p.range.clone(), p.level.number())).collect();
    if paragraphs.is_empty() {
        paragraphs.push((0..text.len(), u8::from(base == Direction::Rtl)));
    }
    let levels = (0..text.len()).map(|b| info.levels.get(b).map_or_else(|| level_in(&paragraphs, b), |l| l.number())).collect();
    (levels, paragraphs)
}

/// The byte ranges of the paragraphs in `text` as rule P1 (and unicode-bidi) splits them: each
/// ends after a paragraph separator, and there is always at least one.
fn split_paragraphs(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (b, c) in text.char_indices() {
        if bidi_class(c) == B {
            out.push(start..b + c.len_utf8());
            start = b + c.len_utf8();
        }
    }
    if start < text.len() || out.is_empty() {
        out.push(start..text.len());
    }
    out
}

/// The level of the paragraph containing byte `byte`; the last paragraph's past the end.
fn level_in(paragraphs: &[(Range<usize>, u8)], byte: usize) -> u8 {
    let i = paragraphs.partition_point(|(r, _)| r.end <= byte);
    paragraphs.get(i).or(paragraphs.last()).map_or(0, |p| p.1)
}

/// Rule L1 walking a line backwards: whether the character of class `class` takes the paragraph
/// level. `trailing` is true while everything after it up to the line end or a separator is
/// whitespace.
fn l1_reset(class: BidiClass, trailing: &mut bool) -> bool {
    match class {
        S | B => {
            *trailing = true;
            true
        }
        WS | FSI | LRI | RLI | PDI | BN | LRE | RLE | LRO | RLO | PDF => *trailing,
        _ => {
            *trailing = false;
            false
        }
    }
}

#[cfg(test)]
mod tests;
