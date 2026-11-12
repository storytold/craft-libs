//! Reading a file's focus point without reading the file: [`read_header`] asks for the few byte
//! ranges [`crate::focus::read`] looks at (a TIFF raw's IFD0, its Exif IFD, the maker-note IFD and
//! the values of the focus tags; a JPEG's Exif segment) and decodes them exactly as a whole-file
//! read would. On the α1 and the A7S III all of it sits in the first 16 KB of a 50–120 MB ARW, so a
//! library of them can be backfilled from a slow disk by reading one small block per photo.
//!
//! The ranges are found by walking the structure as the readers do (the IFD tables and the value
//! offsets they hold), round by round: each round asks only for what the bytes loaded so far point
//! at. A TIFF is decoded from a buffer of the file's length in which only the loaded ranges hold the
//! file's bytes (the rest stays zero and its pages are never touched), so every offset and bound the
//! reader checks is the one it checks on the whole file. Rounds, bytes and the buffer are bounded:
//! a file that wants more is reported as having no focus point.

use std::io;

use crate::focus::FocusPoint;
use craft_tiff::{ByteOrder, FieldType, Tiff, tags as t};

/// Bytes read first: the α1's and A7S III's focus data ends before 16 KB, so one read covers it.
pub const HEADER_PREFIX: u64 = 32 * 1024;
/// Later reads are widened to this granularity (fewer, larger reads on a spinning disk).
const ALIGN: u64 = 4 * 1024;
/// The most rounds of reads one file gets (a real file needs one; the walk takes at most nine).
const MAX_ROUNDS: usize = 12;
/// The most bytes read from one file: past this the layout is not one the decoder knows.
pub const MAX_HEADER_BYTES: u64 = 2 << 20;
/// The largest TIFF given a buffer of its length (its untouched pages are never written, but the
/// allocation must succeed: the engine's planned preview reads use the same bound). Raws are far below it.
const MAX_SPARSE_FILE: u64 = 1 << 30;
/// JPEG segments walked before giving up.
const MAX_SEGMENTS: usize = 1024;
/// Entries read from one IFD (the TIFF reader's limit).
const MAX_ENTRIES: u64 = 8192;

/// The focus tags whose values [`crate::focus`] reads from a Sony maker note.
const FOCUS_TAGS: [u16; 6] = [0x201b, 0x201c, 0x2021, 0x2027, 0x2037, 0x9402];

/// What [`read_header`] found, and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub struct HeaderRead {
    /// The focus point a whole-file read gives ([`crate::extract`]`(file).focus`).
    pub focus: Option<FocusPoint>,
    /// Bytes read from the file.
    pub bytes_read: u64,
}

/// The focus point of a file of `len` bytes, reading only what it needs through `read_at(offset,
/// buf)`, which fills `buf` with the file's bytes at `offset` (an error — a file that shrank or
/// went away — is returned). A malformed or unknown file is `Ok` with no point.
pub fn read_header(len: u64, read_at: &mut dyn FnMut(u64, &mut [u8]) -> io::Result<()>) -> io::Result<HeaderRead> {
    let mut reader = Reader { len, read_at, bytes: 0 };
    let head = reader.fetch(0, HEADER_PREFIX.min(len))?;
    let focus = if head.starts_with(&[0xff, 0xd8]) {
        jpeg(&mut reader, &head)?
    } else if matches!(Tiff::sniff(&head), Some((_, false))) {
        tiff(&mut reader, head)?
    } else {
        None
    };
    Ok(HeaderRead { focus, bytes_read: reader.bytes })
}

struct Reader<'a> {
    len: u64,
    read_at: &'a mut dyn FnMut(u64, &mut [u8]) -> io::Result<()>,
    bytes: u64,
}

impl Reader<'_> {
    /// `[start, end)` of the file (clipped to it), counted against [`MAX_HEADER_BYTES`].
    fn fetch(&mut self, start: u64, end: u64) -> io::Result<Vec<u8>> {
        let end = end.min(self.len);
        let n = end.saturating_sub(start);
        if self.bytes.saturating_add(n) > MAX_HEADER_BYTES {
            return Err(io::Error::other(OverBudget));
        }
        let mut buf = vec![0u8; usize::try_from(n).map_err(|_| io::Error::other("range too large"))?];
        if n > 0 {
            (self.read_at)(start, &mut buf)?;
        }
        self.bytes = self.bytes.saturating_add(n);
        Ok(buf)
    }
}

/// A file whose focus data would take more than [`MAX_HEADER_BYTES`] to read.
#[derive(Debug)]
struct OverBudget;

