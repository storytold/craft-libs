//! PowerPoint 95 text: a shape's text container (4001) and the document's font and ruler tables,
//! learned from files PowerPoint 95 wrote.
//!
//! A text container holds a header (4000) and a body: 4002 on slides, 4068 on the master's
//! placeholders. The body holds a text atom (4003: the text type, then the text rectangle), a
//! ruler reference (4021) and the runs (4064). The runs are the characters (2030 tagged 0x35,
//! Windows-1252 bytes, `\r` between paragraphs), then three 2028 containers, each holding a 2030
//! atom of fixed-size entries: character runs (tag 0x2f, 44 bytes), paragraph runs (0x30, 72
//! bytes) and one entry for the whole text (0x31, not read). Each set of runs covers the text and
//! one more character, the last paragraph's end. The text atom's +4 flags hold word wrap (bit 2),
//! and its +8 byte the anchoring: bits 4–5 top, middle or bottom, bit 1 clear for centred across.
//!
//! The Environment (1010) holds the fonts (2005) and the rulers (4016) as lists of slots: each
//! slot is what comes before a 2017 record, and an empty slot holds a 2018. A run's font and a
//! text's ruler are slot numbers. It also holds the master's text styles: a 1028 container of
//! 4052 atoms, one per text type (0 title, 1 body, 2 notes, 4 other), each five levels of
//! character formatting (24 bytes from +16) and five of paragraph formatting (52 bytes from
//! +160), laid out as in the runs. Slide text carries its formatting in full, as copies of these.

use std::cell::Cell;

use craft_pptx::text::{Align, Anchor, BodyProps, Bullet, LevelStyle, ListStyle, ParaProps, RunKind, Spacing};
use craft_pptx::{ColorRef, Fill, Paragraph, Rgba, Run, RunProps, TextBody};

use super::{MU, RGB_GIVEN, Rec};
use crate::convert::Scheme;

/// Record types.
mod rt {
    pub const ENVIRONMENT: u32 = 1010;
    pub const SLOT_END: u32 = 2017;
    pub const FONTS: u32 = 2005;
    pub const RUN_TABLE: u32 = 2028;
    pub const ATOM: u32 = 2030;
    pub const RULERS: u32 = 4016;
    pub const RULER_ATOM: u32 = 4019;
    pub const RULER_REF: u32 = 4021;
    pub const FONT: u32 = 4022;
    pub const FONT_ATOM: u32 = 4023;
    pub const TEXT_BODY: u32 = 4002;
    pub const TEXT_ATOM: u32 = 4003;
    pub const RUNS: u32 = 4064;
    pub const MASTER_TEXT_BODY: u32 = 4068;
    pub const RULER: u32 = 4069;
    pub const STYLES: u32 = 1028;
    pub const STYLE: u32 = 4052;
}

/// The fourth words telling a text's atoms apart.
mod tag {
    pub const CHARS: u32 = 0x35;
    pub const CHAR_RUNS: u32 = 0x2F;
    pub const PARA_RUNS: u32 = 0x30;
}

const CHAR_RUN: usize = 44;
const PARA_RUN: usize = 72;
/// Where a character run's formatting starts (its font), and a paragraph run's (its bullet flag).
const CHAR_AT: usize = 20;
const PARA_AT: usize = 20;
/// A master style's levels: character formatting from +16, 24 bytes each, and paragraph
/// formatting from +160, 52 bytes each.
const STYLE_CHARS: usize = 16;
const STYLE_CHAR: usize = 24;
const STYLE_PARAS: usize = 160;
const STYLE_PARA: usize = 52;
/// The master styles read: text types 0 to 8.
const STYLE_TYPES: usize = 9;
/// The text types whose styles the master's title, body and other styles take.
pub(super) const TITLE: usize = 0;
pub(super) const BODY: usize = 1;
pub(super) const OTHER: usize = 4;
/// Characters read from one text at most: far beyond what PowerPoint 95 puts in a shape, so a
/// huge character atom can't make a huge text body.
pub(super) const MAX_CHARS: usize = 1 << 20;
/// Paragraphs and runs made from the whole document's text at most. Each is about 2 to 4 KB, so
/// a text of nothing but paragraph or line breaks would otherwise make gigabytes from a megabyte
/// of file. That is far more than any PowerPoint 95 file probed holds.
pub(super) const MAX_TEXT_ITEMS: usize = 1 << 16;
/// Font and ruler slots read at most.
const MAX_SLOTS: usize = 4096;
/// Indent levels: PowerPoint 95 has five.
const LEVELS: usize = 5;

