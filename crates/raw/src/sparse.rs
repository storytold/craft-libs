//! Describing a raw file without reading its pixel data.
//!
//! [`probe_info`](crate::probe_info) and a metadata reader read only a raw file's
//! headers, but those are a few hundred kilobytes of a file of tens of megabytes: an import from a
//! slow drive spends nearly all its time reading samples nothing looks at. [`header_plan`] says
//! which byte ranges of a file those readers need. It is asked about a buffer of the file's full
//! length in which only the ranges in `loaded` hold the file's bytes (the rest is zeros); the caller
//! loads the ranges it names and asks again. Once it says [`HeaderPlan::Ready`], probing the buffer
//! gives what probing the whole file gives.
//!
//! Only bulk sample data is left out: the strips and tiles of each large TIFF image (keeping the
//! start of the first one, where the coding headers a header probe validates sit), a CR3's image
//! tracks (keeping the raw track's coding headers and quantization maps, and the timed-metadata
//! track where Canon puts the as-shot white balance), a RAF's CFA strip (keeping its compression
//! header) and a Panasonic RW2's raw data. Formats whose header probe reads samples (NEF: masked
//! columns; ORF: bit depth and CFA phase; PEF: dark borders; ARW6, for its header plan only: the
//! start of every tile) and anything else get [`HeaderPlan::Whole`], as does a structure that is
//! incomplete, inconsistent or keeps metadata inside its sample data: the plan never guesses.

use std::collections::HashSet;

#[cfg(feature = "cr3")]
use craft_metadata::cr3::{Cr3TrackKind, XMP_UUID, is_cr3, parse_cr3};
use craft_tiff::{ByteOrder, FieldType, Tiff, tags as t};

use crate::RawFormat;

/// Bytes kept at the start of a skipped block: the coding headers a header probe validates there
/// (lossless-JPEG frame headers, a RAF's compression header and stripe table) fit with room to spare.
const HEAD: u64 = 64 * 1024;
/// An image whose chunks hold fewer bytes than this is read (thumbnails, small previews).
const BULK: u64 = 256 * 1024;
/// The same bounds as `craft_tiff`'s reader.
const MAX_IFDS: usize = 1024;
const MAX_ENTRIES: u64 = 8192;
#[cfg(feature = "cr3")]
const MAX_BOXES: usize = 4096;
/// Child IFDs listed by one `SubIFDs` entry.
const MAX_CHILDREN: u64 = 4096;
/// Pending child-IFD offsets the walker may queue at once: no more than it can ever visit
/// (the same [`MAX_IFDS`] the seen set is held to, deduplicated as they are popped), so an alley
/// of hostile `SubIFDs` entries can't grow the queue into the tens of millions of pointers.
/// The plan never guesses: the walk refuses and the plan reports the whole file when it would go
/// past the cap.
const MAX_PENDING: usize = MAX_IFDS;
/// The RAF raw container's pointer to its raw IFD.
#[cfg(feature = "raf")]
const RAF_RAW_IFD: u16 = 0xf000;
/// Panasonic RW2 IFD0 `RawDataOffset` (`0x0118`): its raw data runs from there to the end of the file.
const RW2_RAW_DATA_OFFSET: u16 = 0x0118;

/// Byte ranges `[start, end)` of a file: sorted, disjoint and not touching.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranges(Vec<(u64, u64)>);

impl Ranges {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `[start, end)` (nothing when it is empty).
    pub fn insert(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let (mut start, mut end) = (start, end);
        let mut out = Vec::with_capacity(self.0.len() + 1);
        let mut placed = false;
        for &(a, b) in &self.0 {
            if b < start {
                out.push((a, b));
            } else if a > end {
                if !placed {
                    out.push((start, end));
                    placed = true;
                }
                out.push((a, b));
            } else {
                start = start.min(a);
                end = end.max(b);
            }
        }
        if !placed {
            out.push((start, end));
        }
        self.0 = out;
    }