impl std::fmt::Display for OverBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("focus data larger than a header")
    }
}

impl std::error::Error for OverBudget {}

fn over_budget(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|i| i.is::<OverBudget>())
}

/// An over-budget read is "no focus point here", not a failure to report.
fn budget_is_none<T>(r: io::Result<Option<T>>) -> io::Result<Option<T>> {
    match r {
        Err(e) if over_budget(&e) => Ok(None),
        r => r,
    }
}

/// A JPEG's first `Exif` APP1 segment, found as [`crate::jpeg_segments`] finds it.
fn jpeg(r: &mut Reader, head: &[u8]) -> io::Result<Option<FocusPoint>> {
    budget_is_none(jpeg_inner(r, head))
}

fn jpeg_inner(r: &mut Reader, head: &[u8]) -> io::Result<Option<FocusPoint>> {
    let len = r.len;
    let at = |r: &mut Reader, start: u64, n: u64| -> io::Result<Vec<u8>> {
        let local = usize::try_from(start).ok().zip(usize::try_from(start.saturating_add(n)).ok()).and_then(|(a, b)| head.get(a..b));
        match local {
            Some(s) => Ok(s.to_vec()),
            None => r.fetch(start, start.saturating_add(n)),
        }
    };
    let mut i = 2u64;
    for _ in 0..MAX_SEGMENTS {
        if i.saturating_add(4) > len {
            break;
        }
        let h = at(r, i, 4)?;
        let (Some(&ff), Some(&marker), Some(&a), Some(&b)) = (h.first(), h.get(1), h.get(2), h.get(3)) else { break };
        if ff != 0xff {
            break;
        }
        if marker == 0xff {
            i += 1;
            continue;
        }
        if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        let seg = u64::from(u16::from_be_bytes([a, b]));
        if seg < 2 {
            break;
        }
        let end = i.saturating_add(2).saturating_add(seg).min(len);
        if marker == 0xe1 {
            let payload = at(r, i + 4, end.saturating_sub(i + 4))?;
            if payload.starts_with(b"Exif\0") {
                return Ok(crate::read_exif(payload.get(6..).unwrap_or(&[])).focus);
            }
        }
        i = end;
    }
    Ok(None)
}

/// A TIFF raw, decoded from a buffer of its length holding only the ranges [`plan`] asked for.
fn tiff(r: &mut Reader, head: Vec<u8>) -> io::Result<Option<FocusPoint>> {
    if r.len > MAX_SPARSE_FILE {
        return Ok(None);
    }
    let Ok(len) = usize::try_from(r.len) else { return Ok(None) };
    let mut buf = vec![0u8; len];
    let Some(dst) = buf.get_mut(..head.len()) else { return Ok(None) };
    dst.copy_from_slice(&head);
    let mut loaded = Loaded::default();
    loaded.insert(0, head.len() as u64);
    for _ in 0..MAX_ROUNDS {
        match plan(&buf, &loaded) {
            Step::Ready => {
                // The decoder's allocation budget follows the bytes actually fetched, not the logical
                // file length: only the loaded ranges are data, and an unrelated entry that claims a
                // huge value must be cut by what leaving it cost, not by what the buffer claims to hold.
                // The same 8 × / + 1 MiB shape the general TIFF reader uses, over `reader.bytes` instead.
                let opts = craft_tiff::ParseOptions { value_budget: Some(r.bytes.saturating_mul(8).saturating_add(1 << 20)), ..Default::default() };
                return Ok(crate::try_read_exif_with(&buf, &opts).ok().and_then(|m| m.focus));
            }
            Step::Nothing => return Ok(None),
            Step::Need(ranges) => {
                for (a, b) in ranges {
                    // widen to the read granularity, but never re-read what is loaded
                    let (wa, wb) = ((a / ALIGN) * ALIGN, b.div_ceil(ALIGN).saturating_mul(ALIGN).min(r.len));
                    for (s, e) in loaded.gaps(wa, wb) {
                        let bytes = match r.fetch(s, e) {
                            Err(e) if over_budget(&e) => return Ok(None),
                            other => other?,
                        };
                        let (Ok(su), Ok(eu)) = (usize::try_from(s), usize::try_from(e)) else { return Ok(None) };
                        let Some(dst) = buf.get_mut(su..eu) else { return Ok(None) };
                        dst.copy_from_slice(&bytes);
                        loaded.insert(s, e);
                    }
                }
            }
        }
    }
    Ok(None)
}

/// Sorted, disjoint, non-touching byte ranges.
#[derive(Default)]
struct Loaded(Vec<(u64, u64)>);

