//! Embedded preview extraction: the largest baseline/progressive JPEG stored in a TIFF-based raw (IFD strips
//! with JPEG compression, `JPEGInterchangeFormat` pointers in any IFD, Nikon/others' maker-note preview IFDs),
//! or a DNG 1.7 JPEG XL preview IFD stored as a single tile/strip (a standalone `.jxl` file, as the spec
//! recommends for previews).

use craft_tiff::image::chunk_bytes;
use craft_tiff::tags as t;
use craft_tiff::{Ifd, Tiff, makernote};

/// Colour space of an embedded JPEG when the enclosing raw supplies it instead of the JPEG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewColorSpace {
    Srgb,
    AdobeRgb,
}

/// The colour space the raw says its embedded JPEG is in, for a JPEG that declares none itself (no ICC profile,
/// no EXIF), first available first: the Nikon maker-note ColorSpace (0x001e: 1 = sRGB, 2 = Adobe RGB, per Nikon
/// tag documentation); for a Canon CR2 or CR3, the Canon maker-note ColorSpace (0x00b4: 1 = sRGB, 2 = Adobe RGB);
/// the raw's own EXIF ColorSpace (0xA001: 1 = sRGB), or Uncalibrated (0xFFFF) with the Interoperability IFD's
/// InteropIndex "R03", the DCF option file for Adobe RGB (as a Sony body shot in Adobe RGB writes, storytold/lightcraft#778; per
/// the Exif and DCF specifications), not for a DNG. This is a fallback only: a JPEG's own ICC/EXIF colour
/// declaration takes precedence.
pub fn embedded_preview_color_space(bytes: &[u8]) -> Option<PreviewColorSpace> {
    let tiff = Tiff::parse(bytes).ok()?;
    let make = tiff.ifds.first()?.string(t::MAKE).unwrap_or_default();
    let exif = tiff.exif()?;
    if make.to_ascii_uppercase().starts_with("NIKON")
        && let Some(space) = nikon_color_space(bytes, &tiff, exif, &make)
    {
        return Some(space);
    }
    // a DNG's EXIF describes the original capture; its previews are rendered by the converter
    if tiff.ifds.first()?.contains(t::DNG_VERSION) {
        return None;
    }
    if make.to_ascii_uppercase().starts_with("CANON")
        && let Some(e) = exif.get(t::MAKER_NOTE)
        && let Some(mn) = makernote::parse_makernote(bytes, e.offset, e.count() as u64, tiff.order, &make)
        && let Some(space) = canon_color_space(&mn.ifd)
    {
        return Some(space);
    }
    match exif.u16(0xA001)? {
        1 => Some(PreviewColorSpace::Srgb),
        0xFFFF if exif.interop.as_deref()?.bytes(0x0001)?.starts_with(b"R03") => Some(PreviewColorSpace::AdobeRgb),
        _ => None,
    }
}

fn canon_color_space(note: &Ifd) -> Option<PreviewColorSpace> {
    match note.u16(0x00b4)? {
        1 => Some(PreviewColorSpace::Srgb),
        2 => Some(PreviewColorSpace::AdobeRgb),
        _ => None,
    }
}

fn nikon_color_space(bytes: &[u8], tiff: &Tiff, exif: &Ifd, make: &str) -> Option<PreviewColorSpace> {
    let e = exif.get(t::MAKER_NOTE)?;
    let mn = makernote::parse_makernote(bytes, e.offset, e.count() as u64, tiff.order, make)?;
    match mn.ifd.u16(0x001e)? {
        1 => Some(PreviewColorSpace::Srgb),
        2 => Some(PreviewColorSpace::AdobeRgb),
        _ => None,
    }
}

/// The first frame (SOF) marker of a JPEG and its position, from the marker segments before the first scan.
fn first_frame(b: &[u8]) -> Option<(u8, usize)> {
    if b.len() < 4 || !b.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut i = 2usize;
    while i.checked_add(4)? <= b.len() {
        if *b.get(i)? != 0xff {
            return None;
        }
        let m = *b.get(i + 1)?;
        if m == 0xff {
            i += 1;
            continue;
        }
        match m {
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xcb | 0xcd..=0xcf => return Some((m, i)),
            0xda | 0xd9 => return None,
            _ => {}
        }
        let len = usize::from(u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]));
        i = i.checked_add(2 + len)?;
    }
    None
}