    /// Add every `[start, end)` of `spans` in one sort and merge (empty ones add nothing).
    pub fn extend(&mut self, spans: impl IntoIterator<Item = (u64, u64)>) {
        let mut all: Vec<(u64, u64)> = std::mem::take(&mut self.0);
        all.extend(spans.into_iter().filter(|&(a, b)| a < b));
        all.sort_unstable();
        let mut out: Vec<(u64, u64)> = Vec::with_capacity(all.len());
        for (a, b) in all {
            match out.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => out.push((a, b)),
            }
        }
        self.0 = out;
    }

    /// Whether all of `[start, end)` is covered (an empty range always is).
    pub fn contains(&self, start: u64, end: u64) -> bool {
        start >= end || self.0.iter().any(|&(a, b)| a <= start && end <= b)
    }

    /// Whether some of `[start, end)` is covered.
    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        start < end && self.0.iter().any(|&(a, b)| a < end && start < b)
    }

    pub fn spans(&self) -> &[(u64, u64)] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Bytes covered.
    pub fn total(&self) -> u64 {
        self.0.iter().fold(0u64, |n, &(a, b)| n.saturating_add(b - a))
    }

    /// The parts of `[start, end)` not covered.
    pub fn gaps(&self, start: u64, end: u64) -> Ranges {
        // the gaps come in order, apart from each other: pushed, not merged in one by one
        let mut out = Vec::new();
        let mut at = start;
        for &(a, b) in &self.0 {
            if b <= at {
                continue;
            }
            if a >= end {
                break;
            }
            if a > at {
                out.push((at, a));
            }
            at = at.max(b);
        }
        if at < end {
            out.push((at, end));
        }
        Ranges(out)
    }

    /// These ranges without the ones in `other`.
    pub fn minus(&self, other: &Ranges) -> Ranges {
        let mut out = Ranges::new();
        out.extend(self.0.iter().flat_map(|&(a, b)| other.gaps(a, b).0));
        out
    }
}

/// What [`header_plan`] says about a partly loaded file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderPlan {
    /// Load these ranges (none of them loaded yet) and ask again.
    Need(Ranges),
    /// Everything the header readers look at is loaded: probing the buffer gives the whole file's
    /// result.
    Ready,
    /// The file can't be described from part of it (why): read it whole.
    Whole(&'static str),
}

/// What of the raw file in `bytes` a header probe reads. `bytes` has the file's full length;
/// only the ranges in `loaded` hold its bytes (see the module docs).
pub fn header_plan(bytes: &[u8], loaded: &Ranges) -> HeaderPlan {
    plan(bytes, loaded, false)
}

/// [`header_plan`] for a raw shown from its embedded preview (a coding not decoded yet): also what
/// [`crate::embedded_preview`] reads — a CR3's JPEG track and preview boxes, the strips of a TIFF
/// raw's preview images.
pub fn preview_plan(bytes: &[u8], loaded: &Ranges) -> HeaderPlan {
    plan(bytes, loaded, true)
}

fn plan(bytes: &[u8], loaded: &Ranges, preview: bool) -> HeaderPlan {
    let mut w = Want::new(bytes, loaded);
    if !w.want(0, 16) {
        return w.into_plan().unwrap_or(HeaderPlan::Whole("file too short"));
    }
    if crate::probe(bytes) == Some(RawFormat::Cr3) {
        #[cfg(feature = "cr3")]
        return if is_cr3(bytes) { cr3_plan(bytes, loaded, preview) } else { HeaderPlan::Whole("CR3 structure") };
        #[cfg(not(feature = "cr3"))]
        return HeaderPlan::Whole("CR3 support not built");
    }
    if bytes.starts_with(b"FUJIFILMCCD-RAW") {
        // (its preview, the embedded JPEG, is read anyway: its metadata are there)
        #[cfg(feature = "raf")]
        return raf_plan(bytes, loaded);
        #[cfg(not(feature = "raf"))]
        return HeaderPlan::Whole("RAF support not built");
    }
    if bytes.get(..4).is_some_and(|m| matches!(m, b"IIRO" | b"IIRS" | b"MMOR")) {
        HeaderPlan::Whole("an ORF header probe reads samples")
    } else if Tiff::sniff(bytes).is_some() {
        tiff_plan(bytes, loaded, preview)
    } else {
        HeaderPlan::Whole("not a raw layout read in part")
    }
}

/// Collects the ranges a plan still needs.
struct Want<'a> {
    len: u64,
    loaded: &'a Ranges,
    /// What is missing, as found; merged once into the plan (merging each find into the ranges so
    /// far was a copy of them per find: quadratic over a file of many out-of-line values).
    need: Vec<(u64, u64)>,
    /// `need` is merged when it reaches this long, so repeated finds of one gap stay bounded by
    /// the distinct gaps (merging when it doubles keeps the cost linear).
    compact_at: usize,
}

impl<'a> Want<'a> {
    fn new(bytes: &[u8], loaded: &'a Ranges) -> Self {
        Self { len: bytes.len() as u64, loaded, need: Vec::new(), compact_at: 64 }
    }

