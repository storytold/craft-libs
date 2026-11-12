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
    let Ok(tiff) = Tiff::parse(bytes) else { return cr3_color_space(bytes) };
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

/// A CR3 keeps its EXIF and maker note in `CMT2`/`CMT3` TIFF blocks and its JPEGs carry no EXIF; a body shot in
/// Adobe RGB writes ColorSpace Uncalibrated (0xFFFF) with no Interoperability IFD (storytold/lightcraft#2654), so the Canon
/// maker note says which, else an EXIF ColorSpace of sRGB is taken.
#[cfg(feature = "cr3")]
fn cr3_color_space(bytes: &[u8]) -> Option<PreviewColorSpace> {
    let cr3 = craft_metadata::cr3::parse_cr3(bytes)?;
    let first = |i: usize| cr3.cmt[i].and_then(|b| Tiff::parse(b).ok()).and_then(|t| t.ifds.into_iter().next());
    if let Some(space) = first(2).and_then(|mn| canon_color_space(&mn)) {
        return Some(space);
    }
    (first(1)?.u16(0xA001)? == 1).then_some(PreviewColorSpace::Srgb)
}

#[cfg(not(feature = "cr3"))]
fn cr3_color_space(_: &[u8]) -> Option<PreviewColorSpace> {
    None
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

/// Whether the camera optimised its embedded JPEG's dynamic range with local tone mapping that the raw data doesn't
/// carry: Sony's Dynamic Range Optimizer (maker note `0xb025`). Such a JPEG is brighter in its darker regions than
/// the raw developed with the camera's global tone curve. `Some(false)` when the file says it was off, `None` when
/// it doesn't say (other makers, notes without the tag, undocumented values).
#[cfg(feature = "arw")]
pub fn embedded_preview_dynamic_range_optimized(bytes: &[u8]) -> Option<bool> {
    let tiff = Tiff::parse(bytes).ok()?;
    let make = tiff.ifds.first()?.string(t::MAKE)?;
    if !make.trim().to_ascii_uppercase().starts_with("SONY") {
        return None;
    }
    let e = tiff.exif()?.get(t::MAKER_NOTE)?;
    let mn = makernote::parse_makernote(bytes, e.offset, e.count() as u64, tiff.order, &make)?;
    crate::arw::dynamic_range_optimizer(&mn)
}

/// The first frame (SOF) marker of a JPEG and its position, from the marker segments before the first scan.
fn first_frame(b: &[u8]) -> Option<(u8, usize)> {
    first_frame_within(b, &mut { usize::MAX })
}

/// [`first_frame`] charging `work` one per step (a segment, or a fill byte); `None` too when it runs out.
fn first_frame_within(b: &[u8], work: &mut usize) -> Option<(u8, usize)> {
    if b.len() < 4 || !b.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut i = 2usize;
    while i.checked_add(4)? <= b.len() {
        *work = work.checked_sub(1)?;
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

/// The largest embedded preview, if any: a JPEG, or (DNG 1.7) a JPEG XL file, or for a Hasselblad 3FR / FFF, which
/// has no JPEG, its uncompressed RGB preview wrapped as a baseline TIFF.
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
    if bytes.starts_with(b"FUJIFILMCCD-RAW") {
        return raf_jpeg(bytes).filter(|j| is_dct_jpeg(j)).map(|j| trim_eoi(j).to_vec());
    }
    let format = crate::probe(bytes);
    #[cfg(feature = "cr3")]
    if format == Some(crate::RawFormat::Cr3) {
        // a Canon CR3 keeps its full-size JPEG beside a 1620 px `PRVW` (and a 160 px `THMB`): a
        // thumbnail decodes the small copy, not 45 MP of JPEG (about 150 ms a photo, the grid's
        // "popcorn" on a folder of R5 / R5 C raws)
        // (without a JPEG track, the largest box is the picture's shape: the boxes alone may hold several)
        if let Some(min) = min_edge {
            let mut found = cr3_preview_box_list(bytes);
            let full = cr3_full_jpeg(bytes);
            found.extend(full);
            if let Some(largest) = full.or_else(|| largest_by_pixels(&found))
                && let Some(j) = smallest_covering(&found, largest, min)
            {
                return Some(trim_eoi(j).to_vec());
            }
        }
        return cr3_preview(bytes).map(|j| trim_eoi(j).to_vec());
    }
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
    // Olympus: the CameraSettings preview
    #[cfg(feature = "orf")]
    if let Some(p) = crate::orf::preview(bytes) {
        found.push(p);
    }
    bound_candidates(&mut found, bytes.len());
    let largest = found.iter().copied().filter(|s| is_dct_jpeg(s) || is_jxl(s)).max_by_key(|s| s.len());
    let best = min_edge
        .zip(largest)
        .and_then(|(min, big)| smallest_covering(&found, big, min))
        .or(largest)
        .map(|s| if is_jxl(s) { s.to_vec() } else { trim_eoi(s).to_vec() });
    // a TIFF raw whose preview no tag points to: one we can't decode (Leaf MOS), or Epson ERF (in its maker note)
    let best = best.or_else(|| format.filter(|f| !f.is_supported() || *f == crate::RawFormat::Erf).and_then(|_| scan_for_jpeg(bytes)));
    // Hasselblad 3FR / FFF carry no JPEG, only uncompressed RGB images: one wrapped as a baseline TIFF
    #[cfg(feature = "hasselblad")]
    let best = best.or_else(|| (format == Some(crate::RawFormat::CfaTiff)).then(|| crate::hasselblad::rgb_preview(&tiff, bytes)).flatten());
    best
}

/// Drops candidate ranges that repeat an earlier one, that are not a displayable JPEG or JPEG XL, and valid ones
/// past a shared budget of [`scan_budget`] bytes: a range is charged only once it validates, so invalid ranges
/// covering most of the file cannot use up the budget before a valid preview. The header walks that validate
/// (or reject) the ranges share a second budget of the same size, charged for every walk, so many ranges
/// that start a JPEG header and run through fill bytes cannot each cost a walk of their length.
fn bound_candidates(found: &mut Vec<&[u8]>, file_len: usize) {
    let mut seen = std::collections::HashSet::new();
    let mut left = scan_budget(file_len);
    let mut work = scan_budget(file_len);
    found.retain(|s| {
        let mut dct = || first_frame_within(s, &mut work).is_some_and(|(m, _)| matches!(m, 0xc0..=0xc2));
        if !seen.insert((s.as_ptr() as usize, s.len())) || !(dct() || is_jxl(s)) || s.len() > left {
            return false;
        }
        left -= s.len();
        true
    });
}

/// Where a preview lives in a file, so a caller with a seekable file can read just that part of it.
///
/// Why it exists: a 56 MB Fuji RAF keeps its JPEG in the first 5.6 MB (measured: DSCF9492.RAF at
/// 5.6 MB), so a grid thumbnail or a loupe preview has no reason to pull the remaining 50 MB —
/// which on a network share is the difference between a moment and half a minute. A caller that has
/// only bytes (`wasm`, tests) keeps using [`embedded_preview`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewRange {
    /// Byte offset of the preview in the file.
    pub offset: u64,
    /// Bytes to read at [`Self::offset`]: the container's declared preview extent, which may carry
    /// padding past the preview's end-of-image marker.
    pub window_len: u64,
}

/// The preview's range in the file, when the file's header alone locates it.
///
/// `bytes` is the file's first [`PREVIEW_PREFIX`] bytes (more is fine). `None` means "read the file
/// whole and call [`embedded_preview`]": the formats not covered here (a TIFF-based raw's pointer
/// tag can sit in an IFD the prefix cuts short; CR3 scans box by box) and files whose pointer is
/// damaged.
///
/// The caller reads `[range.offset, range.offset + range.window_len)` and passes those bytes to
/// [`embedded_preview_from_range`], which returns exactly what [`embedded_preview`] returns for the
/// whole file.
///
/// Pass a prefix that actually covers the pointer's range (read [`Self::offset`] +
/// [`Self::window_len`] bytes, or start with [`PREVIEW_PREFIX`] and grow when it is not enough):
/// `raf::header` clamps a slice to the buffer it was given, so a prefix that stops
/// inside the declared extent yields a shorter `window_len` than the file really holds.
pub fn bounded_preview(bytes: &[u8]) -> Option<PreviewRange> {
    let jpeg = raf_jpeg(bytes)?;
    let offset = (jpeg.as_ptr() as usize).checked_sub(bytes.as_ptr() as usize)?;
    if offset.checked_add(jpeg.len())? > bytes.len() {
        return None;
    }
    Some(PreviewRange { offset: offset as u64, window_len: jpeg.len() as u64 })
}

/// The camera JPEG a RAF's header points to (clamped to `bytes`); `None` for any other file.
#[cfg(feature = "raf")]
fn raf_jpeg(bytes: &[u8]) -> Option<&[u8]> {
    bytes.starts_with(b"FUJIFILMCCD-RAW").then(|| crate::raf::header(bytes).ok()?.jpeg).flatten()
}

#[cfg(not(feature = "raf"))]
fn raf_jpeg(_: &[u8]) -> Option<&[u8]> {
    None
}

/// The preview inside `window`, which holds the file's bytes at `[range.offset, range.offset +
/// range.window_len)`: the counterpart of [`bounded_preview`]. Padding past the end-of-image marker
/// is dropped, exactly as [`embedded_preview`] drops it.
pub fn embedded_preview_from_range(window: &[u8], range: PreviewRange) -> Option<Vec<u8>> {
    let preview = window.get(..usize::try_from(range.window_len).ok()?)?;
    if !is_dct_jpeg(preview) {
        return None;
    }
    Some(trim_eoi(preview).to_vec())
}

/// How many bytes of a raw file are enough for [`bounded_preview`]: every covered format keeps its
/// preview pointer in a fixed header, well inside this.
pub const PREVIEW_PREFIX: usize = 4096;

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

/// Canon CR3: the full-size JPEG track (see `craft_metadata::cr3`), else the `PRVW` / `THMB` boxes.
#[cfg(feature = "cr3")]
fn cr3_preview(bytes: &[u8]) -> Option<&[u8]> {
    cr3_full_jpeg(bytes).or_else(|| cr3_preview_boxes(bytes))
}

/// A CR3's full-size JPEG track, when it holds a DCT JPEG.
#[cfg(feature = "cr3")]
fn cr3_full_jpeg(bytes: &[u8]) -> Option<&[u8]> {
    let full = craft_metadata::cr3::parse_cr3(bytes)
        .and_then(|c| c.tracks.iter().find(|t| t.kind == craft_metadata::cr3::Cr3TrackKind::Jpeg).and_then(|t| t.data));
    full.and_then(|(at, len)| bytes.get(at..at.checked_add(len)?)).filter(|j| is_dct_jpeg(j))
}

/// Marker-walk budget of the CR3 fallback: candidates can overlap (each claiming the rest of the file) and
/// [`is_dct_jpeg`] walks each marker by marker, so walking every candidate would be quadratic in file size.
/// The `THMB`/`PRVW` previews this finds are far below the cap together; once it is spent the scan stops.
#[cfg(feature = "cr3")]
const CR3_MAX_WALK: usize = 8 << 20;

/// Canon CR3 (ISO base media file): the `PRVW` box (Laurent Clévy's CR3 notes; layout confirmed on a CC0
/// sample) is `u32 size, "PRVW", u32 0, u16 ?, u16 width, u16 height, u16 ?, u32 jpeg length, JPEG`; the smaller
/// `THMB` box has the same shape. Returns the larger valid one.
#[cfg(feature = "cr3")]
fn cr3_preview_boxes(bytes: &[u8]) -> Option<&[u8]> {
    largest_by_pixels(&cr3_preview_box_list(bytes))
}

/// Of `found`, the JPEG with the longest edge in pixels (a smaller picture can take more bytes); the
/// first of equals. CodeRabbit on #3077.
#[cfg(feature = "cr3")]
fn largest_by_pixels<'a>(found: &[&'a [u8]]) -> Option<&'a [u8]> {
    let long = |j: &[u8]| jpeg_frame(j).map_or(0, |(w, h, _)| w.max(h));
    found.iter().copied().rev().max_by_key(|j| long(j))
}

