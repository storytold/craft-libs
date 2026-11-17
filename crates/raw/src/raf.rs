//! Fujifilm RAF — uncompressed, lossless compressed and lossy compressed Bayer and X-Trans.
//!
//! Sources: the ExifTool FujiFilm tag-name documentation (RAF header record tags `0x0100` RawImageFullSize,
//! `0x0110` RawImageCropTopLeft, `0x0111` RawImageCroppedSize, `0x0131` XTransLayout; raw-IFD tags `0xf001`
//! width, `0xf002` height, `0xf003` bits, `0xf007` strip offset, `0xf008` strip byte count, `0xf00a` black
//! levels, `0xf00e` WB_GRBLevels) and black-box analysis of CC0 samples from raw.pixls.us:
//!
//! - The file starts with a 16-byte magic, version and camera name; at byte 84 eight big-endian `u32`s give the
//!   embedded JPEG (offset, length), the header-record directory (offset, length) and the raw block (offset,
//!   length). The record directory is a `u32` count then `(u16 tag, u16 size, data)` records, big-endian.
//! - In current bodies the raw block is a little-endian TIFF whose IFD0 has a `0xf000` IFD pointer to the raw
//!   IFD; offsets in it are relative to the raw block.
//! - Sample packing, found by testing candidate bit orders for the smoothest image: 16-bit little-endian words;
//!   12-bit LSB-first bit stream; 14-bit (and other depths) stored as little-endian 32-bit words read MSB-first.
//!   Raw-IFD flag `0xf006` = 1 selects the 32-bit-word packing at any depth: 12-bit files with the flag set
//!   (FinePix S1, X-A10, SL1000) are word-packed like the 14-bit ones, the 12-bit files without it are LSB-first.
//!   `0xf005`, when non-zero, is the row stride in 32-bit words.
//! - CFA: the X-Trans layout record (36 bytes, 0 = R, 1 = G, 2 = B) is stored in reverse order and, reversed,
//!   is anchored at raw pixel (0, 0) — verified on X-Trans I and III samples by minimising the difference
//!   between horizontally / vertically adjacent "green" samples over all 72 orientations/phases, and by colour
//!   renders. Bayer FujiLayout (`0x0130`) gives the reversed 2×2 pattern in its low two bits
//!   (0 = R, 1/3 = G, 2 = B); absent layout records retain the assumed RGGB fallback.
//! - RawExposureBias (`0x9650`) is a signed big-endian 16-bit fraction; its negative is the
//!   baseline exposure used to restore the camera's dynamic-range exposure reduction.
//! - Striped predictive compression is decoded by `rafc`; see `docs/raf-compression.md`.

use crate::shared::white_from_data_sourced;
use crate::unpack::{unpack_lsb, unpack_words32_msb};
use crate::{BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result};
use craft_tiff::{ByteOrder, Ifd, Tiff};
use rayon::prelude::*;

const CROP_TOP_LEFT: u16 = 0x0110;
const CROPPED_SIZE: u16 = 0x0111;
const XTRANS_LAYOUT: u16 = 0x0131;
/// Raw-IFD flag `0xf006`: 1 in every 12- and 14-bit file here whose samples are packed into 32-bit words (the 12-bit
/// ones are otherwise an LSB-first stream), 0 where they are an LSB-first stream or 16-bit words.
const PACKED_WORDS: u16 = 0xf006;

/// The fixed-position header pointers.
pub(crate) struct Header<'a> {
    pub jpeg: Option<&'a [u8]>,
    pub records: Vec<(u16, &'a [u8])>,
    pub raw: Option<&'a [u8]>,
}

fn be32(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

fn slice(b: &[u8], off: Option<usize>, len: Option<usize>) -> Option<&[u8]> {
    let (o, l) = (off?, len?);
    if o == 0 || l == 0 {
        return None;
    }
    b.get(o..o.checked_add(l)?.min(b.len())).filter(|s| !s.is_empty())
}

pub(crate) fn header(b: &[u8]) -> Result<Header<'_>> {
    if !b.starts_with(b"FUJIFILMCCD-RAW") || b.len() < 108 {
        return Err(RawError::NotRaw);
    }
    let jpeg = slice(b, be32(b, 84), be32(b, 88));
    let raw = slice(b, be32(b, 100), be32(b, 104));
    let mut records = Vec::new();
    if let Some(dir) = slice(b, be32(b, 92), be32(b, 96)) {
        let n = be32(dir, 0).unwrap_or(0).min(4096);
        let mut p = 4;
        for _ in 0..n {
            let Some(h) = dir.get(p..p + 4) else { break };
            let (tag, size) = (u16::from_be_bytes([h[0], h[1]]), u16::from_be_bytes([h[2], h[3]]) as usize);
            let Some(d) = dir.get(p + 4..p + 4 + size) else { break };
            records.push((tag, d));
            p += 4 + size;
        }
    }
    Ok(Header { jpeg, records, raw })
}

fn record<'a>(h: &Header<'a>, tag: u16) -> Option<&'a [u8]> {
    h.records.iter().find(|r| r.0 == tag).map(|r| r.1)
}

/// A record holding two big-endian `u16`s.
fn pair(h: &Header, tag: u16) -> Option<(usize, usize)> {
    let d = record(h, tag).filter(|d| d.len() >= 4)?;
    Some((u16::from_be_bytes([d[0], d[1]]) as usize, u16::from_be_bytes([d[2], d[3]]) as usize))
}

pub(crate) fn raw_ifd(raw: &[u8]) -> Result<Ifd> {
    let tiff = Tiff::parse(raw).map_err(|_| RawError::Unsupported("RAF without a raw IFD (older FinePix layout)".into()))?;
    let off = tiff.ifds.first().and_then(|i| i.u64(0xf000)).ok_or_else(|| RawError::Unsupported("RAF without a raw IFD".into()))?;
    let (ifd, _) = craft_tiff::parse_ifd_at(raw, off, tiff.order, 0, false, &Default::default())?;
    Ok(ifd)
}