    /// Whether `[start, end)` (clipped to the file) is loaded; asks for what isn't.
    fn want(&mut self, start: u64, end: u64) -> bool {
        let end = end.min(self.len);
        if self.loaded.contains(start, end) {
            return true;
        }
        self.need.extend(self.loaded.gaps(start, end).0);
        if self.need.len() >= self.compact_at {
            let mut merged = Ranges::new();
            merged.extend(std::mem::take(&mut self.need));
            self.need = merged.spans().to_vec();
            self.compact_at = (self.need.len() * 2).max(64);
        }
        false
    }

    /// `Need` when something is missing.
    fn into_plan(self) -> Option<HeaderPlan> {
        let mut need = Ranges::new();
        need.extend(self.need);
        (!need.is_empty()).then_some(HeaderPlan::Need(need))
    }
}

/// Everything but `holes` is needed. `meta` (IFD tables and out-of-line values) must not lie in a
/// hole; once all is loaded, `ready` decides.
fn finish(len: u64, loaded: &Ranges, holes: &Ranges, meta: &Ranges, ready: impl FnOnce() -> HeaderPlan) -> HeaderPlan {
    if holes.spans().iter().any(|&(a, b)| meta.overlaps(a, b)) {
        return HeaderPlan::Whole("metadata inside sample data");
    }
    let missing = holes.gaps(0, len).minus(loaded);
    if missing.is_empty() { ready() } else { HeaderPlan::Need(missing) }
}

fn tiff_plan(bytes: &[u8], loaded: &Ranges, preview: bool) -> HeaderPlan {
    let len = bytes.len() as u64;
    let mut w = Want::new(bytes, loaded);
    let meta = match walk_tiff(&mut w, 0, bytes, &[]) {
        Ok(meta) => meta,
        Err(why) => return HeaderPlan::Whole(why),
    };
    if let Some(plan) = w.into_plan() {
        return plan;
    }
    // (the Make is usually in the first kilobytes: a NEF, PEF or ORF is known before the rest is read)
    if matches!(crate::probe(bytes), Some(RawFormat::Nef | RawFormat::Nrw | RawFormat::Orf | RawFormat::Pef | RawFormat::Erf | RawFormat::Mef)) {
        return HeaderPlan::Whole("its header probe reads samples");
    }
    let Ok(tiff) = Tiff::parse(bytes) else { return HeaderPlan::Whole("TIFF structure") };
    let mut holes = Ranges::new();
    let mut spans = Vec::new();
    for ifd in tiff.all_ifds() {
        let chunks: Vec<(u64, u64)> = match ifd.image() {
            // a preview's strips are read when the preview is wanted: only the raw image is skipped
            Ok(info) if preview && !matches!(info.photometric, t::photometric::CFA | t::photometric::LINEAR_RAW) => continue,
            // (Sony Compressed RAW 2: the header probe decodes the start of every tile; a preview reads none)
            Ok(info) if !preview && info.compression == 32766 => return HeaderPlan::Whole("an ARW6 header probe reads its tiles"),
            Ok(info) => info.chunks(len).iter().map(|c| (c.offset, c.len)).collect(),
            // a raw IFD without the tags a TIFF image needs (an older CR2's) still lists its strips
            Err(_) => match (ifd.u64s(t::STRIP_OFFSETS), ifd.u64s(t::STRIP_BYTE_COUNTS)) {
                (Some(offsets), Some(counts)) if offsets.len() == counts.len() => offsets.into_iter().zip(counts).collect(),
                _ => continue,
            },
        };
        let total = chunks.iter().fold(0u64, |n, c| n.saturating_add(c.1));
        if total < BULK {
            continue;
        }
        spans.extend(chunks.iter().enumerate().map(|(i, &(offset, size))| {
            let start = if i == 0 { offset.saturating_add(HEAD) } else { offset };
            (start, offset.saturating_add(size).min(len))
        }));
    }
    holes.extend(spans);
    // A Panasonic RW2 keeps its raw data from `RawDataOffset` to the end, outside any TIFF image.
    if holes.is_empty()
        && bytes.starts_with(b"IIU\0")
        && let Some(off) = tiff.ifds.first().and_then(|i| i.u64(RW2_RAW_DATA_OFFSET))
    {
        holes.insert(off.saturating_add(HEAD), len);
    }
    finish(len, loaded, &holes, &meta, || match crate::probe(bytes) {
        Some(RawFormat::Nef | RawFormat::Nrw | RawFormat::Orf | RawFormat::Pef | RawFormat::Erf | RawFormat::Mef) => {
            HeaderPlan::Whole("its header probe reads samples")
        }
        Some(f) if f.is_supported() => HeaderPlan::Ready,
        _ => HeaderPlan::Whole("not a supported raw"),
    })
}