/// The valid JPEGs of a CR3's `PRVW` and `THMB` boxes, every one: a file may carry more than one
/// `PRVW`, and the smallest that covers a size is picked from them all.
#[cfg(feature = "cr3")]
fn cr3_preview_box_list(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = Vec::new();
    let mut walked = 0usize;
    // the boxes sit outside the media data: only the rest of the file is searched (the media data
    // is most of the file, and a thumbnail is asked for every photo in view)
    let file = bytes;
    // (every top-level `mdat` is left out, not only the first: a PRVW-shaped JPEG in media data is not a
    // preview; CodeRabbit on #3583)
    let media = cr3_media_data(file);
    let whole_file = media.is_empty();
    let mut regions: Vec<&[u8]> = Vec::new();
    let mut from = 0usize;
    for (start, end) in media {
        if start >= from {
            regions.extend(file.get(from..start));
        }
        from = from.max(end);
    }
    regions.extend(file.get(from..));
    for tag in [b"PRVW", b"THMB"] {
        // the whole file searched (its media data not found): the first valid box of a kind only, as
        // the declared one comes before the media data and a later look-alike must not win
        let mut taken = false;
        for &bytes in &regions {
            if whole_file && taken {
                break;
            }
            let mut from = 0;
            while let Some(i) = bytes.get(from..).and_then(|s| s.windows(4).position(|w| w == tag)).map(|p| p + from) {
                from = i + 4;
                let Some(start) = i.checked_sub(4) else { continue };
                // (the sizes are the file's: checked, a 32-bit build must not wrap them)
                let be32 = |at: usize| bytes.get(at..at.checked_add(4)?).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize);
                let (Some(size), Some(len)) = (be32(start), start.checked_add(20).and_then(be32)) else { continue };
                let (Some(j), Some(box_end)) = (start.checked_add(24), start.checked_add(size.max(24))) else { continue };
                let Some(end) = j.checked_add(len).filter(|e| len >= 4 && *e <= box_end && *e <= bytes.len()) else { continue };
                let Some(jpeg) = bytes.get(j..end) else { continue };
                // One that doesn't fit the budget left is skipped; a smaller one later may still fit.
                if walked.saturating_add(jpeg.len()) > CR3_MAX_WALK {
                    continue;
                }
                walked = walked.saturating_add(jpeg.len());
                if is_dct_jpeg(jpeg) {
                    out.push(jpeg);
                    taken = true;
                    if whole_file {
                        break;
                    }
                    // (past it: a tag inside its JPEG is not another box)
                    from = end;
                }
            }
        }
    }
    out
}