/// Each level's left margin and first-line indent, in master units from the text's left edge.
type Ruler = [(i32, i32); LEVELS];

/// What text in the document draws on: the font faces and rulers, by slot.
#[derive(Default)]
pub(super) struct Env {
    fonts: Vec<Option<String>>,
    rulers: Vec<Option<Ruler>>,
    /// The master's text styles by text type: each level's character and paragraph formatting.
    styles: Vec<Option<[(CharRun, ParaRun); LEVELS]>>,
    /// Paragraphs and runs made so far, against [`MAX_TEXT_ITEMS`].
    items: Cell<usize>,
}

impl Env {
    /// Counts one more paragraph or run; `false` once the document's text has made
    /// [`MAX_TEXT_ITEMS`], when the rest of its text is left out.
    fn take_item(&self) -> bool {
        let n = self.items.get();
        if n == MAX_TEXT_ITEMS {
            log::warn!("ppt: PowerPoint 95 text passes {MAX_TEXT_ITEMS} paragraphs and runs; the rest is left out");
        }
        self.items.set(n.saturating_add(1));
        n < MAX_TEXT_ITEMS
    }

    pub(super) fn new(document: &Rec) -> Env {
        let Some(environment) = document.child(rt::ENVIRONMENT) else { return Env::default() };
        // There may be more than one font list; the first that holds a face is the table.
        let fonts = environment
            .children()
            .filter(|r| r.kind == rt::FONTS)
            .map(|list| slots(&list, rt::FONT, |f| f.child(rt::FONT_ATOM).map(|a| face(&a))))
            .find(|f| f.iter().flatten().any(|face| !face.is_empty()))
            .unwrap_or_default();
        let rulers =
            environment.child(rt::RULERS).map(|list| slots(&list, rt::RULER, |r| r.child(rt::RULER_ATOM).map(|a| ruler(&a)))).unwrap_or_default();
        let styles = environment
            .child(rt::STYLES)
            .map(|list| list.children().filter(|r| r.kind == rt::STYLE).take(STYLE_TYPES).map(|r| style(&r)).collect())
            .unwrap_or_default();
        Env { fonts, rulers, styles, items: Cell::new(0) }
    }
}

/// A master style atom's five levels, when the atom holds them all.
fn style(r: &Rec) -> Option<[(CharRun, ParaRun); LEVELS]> {
    let b = r.data.get(..STYLE_PARAS + LEVELS * STYLE_PARA)?;
    Some(std::array::from_fn(|i| (char_run(b, STYLE_CHARS + i * STYLE_CHAR, 0), para_run(b, STYLE_PARAS + i * STYLE_PARA, 0))))
}

/// The master's style for text type `ty` as a list style. `ruler` is the ruler slot of the
/// master's placeholder holding that text, which gives each level's margins. `None` when the
/// file has no such style.
pub(super) fn master_style(env: &Env, scheme: &Scheme, ty: usize, ruler: Option<u32>) -> Option<ListStyle> {
    let levels = env.styles.get(ty)?.as_ref()?;
    let ruler = ruler.and_then(|i| env.rulers.get(i as usize)).copied().flatten();
    let mut ls = ListStyle::default();
    for (i, (c, p)) in (0u8..).zip(levels) {
        let para = para_props(Some(p), ruler.as_ref().and_then(|r| r.get(usize::from(i))));
        ls.set(i, LevelStyle { para, run: run_props(c, env, scheme) });
    }
    Some(ls)
}

/// The angle a text container's text is drawn at (4003 +16), counter-clockwise in sixteenths of a
/// degree: the geometry's own angle in every probe, whatever quarter-turns its box holds.
pub(super) fn angle(container: &Rec) -> Option<i32> {
    let body = container.children().find(|r| r.kind == rt::TEXT_BODY || r.kind == rt::MASTER_TEXT_BODY)?;
    body.child(rt::TEXT_ATOM)?.i32(16)
}

/// The ruler slot a text container's body names.
pub(super) fn ruler_slot(container: &Rec) -> Option<u32> {
    let body = container.children().find(|r| r.kind == rt::TEXT_BODY || r.kind == rt::MASTER_TEXT_BODY)?;
    body.child(rt::RULER_REF)?.u32(0)
}