/// Whether `b` looks like a displayable (DCT) JPEG: SOI, and the first SOF marker is not lossless.
fn is_dct_jpeg(b: &[u8]) -> bool {
    first_frame(b).is_some_and(|(m, _)| matches!(m, 0xc0..=0xc2))
}

/// Width, height and component count of a displayable (DCT) JPEG, from its frame header.
fn jpeg_frame(b: &[u8]) -> Option<(u32, u32, u8)> {
    let (_, i) = first_frame(b).filter(|(m, _)| matches!(m, 0xc0..=0xc2))?;
    let at = |k: usize| b.get(i.checked_add(k)?).copied();
    let be16 = |k: usize| Some(u32::from(u16::from_be_bytes([at(k)?, at(k + 1)?])));
    Some((be16(7)?, be16(5)?, at(9)?))
}
fn is_jxl(b: &[u8]) -> bool {
    b.starts_with(&[0xff, 0x0a]) || b.starts_with(&[0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a])
}

/// The bytes a file's own (offset, length) pair names, cut at the file's end; `None` when the offset
/// lies past it. Both numbers come from the file: never cast them (a `u64` offset past 4 GiB would
/// wrap to a small, wrong one on a 32-bit build) nor add them unchecked. A span the file's end cuts
/// short is kept only when it is a JPEG whose own end marker is inside it, after its scan (an
/// overstated length with the whole JPEG inside), cut at that marker: a truncated JPEG passes the
/// frame-header check, would win as the largest preview and then fail to decode (CodeRabbit on
/// storytold/lightcraft#3077). An `FF D9` inside an APP payload doesn't count, bytes that don't start a JPEG are refused
/// without a scan, and the walks share `budget` (bytes, as [`scan_with_budget`]'s do), so a file of
/// many such pointers stays linear.
fn file_span<'a>(data: &'a [u8], offset: u64, len: u64, budget: &mut usize) -> Option<&'a [u8]> {
    let start = usize::try_from(offset).ok()?;
    let wanted = usize::try_from(offset.saturating_add(len)).unwrap_or(usize::MAX);
    let span = data.get(start..wanted.min(data.len()))?;
    if wanted > data.len() {
        return span.get(..jpeg_end(span, budget)?);
    }
    Some(span)
}

/// Where the JPEG at the start of `s` ends (just past its end marker), walking its marker segments:
/// a segment's payload is skipped by its stated length (so an `FF D9` inside APP data isn't taken
/// for the end), entropy-coded bytes one at a time (stuffed `FF 00` and restart markers included).
/// `None` when `s` isn't a JPEG, ends first, reaches its end marker before a start of scan (no image
/// data), or the walk spends `budget` (one per step, shared by a file's walks). Each step moves
/// forward, so it ends within `s`.
fn jpeg_end(s: &[u8], budget: &mut usize) -> Option<usize> {
    if !s.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let (mut i, mut scanned) = (2usize, false);
    loop {
        *budget = budget.checked_sub(1)?;
        match (s.get(i), s.get(i.checked_add(1)?)) {
            (None, _) | (Some(&0xff), None) => return None,
            (Some(&0xff), Some(&m)) => match m {
                0xd9 => return if scanned { i.checked_add(2) } else { None },
                // fill byte
                0xff => i += 1,
                // stuffed zero, restart marker, TEM: no length follows
                0x00 | 0x01 | 0xd0..=0xd7 => i = i.checked_add(2)?,
                _ => {
                    let (&hi, &lo) = (s.get(i.checked_add(2)?)?, s.get(i.checked_add(3)?)?);
                    let len = usize::from(u16::from_be_bytes([hi, lo]));
                    if len < 2 {
                        return None;
                    }
                    scanned |= m == 0xda;
                    i = i.checked_add(2)?.checked_add(len)?;
                }
            },
            // entropy-coded data
            (Some(_), _) => i += 1,
        }
    }
}