/// Where a CR3's top-level `mdat` boxes lie (`start..end` each, `end` clamped to the file, in file
/// order): the media data, most of the file. Empty when there is none, or the boxes stop making sense
/// before the first. The metadata and the small previews are in the other top-level boxes.
#[cfg(feature = "cr3")]
fn cr3_media_data(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let _ = cr3_media_walk(bytes, &mut out);
    out
}

/// [`cr3_media_data`]'s walk of the top-level boxes; `None` where they stop making sense.
#[cfg(feature = "cr3")]
fn cr3_media_walk(bytes: &[u8], out: &mut Vec<(usize, usize)>) -> Option<()> {
    let be32 = |at: usize| bytes.get(at..at.checked_add(4)?).map(|s| u64::from(u32::from_be_bytes([s[0], s[1], s[2], s[3]])));
    let be64 = |at: usize| bytes.get(at..at.checked_add(8)?).map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]));
    let mut at = 0usize;
    for _ in 0..64 {
        let size = be32(at)?;
        let len = match size {
            // (a box running to the end of the file)
            0 => u64::try_from(bytes.len().saturating_sub(at)).unwrap_or(u64::MAX),
            1 => be64(at.checked_add(8)?)?,
            n => n,
        };
        let next = usize::try_from(len).ok().filter(|l| *l >= 8).and_then(|l| at.checked_add(l));
        if bytes.get(at.checked_add(4)?..at.checked_add(8)?) == Some(b"mdat".as_slice()) {
            out.push((at, next.unwrap_or(bytes.len()).min(bytes.len())));
        }
        at = next.filter(|n| *n < bytes.len())?;
    }
    None
}