/// A slot list's entries of `kind`, read by `read`: an entry is the slot of the 2017 records
/// before it.
pub(super) fn slots<'a, T>(list: &Rec<'a>, kind: u32, read: impl Fn(&Rec<'a>) -> Option<T>) -> Vec<Option<T>> {
    let mut out = vec![];
    let mut current = None;
    for r in list.children() {
        if r.kind == rt::SLOT_END {
            if out.len() >= MAX_SLOTS {
                break;
            }
            out.push(current.take());
        } else if r.kind == kind && current.is_none() {
            current = read(&r);
        }
    }
    out
}

/// A `FontEntityAtom`'s face: a LOGFONT, whose name is the NUL-terminated Windows-1252 bytes from
/// +28.
fn face(a: &Rec) -> String {
    let name = a.data.get(28..).unwrap_or(&[]);
    name.iter().take(32).take_while(|&&b| b != 0).map(|&b| craft_codepage::cp1252(b)).collect()
}

/// A ruler atom: each level's left margin, then its first-line indent, as i32s.
fn ruler(a: &Rec) -> Ruler {
    let mut out = [(0, 0); LEVELS];
    for (i, level) in out.iter_mut().enumerate() {
        *level = (a.i32(i * 8).unwrap_or(0), a.i32(i * 8 + 4).unwrap_or(0));
    }
    out
}

/// A character run: its length and the formatting it gives in full.
#[derive(Clone, Copy)]
struct CharRun {
    len: u32,
    font: u16,
    size: u16,
    style: u16,
    color: [u8; 4],
}

/// A paragraph run.
#[derive(Clone, Copy)]
struct ParaRun {
    len: u32,
    bullet: bool,
    bullet_char: u8,
    align: u8,
    level: u8,
    /// Line spacing, space before and after: a percentage of a line, or minus a size in points.
    line: Option<i32>,
    before: Option<i32>,
    after: Option<i32>,
}

/// A character run's formatting from `at` (its font): font, size, style bits, then the colour.
fn char_run(e: &[u8], at: usize, len: u32) -> CharRun {
    CharRun {
        len,
        font: u16_at(e, at),
        size: u16_at(e, at + 4),
        style: u16_at(e, at + 8),
        color: [0, 1, 2, 3].map(|k| e.get(at + 10 + k).copied().unwrap_or(0)),
    }
}

/// A paragraph run's formatting from `at` (its bullet flag): the bullet character at +6, the
/// level at +24, the alignment at +28, then line spacing, space before and space after.
fn para_run(e: &[u8], at: usize, len: u32) -> ParaRun {
    let i32_at = |k: usize| crate::record::u32_at(e, at + k).map(|v| v as i32);
    ParaRun {
        len,
        bullet: e.get(at).is_some_and(|&b| b != 0),
        bullet_char: e.get(at + 6).copied().unwrap_or(0),
        level: e.get(at + 24).copied().unwrap_or(0),
        align: e.get(at + 28).copied().unwrap_or(0),
        line: i32_at(32),
        before: i32_at(36),
        after: i32_at(40),
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    crate::record::u16_at(b, at).unwrap_or(0)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    crate::record::u32_at(b, at).unwrap_or(0)
}

/// A run table's entries of `size` bytes, read while they cover less than `total` characters, so
/// there are at most `total` of them.
fn entries<'a>(runs: &Rec<'a>, kind: u32, size: usize, total: u32) -> Vec<&'a [u8]> {
    let mut out = vec![];
    let mut covered = 0u32;
    let tables = runs.children().filter(|r| r.kind == rt::RUN_TABLE && r.tag == kind);
    for atom in tables.flat_map(|t| t.children().filter(|a| a.kind == rt::ATOM)) {
        for e in atom.data.chunks_exact(size) {
            if covered >= total {
                return out;
            }
            // A run of no characters formats nothing, so it is skipped rather than counted.
            let len = u32_at(e, 0);
            if len == 0 {
                continue;
            }
            covered = covered.saturating_add(len);
            out.push(e);
        }
    }
    out
}

/// Where each run starts: a running sum of their lengths, so never decreasing.
fn starts(lens: impl Iterator<Item = u32>) -> Vec<usize> {
    let mut at = 0usize;
    lens.map(|n| {
        let s = at;
        at = at.saturating_add(n as usize);
        s
    })
    .collect()
}