impl Loaded {
    fn insert(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let (mut s, mut e) = (start, end);
        let mut out = Vec::with_capacity(self.0.len() + 1);
        for &(a, b) in &self.0 {
            if b < s || a > e {
                out.push((a, b));
            } else {
                s = s.min(a);
                e = e.max(b);
            }
        }
        out.push((s, e));
        out.sort_unstable();
        self.0 = out;
    }

    fn contains(&self, start: u64, end: u64) -> bool {
        start >= end || self.0.iter().any(|&(a, b)| a <= start && end <= b)
    }

    /// The parts of `[start, end)` not loaded.
    fn gaps(&self, start: u64, end: u64) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut at = start;
        for &(a, b) in &self.0 {
            if b <= at || a >= end {
                continue;
            }
            if a > at {
                out.push((at, a.min(end)));
            }
            at = at.max(b);
            if at >= end {
                break;
            }
        }
        if at < end {
            out.push((at, end));
        }
        out
    }
}

enum Step {
    /// Load these and ask again.
    Need(Vec<(u64, u64)>),
    /// Everything the decoder reads is loaded.
    Ready,
    /// The file has nothing the decoder could read (no Exif, no maker note, a structure it rejects).
    Nothing,
}

/// One IFD entry as the TIFF reader sees it: where its value is (absolute), if it fits the file.
struct RawEntry {
    tag: u16,
    typ: FieldType,
    start: u64,
    end: u64,
}

struct Want<'a> {
    buf: &'a [u8],
    order: ByteOrder,
    loaded: &'a Loaded,
    need: Vec<(u64, u64)>,
}

impl Want<'_> {
    fn len(&self) -> u64 {
        self.buf.len() as u64
    }

    /// Whether `[start, end)` (clipped to the file) is loaded; asks for it if not.
    fn want(&mut self, start: u64, end: u64) -> bool {
        let end = end.min(self.len());
        if self.loaded.contains(start, end) {
            return true;
        }
        self.need.push((start, end));
        false
    }

    /// The entries of the IFD at `off` (values relative to 0), once its table is loaded; `Some(empty)`
    /// for one the reader would reject.
    fn table(&mut self, off: u64) -> Option<Vec<RawEntry>> {
        let len = self.len();
        if off == 0 || off >= len {
            return Some(Vec::new());
        }
        if !self.want(off, off.saturating_add(2)) {
            return None;
        }
        let pos = usize::try_from(off).ok()?;
        let n = u64::from(self.order.read_u16(self.buf, pos)?);
        let table = off + 2;
        let n_read = n.min(len.saturating_sub(table) / 12).min(MAX_ENTRIES);
        let end = table + n_read * 12 + if n_read == n { 4 } else { 0 };
        if !self.want(off, end) {
            return None;
        }
        let mut out = Vec::new();
        for i in 0..n_read {
            let Ok(p) = usize::try_from(table + i * 12) else { continue };
            let (Some(tag), Some(typ), Some(count)) = (
                self.order.read_u16(self.buf, p),
                self.order.read_u16(self.buf, p + 2).and_then(FieldType::from_u16),
                self.order.read_u32(self.buf, p + 4),
            ) else {
                continue;
            };
            let Some(total) = u64::from(count).checked_mul(typ.size() as u64) else { continue };
            let field = p as u64 + 8;
            let start = if total <= 4 {
                field
            } else {
                match self.order.read_u32(self.buf, p + 8) {
                    Some(v) => u64::from(v),
                    None => continue,
                }
            };
            let Some(end) = start.checked_add(total).filter(|&e| e <= len) else { continue };
            out.push(RawEntry { tag, typ, start, end });
        }
        Some(out)
    }

    /// The first unsigned value of an entry (loaded).
    fn first_value(&self, e: &RawEntry) -> Option<u64> {
        let pos = usize::try_from(e.start).ok()?;
        match e.typ.size() {
            1 => self.buf.get(pos).map(|&b| u64::from(b)),
            2 => self.order.read_u16(self.buf, pos).map(u64::from),
            4 => self.order.read_u32(self.buf, pos).map(u64::from),
            _ => self.order.read_u64(self.buf, pos),
        }
    }

    fn step(self) -> Step {
        if self.need.is_empty() { Step::Ready } else { Step::Need(self.need) }
    }
}

