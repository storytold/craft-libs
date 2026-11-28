//! Reader for `.cmx` exchange files with 32-bit coordinates: the RIFF form `CMX1` (R53), its
//! header (R54), the color, pen, line type, dash, outline and arrowhead tables (R55 to R59) and the
//! instructions of each page (R60): pages, layers, groups and curves (R61, R62) with uniform fills
//! and outlines (R63). Text, images, rectangles, ellipses, clipping paths, other fills and lenses
//! are left out with a warning.

use std::collections::BTreeSet;

use crate::bytes::{Rd, u32_at};
use crate::geometry::path_of;
use crate::layout::{self, Profile};
use crate::limits::Budget;
use crate::model::{Cap, Color, Drawing, Geometry, Join, Layer, LayerKind, Matrix, Object, Outline, Page, Pt, Shape};
use crate::records::{name, property_color};
use crate::riff::{LIST, push};
use crate::{Error, Warning};

const FORM: [u8; 4] = *b"CMX1";
/// Meters per coordinate unit when the header gives no usable value: the 0.1 µm of every sample
/// file with 32-bit coordinates (R54).
const DEFAULT_UNIT: f64 = 1e-7;
/// How far past a chunk that runs short the next chunk id is looked for (R53).
const RESYNC: usize = 64;
/// How many steps (a chunk, or a jump over stray bytes) the reader walks on from a position it
/// could resume at, to compare it with the first chunk id found (R53).
const LOOKAHEAD: usize = 16;
/// The work counted for one step of a walk ([`walk`]) against [`PROBE_FLOOR`] (R53).
const STEP_COST: usize = 64;
/// The least work one read may spend on recoveries after stray bytes (R53), counted as
/// [`STEP_COST`] for each step of a walk and as the length of each page body judged. A read may
/// spend four times the length of the RIFF form, cut at the file's length, when that is more. Past it, the reader resumes at the
/// first chunk id found.
const PROBE_FLOOR: usize = 1 << 20;
/// The chunks the reader uses (R53 to R60).
const READ_IDS: [[u8; 4]; 9] = [*b"LIST", *b"cont", *b"page", *b"rclr", *b"rpen", *b"rott", *b"rdot", *b"rotl", *b"rota"];
/// The most UTF-16 units read of a layer's name; [`name`] keeps 256 of them.
const MAX_NAME_UNITS: usize = 4096;

// Instruction codes (R60).
const BEGIN_PAGE: i16 = 9;
const END_PAGE: i16 = 10;
const BEGIN_LAYER: i16 = 11;
const END_LAYER: i16 = 12;
const BEGIN_GROUP: i16 = 13;
const END_GROUP: i16 = 14;
const BEGIN_TEXT: i16 = 20;
const END_TEXT: i16 = 21;
const GLYPH_RUN: i16 = 65;
const ELLIPSE: i16 = 66;
const CURVE: i16 = 67;
const RECTANGLE: i16 = 68;
const IMAGE: i16 = 69;
const ADD_CLIP: i16 = 88;
const REMOVE_CLIP: i16 = 89;
const JUMP: i16 = 111;

/// Does `bytes` start as a CMX file: a RIFF or RIFX form of type `CMX1` (R53)?
pub(crate) fn is_cmx(bytes: &[u8]) -> bool {
    matches!(bytes.get(0..4), Some(b"RIFF" | b"RIFX")) && bytes.get(8..12) == Some(FORM.as_slice())
}

/// A chunk outside the `LIST` chunks: its id, its body and where the body starts in the file.
struct Chunk<'a> {
    id: [u8; 4],
    body: &'a [u8],
    start: usize,
    /// Only a recovery after stray bytes reads the chunk ([`inner_page`]): the header or a table
    /// is taken from it only when no other chunk of its id is read, and a page is kept only when it
    /// reads with objects and without damage in its instructions.
    recovered: bool,
}

/// The first chunk of id `id` that is not recovered, or else the first recovered one.
fn first_of<'c, 'a>(chunks: &'c [Chunk<'a>], id: &[u8; 4]) -> Option<&'c Chunk<'a>> {
    chunks.iter().find(|c| c.id == *id && !c.recovered).or_else(|| chunks.iter().find(|c| c.id == *id))
}

/// The tables that page instructions refer to by 1-based index (R55 to R59).
#[derive(Default)]
struct Tables {
    colors: Vec<Option<Color>>,
    /// Outline widths in points.
    pens: Vec<Option<f64>>,
    /// Flags, and the caps and join.
    line_types: Vec<Option<(u8, u8)>>,
    dashes: Vec<Vec<u16>>,
    /// Line type, screen, color, arrowheads, pen and dashes.
    outlines: Vec<Option<[u16; 6]>>,
    arrowheads: Vec<(u16, u16)>,
    /// Missing entries are not reported as damage: a page is read only to judge its instructions
    /// ([`page_reads`]).
    quiet: bool,
}

/// The drawing in `bytes` (R53 to R63). When the chunks that only a recovery after stray bytes
/// reads make the read stop at a limit, the file is read again without the recovery, with the
/// warning for limits.
pub(crate) fn drawing(bytes: &[u8], budget: &mut Budget) -> Result<Drawing, Error> {
    let fresh = budget.clone();
    match read_drawing(bytes, budget, true) {
        Err(Error::Limit(_)) => {
            *budget = fresh;
            let mut d = read_drawing(bytes, budget, false)?;
            push(&mut d.warnings, Warning::TooDeep);
            Ok(d)
        }
        read => read,
    }
}