fn trim_eoi(s: &[u8]) -> &[u8] {
    let end = s.windows(2).rposition(|w| w == [0xff, 0xd9]).map(|p| p + 2).unwrap_or(s.len());
    &s[..end]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod bound_tests {
    #[test]
    fn repeated_and_over_budget_candidates_are_dropped() {
        let mut data = vec![0u8; 4 << 20];
        // a valid JPEG header every 16 bytes, so each range below validates
        for i in 0..64 {
            data[i * 16..i * 16 + 15].copy_from_slice(&[0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        }
        let mut found: Vec<&[u8]> = vec![&data[..], &data[..], &data[..]];
        super::bound_candidates(&mut found, data.len());
        assert_eq!(found.len(), 1);
        // overlapping but unequal ranges stop at the shared budget (8 x the file, at least 1 MiB)
        let mut found: Vec<&[u8]> = (0..64).map(|i| &data[i * 16..]).collect();
        super::bound_candidates(&mut found, data.len());
        assert!(found.len() <= 8 && found.iter().map(|s| s.len()).sum::<usize>() <= 8 * data.len());
    }

    #[test]
    fn invalid_ranges_do_not_use_up_the_budget_before_a_valid_preview() {
        let mut data = vec![0u8; 2 << 20];
        let jpeg = [0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0];
        data[1 << 20..(1 << 20) + jpeg.len()].copy_from_slice(&jpeg);
        // 40 distinct invalid ranges of nearly the whole file (far past the 8 x budget), then the valid one
        let mut found: Vec<&[u8]> = (0..40).map(|i| &data[i..1 << 20]).collect();
        found.push(&data[1 << 20..]);
        super::bound_candidates(&mut found, data.len());
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with(&[0xff, 0xd8]));
    }

    #[test]
    fn header_walks_of_many_fill_byte_ranges_share_a_budget() {
        // a JPEG header followed by 100 000 fill bytes and no frame: each walk would run to the end
        let mut data = vec![0xffu8; 100_100];
        data[..2].copy_from_slice(&[0xff, 0xd8]);
        let valid = [0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0];
        // 50 distinct such ranges (5 M steps against the 1 MiB budget), then a valid header
        let mut found: Vec<&[u8]> = (0..50).map(|i| &data[..100_000 + i]).collect();
        found.push(&valid);
        super::bound_candidates(&mut found, data.len());
        assert!(found.is_empty(), "the walks stop at the shared budget, so the later header is not reached");
        // the same header with no hostile ranges before it is kept
        let mut found: Vec<&[u8]> = vec![&valid];
        super::bound_candidates(&mut found, data.len());
        assert_eq!(found.len(), 1);
    }
}