fn candidates<'a>(data: &'a [u8], ifd: &Ifd, base: u64, out: &mut Vec<&'a [u8]>, budget: &mut usize) {
    // whole JPEG files stored as an undefined-type tag value (e.g. Panasonic `JpgFromRaw` 0x002e)
    for e in &ifd.entries {
        if matches!(e.value, craft_tiff::Value::Undefined(_))
            && e.count() > 1024
            && let Some(s) = file_span(data, e.offset, e.count() as u64, budget)
            && s.starts_with(&[0xff, 0xd8])
        {
            out.push(s);
        }
    }
    if let (Some(off), Some(len)) = (ifd.u64(t::JPEG_INTERCHANGE_FORMAT), ifd.u64(t::JPEG_INTERCHANGE_FORMAT_LENGTH)) {
        let off = off.saturating_add(base);
        if let Some(s) = file_span(data, off, len, budget) {
            out.push(s);
        }
    }
    if matches!(ifd.u16(t::COMPRESSION), Some(6) | Some(7) | Some(34892))
        && let Ok(info) = ifd.image()
        && let Some(chunk) = info.single_chunk(data.len() as u64)
        && let Some(s) = chunk_bytes(data, &chunk)
    {
        out.push(s);
    }
    // a rendered (RGB or grey, never CFA/LinearRaw/mask) JPEG XL preview in one chunk
    if ifd.u16(t::COMPRESSION) == Some(t::compression::JPEG_XL)
        && matches!(ifd.u16(t::PHOTOMETRIC), Some(t::photometric::BLACK_IS_ZERO | t::photometric::RGB))
        && let Ok(info) = ifd.image()
        && let Some(chunk) = info.single_chunk(data.len() as u64)
        && let Some(s) = chunk_bytes(data, &chunk)
        && is_jxl(s)
    {
        out.push(s);
    }
}

/// The largest embedded preview, if any: a JPEG, or (DNG 1.7) a JPEG XL file.
pub fn embedded_preview(bytes: &[u8]) -> Option<Vec<u8>> {
    preview_covering(bytes, None)
}

/// The smallest embedded JPEG whose long edge is at least `min_edge` and that looks like a copy of the largest preview
/// (its aspect ratio within 1 %, so not a letterboxed or cropped thumbnail, and as many colour components, so not a
/// grey mask or depth map), else [`embedded_preview`]. Only TIFF-based raws keep several: an A7R VI's 1616 × 1080
/// preview is 0.3 MB beside its 9984 × 6656 one (5.6 MB), so a stand-in up to 1616 px decodes about a twentieth of the data.
pub fn embedded_preview_covering(bytes: &[u8], min_edge: u32) -> Option<Vec<u8>> {
    preview_covering(bytes, Some(min_edge))
}

/// Of the DCT JPEGs in `found`, the one with the shortest long edge of at least `min_edge` with `largest`'s aspect
/// ratio (within 1 %) and component count.
fn smallest_covering<'a>(found: &[&'a [u8]], largest: &[u8], min_edge: u32) -> Option<&'a [u8]> {
    let (bw, bh, bc) = jpeg_frame(largest)?;
    let like = |(w, h, c): (u32, u32, u8)| {
        let (a, b) = (u64::from(w) * u64::from(bh), u64::from(h) * u64::from(bw));
        w > 0 && h > 0 && c == bc && a.abs_diff(b) * 100 <= a
    };
    // (one cut short before its end marker would fail to decode, and nothing would try another: CodeRabbit
    // on storytold/lightcraft#3583)
    let whole = |s: &[u8]| trim_eoi(s).ends_with(&[0xff, 0xd9]);
    found
        .iter()
        .filter(|s| whole(s))
        .filter_map(|&s| jpeg_frame(s).filter(|&f| like(f) && f.0.max(f.1) >= min_edge).map(|(w, h, _)| (s, w.max(h))))
        .min_by_key(|&(_, long)| long)
        .map(|(s, _)| s)
}