/// Walk the IFDs of the TIFF in `data` (which starts at `origin` in the file) as `craft_tiff`'s
/// reader does — the chain, `SubIFDs`, Exif, GPS and Interop, plus the `extra` pointer tags —
/// asking for every IFD table and for the values that locate IFDs and image chunks. Returns the IFD
/// tables and out-of-line values found (absolute ranges); incomplete while `w` needs more.
fn walk_tiff(w: &mut Want, origin: u64, data: &[u8], extra: &[u16]) -> Result<Ranges, &'static str> {
    let len = data.len() as u64;
    let at = |rel: u64| origin.saturating_add(rel);
    let mut meta = Ranges::new();
    if !w.want(origin, at(len.min(16))) {
        return Ok(meta);
    }
    let (order, big) = Tiff::sniff(data).ok_or("not a TIFF")?;
    let first = if big { order.read_u64(data, 8) } else { order.read_u32(data, 4).map(u64::from) }.ok_or("truncated TIFF header")?;
    meta.insert(origin, at(len.min(if big { 16 } else { 8 })));
    let (head, entry, inline) = if big { (8u64, 20u64, 8u64) } else { (2, 12, 4) };
    let offset_at = |pos: u64| -> Option<u64> {
        let pos = usize::try_from(pos).ok()?;
        if big { order.read_u64(data, pos) } else { order.read_u32(data, pos).map(u64::from) }
    };
    let mut stack = vec![first];
    let mut seen = HashSet::new();
    // Every offset ever queued: the pending cap counts distinct IFDs, so a thousand references to
    // one child cost one visit, not a thousand.
    let mut queued = HashSet::from([first]);
    // Every IFD table and out-of-line value, collected and merged once at the end: an insert
    // per entry would scan the ranges of every entry before it — quadratic in the metadata
    // this walker collects.
    let mut spans: Vec<(u64, u64)> = Vec::new();
    while let Some(off) = stack.pop() {
        if off == 0 || off >= len || seen.len() >= MAX_IFDS || !seen.insert(off) {
            continue;
        }
        if !w.want(at(off), at(off.saturating_add(head))) {
            continue;
        }
        let Ok(pos) = usize::try_from(off) else { continue };
        let n = if big { order.read_u64(data, pos) } else { order.read_u16(data, pos).map(u64::from) }.unwrap_or(0);
        let table = off.saturating_add(head);
        let n_read = n.min(len.saturating_sub(table) / entry).min(MAX_ENTRIES);
        if n_read == 0 {
            continue;
        }
        let table_end = table.saturating_add(n_read * entry);
        // the reader reads the next-IFD pointer only when every entry fits
        let complete = n_read == n;
        let end = if complete { table_end.saturating_add(inline).min(len) } else { table_end };
        if !w.want(at(off), at(end)) {
            continue;
        }
        spans.push((at(off), at(end)));
        for i in 0..n_read {
            let p = table + i * entry;
            let Ok(pu) = usize::try_from(p) else { continue };
            let (Some(tag), Some(typ)) = (order.read_u16(data, pu), order.read_u16(data, pu + 2).and_then(FieldType::from_u16)) else { continue };
            let count = if big { order.read_u64(data, pu + 4) } else { order.read_u32(data, pu + 4).map(u64::from) };
            let Some(count) = count else { continue };
            let field = p + if big { 12 } else { 8 };
            let Some(total) = count.checked_mul(typ.size() as u64) else { continue };
            let start = if total <= inline {
                field
            } else {
                match offset_at(field) {
                    Some(v) => v,
                    None => continue,
                }
            };
            let Some(value_end) = start.checked_add(total).filter(|&e| e <= len) else { continue };
            if total > inline {
                spans.push((at(start), at(value_end)));
            }
            let child = matches!(tag, t::SUB_IFDS | t::EXIF_IFD | t::GPS_IFD | t::INTEROP_IFD) || extra.contains(&tag);
            let locates = child || matches!(tag, t::STRIP_OFFSETS | t::STRIP_BYTE_COUNTS | t::TILE_OFFSETS | t::TILE_BYTE_COUNTS);
            if !locates || !w.want(at(start), at(value_end)) || !child {
                continue;
            }
            // the reader follows every `SubIFDs` value and the first value of the other pointers
            let take = if tag == t::SUB_IFDS { count.min(MAX_CHILDREN) } else { count.min(1) };
            for k in 0..take {
                if let Some(v) = unsigned(data, order, typ, start + k * typ.size() as u64) {
                    if v == 0 || v >= len || !queued.insert(v) {
                        continue;
                    }
                    if stack.len() >= MAX_PENDING {
                        return Err("too many child IFDs queued to visit");
                    }
                    stack.push(v);
                }
            }
        }
        if complete && let Some(next) = offset_at(table_end).filter(|&next| next != 0 && next < len && queued.insert(next)) {
            if stack.len() >= MAX_PENDING {
                return Err("too many child IFDs queued to visit");
            }
            stack.push(next);
        }
    }
    meta.extend(spans);
    Ok(meta)
}