/// IFD0 of the Exif block (APP1 `Exif\0\0`) of the embedded camera JPEG, where a RAF keeps its orientation, make
/// and model. Walks the marker segments up to the start of scan; `None` when there is no readable Exif block.
fn jpeg_exif_tiff(jpeg: &[u8]) -> Option<Tiff> {
    let mut at = 2usize;
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    // every segment moves `at` forward by at least 4 bytes, so the walk ends within the JPEG's length
    while let (Some(&0xff), Some(&marker)) = (jpeg.get(at), jpeg.get(at + 1)) {
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([*jpeg.get(at + 2)?, *jpeg.get(at + 3)?]));
        let segment = jpeg.get(at + 4..(at + 2).checked_add(len)?)?;
        if marker == 0xe1
            && let Some(tiff) = segment.strip_prefix(b"Exif\0\0")
        {
            return Tiff::parse(tiff).ok();
        }
        at = at.checked_add(2 + len.max(2))?;
    }
    None
}

/// The camera name in the file header (bytes 28..60, NUL padded): the Exif `Model` of every sample.
fn camera_name(b: &[u8]) -> String {
    let name = b.get(28..60).unwrap_or_default();
    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    String::from_utf8_lossy(name.get(..end).unwrap_or_default()).trim().to_string()
}

/// Layout records are reversed and anchored at sensor (0, 0). FujiLayout's low
/// two bits encode Bayer colour; both 1 and 3 denote green.
fn cfa(h: &Header) -> Result<Cfa> {
    if let Some(l) = record(h, XTRANS_LAYOUT) {
        if l.len() != 36 || l.iter().any(|&c| c > 2) {
            return Err(RawError::Unsupported("Fujifilm RAF with an unknown X-Trans layout".into()));
        }
        return Ok(Cfa { width: 6, height: 6, pattern: l.iter().rev().copied().collect(), origin_x: 0, origin_y: 0 });
    }
    let Some(l) = record(h, 0x0130) else { return Ok(Cfa::bayer_static("RGGB")) };
    let cfa = Cfa { width: 2, height: 2, pattern: l.iter().rev().map(|&c| if c & 3 == 3 { 1 } else { c & 3 }).collect(), origin_x: 0, origin_y: 0 };
    // Colour counts alone also accept adjacent green sites; the two greens
    // must occupy opposite corners of the 2x2 Bayer repeat.
    if l.len() != 4 || !matches!(cfa.pattern.as_slice(), [0, 1, 1, 2] | [2, 1, 1, 0] | [1, 0, 2, 1] | [1, 2, 0, 1]) {
        return Err(RawError::Unsupported("Fujifilm RAF with an unknown Bayer layout".into()));
    }
    Ok(cfa)
}

/// Fuji RawExposureBias is a signed big-endian 16-bit fraction, undone like
/// DNG BaselineExposure. Invalid or implausible metadata does not alter exposure.
fn baseline_exposure(h: &Header) -> f64 {
    record(h, 0x9650)
        .and_then(|d| {
            let num = i16::from_be_bytes(d.get(..2)?.try_into().ok()?);
            let den = i16::from_be_bytes(d.get(2..4)?.try_into().ok()?);
            (den != 0).then(|| -f64::from(num) / f64::from(den))
        })
        .filter(|v| v.is_finite() && v.abs() <= 8.0)
        .unwrap_or(0.0)
}

/// The codec fixes the green-site geometry; exchanging the two non-green
/// planes only exchanges their colour labels, so RGGB and BGGR are both valid.
fn compressed_layout_supported(cfa: &Cfa) -> bool {
    *cfa == Cfa::xtrans() || *cfa == Cfa::bayer_static("RGGB") || *cfa == Cfa::bayer_static("BGGR")
}

/// Unpack one row of `bits`-bit samples in Fujifilm's packing. `words32`: the raw IFD's `0xf006` flag is set, the
/// samples are MSB-first in little-endian 32-bit words whatever the depth; otherwise 16-bit words, a 12-bit
/// LSB-first bit stream, or (other depths) the same 32-bit words.
pub(crate) fn unpack_row(src: &[u8], bits: u32, words32: bool, out: &mut [u16]) {
    match bits {
        16 if !words32 => out.iter_mut().zip(src.as_chunks::<2>().0).for_each(|(o, c)| *o = u16::from_le_bytes([c[0], c[1]])),
        12 if !words32 => unpack_lsb(src, 12, out),
        _ => unpack_words32_msb(src, ByteOrder::Little, bits, out),
    }
}