fn preview_covering(bytes: &[u8], min_edge: Option<u32>) -> Option<Vec<u8>> {
    let format = crate::probe(bytes);
    if matches!(format, Some(crate::RawFormat::Crw | crate::RawFormat::Mrw | crate::RawFormat::X3f)) {
        return scan_for_jpeg(bytes);
    }
    let tiff = Tiff::parse(bytes).ok()?;
    let mut found: Vec<&[u8]> = Vec::new();
    // the walks of pointers past the file's end share one budget (see file_span)
    let mut budget = scan_budget(bytes.len());
    for ifd in tiff.all_ifds() {
        candidates(bytes, ifd, 0, &mut found, &mut budget);
    }
    // maker-note preview IFDs (e.g. Nikon PreviewIFD 0x0011 holds JPEGInterchangeFormat relative to the note base)
    if let Some(exif) = tiff.exif()
        && let Some(e) = exif.get(t::MAKER_NOTE)
    {
        let make = tiff.find(t::MAKE).and_then(|e| e.value.as_str()).unwrap_or_default();
        if let Some(mn) = makernote::parse_makernote(bytes, e.offset, e.count() as u64, tiff.order, &make) {
            candidates(bytes, &mn.ifd, mn.base, &mut found, &mut budget);
            if let Some(off) = mn.ifd.u64(0x0011)
                && let Ok((pifd, _)) = craft_tiff::parse_ifd_at(bytes, mn.base + off, mn.order, mn.base, false, &Default::default())
            {
                candidates(bytes, &pifd, mn.base, &mut found, &mut budget);
            }
        }
    }
    let largest = found.iter().copied().filter(|s| is_dct_jpeg(s) || is_jxl(s)).max_by_key(|s| s.len());
    let best = min_edge
        .zip(largest)
        .and_then(|(min, big)| smallest_covering(&found, big, min))
        .or(largest)
        .map(|s| if is_jxl(s) { s.to_vec() } else { trim_eoi(s).to_vec() });
    // a TIFF raw whose preview no tag points to: one we can't decode (Leaf MOS), or Epson ERF (in its maker note)
    best.or_else(|| format.filter(|f| !f.is_supported() || *f == crate::RawFormat::Erf).and_then(|_| scan_for_jpeg(bytes)))
}

/// The end of the DCT JPEG whose SOI is at `soi` (exclusive index after its EOI) and its component count,
/// walking the marker segments and the entropy-coded data. `None` when it is not a complete, displayable
/// (baseline or progressive) JPEG, so a stray `D8 FF` in other data is rejected. `*reach` is left at
/// the furthest byte looked at (what the walk cost; see [`scan_for_jpeg`]).
fn jpeg_extent(b: &[u8], soi: usize, reach: &mut usize) -> Option<(usize, u8)> {
    let mut i = soi + 2;
    let r = walk_jpeg(b, &mut i);
    *reach = i;
    r
}

fn walk_jpeg(b: &[u8], i: &mut usize) -> Option<(usize, u8)> {
    let mut components = None;
    for _ in 0..4096 {
        if *b.get(*i)? != 0xff {
            return None;
        }
        while *b.get(*i)? == 0xff {
            *i += 1;
        }
        let m = *b.get(*i)?;
        *i += 1;
        match m {
            0xd9 => return components.map(|c| (*i, c)),
            0x01 | 0xd0..=0xd8 => continue,
            _ => {}
        }
        let len = usize::from(u16::from_be_bytes([*b.get(*i)?, *b.get(*i + 1)?]));
        if len < 2 {
            return None;
        }
        match m {
            0xc0..=0xc2 => {
                // precision, height, width, component count
                let h = u16::from_be_bytes([*b.get(*i + 3)?, *b.get(*i + 4)?]);
                let w = u16::from_be_bytes([*b.get(*i + 5)?, *b.get(*i + 6)?]);
                if h == 0 || w == 0 || components.is_some() {
                    return None;
                }
                components = Some(*b.get(*i + 7)?);
            }
            0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => return None, // lossless, arithmetic, hierarchical
            _ => {}
        }
        *i += len;
        if m == 0xda {
            // entropy-coded data up to the next marker that is not a stuffed 0xff or a restart
            loop {
                let Some(p) = b.get(*i..)?.iter().position(|&x| x == 0xff) else {
                    // the terminal search examined the rest of the slice: charge it before failing,
                    // or a crafted file can repeat full-file searches while the budget stays positive
                    *i = b.len();
                    return None;
                };
                *i += p;
                match *b.get(*i + 1)? {
                    0x00 | 0xd0..=0xd7 => *i += 2,
                    0xff => *i += 1,
                    _ => break,
                }
            }
        }
    }
    None
}