/// The drawing as the reader reads it when it resumes after stray bytes only at the first chunk
/// id found, never at a page inside that chunk (R53).
#[cfg(any(test, feature = "synth"))]
pub(crate) fn drawing_without_recovery(bytes: &[u8], budget: &mut Budget) -> Result<Drawing, Error> {
    read_drawing(bytes, budget, false)
}

fn read_drawing(bytes: &[u8], budget: &mut Budget, recover: bool) -> Result<Drawing, Error> {
    if !is_cmx(bytes) {
        return Err(Error::NotCmx);
    }
    if bytes.get(0..4) == Some(b"RIFX") {
        return Err(Error::Unsupported("big-endian CMX files"));
    }
    let mut warn = Vec::new();
    let chunks = chunks(bytes, budget, &mut warn, recover)?;
    let cont = first_of(&chunks, b"cont").map(|c| c.body).ok_or(Error::Damaged("CMX header"))?;
    match cont.get(52) {
        Some(b'4') => {}
        Some(b'2') => return Err(Error::Unsupported("CMX files with 16-bit coordinates")),
        _ => return Err(Error::Damaged("CMX header")),
    }
    let meters = Rd::at(cont, 64).f64().filter(|m| *m > 0.0 && *m <= 1.0).unwrap_or(DEFAULT_UNIT);
    let unit = meters / 0.0254 * 72.0;
    let tables = Tables::read(&chunks, unit, budget, &mut warn)?;
    // The pages read without the recovery go first, so that recovered pages spend only what the
    // limits leave after them; the pages keep the order of the file.
    let mut pages = Vec::new();
    for c in chunks.iter().filter(|c| c.id == *b"page" && !c.recovered) {
        pages.push((c.start, PageRead::new(&tables, unit, budget, &mut warn).read(c)?));
    }
    for c in chunks.iter().filter(|c| c.id == *b"page" && c.recovered) {
        // A recovered page is judged however much the recoveries spent. One that a limit stops,
        // or leaves without objects, is left out with the warning that says so; one the limits
        // cut short is read as far as they allow, as the other pages are.
        let read = match judgement(c, budget) {
            Ok((true, false, _)) => PageRead::new(&tables, unit, budget, &mut warn).read(c),
            Ok((_, _, true)) | Err(Error::Limit(_)) => {
                push(&mut warn, Warning::TooDeep);
                continue;
            }
            _ => continue,
        };
        match read {
            Ok(page) => pages.push((c.start, page)),
            Err(Error::Limit(_)) => push(&mut warn, Warning::TooDeep),
            Err(e) => return Err(e),
        }
    }
    pages.sort_by_key(|(start, _)| *start);
    let pages = pages.into_iter().map(|(_, page)| page).collect();
    Ok(Drawing { version: 0, page_size: None, pages, master: Vec::new(), guides: Vec::new(), warnings: warn })
}

/// The chunks of the file in order, those inside `LIST` chunks in place (R53). With `recover`, the
/// reader may resume after stray bytes at a page inside the first chunk id found ([`inner_page`]).
fn chunks<'a>(b: &'a [u8], budget: &mut Budget, warn: &mut Vec<Warning>, recover: bool) -> Result<Vec<Chunk<'a>>, Error> {
    let declared = u32_at(b, 4).and_then(|n| (n as usize).checked_add(8)).unwrap_or(b.len());
    if declared > b.len() {
        // The file was cut short of the size its header gives.
        push(warn, Warning::Damaged);
    }
    let end = declared.min(b.len());
    let mut out = Vec::new();
    // Where the chunks start that only a recovery reads ([`rejoins`]).
    let mut extra = BTreeSet::new();
    // The work the recoveries may still spend ([`PROBE_FLOOR`]).
    let mut left = end.saturating_mul(4).max(PROBE_FLOOR);
    // (position, end, depth, recovered) of the lists still to walk; a list's children go before
    // the rest of the list that holds it.
    let mut stack = vec![(12usize, end, 0usize, false)];
    while let Some((mut pos, end, depth, in_recovered)) = stack.pop() {
        while pos.saturating_add(8) <= end {
            let Some(id) = Rd::at(b, pos).fourcc() else { break };
            if !is_id(&id) {
                match resync(b, pos, end, depth, recover.then_some(&*budget), &mut left) {
                    Some((next, damaged, found)) => {
                        if damaged {
                            push(warn, Warning::Damaged);
                        }
                        extra.extend(found);
                        pos = next;
                        continue;
                    }
                    None => {
                        push(warn, Warning::Damaged);
                        break;
                    }
                }
            }
            budget.take_chunk()?;
            let size = u32_at(b, pos + 4).map_or(0, |s| s as usize);
            let start = pos + 8;
            let stop = start.saturating_add(size);
            if stop > end {
                // Stray bytes that happen to start with four letters give a length past the list:
                // look for the next chunk id after them, and report the damage. A chunk the reader
                // uses is kept, cut at the end of the list, unless it is a page that holds a page
                // the reader resumes at.
                push(warn, Warning::Damaged);
                let resume = if !READ_IDS.contains(&id) {
                    resync(b, pos, end, depth, recover.then_some(&*budget), &mut left).map(|(next, _, found)| (next, found))
                } else if recover && id == *b"page" {
                    inner_page(b, pos, end, depth, budget, &mut left)
                } else {
                    None
                };
                if let Some((next, found)) = resume {
                    extra.extend(found);
                    pos = next;
                    continue;
                }
            }
            let stop = stop.min(end);
            let next = stop.saturating_add(size & 1);
            let recovered = in_recovered || extra.contains(&pos);
            if id == LIST {
                if depth >= budget.limits.depth {
                    push(warn, Warning::TooDeep);
                    pos = next;
                    continue;
                }
                stack.push((next, end, depth, in_recovered));
                stack.push((start + 4, stop, depth + 1, recovered));
                break;
            }
            if let Some(body) = b.get(start..stop) {
                out.push(Chunk { id, body, start, recovered });
            }
            pos = next;
        }
    }
    Ok(out)
}