/// The Kelvin scale of a body without colour matrices, from tag `0xf00c` (`WB_GRBLevelsStandard`; tag number from
/// `exiftool -v3` run as a black-box tool on the files): `G R B 17 G R B 21`, the camera's own neutral multipliers
/// for Illuminant A and D65 (see [`crate::kelvin::KelvinPresets::from_neutral_pair`]).
fn fuji_kelvin_presets(ifd: &Ifd) -> Option<crate::kelvin::KelvinPresets> {
    let v = ifd.f64s(0xf00c).filter(|v| v.len() >= 8)?;
    let grb = |at: usize, code: f64| {
        (v[at + 3] == code && v[at] > 0.0 && v[at + 1] > 0.0 && v[at + 2] > 0.0)
            .then(|| [(v[at + 1] / v[at]) as f32, 1.0, (v[at + 2] / v[at]) as f32])
    };
    let (a, d65) = (grb(0, 17.0)?, grb(4, 21.0)?);
    crate::kelvin::KelvinPresets::from_neutral_pair(a, d65)
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let h = header(bytes)?;
    let raw = h.raw.ok_or_else(|| RawError::Corrupt("RAF without raw data".into()))?;
    let ifd = raw_ifd(raw)?;
    let get = |t: u16| ifd.u64(t).map(|v| usize::try_from(v).map_err(|_| RawError::Limit("RAF IFD value too large"))).transpose();
    let (w, hgt) = (get(0xf001)?.unwrap_or(0), get(0xf002)?.unwrap_or(0));
    let bits = ifd.u64(0xf003).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let (off, len) = (get(0xf007)?.unwrap_or(0), get(0xf008)?.unwrap_or(0));
    if w == 0 || hgt == 0 || !(8..=16).contains(&bits) {
        return Err(RawError::Corrupt(format!("RAF raw IFD: {w}x{hgt}, {bits} bits")));
    }
    let n = w.checked_mul(hgt).filter(|n| *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let src = raw
        .get(off..off.checked_add(len).ok_or_else(|| RawError::Corrupt("RAF strip size overflow".into()))?)
        .ok_or_else(|| RawError::Corrupt("RAF strip outside file".into()))?;
    let cfa = cfa(&h)?;
    let compression = be32(bytes, 108).unwrap_or(0);
    let compressed = match compression {
        0 => false,
        2 | 3 => true,
        _ => return Err(RawError::Unsupported(format!("Fujifilm RAF compression mode {compression}"))),
    };
    let mut data = Vec::new();
    if compressed {
        if !compressed_layout_supported(&cfa) {
            return Err(RawError::Unsupported("Fujifilm compressed RAF with an unsupported CFA layout".into()));
        }
        if !src.starts_with(b"IS") {
            // A recognized RAF compression mode can wrap an unknown packing. Do not
            // feed it to the IS codec; damage inside an identified IS stream stays Corrupt.
            return Err(RawError::Unsupported("Fujifilm compressed RAF without a supported IS signature".into()));
        }
        let flag = if compression == 2 { 1 } else { 0 };
        if src.get(2) != Some(&flag) {
            return Err(RawError::Corrupt("RAF container and stream compression modes disagree".into()));
        }
        data = crate::rafc::decode(src, w, hgt, bits, cfa.width == 6, mode, limits)?;
    } else {
        let words32 = ifd.u64(PACKED_WORDS).is_some_and(|v| v != 0);
        // An explicit packed-word flag wins over the size heuristic: padding in short
        // packed rows can occupy as many bytes as unpacked 16-bit samples.
        // Unflagged samples may use 16-bit words (e.g. 14-bit X-T20).
        let store = if !words32 && len as u64 >= (n as u64) * 2 { 16 } else { bits };
        let row_bits = w.checked_mul(store as usize).ok_or(RawError::Limit("RAF row too large"))?;
        // Word-packed rows store the high bits in the last little-endian word's final bytes.
        // Reading only ceil(bits / 8) bytes would lose those samples, not just padding.
        let packed_row = if words32 || (store != 16 && store != 12) {
            row_bits.div_ceil(32).checked_mul(4).ok_or(RawError::Limit("RAF row too large"))?
        } else {
            row_bits.div_ceil(8)
        };
        let stride = match get(0xf005)?.unwrap_or(0) {
            0 => packed_row,
            words => words.checked_mul(4).ok_or(RawError::Limit("RAF stride too large"))?.max(packed_row),
        };
        let required = stride.checked_mul(hgt - 1).and_then(|n| n.checked_add(packed_row)).ok_or(RawError::Limit("RAF strip too large"))?;
        if len < required {
            return Err(RawError::Unsupported(format!("Fujifilm RAF packing ({len} bytes for {w}x{hgt} {bits}-bit)")));
        }
        if mode == Mode::Full {
            data = vec![0u16; n];
            data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                // `required <= src.len()` proves these row offsets fit, including the final row.
                let s = src.get(y * stride..y * stride + packed_row).unwrap_or(&[]);
                if !words32 && store == 16 && raw.starts_with(b"MM") {
                    crate::unpack::read_u16s(s, ByteOrder::Big, row);
                } else {
                    unpack_row(s, store, words32, row);
                }
            });
        }
    }

    let active = match (pair(&h, CROP_TOP_LEFT), pair(&h, CROPPED_SIZE)) {
        (Some((top, left)), Some((ch, cw))) if ch > 0 && cw > 0 => Rect::new(left, top, cw, ch).clipped(w, hgt),
        _ => Rect::new(0, 0, w, hgt),
    };
    let active = if active.width == 0 || active.height == 0 { Rect::new(0, 0, w, hgt) } else { active };
    // black levels: one per CFA position, anchored at (0, 0) of the raw data; re-anchor at the active area
    let (black, black_source) = match ifd.f64s(0xf00a) {
        Some(v) if v.len() == cfa.pattern.len() && v.iter().any(|b| *b != v[0]) => {
            let (cw, chh) = (cfa.width, cfa.height);
            let values = (0..chh * cw).map(|i| v[((i / cw + active.y) % chh) * cw + (i % cw + active.x) % cw] as f32).collect();
            (BlackLevel { repeat_rows: chh, repeat_cols: cw, values, ..Default::default() }, crate::LevelSource::Metadata)
        }
        Some(v) if !v.is_empty() => (BlackLevel::uniform(v[0] as f32), crate::LevelSource::Metadata),
        _ => (BlackLevel::uniform(0.0), crate::LevelSource::Fallback),
    };
    let wb =
        ifd.f64s(0xf00e).filter(|v| v.len() >= 3 && v.iter().take(3).all(|x| *x > 0.0)).map(|v| [(v[1] / v[0]) as f32, 1.0, (v[2] / v[0]) as f32]);
    let (white, white_source) = white_from_data_sourced(&data, bits);
    let kelvin_presets = fuji_kelvin_presets(&ifd);
    let exif = h.jpeg.and_then(jpeg_exif_tiff);
    let ifd0 = exif.as_ref().and_then(|t| t.ifds.first());
    let orientation = Orientation::from_exif(ifd0.and_then(|i| i.u16(craft_tiff::tags::ORIENTATION)).unwrap_or(1));
    let img = RawImage {
        format: RawFormat::Raf,
        width: w,
        height: hgt,
        cpp: 1,
        data: RawData::U16(data),
        cfa: Some(cfa),
        bits,
        black,
        white: vec![white],
        active_area: active,
        crop: Rect::new(0, 0, active.width, active.height),
        orientation,
        color: ColorData { kelvin_presets, baseline_exposure: baseline_exposure(&h), ..Default::default() },
        wb_multipliers: wb,
        linearized: false,
        linearization_table: None,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists { list3: crate::raf_lens::corrections(&camera_name(bytes), &ifd, active), ..Default::default() },
        camera: ifd0
            .map(Camera::from_ifd0)
            .unwrap_or_else(|| Camera { model: Some(camera_name(bytes)).filter(|n| !n.is_empty()), ..Default::default() }),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        notices: crate::RawNotices { assumed_cfa: record(&h, XTRANS_LAYOUT).is_none() && record(&h, 0x0130).is_none(), ..Default::default() },
        level_sources: crate::LevelSources { black: black_source, white: white_source },
    };
    img.validate_for(mode)?;
    Ok(img)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Build a minimal RAF: header, records, raw block (LE TIFF whose IFD0 points to a 0xf000 raw IFD).
    pub(crate) fn raf(w: u32, h: u32, bits: u32, strip: Vec<u8>, layout: Option<[u8; 36]>, jpeg: &[u8]) -> Vec<u8> {
        raf_flagged(w, h, bits, strip, layout, jpeg, 0)
    }

    /// [`raf`] with the raw IFD's `0xf006` flag set to `words32`.
    pub(crate) fn raf_flagged(w: u32, h: u32, bits: u32, strip: Vec<u8>, layout: Option<[u8; 36]>, jpeg: &[u8], words32: u32) -> Vec<u8> {
        raf_full(w, h, bits, strip, layout, jpeg, words32, "TEST", &[])
    }

    /// [`raf_flagged`] with the camera name in the header and extra raw-IFD entries of signed rationals.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn raf_full(
        w: u32,
        h: u32,
        bits: u32,
        strip: Vec<u8>,
        layout: Option<[u8; 36]>,
        jpeg: &[u8],
        words32: u32,
        model: &str,
        extra: &[(u16, Vec<(i32, i32)>)],
    ) -> Vec<u8> {
        let mut block = b"II*\0\x08\0\0\0".to_vec();
        let sub_off: u32 = 8 + 2 + 12 + 4;
        block.extend_from_slice(&1u16.to_le_bytes());
        block.extend_from_slice(&[0x00, 0xf0, 13, 0, 1, 0, 0, 0]);
        block.extend_from_slice(&sub_off.to_le_bytes());
        block.extend_from_slice(&0u32.to_le_bytes());
        let n = 8u32 + extra.len() as u32;
        let wb_off = sub_off + 2 + 12 * n + 4;
        // out-of-line data after the white balance: each extra table in turn, then the strip
        let mut next = wb_off + 12;
        let mut table_at = Vec::new();
        for (_, v) in extra {
            table_at.push(next);
            next += 8 * v.len() as u32;
        }
        let strip_off = next;
        // (tag, type, count, value or offset), in tag order
        let mut entries: Vec<(u16, u16, u32, u32)> = vec![
            (0xf001, 4, 1, w),
            (0xf002, 4, 1, h),
            (0xf003, 4, 1, bits),
            (0xf006, 4, 1, words32),
            (0xf007, 4, 1, strip_off),
            (0xf008, 4, 1, strip.len() as u32),
            (0xf00a, 4, 1, 64),
            (0xf00e, 4, 3, wb_off),
        ];
        for ((tag, v), at) in extra.iter().zip(&table_at) {
            entries.push((*tag, 10, v.len() as u32, *at));
        }
        entries.sort_by_key(|e| e.0);
        block.extend_from_slice(&(n as u16).to_le_bytes());
        for (tag, ty, count, v) in entries {
            block.extend_from_slice(&tag.to_le_bytes());
            block.extend_from_slice(&ty.to_le_bytes());
            block.extend_from_slice(&count.to_le_bytes());
            block.extend_from_slice(&v.to_le_bytes());
        }
        block.extend_from_slice(&0u32.to_le_bytes());
        for v in [300u32, 600, 450] {
            block.extend_from_slice(&v.to_le_bytes());
        }
        for (_, v) in extra {
            for (num, den) in v {
                block.extend_from_slice(&num.to_le_bytes());
                block.extend_from_slice(&den.to_le_bytes());
            }
        }
        block.extend_from_slice(&strip);
        let mut recs: Vec<u8> = Vec::new();
        let mut nrec = 0u32;
        let mut add = |tag: u16, d: &[u8]| {
            recs.extend_from_slice(&tag.to_be_bytes());
            recs.extend_from_slice(&(d.len() as u16).to_be_bytes());
            recs.extend_from_slice(d);
            nrec += 1;
        };
        add(0x0100, &[(h >> 8) as u8, h as u8, (w >> 8) as u8, w as u8]);
        add(0x0110, &[0, 2, 0, 4]);
        add(0x0111, &[0, (h - 2) as u8, 0, (w - 4) as u8]);
        if let Some(l) = layout {
            add(0x0131, &l);
        }
        let mut dir = nrec.to_be_bytes().to_vec();
        dir.extend_from_slice(&recs);
        let mut out = b"FUJIFILMCCD-RAW 0201FF000000".to_vec();
        out.extend_from_slice(model.as_bytes());
        out.resize(108, 0);
        if strip.starts_with(b"IS") {
            let compression = if strip.get(2) == Some(&0) { 3u32 } else { 2u32 };
            out.extend_from_slice(&compression.to_be_bytes());
        }
        let jpeg_off = 160u32;
        let dir_off = jpeg_off + jpeg.len() as u32;
        let raw_off = dir_off + dir.len() as u32;
        for (i, v) in [jpeg_off, jpeg.len() as u32, dir_off, dir.len() as u32, raw_off, block.len() as u32].iter().enumerate() {
            out[84 + i * 4..88 + i * 4].copy_from_slice(&v.to_be_bytes());
        }
        out.resize(160, 0);
        out.extend_from_slice(jpeg);
        out.extend_from_slice(&dir);
        out.extend_from_slice(&block);
        out
    }

    /// The embedded JPEG's Exif block gives orientation, make and model: found after other segments, and not
    /// past the start of scan, a truncated segment or a missing SOI.
    #[test]
    fn orientation_and_camera_come_from_the_jpeg_exif() {
        use craft_tiff::{IfdBuilder, TiffWriter, Value, tags as t};
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("FUJIFILM".into()));
        ifd0.set(t::MODEL, Value::Ascii("X-T4".into()));
        ifd0.set(t::ORIENTATION, Value::Short(vec![6]));
        let exif = [&b"Exif\0\0"[..], &TiffWriter::new(ByteOrder::Big, false).write(&[ifd0]).unwrap()].concat();
        let segment = |marker: u8, body: &[u8]| [&[0xff, marker][..], &((body.len() + 2) as u16).to_be_bytes(), body].concat();
        let jpeg = |parts: &[Vec<u8>]| [&[0xff, 0xd8][..], &parts.concat(), &[0xff, 0xd9]].concat();
        let app0 = segment(0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        let decoded = |j: &[u8]| crate::decode(&raf(4, 4, 16, pack(&[1000; 16], 16), None, j)).unwrap();
        let r = decoded(&jpeg(&[app0.clone(), segment(0xe1, &exif)]));
        assert_eq!(r.orientation, Orientation::Rotate90);
        assert_eq!((r.camera.make.as_deref(), r.camera.model.as_deref()), (Some("FUJIFILM"), Some("X-T4")));
        // after the start of scan, or cut short: not read; the header's camera name stands in for the model
        for j in [jpeg(&[segment(0xda, &[0; 4]), segment(0xe1, &exif)]), jpeg(&[app0])[..8].to_vec(), exif.clone()] {
            let r = decoded(&j);
            assert_eq!(r.orientation, Orientation::Normal);
            assert_eq!((r.camera.make, r.camera.model.as_deref()), (None, Some("TEST")));
        }
    }

    fn pack(px: &[u16], bits: u32) -> Vec<u8> {
        match bits {
            16 => px.iter().flat_map(|v| v.to_le_bytes()).collect(),
            12 => {
                let mut o = Vec::new();
                for p in px.chunks(2) {
                    let v = p[0] as u32 | (p[1] as u32) << 12;
                    o.extend_from_slice(&[v as u8, (v >> 8) as u8, (v >> 16) as u8]);
                }
                o
            }
            _ => {
                // MSB-first stream, then byte-swap every 32-bit word
                let mut bitsv: Vec<u8> = Vec::new();
                let (mut acc, mut n) = (0u64, 0u32);
                for &v in px {
                    acc = (acc << bits) | v as u64;
                    n += bits;
                    while n >= 8 {
                        bitsv.push((acc >> (n - 8)) as u8);
                        n -= 8;
                    }
                }
                if n > 0 {
                    bitsv.push((acc << (8 - n)) as u8);
                }
                bitsv.resize(bitsv.len().div_ceil(4) * 4, 0);
                bitsv.chunks(4).flat_map(|c| [c[3], c[2], c[1], c[0]]).collect()
            }
        }
    }

    /// The black is `Metadata` when the raw IFD states it (`0xf00a`), else a `Fallback` 0; the white is `Inferred`
    /// from a saturation plateau, else the full range, a `Fallback`.
    #[test]
    fn level_sources_follow_the_black_tag_and_the_plateau() {
        let (w, h) = (64u32, 32u32);
        let flat: Vec<u16> = (0..w * h).map(|i| 300 + (i % 50) as u16).collect();
        let mut peaked = flat.clone();
        peaked[..200].fill(15000);
        let r = crate::decode(&raf(w, h, 14, pack(&peaked, 16), None, b"")).unwrap();
        assert_eq!(r.black.mean(), 64.0);
        assert!((14900.0..=15000.0).contains(&r.white[0]), "{}", r.white[0]);
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Metadata, white: crate::LevelSource::Inferred });
        // the same file with tag 0xf00a renamed to 0xf00b: no black is stated
        let mut bytes = raf(w, h, 14, pack(&flat, 16), None, b"");
        let at = bytes.windows(4).position(|b| b == [0x0a, 0xf0, 4, 0]).unwrap();
        bytes[at] = 0x0b;
        let r = crate::decode(&bytes).unwrap();
        assert_eq!((r.black.mean(), r.white[0]), (0.0, 16383.0));
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Fallback, white: crate::LevelSource::Fallback });
    }

    #[test]
    fn decodes_all_packings() {
        let (w, h) = (16u32, 6u32);
        for (bits, store) in [(12u32, 12u32), (14, 14), (14, 16), (16, 16)] {
            let px: Vec<u16> = (0..w * h).map(|i| ((i * 977) % (1 << bits.min(14))) as u16).collect();
            let layout: [u8; 36] = std::array::from_fn(|i| Cfa::xtrans().pattern[35 - i]);
            let bytes = raf(w, h, bits, pack(&px, store), Some(layout), b"\xff\xd8\xff\xd9");
            assert_eq!(crate::probe(&bytes), Some(RawFormat::Raf));
            let r = crate::decode(&bytes).unwrap_or_else(|e| panic!("{bits}: {e}"));
            assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
            assert_eq!(r.data, RawData::U16(px), "{bits}-bit");
            assert_eq!(r.cfa, Some(Cfa::xtrans()));
            assert_eq!(r.active_area, Rect::new(4, 2, 12, 4));
            assert_eq!(r.black.mean(), 64.0);
            assert_eq!(r.wb_multipliers, Some([2.0, 1.0, 1.5]));
        }
        let bytes = raf(w, h, 16, pack(&[0; 96], 16), None, b"");
        assert_eq!(crate::decode(&bytes).unwrap().cfa.unwrap().name(), "RGGB");
        let mut px = vec![0; 96];
        px[0] = 0x5349;
        let mut bytes = raf(w, h, 16, pack(&px, 16), None, b"");
        bytes[108..112].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(crate::decode(&bytes).unwrap().data, RawData::U16(px));
        // too little data for the size: compressed
        let bytes = raf(w, h, 14, vec![0; 20], None, b"");
        assert!(matches!(crate::decode(&bytes), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn word_packed_rows_keep_the_final_partial_word_and_reject_truncation() {
        for (w, bits, flag) in [(30u32, 14u32, 0u32), (22, 12, 1), (6, 12, 1), (5, 16, 1)] {
            let h = 6u32;
            let px: Vec<u16> = (0..w * h).map(|i| ((i * 977 + 101) % (1 << bits)) as u16).collect();
            // Independently place each sample bit into whole little-endian 32-bit row words.
            let strip: Vec<u8> = px
                .chunks(w as usize)
                .flat_map(|row| {
                    let mut words = vec![0u32; (row.len() * bits as usize).div_ceil(32)];
                    for (i, &sample) in row.iter().enumerate() {
                        for bit in 0..bits as usize {
                            let at = i * bits as usize + bit;
                            words[at / 32] |= u32::from((sample >> (bits as usize - 1 - bit)) & 1) << (31 - at % 32);
                        }
                    }
                    words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>()
                })
                .collect();
            let bytes = raf_flagged(w, h, bits, strip, None, &[], flag);
            assert_eq!(crate::decode(&bytes).unwrap().data, RawData::U16(px));
            assert!(crate::decode(&bytes[..bytes.len() - 1]).is_err());
        }
    }

    #[test]
    fn big_endian_words_and_bayer_layout_preserve_exact_sensor_values() {
        use craft_tiff::{IfdBuilder, TiffWriter, Value};
        let (w, h) = (6u32, 4u32);
        let px: Vec<u16> = (0..w * h).map(|i| (i * 977 + 123) as u16).collect();
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let build = |at| {
                let mut sensor = IfdBuilder::new();
                for (tag, value) in [(0xf001, w), (0xf002, h), (0xf003, 16), (0xf007, at), (0xf008, w * h * 2)] {
                    sensor.set(tag, Value::Long(vec![value]));
                }
                let mut root = IfdBuilder::new();
                root.set_child(0xf000, sensor);
                TiffWriter::new(order, false).write(&[root]).unwrap()
            };
            let mut raw = build(build(0).len() as u32);
            raw.extend(px.iter().flat_map(|v| if order == ByteOrder::Big { v.to_be_bytes() } else { v.to_le_bytes() }));
            let mut bytes = raf(w, h, 16, vec![0; (w * h * 2) as usize], None, &[]);
            let raw_at = be32(&bytes, 100).unwrap();
            bytes.truncate(raw_at);
            bytes[104..108].copy_from_slice(&(raw.len() as u32).to_be_bytes());
            bytes.extend(raw);
            // A four-site Bayer record is reversed in storage, just like the X-Trans record.
            let dir_at = be32(&bytes, 92).unwrap();
            let mut records = bytes[dir_at..raw_at].to_vec();
            let n = u32::from_be_bytes(records[..4].try_into().unwrap()) + 1;
            records[..4].copy_from_slice(&n.to_be_bytes());
            records.extend_from_slice(&[0x01, 0x30, 0, 4, 0x08, 0x09, 0x0b, 0x0a]);
            bytes.splice(dir_at..raw_at, records.clone());
            bytes[96..100].copy_from_slice(&(records.len() as u32).to_be_bytes());
            bytes[100..104].copy_from_slice(&((raw_at + 8) as u32).to_be_bytes());
            let image = crate::decode(&bytes).unwrap();
            assert_eq!(image.data, RawData::U16(px.clone()));
            assert_eq!(image.cfa.unwrap().name(), "BGGR");
        }
    }

    #[test]
    fn raw_exposure_bias_is_preserved_as_baseline_exposure_and_invalid_values_are_ignored() {
        for (num, den, expected) in [(-72i16, 100i16, 0.72f64), (1, 0, 0.0), (32767, 1, 0.0)] {
            let mut bytes = raf(6, 4, 16, pack(&[1000; 24], 16), None, &[]);
            let (dir_at, raw_at) = (be32(&bytes, 92).unwrap(), be32(&bytes, 100).unwrap());
            let mut records = bytes[dir_at..raw_at].to_vec();
            let n = u32::from_be_bytes(records[..4].try_into().unwrap()) + 1;
            records[..4].copy_from_slice(&n.to_be_bytes());
            records.extend_from_slice(&[0x96, 0x50, 0, 4]);
            records.extend(num.to_be_bytes());
            records.extend(den.to_be_bytes());
            bytes.splice(dir_at..raw_at, records.clone());
            bytes[96..100].copy_from_slice(&(records.len() as u32).to_be_bytes());
            bytes[100..104].copy_from_slice(&((raw_at + 8) as u32).to_be_bytes());
            for mode in [Mode::Header, Mode::Full] {
                assert!((decode(&bytes, mode, &Limits::default()).unwrap().color.baseline_exposure - expected).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn invalid_declared_cfa_layouts_are_not_silently_replaced() {
        for (tag, data) in [(0x0130, vec![8, 8, 10, 10]), (0x0130, vec![1, 2, 1, 0]), (0x0131, vec![3; 36])] {
            let header = Header { jpeg: None, records: vec![(tag, data.as_slice())], raw: None };
            assert!(matches!(cfa(&header), Err(RawError::Unsupported(_))));
        }
    }

    fn with_bayer_record(mut bytes: Vec<u8>, pattern: [u8; 4]) -> Vec<u8> {
        let (dir_at, raw_at) = (be32(&bytes, 92).unwrap(), be32(&bytes, 100).unwrap());
        let mut records = bytes[dir_at..raw_at].to_vec();
        let count = u32::from_be_bytes(records[..4].try_into().unwrap()) + 1;
        records[..4].copy_from_slice(&count.to_be_bytes());
        records.extend_from_slice(&[0x01, 0x30, 0, 4]);
        records.extend_from_slice(&pattern);
        bytes.splice(dir_at..raw_at, records.clone());
        bytes[96..100].copy_from_slice(&(records.len() as u32).to_be_bytes());
        bytes[100..104].copy_from_slice(&((raw_at + 8) as u32).to_be_bytes());
        bytes
    }

    #[test]
    fn non_bayer_geometry_is_rejected_in_header_and_full_modes() {
        // Reversal gives RGBG: correct colour counts, but both greens in one column.
        let bytes = with_bayer_record(raf(6, 4, 16, pack(&[1000; 24], 16), None, &[]), [1, 2, 1, 0]);
        for mode in [Mode::Header, Mode::Full] {
            assert!(matches!(decode(&bytes, mode, &Limits::default()), Err(RawError::Unsupported(why)) if why.contains("Bayer layout")));
        }
    }

    #[test]
    fn all_four_bayer_geometries_preserve_the_stored_samples() {
        let pixels: Vec<u16> = (0..24).map(|i| 123 + i * 977).collect();
        for (stored, expected) in [([2, 1, 1, 0], "RGGB"), ([0, 1, 1, 2], "BGGR"), ([1, 2, 0, 1], "GRBG"), ([1, 0, 2, 1], "GBRG")] {
            let bytes = with_bayer_record(raf(6, 4, 16, pack(&pixels, 16), None, &[]), stored);
            for mode in [Mode::Header, Mode::Full] {
                let image = decode(&bytes, mode, &Limits::default()).unwrap();
                assert_eq!((image.width, image.height), (6, 4));
                assert_eq!(image.cfa.as_ref().unwrap().name(), expected);
                assert_eq!(image.data, RawData::U16(if mode == Mode::Full { pixels.clone() } else { Vec::new() }));
            }
        }
    }

    #[test]
    fn compressed_bayer_requires_the_layout_assumed_by_its_predictor() {
        let strip = b"IS\x01\x00\x0e\0\x12\x06\0\x03\x0c\x03\0\x02\0\x03".to_vec();
        let mut bytes = raf(780, 18, 14, strip, None, &[]);
        let (dir_at, raw_at) = (be32(&bytes, 92).unwrap(), be32(&bytes, 100).unwrap());
        let mut records = bytes[dir_at..raw_at].to_vec();
        let n = u32::from_be_bytes(records[..4].try_into().unwrap()) + 1;
        records[..4].copy_from_slice(&n.to_be_bytes());
        // Reversed GRBG: a valid Bayer pattern, but not the RGGB order of this codec.
        records.extend_from_slice(&[0x01, 0x30, 0, 4, 0x0b, 0x0a, 0x08, 0x09]);
        bytes.splice(dir_at..raw_at, records.clone());
        bytes[96..100].copy_from_slice(&(records.len() as u32).to_be_bytes());
        bytes[100..104].copy_from_slice(&((raw_at + 8) as u32).to_be_bytes());
        for mode in [Mode::Header, Mode::Full] {
            assert!(matches!(decode(&bytes, mode, &Limits::default()), Err(RawError::Unsupported(why)) if why.contains("layout")));
        }
    }

    #[test]
    fn compressed_layout_accepts_both_red_blue_orders_without_accepting_wrong_green_phase() {
        for name in ["RGGB", "BGGR"] {
            assert!(compressed_layout_supported(&Cfa::bayer_static(name)), "{name}");
        }
        for name in ["GRBG", "GBRG"] {
            assert!(!compressed_layout_supported(&Cfa::bayer_static(name)), "{name}");
        }
        let mut other = Cfa::xtrans();
        assert!(compressed_layout_supported(&other));
        other.pattern.swap(0, 2);
        assert!(!compressed_layout_supported(&other));
    }

    #[test]
    fn unknown_compressed_signature_is_unsupported_but_identified_stream_damage_is_corrupt() {
        let header = b"IS\x01\x00\x0e\0\x12\x06\0\x03\x0c\x03\0\x02\0\x03".to_vec();
        let known = raf(780, 18, 14, header, None, &[]);
        let raw_at = be32(&known, 100).unwrap();
        let strip_at = raw_at + raw_ifd(&known[raw_at..]).unwrap().u64(0xf007).unwrap() as usize;
        let mut unknown = known.clone();
        unknown[strip_at..strip_at + 2].copy_from_slice(b"XX");
        for mode in [Mode::Header, Mode::Full] {
            assert!(matches!(decode(&unknown, mode, &Limits::default()), Err(RawError::Unsupported(why)) if why.contains("signature")));
            assert!(
                matches!(decode(&known, mode, &Limits::default()), Err(RawError::Corrupt(_))),
                "identified but truncated IS stream must remain corrupt"
            );
        }
    }

    /// Raw IFD flag `0xf006` selects the 32-bit-word packing at 12 bits; without it 12 bits are an LSB-first stream.
    #[test]
    fn flag_selects_word_packed_12_bit() {
        let (w, h) = (16u32, 6u32);
        let px: Vec<u16> = (0..w * h).map(|i| ((i * 977) % 4096) as u16).collect();
        // the same 12-bit samples as little-endian 32-bit words (what the 14-bit branch of `pack` writes)
        let mut stream: Vec<u8> = Vec::new();
        let (mut acc, mut n) = (0u64, 0u32);
        for &v in &px {
            acc = (acc << 12) | v as u64;
            n += 12;
            while n >= 8 {
                stream.push((acc >> (n - 8)) as u8);
                n -= 8;
            }
        }
        let le: Vec<u8> = stream.chunks(4).flat_map(|c| [c[3], c[2], c[1], c[0]]).collect();
        let decode = |bytes: Vec<u8>| crate::decode(&bytes).unwrap().data;
        assert_eq!(decode(raf_flagged(w, h, 12, le.clone(), None, b"", 1)), RawData::U16(px.clone()));
        // flag clear: the same bytes are read as the LSB-first stream, which gives other samples
        assert_ne!(decode(raf_flagged(w, h, 12, le, None, b"", 0)), RawData::U16(px.clone()));
        // flag clear: the LSB-first stream still decodes
        assert_eq!(decode(raf_flagged(w, h, 12, pack(&px, 12), None, b"", 0)), RawData::U16(px));
    }

    /// The lens tables of the raw IFD reach `Mode::Header` and `Mode::Full` alike, only for validated cameras,
    /// and survive a DNG round trip. The frame is 12 × 4 after the crop records (4, 2).
    #[test]
    fn lens_tables_become_opcodes_for_validated_cameras() {
        let (w, h) = (16u32, 6u32);
        let half = 12.0f64.hypot(4.0) / 2.0;
        let rat = |v: &[f64]| -> Vec<(i32, i32)> { v.iter().map(|x| ((x * 1.0e6).round() as i32, 1_000_000)).collect() };
        let knots = [0.3535211268, 0.5, 0.6126760563, 0.7070422535, 0.7908450704, 0.8661971831, 0.9352112676, 1.0, 1.06056338];
        let table = |values: &[f64]| {
            let mut v = vec![half / 9.0];
            v.extend_from_slice(&knots);
            v.extend_from_slice(values);
            rat(&v)
        };
        let dist = [-1.2, -2.4, -3.5, -4.4, -5.3, -5.9, -6.3, -6.4, -6.4];
        let vig = [98.5, 96.9, 94.9, 91.0, 84.7, 78.4, 73.0, 67.5, 67.5];
        // the colour-plane table is present in real files and ignored
        let ca = [-0.00003, -0.00009, -0.00015, -0.0002, -0.00027, -0.00034, -0.00046, -0.0006, -0.0006];
        let extra = vec![(0xf00b, table(&dist)), (0xf00f, table(&ca)), (0xf010, table(&vig))];
        for (model, expected) in [("X-T4", 2), ("GFX 100", 2), ("X-T6", 0), ("X-T4 II", 0), ("", 0)] {
            let bytes = raf_full(w, h, 16, pack(&[1000; 96], 16), None, b"", 0, model, &extra);
            let header = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
            let full = crate::decode(&bytes).unwrap();
            assert_eq!(header.info(), full.info(), "{model}");
            assert_eq!(full.opcodes.list3.len(), expected, "{model}");
            let Some(crate::Opcode::WarpRectilinear { planes, center }) = full.opcodes.list3.first() else { continue };
            assert_eq!((planes.len(), *center), (1, [0.5, 0.5]), "{model}");
            assert!(matches!(full.opcodes.list3[1], crate::Opcode::FixVignetteRadial { .. }));
            #[cfg(feature = "dng")]
            {
                let dng = crate::write_dng(&full, &Default::default()).unwrap();
                assert_eq!(crate::decode(&dng).unwrap().opcodes.list3, full.opcodes.list3, "{model}");
            }
        }
        // a table in another scale is left alone, and so is a file without tables
        let wrong: Vec<(u16, Vec<(i32, i32)>)> =
            extra.iter().map(|(t, v)| (*t, v.iter().map(|&(n, d)| if n == v[0].0 { (n * 2, d) } else { (n, d) }).collect())).collect();
        let bytes = raf_full(w, h, 16, pack(&[1000; 96], 16), None, b"", 0, "X-T4", &wrong);
        assert!(crate::decode(&bytes).unwrap().opcodes.list3.iter().all(|o| !matches!(o, crate::Opcode::WarpRectilinear { .. })));
        let bytes = raf_full(w, h, 16, pack(&[1000; 96], 16), None, b"", 0, "X-T4", &[]);
        assert!(crate::decode(&bytes).unwrap().opcodes.list3.is_empty());
    }

    #[test]
    fn overflowing_row_stride_is_an_error() {
        let mut bytes = raf(16, 6, 16, vec![0; 192], None, &[]);
        let off = be32(&bytes, 100).unwrap();
        // Replace WB with a LONG8 stride, pointing to the existing out-of-line WB storage.
        bytes[off + 112..off + 116].copy_from_slice(&[0x05, 0xf0, 16, 0]);
        bytes[off + 116..off + 120].copy_from_slice(&1u32.to_le_bytes());
        bytes[off + 128..off + 136].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(crate::decode(&bytes), Err(RawError::Limit(_))));
    }
}