/// Last resort for the containers whose structure is not walked (Canon CRW, Minolta MRW, Sigma X3F, TIFF raws
/// with private blocks such as Leaf MOS or Epson ERF): the largest complete colour DCT JPEG stored as a plain
/// byte run in the file. A one-component JPEG is skipped (a Canon PowerShot CRW stores its sensor mosaic as one).
///
/// Evidence: in the Minolta and Epson files the first byte of the stored preview is not `ff` (it is `00`, `02` or
/// `ee`) while the rest is a regular JPEG, so a run is found by its `d8 ff` and starts one byte earlier,
/// with that byte restored to `ff`.
///
/// A crafted file can hold millions of JPEG-like starts whose walks each run to its end: the walks
/// share a budget of eight times the file's size, so the scan stays linear (a real file's candidates
/// fail within a few bytes, or are the preview itself).
fn scan_for_jpeg(data: &[u8]) -> Option<Vec<u8>> {
    scan_with_budget(data, scan_budget(data.len())).0
}

/// The walks' shared budget: eight times the file's size, at least 1 MiB (a tiny file still gets a
/// useful allowance), saturating so a hostile length cannot wrap it.
fn scan_budget(len: usize) -> usize {
    len.saturating_mul(8).max(1 << 20)
}

/// [`scan_for_jpeg`] with the walks' shared budget passed in, returning the walks' charged extent:
/// `(preview, bytes charged)`. The counter records the work each walk did whether or not budget
/// remains, so it stays a measure of work rather than of `budget - left`; a last walk may overshoot
/// the remaining budget by its own reach. It is a conservative charged-work measure, not literally
/// the bytes read: marker payloads skipped by `*i += len` are charged without being read, and a
/// failed terminal search is charged to the end of the slice. That over-count is safe for the
/// budget. It is a `u64` with saturating addition because the unbudgeted worst case is quadratic in
/// the file size (32-bit `usize` could otherwise overflow), and it costs one addition per
/// candidate, no allocation.
fn scan_with_budget(data: &[u8], budget: usize) -> (Option<Vec<u8>>, u64) {
    let mut best: Option<(usize, usize)> = None;
    let mut at = 1;
    let mut left = budget;
    let mut walked = 0u64;
    while let Some(off) = data.get(at..).and_then(|s| s.iter().position(|&b| b == 0xd8)) {
        let d8 = at + off;
        at = d8 + 1;
        if data.get(d8 + 1) != Some(&0xff) || !matches!(data.get(d8 + 2), Some(0xc0..=0xc4 | 0xdb | 0xe0..=0xef | 0xfe)) {
            continue;
        }
        let start = d8 - 1; // `at` starts at 1, so d8 >= 1
        let mut reach = start;
        let found = jpeg_extent(data, start, &mut reach);
        let spent = reach.saturating_sub(start);
        left = left.saturating_sub(spent);
        walked = walked.saturating_add(spent as u64);
        if left == 0 {
            break;
        }
        if let Some((end, 3)) = found {
            if best.is_none_or(|(s, e)| end - start > e - s) {
                best = Some((start, end));
            }
            at = end;
        }
    }
    let jpeg = best.and_then(|(start, end)| {
        let mut jpeg = data.get(start..end)?.to_vec();
        if let Some(first) = jpeg.first_mut() {
            *first = 0xff;
        }
        Some(jpeg)
    });
    (jpeg, walked)
}

fn trim_eoi(s: &[u8]) -> &[u8] {
    let end = s.windows(2).rposition(|w| w == [0xff, 0xd9]).map(|p| p + 2).unwrap_or(s.len());
    &s[..end]
}

#[cfg(test)]
mod tests;