/// Is `t` a chunk id: an ASCII letter, then three ASCII letters, digits or spaces (R53)?
fn is_id(t: &[u8]) -> bool {
    matches!(t, [first, rest @ ..] if t.len() == 4 && first.is_ascii_alphabetic() && rest.iter().all(|c| c.is_ascii_alphanumeric() || *c == b' '))
}

/// The chunk id at `p`, when the four bytes there read as one.
fn id_at(b: &[u8], p: usize) -> Option<[u8; 4]> {
    Rd::at(b, p).fourcc().filter(|id| is_id(id))
}

/// Where the body of the chunk at `p` ends by its length, before its pad byte.
fn chunk_stop(b: &[u8], p: usize) -> Option<usize> {
    u32_at(b, p + 4).and_then(|s| (p + 8).checked_add(s as usize))
}

/// The first chunk id after `pos`, at an even distance of 2 to [`RESYNC`] bytes, that names a
/// chunk the reader uses or another chunk whose length keeps it inside the list that ends at `end`
/// (R53).
fn first_chunk(b: &[u8], pos: usize, end: usize) -> Option<usize> {
    let fits = |p: usize| chunk_stop(b, p).is_some_and(|stop| stop <= end);
    let chunk_at = |p: usize| id_at(b, p).is_some_and(|id| READ_IDS.contains(&id) || fits(p));
    (pos + 2..=pos.saturating_add(RESYNC).min(end.saturating_sub(8))).step_by(2).find(|&p| chunk_at(p))
}

/// Where the next chunk after stray bytes at `pos` lies, whether the bytes passed over are
/// reported as damage, and where the chunks start that only the recovery reads (R53). It is
/// [`first_chunk`], or, with `judge` (the read's budget), a page inside that chunk ([`inner_page`]),
/// found with at most `left` work ([`PROBE_FLOOR`]). The flag is set when the reader resumes at an
/// inner page, or at a chunk it passes over that is not followed by another chunk id or by the end
/// of the list. `depth` is how many lists hold the one that ends at `end`.
fn resync(b: &[u8], pos: usize, end: usize, depth: usize, judge: Option<&Budget>, left: &mut usize) -> Option<(usize, bool, Vec<usize>)> {
    let p = first_chunk(b, pos, end)?;
    if let Some((k, found)) = judge.and_then(|budget| inner_page(b, p, end, depth, budget, left)) {
        return Some((k, true, found));
    }
    let known = id_at(b, p).is_some_and(|id| READ_IDS.contains(&id));
    let followed = chunk_stop(b, p).filter(|stop| *stop <= end).is_some_and(|stop| {
        let n = stop.saturating_add((stop - p - 8) & 1);
        n >= end || (n.saturating_add(8) <= end && id_at(b, n).is_some())
    });
    Some((p, !known && !followed, Vec::new()))
}

/// A page the reader resumes at instead of the chunk at `p`, the first chunk id after stray bytes
/// or letters spelling `page` whose length runs past the list (R53). It is a page id at an even
/// distance of 8 to [`RESYNC`] bytes after `p`, inside the body of that chunk, whose length is not
/// 0, keeps it inside the list and, when the chunk at `p` ends inside the list, reaches the end of
/// that chunk. A `LIST`, the header and the tables at `p` are always kept. Walking on from `p` and
/// from the page by the reader's rule without this recovery must meet ([`rejoins`]). The first page
/// that meets is taken, with the chunks only the walk from it reads. The walks and the pages judged
/// spend `left` ([`PROBE_FLOOR`]); when it runs out, no page is taken. `depth` is how many lists
/// hold the one that ends at `end`.
fn inner_page(b: &[u8], p: usize, end: usize, depth: usize, budget: &Budget, left: &mut usize) -> Option<(usize, Vec<usize>)> {
    let id = id_at(b, p)?;
    if READ_IDS.contains(&id) && id != *b"page" {
        return None;
    }
    let stop = chunk_stop(b, p).filter(|stop| *stop <= end);
    let body_end = stop.unwrap_or(end);
    // The chunk at `k` ends, with its pad byte, at or past the end of the chunk at `p`.
    let reaches =
        |k: usize| chunk_stop(b, k).is_some_and(|s| s > k + 8 && s <= end && stop.is_none_or(|stop| s.saturating_add((s - k - 8) & 1) >= stop));
    let mut found = (p + 8..=p.saturating_add(RESYNC).min(end.saturating_sub(8)))
        .step_by(2)
        .filter(|&k| k < body_end && id_at(b, k) == Some(*b"page") && reaches(k))
        .peekable();
    found.peek()?;
    let lists = budget.limits.depth.saturating_sub(depth);
    let first = walk(b, p, end, lists, left)?;
    found.find_map(|k| {
        let alt = walk(b, k, end, lists, left)?;
        rejoins(b, &first, &alt, budget, left).map(|extra| (k, extra))
    })
}