/// What of a classic TIFF the focus decoder still needs, given what is `loaded`: IFD0 (Make, Model,
/// the Exif pointer), the Exif IFD (the maker-note entry), the maker note's header and IFD as
/// `craft_tiff::makernote` locates it, and the focus tags' values.
fn plan(buf: &[u8], loaded: &Loaded) -> Step {
    let Some((order, false)) = Tiff::sniff(buf) else { return Step::Nothing };
    let mut w = Want { buf, order, loaded, need: Vec::new() };
    let Some(ifd0) = order.read_u32(buf, 4).map(u64::from) else { return Step::Nothing };
    let Some(e0) = w.table(ifd0) else { return w.step() };
    let mut exif = None;
    for e in &e0 {
        if matches!(e.tag, t::MAKE | t::MODEL | t::UNIQUE_CAMERA_MODEL | t::ORIENTATION) {
            w.want(e.start, e.end);
        }
        if e.tag == t::EXIF_IFD && exif.is_none() && w.want(e.start, e.end) {
            exif = w.first_value(e);
        }
    }
    if !w.need.is_empty() {
        return w.step();
    }
    let Some(exif) = exif.filter(|&o| o != 0) else { return Step::Nothing };
    let Some(ee) = w.table(exif) else { return w.step() };
    let Some(note) = ee.iter().find(|e| e.tag == t::MAKER_NOTE) else { return Step::Nothing };
    let head_end = note.start.saturating_add(12).min(note.end);
    if !w.want(note.start, head_end) {
        return w.step();
    }
    let head = usize::try_from(note.start).ok().zip(usize::try_from(head_end).ok()).and_then(|(a, b)| buf.get(a..b)).unwrap_or(&[]);
    let sony = head.starts_with(b"SONY") || head.starts_with(b"VHAB     \0");
    // another maker's header: `focus::read` only decodes Sony and header-less notes
    let foreign: [&[u8]; 13] = [
        b"Nikon\0",
        b"FUJIFILM",
        b"GENERALE",
        b"OLYMPUS\0",
        b"OM SYSTEM\0",
        b"OLYMP\0",
        b"EPSON\0",
        b"MINOL\0",
        b"CAMER\0",
        b"SANYO\0",
        b"Panasonic\0",
        b"AOC\0",
        b"PENTAX \0",
    ];
    if !sony && foreign.iter().any(|p| head.starts_with(p)) {
        return Step::Nothing;
    }
    let mut entries = None;
    if sony {
        match w.table(note.start.saturating_add(12)) {
            None => return w.step(),
            Some(e) if !e.is_empty() => entries = Some(e),
            Some(_) => {}
        }
    }
    let entries = match entries {
        Some(e) => e,
        None => match w.table(note.start) {
            Some(e) => e,
            None => return w.step(),
        },
    };
    for e in entries.iter().filter(|e| FOCUS_TAGS.contains(&e.tag)) {
        w.want(e.start, e.end);
    }
    w.step()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader over `file` that counts what it is asked for.
    fn read(file: &[u8]) -> (io::Result<HeaderRead>, u64) {
        let mut asked = 0u64;
        let r = read_header(file.len() as u64, &mut |off, buf: &mut [u8]| {
            asked += buf.len() as u64;
            let start = usize::try_from(off).map_err(io::Error::other)?;
            let src = file.get(start..start + buf.len()).ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
            buf.copy_from_slice(src);
            Ok(())
        });
        (r, asked)
    }

    #[test]
    fn loaded_ranges() {
        let mut l = Loaded::default();
        l.insert(10, 20);
        l.insert(30, 40);
        assert_eq!(l.gaps(0, 50), vec![(0, 10), (20, 30), (40, 50)]);
        l.insert(20, 30);
        assert_eq!(l.0, vec![(10, 40)]);
        assert!(l.contains(12, 38) && !l.contains(5, 12));
        assert!(l.gaps(15, 35).is_empty());
    }

    #[test]
    fn garbage_and_empty_files_are_no_point() {
        for file in [vec![], vec![0xff, 0xd8], b"II*\0\x08\0\0\0".to_vec(), b"MM\0*\xff\xff\xff\xff".to_vec(), vec![0x42; 100_000]] {
            let (r, _) = read(&file);
            assert_eq!(r.unwrap().focus, None);
        }
        // a JPEG of nothing but fill bytes and tiny segments stops at its bounds
        let mut j = vec![0xff, 0xd8];
        for _ in 0..5000 {
            j.extend_from_slice(&[0xff, 0xe2, 0, 2]);
        }
        assert_eq!(read(&j).0.unwrap().focus, None);
    }

    #[test]
    fn a_file_that_shrinks_mid_read_is_an_error() {
        // the reader is told the file is longer than it is (replaced between the stat and the read)
        let r = read_header(1 << 20, &mut |_, _| Err(io::Error::from(io::ErrorKind::UnexpectedEof)));
        assert!(r.is_err());
    }
}