/// One unsigned value of type `typ` at `pos`.
fn unsigned(data: &[u8], order: ByteOrder, typ: FieldType, pos: u64) -> Option<u64> {
    let pos = usize::try_from(pos).ok()?;
    match typ.size() {
        1 => data.get(pos).map(|&b| u64::from(b)),
        2 => order.read_u16(data, pos).map(u64::from),
        4 => order.read_u32(data, pos).map(u64::from),
        _ => order.read_u64(data, pos),
    }
}

#[cfg(any(feature = "cr3", feature = "raf"))]
fn be32(bytes: &[u8], at: u64) -> Option<u64> {
    let at = usize::try_from(at).ok()?;
    bytes.get(at..at.checked_add(4)?).map(|s| u64::from(u32::from_be_bytes([s[0], s[1], s[2], s[3]])))
}

#[cfg(feature = "cr3")]
fn be64(bytes: &[u8], at: u64) -> Option<u64> {
    let at = usize::try_from(at).ok()?;
    bytes.get(at..at.checked_add(8)?).map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

#[cfg(feature = "cr3")]
/// A CR3 is read where `craft_metadata::cr3::parse_cr3` and the CR3 decoder look: the file type,
/// `moov` and the XMP `uuid` box, the first sample of each timed-metadata (`CTMD`) track, and the
/// main raw track's coding headers and quantization maps. Previews and image tracks are skipped,
/// unless `preview`: then every top-level box but `mdat` and the JPEG tracks are read too.
fn cr3_plan(bytes: &[u8], loaded: &Ranges, preview: bool) -> HeaderPlan {
    let len = bytes.len() as u64;
    let mut w = Want::new(bytes, loaded);
    // top-level boxes, walked as `parse_cr3` walks them (it stops at the first malformed header)
    let mut at = 0u64;
    let mut boxes = 0;
    while at.saturating_add(8) <= len && boxes < MAX_BOXES {
        boxes += 1;
        // size, type, a 64-bit size and a uuid
        if !w.want(at, at.saturating_add(32)) {
            break;
        }
        let Some(size) = be32(bytes, at) else { break };
        let (box_len, hdr) = match size {
            0 => (len - at, 8),
            1 => match be64(bytes, at + 8) {
                Some(l) => (l, 16),
                None => break,
            },
            n => (n, 8),
        };
        let Some(end) = at.checked_add(box_len).filter(|&e| e <= len && box_len >= hdr) else { break };
        let kind = bytes.get(at as usize + 4..at as usize + 8);
        let needed = match kind {
            Some(b"mdat") => false,
            Some(b"moov") => true,
            Some(b"uuid") => preview || bytes.get((at + hdr) as usize..(at + hdr + 16) as usize) == Some(XMP_UUID.as_slice()),
            _ => preview,
        };
        if needed {
            w.want(at, end);
        }
        at = end;
    }
    if let Some(plan) = w.into_plan() {
        return plan;
    }
    let mut w = Want::new(bytes, loaded);
    let Some(cr3) = parse_cr3(bytes) else { return HeaderPlan::Whole("CR3 structure") };
    for track in &cr3.tracks {
        if (track.kind == Cr3TrackKind::Other(*b"CTMD") || preview && track.kind == Cr3TrackKind::Jpeg)
            && let Some((offset, size)) = track.data
        {
            w.want(offset as u64, offset.saturating_add(size) as u64);
        }
    }
    if let Some(track) = crate::cr3::main_track(&cr3.tracks)
        && let Some((offset, size)) = track.data
        && let Some(coding) = track.compression(bytes)
    {
        let o = offset as u64;
        if w.want(o, o.saturating_add(u64::from(coding.header_size)))
            && let Some(sample) = bytes.get(offset..offset.saturating_add(size))
            && let Ok(ranges) = crate::crx::header_ranges(sample, &coding)
        {
            for r in ranges {
                w.want(o.saturating_add(r.start as u64), o.saturating_add(r.end as u64));
            }
        }
    }
    w.into_plan().unwrap_or(HeaderPlan::Ready)
}

#[cfg(feature = "raf")]
/// A RAF is read whole but for its CFA strip: the header, the embedded JPEG (its metadata), the
/// record directory and the raw container's IFDs and compression header.
fn raf_plan(bytes: &[u8], loaded: &Ranges) -> HeaderPlan {
    let len = bytes.len() as u64;
    let mut w = Want::new(bytes, loaded);
    if !w.want(0, 160) {
        return w.into_plan().unwrap_or(HeaderPlan::Whole("RAF header"));
    }
    let (Some(start), Some(size)) = (be32(bytes, 100), be32(bytes, 104)) else { return HeaderPlan::Whole("RAF header") };
    let Some(end) = start.checked_add(size).filter(|&e| start > 0 && size > 0 && e <= len) else { return HeaderPlan::Whole("RAF raw container") };
    let Some(container) = bytes.get(start as usize..end as usize) else { return HeaderPlan::Whole("RAF raw container") };
    let meta = match walk_tiff(&mut w, start, container, &[RAF_RAW_IFD]) {
        Ok(meta) => meta,
        Err(why) => return HeaderPlan::Whole(why),
    };
    if let Some(plan) = w.into_plan() {
        return plan;
    }
    let Ok(ifd) = crate::raf::raw_ifd(container) else { return HeaderPlan::Whole("RAF raw IFD") };
    let (Some(off), Some(strip)) = (ifd.u64(0xf007), ifd.u64(0xf008)) else { return HeaderPlan::Whole("RAF raw strip") };
    let mut holes = Ranges::new();
    if strip >= BULK {
        let s = start.saturating_add(off);
        holes.insert(s.saturating_add(HEAD), s.saturating_add(strip).min(end));
    }
    finish(len, loaded, &holes, &meta, || HeaderPlan::Ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a plan to its end, loading from `file` what it asks for; the buffer's unloaded bytes are `fill`.
    pub(crate) fn run(file: &[u8], prefix: usize, fill: u8) -> (HeaderPlan, Vec<u8>, Ranges) {
        let mut buf = vec![fill; file.len()];
        let mut loaded = Ranges::new();
        let load = |buf: &mut Vec<u8>, loaded: &mut Ranges, a: u64, b: u64| {
            buf[a as usize..b as usize].copy_from_slice(&file[a as usize..b as usize]);
            loaded.insert(a, b);
        };
        load(&mut buf, &mut loaded, 0, prefix.min(file.len()) as u64);
        for _ in 0..32 {
            match header_plan(&buf, &loaded) {
                HeaderPlan::Need(r) => {
                    for &(a, b) in r.spans() {
                        assert!(!loaded.overlaps(a, b), "asked again for {a}..{b}");
                        load(&mut buf, &mut loaded, a, b);
                    }
                }
                done => return (done, buf, loaded),
            }
        }
        panic!("no answer after 32 rounds");
    }

    #[test]
    fn ranges_merge_and_subtract() {
        let mut r = Ranges::new();
        r.insert(10, 20);
        r.insert(30, 40);
        r.insert(20, 25); // touching: merged
        r.insert(5, 5); // empty
        assert_eq!(r.spans(), &[(10, 25), (30, 40)]);
        r.insert(0, 100);
        assert_eq!(r.spans(), &[(0, 100)]);
        let mut r = Ranges::new();
        r.insert(10, 20);
        r.insert(30, 40);
        assert!(r.contains(12, 18) && r.contains(5, 5) && !r.contains(15, 35));
        assert!(r.overlaps(15, 35) && !r.overlaps(20, 30));
        assert_eq!(r.gaps(0, 50).spans(), &[(0, 10), (20, 30), (40, 50)]);
        assert_eq!(r.gaps(12, 18).spans(), &[] as &[(u64, u64)]);
        let mut cut = Ranges::new();
        cut.insert(15, 32);
        assert_eq!(r.minus(&cut).spans(), &[(10, 15), (32, 40)]);
        assert_eq!(r.total(), 20);
    }

    #[test]
    fn a_plan_of_many_missing_values_is_gathered_in_one_merge() {
        // 200 000 loaded spans with a byte missing between each: every gap wanted once
        let mut loaded = Ranges::new();
        loaded.extend((0..20_000u64).map(|i| (i * 4, i * 4 + 3)));
        let bytes = vec![0u8; 80_000];
        let mut w = Want::new(&bytes, &loaded);
        for i in 0..20_000u64 {
            assert!(!w.want(i * 4 + 2, i * 4 + 4));
        }
        let Some(HeaderPlan::Need(need)) = w.into_plan() else { panic!("a plan") };
        assert_eq!(need.spans().len(), 20_000);
        assert_eq!(need.total(), 20_000);
        assert_eq!(loaded.gaps(0, 12).spans(), &[(3, 4), (7, 8), (11, 12)]);
    }

    #[test]
    fn repeated_wants_of_one_gap_keep_the_buffer_bounded() {
        let loaded = Ranges::new();
        let bytes = vec![0u8; 1000];
        let mut w = Want::new(&bytes, &loaded);
        for _ in 0..100_000 {
            assert!(!w.want(10, 20));
            assert!(w.need.len() <= 64);
        }
        let Some(HeaderPlan::Need(need)) = w.into_plan() else { panic!("a plan") };
        assert_eq!(need.spans(), &[(10, 20)]);
    }

    #[test]
    fn hostile_input_never_panics_and_ends() {
        let mut loaded = Ranges::new();
        for len in [0usize, 1, 8, 15, 16, 200] {
            let bytes = vec![0u8; len];
            loaded.insert(0, len as u64);
            assert!(matches!(header_plan(&bytes, &loaded), HeaderPlan::Whole(_)));
        }
        // an IFD that points at itself, an IFD count past the end, a huge SubIFDs count
        let mut tiff = b"II*\0\x08\0\0\0".to_vec();
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&[0x4a, 0x01, 4, 0, 0xff, 0xff, 0xff, 0x0f, 8, 0, 0, 0]); // SubIFDs, 0x0fffffff values at 8
        tiff.extend_from_slice(&[0x69, 0x87, 4, 0, 1, 0, 0, 0, 8, 0, 0, 0]); // Exif → itself
        tiff.extend_from_slice(&8u32.to_le_bytes()); // next → itself
        let (plan, _, _) = run(&tiff, 4, 0);
        assert!(matches!(plan, HeaderPlan::Whole(_)), "{plan:?}");
        // a CR3 whose `moov` claims more than the file holds: walked as the parser walks it (it
        // stops there), so the plan ends with the little there is, and the probe then fails as
        // it does on the whole file
        #[cfg(feature = "cr3")]
        {
            let mut cr3 = 24u32.to_be_bytes().to_vec();
            cr3.extend_from_slice(b"ftypcrx \0\0\0\x01crx isom");
            cr3.extend_from_slice(&u32::MAX.to_be_bytes());
            cr3.extend_from_slice(b"moov");
            cr3.resize(4096, 0);
            let (plan, buf, loaded) = run(&cr3, 16, 0);
            assert_eq!(plan, HeaderPlan::Ready);
            assert_eq!(loaded.total(), 24 + 32, "only the two box headers are read");
            assert_eq!(format!("{:?}", crate::probe_info(&buf)), format!("{:?}", crate::probe_info(&cr3)));
        }
        // a RAF whose container lies outside the file
        #[cfg(feature = "raf")]
        {
            let mut raf = b"FUJIFILMCCD-RAW 0201FF383501".to_vec();
            raf.resize(160, 0);
            raf[100..104].copy_from_slice(&1000u32.to_be_bytes());
            raf[104..108].copy_from_slice(&1000u32.to_be_bytes());
            assert_eq!(run(&raf, 160, 0).0, HeaderPlan::Whole("RAF raw container"));
        }
    }

    /// An ARW6 header probe reads every tile's headers and the first lines of its data, which lie in the strip a
    /// plan would skip: here the second tile, past a first one of bulk size.
    #[cfg(feature = "arw")]
    #[test]
    fn arw6_is_described_from_the_whole_file() {
        use crate::arw6_testenc::{Qis, arw6_file, one_tile_quantised};
        let (tile, _) = one_tile_quantised(64, 48, 0, Qis::REAL);
        let mut first = tile.clone();
        first.resize(BULK as usize, 0);
        let file = arw6_file(&[(0, 0, 64, 48, first), (64, 0, 64, 48, tile)], 128, 48);
        assert!(crate::probe_info(&file).is_ok());
        assert_eq!(run(&file, 16, 0).0, HeaderPlan::Whole("an ARW6 header probe reads its tiles"));
    }

    /// An IFD whose `SubIFDs` entries would queue 33 million child pointers (8192 entries of
    /// 4096 values apiece, all pointing at the same array) is refused at the walker's pending
    /// cap instead of growing the queue — and the plan reports the whole file, never guessing.
    #[test]
    fn a_subifds_alley_is_refused_at_the_pending_cap() {
        // First read everything, so the walk reaches the directory table itself.
        let n = 8192u32;
        let len = 8 + 2 + (n * 12 + 4 + 4 * 4096) as usize;
        let mut tiff = b"II*\0\x08\0\0\0".to_vec();
        tiff.extend_from_slice(&(n as u16).to_le_bytes());
        let mut entry = |tag: u16, typ: u16, count: u32, value: u32| {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&typ.to_le_bytes());
            tiff.extend_from_slice(&count.to_le_bytes());
            tiff.extend_from_slice(&value.to_le_bytes());
        };
        // header (8) + count field (2) + the {n} numbers and pointer (4), then the value array
        let values_at = (8 + 2u64 + u64::from(n) * 12 + 4) as u32;
        for _ in 0..n {
            entry(t::SUB_IFDS, 4, MAX_CHILDREN as u32, values_at);
        }
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend((1..=4096u32).flat_map(u32::to_le_bytes));
        let mut loaded = Ranges::new();
        loaded.insert(0, len as u64);
        let mut w = Want::new(&tiff, &loaded);
        assert_eq!(walk_tiff(&mut w, 0, &tiff, &[]), Err("too many child IFDs queued to visit"));
    }

    /// A TIFF with many out-of-line values across 512 directories is planned in one sort and
    /// merge, not one pass over all accumulated ranges per value (CodeRabbit storytold/lightcraft#52: before, each
    /// `Ranges::insert` copied every range found before it — the ~million disjoint values of the
    /// fixture below ask for more copying than the timeout allows).
    #[test]
    fn many_out_of_line_values_plan_fast() {
        let (ifds, values) = (512usize, 2048usize);
        let ifd_len = 2 + values * 12 + 4;
        let values_at = 8 + ifds * ifd_len;
        let len = values_at + values * 8;
        let mut tiff = vec![0u8; len];
        tiff[..8].copy_from_slice(b"II*\0\x08\0\0\0");
        for i in 0..ifds {
            let at = 8 + i * ifd_len;
            // {values} out-of-line strings (count 8 > the 4 inline bytes), then the next-IFD
            // pointer; every value is a disjoint 8-byte range for the merge to handle.
            tiff[at..at + 2].copy_from_slice(&(values as u16).to_le_bytes());
            for k in 0..values {
                let e = at + 2 + k * 12;
                tiff[e..e + 2].copy_from_slice(&t::MAKE.to_le_bytes());
                tiff[e + 2..e + 4].copy_from_slice(&2u16.to_le_bytes()); // ASCII: 8 bytes > inline — a span per entry
                tiff[e + 4..e + 8].copy_from_slice(&8u32.to_le_bytes());
                tiff[e + 8..e + 12].copy_from_slice(&(values_at as u32 + (k * 8) as u32).to_le_bytes());
            }
            let next = if i == ifds - 1 { 0u32 } else { (at + ifd_len) as u32 };
            tiff[at + ifd_len - 4..at + ifd_len].copy_from_slice(&next.to_le_bytes());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut loaded = Ranges::new();
            loaded.insert(0, len as u64);
            let _ = tx.send(header_plan(&tiff, &loaded));
        });
        rx.recv_timeout(std::time::Duration::from_secs(20)).expect("header_plan took longer than 20 s");
    }

    /// A TIFF with a million one-byte strips listed in descending order and all byte counts 0:
    /// the plan is built in one sort and merge, not one pass over all strips (or all holes) per
    /// strip.
    #[test]
    fn many_strips_without_byte_counts_plan_fast() {
        let n = 1_000_000u32;
        let (offsets_at, counts_at) = (8 + 2 + 5 * 12 + 4, 8 + 2 + 5 * 12 + 4 + 4 * n);
        let data = counts_at + 4 * n;
        let mut tiff = b"II*\0\x08\0\0\0".to_vec();
        tiff.extend_from_slice(&5u16.to_le_bytes());
        let entry = |tiff: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: u32| {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&typ.to_le_bytes());
            tiff.extend_from_slice(&count.to_le_bytes());
            tiff.extend_from_slice(&value.to_le_bytes());
        };
        entry(&mut tiff, 256, 4, 1, 1);
        entry(&mut tiff, 257, 4, 1, n);
        entry(&mut tiff, 273, 4, n, offsets_at);
        entry(&mut tiff, 278, 4, 1, 1);
        entry(&mut tiff, 279, 4, n, counts_at);
        tiff.extend_from_slice(&0u32.to_le_bytes());
        for i in (0..n).rev() {
            tiff.extend_from_slice(&(data + i).to_le_bytes());
        }
        tiff.resize(data as usize + n as usize, 0);
        let len = tiff.len() as u64;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut loaded = Ranges::new();
            loaded.insert(0, len);
            let _ = tx.send(header_plan(&tiff, &loaded));
        });
        let plan = rx.recv_timeout(std::time::Duration::from_secs(20)).expect("header_plan took longer than 20 s");
        assert!(!matches!(plan, HeaderPlan::Need(_)), "{plan:?}");
    }
}