/// Positions a walk through a list passes and the chunks it reads, for at most [`LOOKAHEAD`]
/// steps.
struct Walk {
    /// Where the walk starts, and its place after each step: the position, and for each `LIST`
    /// chunk it is inside, where that list's body ends and where the reader goes on after it.
    at: Vec<(usize, Vec<(usize, usize)>)>,
    /// Where each chunk the walk read starts, and where its body ends, cut at the end of the list
    /// that holds it; a `LIST` chunk it enters counts by the chunks inside.
    read: Vec<(usize, usize)>,
    /// The walk left the list it started in: at its end, or at stray bytes with no chunk id after
    /// them, where [`chunks`] stops reading that list too.
    ended: bool,
}

/// Walk the list that ends at `end` from `from` as [`chunks`] does without recovery, entering at
/// most `lists` nested `LIST` chunks as it does, and spending [`STEP_COST`] of `left` for each step;
/// `None` when `left` runs out first.
fn walk(b: &[u8], from: usize, end: usize, lists: usize, left: &mut usize) -> Option<Walk> {
    let mut w = Walk { at: vec![(from, Vec::new())], read: Vec::new(), ended: false };
    let mut pos = from;
    let mut inside: Vec<(usize, usize)> = Vec::new();
    for _ in 0..LOOKAHEAD {
        *left = left.checked_sub(STEP_COST)?;
        let list_end = inside.last().map_or(end, |&(stop, _)| stop);
        match step(b, pos, list_end) {
            Some((next, Some(chunk))) if id_at(b, chunk) == Some(LIST) && inside.len() < lists => {
                // [`chunks`] reads the list's children before the rest of the list that holds it.
                let stop = chunk_stop(b, chunk).map_or(list_end, |stop| stop.min(list_end));
                inside.push((stop, next));
                pos = chunk + 12;
            }
            Some((next, chunk)) => {
                w.read.extend(chunk.map(|c| (c, chunk_stop(b, c).map_or(list_end, |stop| stop.min(list_end)))));
                pos = next;
            }
            None => match inside.pop() {
                Some((_, after)) => pos = after,
                None => {
                    w.ended = true;
                    break;
                }
            },
        }
        w.at.push((pos, inside.clone()));
    }
    Some(w)
}

/// One step of [`chunks`] without recovery at `pos`: the position after it, and where the chunk
/// it read starts when it read one; `None` when the walk ends there.
fn step(b: &[u8], pos: usize, end: usize) -> Option<(usize, Option<usize>)> {
    if pos.saturating_add(8) > end {
        return None;
    }
    let id = Rd::at(b, pos).fourcc()?;
    if !is_id(&id) {
        return first_chunk(b, pos, end).map(|next| (next, None));
    }
    let size = u32_at(b, pos + 4).map_or(0, |s| s as usize);
    let stop = (pos + 8).saturating_add(size);
    if stop > end
        && !READ_IDS.contains(&id)
        && let Some(next) = first_chunk(b, pos, end)
    {
        return Some((next, None));
    }
    Some((stop.min(end).saturating_add(size & 1), Some(pos)))
}

/// Where the chunks start that the walk from an inner page `alt` reads and the walk from the first
/// chunk id `first` does not read with the same body, when the two walks meet at a place both pass (a position inside the
/// same lists) or both end, and `alt` reads at least what `first` reads until then (R53). The page
/// `alt` starts at reads with objects and without damage in its instructions. Each chunk the reader
/// uses that `first` reads before that point, `alt` reads too, with the same body, unless it is a
/// page that reads without objects. The pages judged spend `left`; when it runs out, or when the
/// read of a page judged stops, the walks do not meet.
fn rejoins(b: &[u8], first: &Walk, alt: &Walk, budget: &Budget, left: &mut usize) -> Option<Vec<usize>> {
    let meet = first.at.iter().find(|q| alt.at.contains(q)).map(|(q, _)| *q).or((first.ended && alt.ended).then_some(usize::MAX))?;
    let uses = |q: usize| q < meet && id_at(b, q).is_some_and(|id| READ_IDS.contains(&id));
    let used = |w: &Walk| -> Vec<(usize, usize)> { w.read.iter().copied().filter(|&(q, _)| uses(q)).collect() };
    let (kept, taken) = (used(first), used(alt));
    let (start, stop) = *alt.read.first()?;
    if page_reads(b, start, stop, budget, left)? != (true, false) {
        return None;
    }
    for &(q, stop) in kept.iter().filter(|q| !taken.contains(q)) {
        // A page that reads without objects is the only chunk the walk from `alt` may leave out,
        // or read with a body cut shorter by a list.
        if id_at(b, q) != Some(*b"page") || page_reads(b, q, stop, budget, left)?.0 {
            return None;
        }
    }
    Some(taken.into_iter().filter(|q| !kept.contains(q)).map(|(q, _)| q).collect())
}