/// The run holding character `i`.
fn run_at(starts: &[usize], i: usize) -> Option<usize> {
    starts.partition_point(|&s| s <= i).checked_sub(1)
}

/// A shape's text. `shape` is its box (left, top, right, bottom in master units), from which the
/// text rectangle gives the insets. `None` when the container holds no text body.
pub(super) fn text_body(container: &Rec, env: &Env, scheme: &Scheme, shape: Option<[i32; 4]>, text_box: bool) -> Option<TextBody> {
    let body = container.children().find(|r| r.kind == rt::TEXT_BODY || r.kind == rt::MASTER_TEXT_BODY)?;
    let runs = body.child(rt::RUNS);
    let chars: Vec<char> = runs
        .and_then(|r| r.children().find(|a| a.kind == rt::ATOM && a.tag == tag::CHARS))
        .map(|a| a.data.iter().take(MAX_CHARS).map(|&b| craft_codepage::cp1252(b)).collect())
        .unwrap_or_default();
    // The runs cover the text and its last paragraph's end.
    let total = u32::try_from(chars.len()).unwrap_or(u32::MAX).saturating_add(1);
    let char_runs: Vec<CharRun> = runs
        .map(|r| entries(&r, tag::CHAR_RUNS, CHAR_RUN, total))
        .unwrap_or_default()
        .into_iter()
        .map(|e| char_run(e, CHAR_AT, u32_at(e, 0)))
        .collect();
    let para_runs: Vec<ParaRun> = runs
        .map(|r| entries(&r, tag::PARA_RUNS, PARA_RUN, total))
        .unwrap_or_default()
        .into_iter()
        .map(|e| para_run(e, PARA_AT, u32_at(e, 0)))
        .collect();
    let ruler = body.child(rt::RULER_REF).and_then(|r| r.u32(0)).and_then(|i| env.rulers.get(i as usize)).copied().flatten();

    let mut props = BodyProps::default();
    if text_box {
        props.anchor = Some(Anchor::Top);
        props.wrap = Some(true);
    }
    let atom = body.child(rt::TEXT_ATOM);
    if let Some(flags) = atom.and_then(|a| a.u8(4)) {
        props.wrap = Some(flags & 4 != 0);
    }
    if let Some(a) = atom.and_then(|a| a.u8(8)) {
        // Bits 4–5: top, middle or bottom; bit 1 clear: centred across. Bit 0 is always set.
        let anchor = match (a >> 4) & 3 {
            0 => Some(Anchor::Top),
            1 => Some(Anchor::Middle),
            2 => Some(Anchor::Bottom),
            _ => None,
        };
        if anchor.is_some() {
            props.anchor = anchor;
            props.anchor_ctr = Some(a & 2 == 0);
        }
    }
    if let (Some(atom), Some([l, t, r, b])) = (atom, shape) {
        // The text rectangle, inside the shape's box.
        let inset = |from: i32, to: Option<i32>, sign: f64| {
            to.map(|to| (f64::from(to) - f64::from(from)) * sign / MU).filter(|v| v.is_finite() && (0.0..=1000.0).contains(v))
        };
        props.inset_l = inset(l, atom.i32(20), 1.0);
        props.inset_t = inset(t, atom.i32(24), 1.0);
        props.inset_r = inset(r, atom.i32(28), -1.0);
        props.inset_b = inset(b, atom.i32(32), -1.0);
    }
    Some(TextBody { body: props, paragraphs: paragraphs(&chars, &char_runs, &para_runs, ruler.as_ref(), env, scheme), ..Default::default() })
}