/// Whether the page chunk at `at` reads with objects, and whether its instructions read with
/// damage ([`judge_page`]); `None` when `left` does not cover its body or the read stops.
fn page_reads(b: &[u8], at: usize, end: usize, budget: &Budget, left: &mut usize) -> Option<(bool, bool)> {
    let stop = chunk_stop(b, at).map_or(end, |stop| stop.min(end));
    let Some(body) = b.get(at + 8..stop) else { return Some((false, true)) };
    judge_page(&Chunk { id: *b"page", body, start: at + 8, recovered: true }, budget, left)
}

/// Whether page `c` reads with objects, and whether its instructions read with damage. It is read
/// with empty tables whose missing entries are not reported, and with a copy of `budget` so that the
/// read's own budget is not spent. The length of its body is taken from `left` first; `None` when
/// `left` holds less, or when the read stops.
fn judge_page(c: &Chunk, budget: &Budget, left: &mut usize) -> Option<(bool, bool)> {
    *left = left.checked_sub(c.body.len())?;
    judgement(c, budget).ok().map(|(objects, damaged, _)| (objects, damaged))
}

/// Whether page `c` reads with objects, whether its instructions read with damage, and whether
/// the read left something out at one of the limits, read as [`judge_page`] reads it; the error
/// when the read stops.
fn judgement(c: &Chunk, budget: &Budget) -> Result<(bool, bool, bool), Error> {
    let tables = Tables { quiet: true, ..Tables::default() };
    let mut scratch = budget.clone();
    let mut warn = Vec::new();
    let page = PageRead::new(&tables, 1.0, &mut scratch, &mut warn).read(c)?;
    Ok((!page.layers.is_empty(), warn.contains(&Warning::Damaged), warn.contains(&Warning::TooDeep)))
}

/// The end of the field list at `pos` of `b`: after its 0xff, or the end of `b` when a field runs
/// past it (R55).
fn list_end(b: &[u8], mut pos: usize) -> usize {
    while let Some(&tag) = b.get(pos) {
        if tag == 0xff {
            return pos + 1;
        }
        match Rd::at(b, pos + 1).u16().map(usize::from).filter(|n| *n >= 3) {
            Some(n) if pos + n <= b.len() => pos += n,
            _ => break,
        }
    }
    b.len()
}

/// The data of field `tag` in the field list `b` (R55).
fn field(b: &[u8], tag: u8) -> Option<&[u8]> {
    let mut pos = 0;
    while let Some(&t) = b.get(pos) {
        if t == 0xff {
            return None;
        }
        let n = usize::from(Rd::at(b, pos + 1).u16()?);
        let data = b.get(pos + 3..pos.checked_add(n)?).filter(|_| n >= 3)?;
        if t == tag {
            return Some(data);
        }
        pos += n;
    }
    None
}

/// The field lists of a table: a u16 count, then that many lists (R55). Each entry counts toward
/// the chunk limit.
fn entries<'a>(d: &'a [u8], budget: &mut Budget) -> Result<Vec<&'a [u8]>, Error> {
    let count = Rd::new(d).u16().map_or(0, usize::from);
    let mut out = Vec::new();
    let mut pos = 2;
    for _ in 0..count {
        if pos >= d.len() {
            break;
        }
        budget.take_chunk()?;
        let end = list_end(d, pos);
        out.push(d.get(pos..end).unwrap_or_default());
        pos = end;
    }
    Ok(out)
}

impl Tables {
    /// The first chunk of each table; a table that is missing is empty.
    fn read(chunks: &[Chunk], unit: f64, budget: &mut Budget, warn: &mut Vec<Warning>) -> Result<Tables, Error> {
        let first = |id: &[u8; 4]| first_of(chunks, id).map_or(&[][..], |c| c.body);
        // A color record of field 10 holds a property list as a `.cdr` fill does (R56).
        let record = layout::for_version(1300);
        let mut t = Tables::default();
        for e in entries(first(b"rclr"), budget)? {
            t.colors.push(color(e, record.as_ref(), warn));
        }
        for e in entries(first(b"rpen"), budget)? {
            t.pens.push(field(e, 1).and_then(|f| Rd::new(f).u32()).map(|w| f64::from(w) * unit));
        }
        for e in entries(first(b"rott"), budget)? {
            t.line_types.push(field(e, 1).and_then(|f| Some((*f.first()?, f.get(1).copied().unwrap_or(0)))));
        }
        for e in entries(first(b"rdot"), budget)? {
            let mut r = Rd::new(field(e, 1).unwrap_or_default());
            let n = r.u16().map_or(0, usize::from);
            let mut pattern: Vec<u16> = (0..n).map_while(|_| r.u16()).collect();
            // A pattern of zero lengths draws as a solid line, as in a `.cdr` outline (R48).
            if pattern.iter().all(|x| *x == 0) {
                pattern.clear();
            }
            t.dashes.push(pattern);
        }
        for e in entries(first(b"rotl"), budget)? {
            t.outlines.push(field(e, 1).and_then(|f| {
                let mut r = Rd::new(f);
                Some([r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?])
            }));
        }
        // Arrowhead pairs are plain u16 pairs after the count, not field lists (R59).
        let mut r = Rd::new(first(b"rota"));
        let n = r.u16().map_or(0, usize::from);
        for _ in 0..n {
            let Some(pair) = r.u16().zip(r.u16()) else { break };
            budget.take_chunk()?;
            t.arrowheads.push(pair);
        }
        Ok(t)
    }

    /// Entry `index` (1-based) of `list`, or `None` with a warning when there is none.
    fn get<T: Clone>(&self, list: &[Option<T>], index: u16, warn: &mut Vec<Warning>) -> Option<T> {
        let found = usize::from(index).checked_sub(1).and_then(|i| list.get(i)).cloned().flatten();
        if found.is_none() && !self.quiet {
            push(warn, Warning::Damaged);
        }
        found
    }

    fn color(&self, index: u16, warn: &mut Vec<Warning>) -> Color {
        self.get(&self.colors, index, warn).unwrap_or(Color::Rgb([0.0; 3]))
    }

    /// The outline of `rotl` entry `index`, or `None` when its line type draws none (R57, R58).
    fn outline(&self, index: u16, budget: &mut Budget, warn: &mut Vec<Warning>) -> Option<Outline> {
        let [line, _screen, color, arrows, pen, dash] = self.get(&self.outlines, index, warn)?;
        let (flags, caps_join) = self.get(&self.line_types, line, warn).unwrap_or((2, 0));
        if flags & 1 != 0 {
            return None;
        }
        let width = self.get(&self.pens, pen, warn).unwrap_or(0.0);
        let mut dashes = Vec::new();
        if flags & 4 != 0 {
            let stored = usize::from(dash).checked_sub(1).and_then(|i| self.dashes.get(i));
            match stored {
                Some(d) if d.is_empty() => {}
                Some(d) if budget.take_dashes(d.len()) => dashes = d.iter().map(|x| f64::from(*x)).collect(),
                Some(_) => push(warn, Warning::TooDeep),
                None => push(warn, Warning::Damaged),
            }
        }
        if usize::from(arrows).checked_sub(1).and_then(|i| self.arrowheads.get(i)).is_some_and(|(s, e)| *s != 0 || *e != 0) {
            push(warn, Warning::Arrowheads);
        }
        Some(Outline {
            width,
            color: self.color(color, warn),
            cap: match caps_join & 0x0f {
                1 => Cap::Round,
                2 => Cap::Square,
                _ => Cap::Butt,
            },
            join: match caps_join >> 4 {
                1 => Join::Round,
                2 => Join::Bevel,
                _ => Join::Miter,
            },
            dashes,
            behind_fill: flags & 8 != 0,
            scale_with: flags & 16 != 0,
        })
    }
}

/// A color entry (R56): the property list of field 10 when there is one, else the model and
/// palette bytes of field 1 and the values of field 2. Colors in other models read as black with a
/// warning.
fn color(e: &[u8], record: Option<&Profile>, warn: &mut Vec<Warning>) -> Option<Color> {
    if let (Some(f), Some(p)) = (field(e, 10), record) {
        let mut r = Rd::new(f);
        let len = r.u32()? as usize;
        return property_color(r.bytes(len.min(r.left()))?, p, warn);
    }
    let model = *field(e, 1)?.first()?;
    let v = field(e, 2)?;
    let byte = |i: usize| v.get(i).map(|b| f32::from(*b));
    let pct = |i: usize| byte(i).map(|x| (x / 100.0).clamp(0.0, 1.0));
    let full = |i: usize| byte(i).map(|x| x / 255.0);
    Some(match model {
        2 => Color::Cmyk([pct(0)?, pct(1)?, pct(2)?, pct(3)?]),
        3 => Color::Cmyk([full(0)?, full(1)?, full(2)?, full(3)?]),
        4 => Color::Cmyk([full(0)?, full(1)?, full(2)?, 0.0]),
        5 => Color::Rgb([full(0)?, full(1)?, full(2)?]),
        9 => Color::Gray(1.0 - full(0)?),
        _ => {
            push(warn, Warning::UnknownColors);
            Color::Rgb([0.0; 3])
        }
    })
}

/// The objects of one layer while it is read: the layer's own list, then one list for each group
/// left open.
struct LayerRead {
    name: String,
    lists: Vec<Vec<Object>>,
}

impl LayerRead {
    /// The layer, its open groups closed. Lists are drawn back to front (R60), and the model holds
    /// them front to back.
    fn finish(mut self) -> Layer {
        while self.lists.len() > 1 {
            close_group(&mut self.lists);
        }
        let mut objects = self.lists.pop().unwrap_or_default();
        objects.reverse();
        Layer { name: self.name, kind: LayerKind::Drawing, flags: Default::default(), objects, guides: Vec::new() }
    }
}

/// Close the innermost open group of `lists` into the one that holds it.
fn close_group(lists: &mut Vec<Vec<Object>>) {
    if lists.len() < 2 {
        return;
    }
    let mut objects = lists.pop().unwrap_or_default();
    objects.reverse();
    if let Some(top) = lists.last_mut() {
        top.push(Object::Group { objects, flags: Default::default() });
    }
}

/// The reading of one page chunk.
struct PageRead<'t, 'b> {
    tables: &'t Tables,
    unit: f64,
    budget: &'b mut Budget,
    warn: &'b mut Vec<Warning>,
    size: Option<(f64, f64)>,
    /// The page's center in the file's coordinates when the page takes its size from the
    /// drawing's bounds; points are moved by it so the art lies on the page.
    center: (f64, f64),
    layers: Vec<Layer>,
    open: Option<LayerRead>,
    /// Groups nested past the depth limit that are still open; their content is left out.
    skipped: usize,
}