fn paragraphs(chars: &[char], char_runs: &[CharRun], para_runs: &[ParaRun], ruler: Option<&Ruler>, env: &Env, scheme: &Scheme) -> Vec<Paragraph> {
    let total = chars.len();
    let para_starts = starts(para_runs.iter().map(|r| r.len));
    let char_starts = starts(char_runs.iter().map(|r| r.len));
    let props_at = |i: usize| run_at(&char_starts, i).and_then(|k| char_runs.get(k)).map(|r| run_props(r, env, scheme)).unwrap_or_default();
    let mut out = vec![];
    let mut start = 0usize;
    loop {
        if !env.take_item() {
            break;
        }
        let end = chars.get(start..).and_then(|rest| rest.iter().position(|&c| c == '\r')).map_or(total, |e| start + e);
        let pr = run_at(&para_starts, start).and_then(|i| para_runs.get(i));
        let level = pr.map_or(0, |r| r.level.min(LEVELS as u8 - 1));
        let mut para = Paragraph { level, props: para_props(pr, ruler.and_then(|r| r.get(usize::from(level)))), ..Default::default() };
        let mut i = start;
        while i < end {
            if !env.take_item() {
                break;
            }
            let props = props_at(i);
            if chars.get(i) == Some(&'\u{b}') {
                para.runs.push(Run { text: String::new(), props, kind: RunKind::Break });
                i += 1;
                continue;
            }
            // A text run: up to the next format change, line break or the paragraph's end.
            let next_change = char_starts.get(char_starts.partition_point(|&s| s <= i)).copied().unwrap_or(end).min(end);
            let mut j = i;
            while j < next_change && chars.get(j) != Some(&'\u{b}') {
                j += 1;
            }
            let j = j.max(i + 1);
            let text: String = chars.get(i..j).unwrap_or(&[]).iter().filter(|&&c| c >= ' ' || c == '\t').collect();
            para.runs.push(Run::with(text, props));
            i = j;
        }
        // The paragraph mark's own formatting: the character run at the CR (PowerPoint 95 gives
        // it one of its own when it differs), or the last run for the last paragraph.
        para.end_props = props_at(end);
        para.normalize();
        out.push(para);
        if end >= total {
            break;
        }
        start = end + 1;
    }
    out
}

/// A character run's formatting, which PowerPoint 95 gives in full: font, size, bold, italic,
/// underline and colour (an RGB, or a slot of the colour scheme).
fn run_props(r: &CharRun, env: &Env, scheme: &Scheme) -> RunProps {
    let [red, g, b, index] = r.color;
    let color = if index == RGB_GIVEN { Some(Rgba::rgb(red, g, b)) } else { scheme.get(usize::from(index)).copied() };
    RunProps {
        font: env.fonts.get(usize::from(r.font)).cloned().flatten().filter(|f| !f.is_empty()),
        size: Some(f64::from(r.size)).filter(|s| (1.0..=4000.0).contains(s)),
        bold: Some(r.style & 1 != 0),
        italic: Some(r.style & 2 != 0),
        underline: Some(if r.style & 4 != 0 { "sng" } else { "none" }.into()),
        fill: color.map(|c| Fill::solid(ColorRef::rgb(c))),
        ..Default::default()
    }
}

/// The most points of spacing DrawingML holds (its `spcPts` reaches 158400 hundredths), as the
/// writer clamps it, so an imported value survives export.
const MAX_SPACING_PTS: f64 = 1584.0;

/// A paragraph run's spacing: a percentage of a line when positive, else minus a size in points.
/// Bounded, so a hostile value can't push text off any page.
fn spacing(v: i32) -> Spacing {
    if v >= 0 { Spacing::Pct((f64::from(v) / 100.0).min(100.0)) } else { Spacing::Pts((-f64::from(v)).min(MAX_SPACING_PTS)) }
}

/// A paragraph run's alignment, spacing and bullet, and the margins its level has in the text's
/// ruler.
fn para_props(r: Option<&ParaRun>, level: Option<&(i32, i32)>) -> ParaProps {
    let mut p = ParaProps::default();
    if let Some(r) = r {
        p.align = match r.align {
            0 => Some(Align::Left),
            1 => Some(Align::Center),
            2 => Some(Align::Right),
            3 => Some(Align::Justify),
            _ => None,
        };
        p.line_spacing = r.line.map(spacing);
        p.space_before = r.before.map(spacing);
        p.space_after = r.after.map(spacing);
        p.bullet = Some(if r.bullet {
            let c = if r.bullet_char >= 0x20 { craft_codepage::cp1252(r.bullet_char) } else { '•' };
            Bullet::Char { char: c.to_string() }
        } else {
            Bullet::None
        });
    }
    if let Some(&(left, first)) = level {
        // Bounded, so a hostile ruler can't push text off any page.
        let (left, first) = ((f64::from(left) / MU).clamp(0.0, 4000.0), (f64::from(first) / MU).clamp(0.0, 4000.0));
        p.margin_left = Some(left);
        p.indent = Some(first - left);
    }
    p
}