impl<'t, 'b> PageRead<'t, 'b> {
    fn new(tables: &'t Tables, unit: f64, budget: &'b mut Budget, warn: &'b mut Vec<Warning>) -> Self {
        PageRead { tables, unit, budget, warn, size: None, center: (0.0, 0.0), layers: Vec::new(), open: None, skipped: 0 }
    }

    /// The instructions of a page chunk: a u16 size counting the whole instruction and an i16 code,
    /// or 0xFFFF, a u32 size and the code; then the code's field lists (R60).
    fn read(mut self, c: &Chunk) -> Result<Page, Error> {
        let d = c.body;
        let mut pos = 0usize;
        while pos.saturating_add(4) <= d.len() {
            self.budget.take_chunk()?;
            let mut r = Rd::at(d, pos);
            let (size, code, head) = match r.u16() {
                Some(0xFFFF) => match (r.u32(), r.i16()) {
                    (Some(size), Some(code)) => (size as usize, code, 8),
                    _ => break,
                },
                Some(size) => match r.i16() {
                    Some(code) => (usize::from(size), code, 4),
                    None => break,
                },
                None => break,
            };
            if size < head {
                push(self.warn, Warning::Damaged);
                break;
            }
            let end = pos.saturating_add(size);
            if end > d.len() {
                push(self.warn, Warning::Damaged);
            }
            let body = d.get(pos + head..end.min(d.len())).unwrap_or_default();
            if code == JUMP {
                // Field 1 is the file offset the instructions go on from; the bytes passed over
                // hold the page's images (R60).
                let target = field(body, 1).and_then(|f| Rd::new(f).u32()).and_then(|t| (t as usize).checked_sub(c.start));
                pos = target.filter(|t| *t >= end && *t <= d.len()).unwrap_or(end);
                if pos > end {
                    push(self.warn, Warning::UnsupportedObjects);
                }
                continue;
            }
            self.instruction(code, body);
            pos = end;
        }
        if pos < d.len() {
            // An instruction whose head runs past the page was not read.
            push(self.warn, Warning::Damaged);
        }
        self.close_layer();
        // A layer that holds nothing (the special layers every sample file lists empty) is left
        // out. The file lists layers back to front.
        let mut layers = self.layers;
        layers.retain(|l| !l.objects.is_empty());
        layers.reverse();
        Ok(Page { size: self.size, layers })
    }

    fn instruction(&mut self, code: i16, body: &[u8]) {
        match code {
            BEGIN_PAGE => {
                // Field 4: the page's width and height (R61).
                let mut r = Rd::new(field(body, 4).unwrap_or_default());
                if let (Some(w), Some(h)) = (r.u32(), r.u32()) {
                    let (w, h) = (f64::from(w) * self.unit, f64::from(h) * self.unit);
                    self.size = (w > 0.0 && h > 0.0).then_some((w, h));
                }
                if self.size.is_none() {
                    self.size_from_bounds(field(body, 1).unwrap_or_default());
                }
            }
            BEGIN_LAYER => {
                self.close_layer();
                // A layer without a name is left unnamed, as in a `.cdr` drawing.
                let name = self.layer_name(body).unwrap_or_default();
                self.open = Some(LayerRead { name, lists: vec![Vec::new()] });
            }
            END_LAYER => self.close_layer(),
            BEGIN_GROUP => {
                let open = self.layer().lists.len();
                if self.skipped > 0 || open > self.budget.limits.depth {
                    self.skipped += 1;
                    push(self.warn, Warning::TooDeep);
                } else if self.budget.take_object() {
                    self.layer().lists.push(Vec::new());
                } else {
                    // Past the object limit the group's content joins the group that holds it.
                    self.skipped += 1;
                    push(self.warn, Warning::TooDeep);
                }
            }
            END_GROUP => {
                if self.skipped > 0 {
                    self.skipped -= 1;
                } else {
                    close_group(&mut self.layer().lists);
                }
            }
            CURVE if self.skipped == 0 => {
                if let Some(shape) = self.curve(body) {
                    if self.budget.take_object() {
                        if let Some(list) = self.layer().lists.last_mut() {
                            list.push(Object::Shape(Box::new(shape)));
                        }
                    } else {
                        push(self.warn, Warning::TooDeep);
                    }
                }
            }
            BEGIN_TEXT | GLYPH_RUN | ELLIPSE | RECTANGLE | IMAGE | ADD_CLIP => push(self.warn, Warning::UnsupportedObjects),
            // These and the other codes (text styles and characters, color contexts) draw nothing
            // on their own.
            END_PAGE | END_TEXT | REMOVE_CLIP => {}
            _ => {}
        }
    }

    /// A page without a size of its own is sized to the drawing's bounds in field 1 (R61): a u16
    /// page number, a u32 of flags, then the left, top, right and bottom edges as i32 in
    /// coordinate units. The page is centered on those bounds.
    fn size_from_bounds(&mut self, f: &[u8]) {
        let mut r = Rd::new(f);
        if r.u16().is_none() || r.u32().is_none() {
            return;
        }
        let (Some(l), Some(t), Some(rt), Some(b)) = (r.i32(), r.i32(), r.i32(), r.i32()) else {
            return;
        };
        let (l, t, rt, b) = (f64::from(l) * self.unit, f64::from(t) * self.unit, f64::from(rt) * self.unit, f64::from(b) * self.unit);
        let (w, h) = ((rt - l).abs(), (t - b).abs());
        if w > 0.0 && h > 0.0 {
            self.size = Some((w, h));
            self.center = ((l + rt) / 2.0, (t + b) / 2.0);
        }
    }

    /// The open layer, or a new unnamed one when objects come before any layer begins.
    fn layer(&mut self) -> &mut LayerRead {
        self.open.get_or_insert_with(|| LayerRead { name: String::new(), lists: vec![Vec::new()] })
    }

    fn close_layer(&mut self) {
        self.skipped = 0;
        if let Some(l) = self.open.take() {
            self.layers.push(l.finish());
        }
    }

    /// A layer's name: field 4 as a u32 count and UTF-16 units, else field 1 from byte 12 as a
    /// u16 count and 8-bit characters (R62).
    fn layer_name(&mut self, body: &[u8]) -> Option<String> {
        // A field 4 that can't be read leaves the name to field 1.
        let wide = field(body, 4).and_then(|f| {
            let mut r = Rd::new(f);
            let n = (r.u32()? as usize).min(MAX_NAME_UNITS).min(r.left() / 2);
            Some((r.bytes(n * 2)?, layout::for_version(1200)?))
        });
        if let Some((units, l)) = wide {
            let s = name(units, &l, self.warn);
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
        let mut r = Rd::at(field(body, 1)?, 12);
        let n = usize::from(r.u16()?).min(r.left());
        let s = name(r.bytes(n)?, &layout::for_version(1100)?, self.warn);
        (!s.trim().is_empty()).then_some(s)
    }

    /// A curve (R62): field 1 its rendering attributes, field 2 a u16 point count, the points (i32
    /// x and y) and one type byte for each. Points are from the page's center, y up.
    fn curve(&mut self, body: &[u8]) -> Option<Shape> {
        let mut r = Rd::new(field(body, 2).unwrap_or_default());
        let Some(count) = r.u16().map(usize::from) else {
            // A curve without its points is lost.
            push(self.warn, Warning::Damaged);
            return None;
        };
        // `stored` points are read, so the type bytes are found after all of them, and the first
        // `n`, those that have a type byte, are kept. When the field holds every point, a short
        // field has lost type bytes. When it holds whole points and their types for fewer points,
        // the count is too large. Otherwise the field lost points, and its type bytes can't be
        // found: the curve is lost.
        let left = r.left();
        let (stored, n) = match count.checked_mul(8).filter(|c| *c <= left) {
            Some(c) => (count, count.min(left - c)),
            None if left.is_multiple_of(9) => (left / 9, left / 9),
            None => {
                push(self.warn, Warning::Damaged);
                return None;
            }
        };
        if n < count {
            push(self.warn, Warning::Damaged);
        }
        if !self.budget.take_coordinates(stored) {
            push(self.warn, Warning::TooDeep);
            return None;
        }
        let mut points = Vec::with_capacity(stored);
        for _ in 0..stored {
            let (x, y) = (f64::from(r.i32()?) * self.unit, f64::from(r.i32()?) * self.unit);
            points.push(Pt { x: x - self.center.0, y: y - self.center.1 });
        }
        points.truncate(n);
        let path = path_of(&points, r.bytes(n)?);
        if path.subpaths.is_empty() {
            return None;
        }
        let (fill, outline) = self.attributes(field(body, 1).unwrap_or_default());
        Some(Shape::new(Geometry::Path(path), Matrix::IDENTITY, fill, outline))
    }

    /// Rendering attributes (R63): a mask (1 fill, 2 outline, 4 lens), then a field list for the
    /// fill and one for the outline when the mask names them.
    fn attributes(&mut self, a: &[u8]) -> (Option<Color>, Option<Outline>) {
        let Some(&mask) = a.first() else { return (None, None) };
        let mut pos = 1;
        let mut fill = None;
        if mask & 1 != 0 {
            let end = list_end(a, pos);
            let list = a.get(pos..end).unwrap_or_default();
            pos = end;
            // Field 1: the fill type (u16), then its own field list; type 1, uniform, names its
            // color in field 1.
            let v = field(list, 1).unwrap_or_default();
            match Rd::new(v).u16() {
                Some(0) | None => {}
                Some(1) => {
                    let index = field(v.get(2..).unwrap_or_default(), 1).and_then(|c| Rd::new(c).u16());
                    fill = Some(match index {
                        Some(i) => self.tables.color(i, self.warn),
                        None => {
                            push(self.warn, Warning::Damaged);
                            Color::Rgb([0.0; 3])
                        }
                    });
                }
                Some(_) => push(self.warn, Warning::UnsupportedFills),
            }
        }
        let mut outline = None;
        if mask & 2 != 0 {
            let list = a.get(pos..list_end(a, pos)).unwrap_or_default();
            match field(list, 1).and_then(|f| Rd::new(f).u16()) {
                Some(i) => outline = self.tables.outline(i, self.budget, self.warn),
                None => push(self.warn, Warning::Damaged),
            }
        }
        if mask & 4 != 0 {
            push(self.warn, Warning::Lenses);
        }
        // A canvas (8) or a container (16) is not read.
        if mask & (8 | 16) != 0 {
            push(self.warn, Warning::UnsupportedObjects);
        }
        (fill, outline)
    }
}
