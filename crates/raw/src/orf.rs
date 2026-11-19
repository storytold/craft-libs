//! Olympus ORF — uncompressed variants; compressed sensor data is unsupported.
//!
//! Sources: TIFF 6.0 (the `IIRO`/`MMOR` container is a TIFF with a different magic number), Exif 2.3 (`CFAPattern`,
//! tag `0xa302`), the ExifTool Olympus tag-name documentation (maker-note sub-directories `0x2020` CameraSettings —
//! `0x0101/0x0102` PreviewImageStart/Length — and `0x2040` ImageProcessing — `0x0100` WB_RBLevels, `0x0600`
//! BlackLevel2, `0x0612–0x0615` CropLeft/Top/Width/Height) and black-box analysis of CC0 samples from
//! raw.pixls.us (E-1, E-400, XZ-2):
//!
//! - 16 bits per sample: little-endian words; some bodies (E-1, E-400) store 12-bit values in the top bits (the low
//!   four bits zero in more than 99% of samples), which we shift down.
//! - 12-bit packed (XZ-2): each row is a sequence of little-endian 32-bit words read MSB-first (found by testing
//!   candidate bit orders for the smoothest image).
//! - 12-bit packed in two fields (a few old compacts): IFD0 says uncompressed, 12 bits, many strips, and the strip
//!   byte counts add up to exactly width × height × 1.5. Samples are packed MSB-first, two per three bytes
//!   (`b0 << 4 | b1 >> 4`, `(b1 & 15) << 8 | b2`), rows are `width × 1.5` bytes with no padding. Stored rows
//!   `0..h0` are the even output rows and the rows from `h0` on the odd ones (`h0 = (height + 1) / 2`, height
//!   odd); the strip table has one gap where the second field starts (follow the offsets) and omits the last row
//!   of the second field, which sits in the bytes right after the last strip. Derived by black-box analysis of
//!   six CC0 files (C5060WZ, C7070WZ, SP-510UZ, SP-550UZ, SP-565UZ, SP-570UZ). Their Exif `CFAPattern` states the
//!   layout (it matches the embedded thumbnails), the maker note's entries `0x1017`/`0x1018` hold the red and
//!   blue gain (first value, 256 = 1.0; checked against neutral areas of the thumbnail), and nothing states a
//!   black level or an active area.
//! - 12-bit blocks (E-300, E-500, E-330; the E-M5 Mark II and PEN-F high-resolution files): one strip of 12.8 bits
//!   per pixel, `width * 8 / 5` bytes per row, made of 16-byte blocks. A block is 15 sample bytes followed by one
//!   pad byte that is always zero; the 15 bytes are a little-endian 120-bit string holding ten 12-bit samples, sample
//!   `i` at bits `[12 i, 12 i + 12)`, i.e. per byte triple `s0 = b0 | (b1 & 15) << 8`, `s1 = b1 >> 4 | b2 << 4`, left
//!   to right. The layout was first recalled only roughly ("ten pixels, then a skipped byte"); the pad position
//!   (last), the bit order and the pixel order were fixed by measurement on the CC0 files 2878 (E-300), 3540 (E-500)
//!   and 3624 (E-330), as was the sensor-origin RGGB anchoring (the maker-note crop offsets are odd). The E-M5 Mark II
//!   and PEN-F files were only checked for smoothness, not for CFA anchoring or black level.
//! - Compressed ORF sensor data is unsupported. The previous decoder was removed pending provenance
//!   resolution; embedded previews remain available. See `docs/orf-compression.md`.
//! - The colour-filter layout is the file's Exif `CFAPattern`: GRBG on the E-1 and E-400, RGGB on the XZ-2, where
//!   the colours of the decoded mosaic follow the embedded JPEG best with exactly that layout
//!   (`corpus_orf_cfa_patterns`). The samples' active areas start at even offsets, so they can't tell whether the
//!   pattern is anchored at the sensor origin (assumed) or at the active area. Files without a usable tag fall back
//!   to the green diagonal found from the data, which can't tell red from blue (GRBG or RGGB is assumed).

use crate::shared::{cfa_from_exif, white_from_data_sourced};
use crate::tiffraw::{Packing, read_image};
use crate::unpack::unpack_msb;
use crate::{BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result};
use craft_tiff::image::chunk_bytes;
use craft_tiff::makernote::MakerNote;
use craft_tiff::{Ifd, Tiff, Value, makernote, tags as t};
use rayon::prelude::*;

const CAMERA_SETTINGS: u16 = 0x2020;
const IMAGE_PROCESSING: u16 = 0x2040;
const PREVIEW_START: u16 = 0x0101;
const PREVIEW_LENGTH: u16 = 0x0102;
const WB_RB: u16 = 0x0100;
/// ImageProcessing WB_GLevel: the green level the red and blue levels are relative to (256 when absent).
const WB_G: u16 = 0x011f;
const BLACK: u16 = 0x0600;
const MN_WB_RED: u16 = 0x1017;
const MN_WB_BLUE: u16 = 0x1018;
const MN_BLACK: u16 = 0x1012;
const MN_VALID_BITS: u16 = 0x102c;
const CROP: [u16; 4] = [0x0612, 0x0613, 0x0614, 0x0615];

/// The Olympus maker note.
pub(crate) fn maker_note(bytes: &[u8], tiff: &Tiff) -> Option<MakerNote> {
    let make = tiff.find(t::MAKE).and_then(|e| e.value.as_str()).unwrap_or_default().to_string();
    let e = tiff.exif().and_then(|e| e.get(t::MAKER_NOTE))?;
    makernote::parse_makernote(bytes, e.offset, e.count() as u64, tiff.order, &make)
}

/// A maker-note sub-directory: an IFD pointer (offset relative to the note's base) in new-style notes, or the
/// IFD stored inline as an undefined blob (offsets relative to the base) in old-style ones.
pub(crate) fn sub_ifd(bytes: &[u8], mn: &MakerNote, tag: u16) -> Option<Ifd> {
    let e = mn.ifd.get(tag)?;
    let at = match &e.value {
        Value::Undefined(_) | Value::Byte(_) => e.offset,
        _ => mn.base.checked_add(mn.ifd.u64(tag)?)?,
    };
    let opts = craft_tiff::ParseOptions { max_ifds: 4, max_depth: 1, follow_children: false, ..Default::default() };
    craft_tiff::parse_ifd_at(bytes, at, mn.order, mn.base, false, &opts).ok().map(|(i, _)| i)
}

/// Black level of a body without ImageProcessing: the main note's `0x1012` BlackLevel (the mean of its four values)
/// in `0x102c` ValidBits units, scaled to the decoded `bits`. The E-10 states 32 at 10 bits, and its 12-bit samples
/// bottom out at 124 to 132; the two-field compacts state 0 at 12 bits.
fn maker_note_black(m: &MakerNote, bits: u32) -> Option<f32> {
    let bits = bits.min(16);
    let v = m.ifd.f64s(MN_BLACK).filter(|v| v.len() == 4)?;
    let valid = m.ifd.u64(MN_VALID_BITS).and_then(|b| u32::try_from(b).ok()).filter(|b| (8..=bits).contains(b))?;
    let level = v.iter().sum::<f64>() / 4.0 * f64::from(1u32 << (bits - valid));
    (level >= 0.0 && level < f64::from(1u32 << bits) / 4.0).then_some(level as f32)
}

/// Unpack one row of 12-bit samples stored as little-endian 32-bit words read MSB-first.
pub(crate) fn unpack_row_le32_msb(src: &[u8], bits: u32, out: &mut [u16]) {
    let swapped: Vec<u8> = src
        .chunks(4)
        .flat_map(|c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            [w[3], w[2], w[1], w[0]]
        })
        .collect();
    unpack_msb(&swapped, bits, out);
}

/// Unpack one row of 12-bit samples stored as 16-byte blocks: ten little-endian 12-bit fields in bytes 0..15, byte 15
/// a pad. `src` must hold exactly `out.len() / 10` blocks (`out.len()` a multiple of 10).
pub(crate) fn unpack_row_blocks16(src: &[u8], out: &mut [u16]) -> Result<()> {
    if !out.len().is_multiple_of(10) || src.len() != out.len() / 10 * 16 {
        return Err(RawError::Corrupt("ORF 12-bit block row has the wrong size".into()));
    }
    let (blocks, _) = src.as_chunks::<16>();
    let (pixels, _) = out.as_chunks_mut::<10>();
    for (block, px) in blocks.iter().zip(pixels) {
        for k in 0..5 {
            let (b0, b1, b2) = (block[3 * k] as u16, block[3 * k + 1] as u16, block[3 * k + 2] as u16);
            px[2 * k] = b0 | (b1 & 15) << 8;
            px[2 * k + 1] = b1 >> 4 | b2 << 4;
        }
    }
    Ok(())
}

/// Whether the strip is the 12.8 bits per pixel block layout: a single strip, rows a whole number of 16-byte
/// blocks of ten pixels.
fn is_block16(w: usize, n: usize, total: u64, chunks: usize) -> bool {
    chunks == 1 && w.is_multiple_of(10) && total.checked_mul(10) == (n as u64).checked_mul(16)
}

/// The layout found from the samples, for files without the Exif tag: GRBG when the greens sit on the main
/// diagonal of the 2×2 cell at the sensor origin, else RGGB. Over 32×32-pixel blocks of `a` (every fourth in both
/// directions) it compares the block totals of the two sites on each diagonal: the two greens of a block see the
/// same light, red and blue rarely do. Totals rather than single pixels, so that fine texture, which makes
/// neighbouring greens differ, doesn't outweigh a small difference between red and blue.
pub(crate) fn cfa_from_data(d: &[u16], w: usize, a: Rect) -> Cfa {
    const BLOCK: usize = 32;
    let (x0, y0) = (a.x.saturating_add(1) & !1, a.y.saturating_add(1) & !1);
    let across = a.x.saturating_add(a.width).min(w).saturating_sub(x0) / BLOCK;
    let down = a.y.saturating_add(a.height).saturating_sub(y0) / BLOCK;
    let (mut main, mut anti) = (0u64, 0u64);
    for by in (0..down).step_by(4) {
        for bx in (0..across).step_by(4) {
            let mut sums = [0i64; 4];
            for y in 0..BLOCK {
                let row = (y0 + by * BLOCK + y).checked_mul(w).and_then(|r| r.checked_add(x0 + bx * BLOCK));
                // rows past the end of the samples: nothing more to read
                let Some(row) = row.and_then(|start| d.get(start..start.checked_add(BLOCK)?)) else { break };
                for (x, &v) in row.iter().enumerate() {
                    sums[(y & 1) * 2 + (x & 1)] += v as i64;
                }
            }
            let [s0, s1, s2, s3] = sums;
            main += (s0 - s3).unsigned_abs();
            anti += (s1 - s2).unsigned_abs();
        }
    }
    Cfa::bayer_static(if main < anti { "GRBG" } else { "RGGB" })
}

/// Byte offsets of the stored rows of the two-field 12-bit layout, or `None` unless every condition holds
/// exactly: odd height, even width, `strips` (offset, byte count; the whole table, not just the strips that fit the
/// rows-per-strip grid, since the field seam leaves a short strip) that are whole rows summing to
/// `width × height × 1.5` bytes, exactly one gap between consecutive strips (the second field starts right after
/// it, at row `(height + 1) / 2`), and a last row, right after the final strip, inside the file. Returns
/// `height + 1` offsets.
fn field_rows(strips: &[(u64, u64)], w: usize, h: usize, file_len: usize) -> Option<Vec<usize>> {
    let rb = w.checked_mul(3)? / 2;
    if rb == 0 || !w.is_multiple_of(2) || h.is_multiple_of(2) || h < 3 || strips.len() < 2 {
        return None;
    }
    let h0 = h.div_ceil(2);
    let total = strips.iter().try_fold(0u64, |a, s| a.checked_add(s.1))?;
    if total != (rb as u64).checked_mul(h as u64)? {
        return None;
    }
    let mut rows = Vec::with_capacity(h + 1);
    let mut gaps = 0;
    let mut end = None;
    for &(offset, len) in strips {
        let next_end = offset.checked_add(len).filter(|e| len > 0 && len % rb as u64 == 0 && *e <= file_len as u64)?;
        match end {
            Some(e) if offset < e => return None,
            // the gap must be the field seam
            Some(e) if offset > e => {
                if rows.len() != h0 {
                    return None;
                }
                gaps += 1;
            }
            _ => {}
        }
        end = Some(next_end);
        let (offset, rb) = (usize::try_from(offset).ok()?, rb);
        rows.extend((0..usize::try_from(len).ok()? / rb).map(|r| offset + r * rb));
    }
    let tail = usize::try_from(end?).ok()?;
    if gaps != 1 || rows.len() != h || tail.checked_add(rb)? > file_len {
        return None;
    }
    rows.push(tail);
    Some(rows)
}

/// Decode the supported uncompressed Olympus ORF layouts into a mosaic raw image.
pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("TIFF without IFDs".into()))?;
    let info = ifd0.image()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let n = w.checked_mul(h).filter(|n| *n > 0 && *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let chunks = info.chunks(bytes.len() as u64);
    let total = chunks.iter().fold(0u64, |a, c| a.saturating_add(c.len));
    let stated = cfa_from_exif(&tiff);
    let mut two_field = false;
    let mn = maker_note(bytes, &tiff);
    let ip = mn.as_ref().and_then(|m| sub_ifd(bytes, m, IMAGE_PROCESSING));
    let (mut data, bits, from_word16) = if info.compression != 1 {
        return Err(RawError::Unsupported(format!("ORF compression {}", info.compression)));
    } else if total >= (n as u64) * 2 {
        let d = read_image(bytes, &info, tiff.order, Packing::Word16, limits)?;
        (d, 16, true)
    } else if is_block16(w, n, total, chunks.len()) {
        let src = chunk_bytes(bytes, &chunks[0]).ok_or_else(|| RawError::Corrupt("ORF strip outside file".into()))?;
        let stride = w / 10 * 16;
        if src.len() as u64 != total || src.len() / stride != h {
            return Err(RawError::Corrupt("ORF strip shorter than its rows".into()));
        }
        let d = if mode == Mode::Full || stated.is_none() {
            let mut d = vec![0u16; n];
            d.par_chunks_mut(w).enumerate().try_for_each(|(y, row)| unpack_row_blocks16(&src[y * stride..(y + 1) * stride], row))?;
            d
        } else {
            Vec::new()
        };
        (RawData::U16(d), 12, false)
    } else if info.bits() == 12 && total * 8 >= (n as u64) * 12 && total * 8 < (n as u64) * 13 && chunks.len() == 1 {
        let src = chunk_bytes(bytes, &chunks[0]).ok_or_else(|| RawError::Corrupt("ORF strip outside file".into()))?;
        // Only the declared 12-bit layout is supported here. A compressed stream in a 16-bit
        // ORF container must not fall through to packed decoding just because its length fits.
        // the strip as it is in the file (clipped at its end), not as its tag states it: one too short
        // for the samples is a truncated file, refused before they are allocated
        if (src.len() as u64).saturating_mul(8) < (n as u64).saturating_mul(12) {
            return Err(RawError::Corrupt("ORF strip shorter than its samples".into()));
        }
        // the depth is fixed here, so a header-only probe needs the samples only when the file doesn't state its
        // colour-filter layout
        let d = if mode == Mode::Full || stated.is_none() {
            let stride = src.len() / h;
            let mut d = vec![0u16; n];
            d.par_chunks_mut(w).enumerate().for_each(|(y, row)| unpack_row_le32_msb(&src[y * stride..(y + 1) * stride], 12, row));
            d
        } else {
            Vec::new()
        };
        (RawData::U16(d), 12, false)
    } else if let Some(rows) = (info.bits() == 12 && info.samples_per_pixel == 1 && info.planar != 2 && info.offsets.len() == info.byte_counts.len())
        .then(|| field_rows(&info.offsets.iter().copied().zip(info.byte_counts.iter().copied()).collect::<Vec<_>>(), w, h, bytes.len()))
        .flatten()
    {
        two_field = true;
        let d = if mode == Mode::Full || stated.is_none() {
            let (rb, h0) = (w * 3 / 2, h.div_ceil(2));
            let mut d = vec![0u16; n];
            d.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                let stored = if y % 2 == 0 { y / 2 } else { h0 + y / 2 };
                // every offset was checked against the file by `field_rows`
                if let Some(src) = rows.get(stored).and_then(|&at| bytes.get(at..at + rb)) {
                    unpack_msb(src, 12, row);
                }
            });
            d
        } else {
            Vec::new()
        };
        (RawData::U16(d), 12, false)
    } else {
        return Err(RawError::Unsupported("Olympus compressed ORF".into()));
    };
    let RawData::U16(ref mut samples) = data else { return Err(RawError::Unsupported("float ORF".into())) };
    let bits = if from_word16 && bits == 16 && samples.iter().step_by(7).filter(|v| *v & 15 != 0).count() * 700 <= n {
        // 12-bit values stored in the top bits
        samples.par_iter_mut().for_each(|v| *v >>= 4);
        12
    } else if from_word16 && bits == 16 {
        let mx = samples.iter().step_by(31).max().copied().unwrap_or(0);
        if mx < 4096 {
            12
        } else if mx < 16384 {
            14
        } else {
            16
        }
    } else {
        bits
    };

    let active = match ip.as_ref().map(|i| CROP.map(|tag| i.u64(tag).map(|v| v as usize))) {
        // checked: the tags are file data; `x + cw` on a hostile value would wrap past the bounds
        Some([Some(x), Some(y), Some(cw), Some(ch)])
            if cw > 0 && ch > 0 && x.checked_add(cw).is_some_and(|end| end <= w) && y.checked_add(ch).is_some_and(|end| end <= h) =>
        {
            Rect::new(x, y, cw, ch)
        }
        _ => Rect::new(0, 0, w, h),
    };
    let cfa = stated.unwrap_or_else(|| cfa_from_data(samples, w, active));
    let (black, black_source) = match ip.as_ref().and_then(|i| i.f64s(BLACK)).as_deref() {
        Some(v @ [_, _, _, _]) => {
            let a = cfa.shifted(active.x, active.y);
            let values = a.pattern.iter().map(|&c| [v[0], (v[1] + v[2]) / 2.0, v[3]][c as usize] as f32).collect();
            (BlackLevel { repeat_rows: 2, repeat_cols: 2, values, ..Default::default() }, crate::LevelSource::Metadata)
        }
        _ => match mn.as_ref().filter(|_| ip.is_none()).and_then(|m| maker_note_black(m, bits)) {
            Some(b) => (BlackLevel::uniform(b), crate::LevelSource::Metadata),
            None => (BlackLevel::uniform(0.0), crate::LevelSource::Fallback),
        },
    };
    let wb = ip
        .as_ref()
        .and_then(|i| Some((i.f64s(WB_RB)?, i.f64(WB_G).filter(|g| *g > 0.0 && g.is_finite()).unwrap_or(256.0))))
        .filter(|(v, _)| v.len() >= 2 && v[0] > 0.0 && v[1] > 0.0)
        .map(|(v, g)| [(v[0] / g) as f32, 1.0, (v[1] / g) as f32])
        .or_else(|| {
            // the red and blue gains lead the main note's entries 0x1017/0x1018 on the two-field compacts and on
            // the E-10 (16-bit words, no ImageProcessing; 384 and 406 on its CC0 file 5424)
            let m = mn.as_ref().filter(|_| two_field || ip.is_none())?;
            let g = |tag| m.ifd.f64s(tag).and_then(|v| v.first().copied()).filter(|g| *g > 25.6 && *g < 2560.0);
            Some([(g(MN_WB_RED)? / 256.0) as f32, 1.0, (g(MN_WB_BLUE)? / 256.0) as f32])
        });
    let (white, white_source) = white_from_data_sourced(samples, bits);
    let img = RawImage {
        format: RawFormat::Orf,
        width: w,
        height: h,
        cpp: 1,
        data,
        cfa: Some(cfa),
        bits,
        black,
        white: vec![white],
        active_area: active,
        crop: Rect::new(0, 0, active.width, active.height),
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: ColorData::default(),
        wb_multipliers: wb,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: crate::LevelSources { black: black_source, white: white_source },
    };
    img.validate_for(mode)?;
    Ok(img)
}

/// The large preview JPEG referenced by CameraSettings `PreviewImageStart/Length` (relative to the note base).
pub(crate) fn preview(bytes: &[u8]) -> Option<&[u8]> {
    let tiff = Tiff::parse(bytes).ok()?;
    let mn = maker_note(bytes, &tiff)?;
    let cs = sub_ifd(bytes, &mn, CAMERA_SETTINGS)?;
    let start = mn.base.checked_add(cs.u64(PREVIEW_START)?)? as usize;
    let len = cs.u64(PREVIEW_LENGTH)? as usize;
    bytes.get(start..start.checked_add(len)?.min(bytes.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EXIF_CFA_PATTERN;
    use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter};

    fn orf(w: u32, h: u32, bits: u16, strip: Vec<u8>) -> Vec<u8> {
        orf_with(w, h, bits, strip, None)
    }

    /// A minimal ORF with one strip in IFD0 and, optionally, an Exif `CFAPattern`.
    fn orf_with(w: u32, h: u32, bits: u16, strip: Vec<u8>, cfa_pattern: Option<&[u8]>) -> Vec<u8> {
        orf_with_note(w, h, bits, strip, cfa_pattern, None)
    }

    /// [`orf_with`] plus an optional maker note blob in the Exif IFD.
    fn orf_with_note(w: u32, h: u32, bits: u16, strip: Vec<u8>, cfa_pattern: Option<&[u8]>, note: Option<Vec<u8>>) -> Vec<u8> {
        let mut ifd = IfdBuilder::new();
        ifd.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
        ifd.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
        ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![bits]));
        ifd.set(t::COMPRESSION, Value::Short(vec![1]));
        ifd.set(t::PHOTOMETRIC, Value::Short(vec![1]));
        ifd.set(t::MAKE, Value::Ascii("OLYMPUS IMAGING CORP.".into()));
        ifd.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![strip] });
        if cfa_pattern.is_some() || note.is_some() {
            let mut exif = IfdBuilder::new();
            if let Some(p) = cfa_pattern {
                exif.set(EXIF_CFA_PATTERN, Value::Undefined(p.to_vec()));
            }
            if let Some(n) = note {
                exif.set(t::MAKER_NOTE, Value::Undefined(n));
            }
            ifd.set_child(t::EXIF_IFD, exif);
        }
        let mut b = TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).unwrap();
        b[2] = b'R';
        b[3] = b'O';
        b
    }

    /// A 16-bit container with a short sensor stream is not the supported 12-bit packed layout.
    /// Both entry points must report unsupported rather than returning fabricated sensor pixels.
    #[test]
    fn compressed_sensor_data_is_unsupported_in_probe_and_decode() {
        for strip_len in [8, 24, 25] {
            let bytes = orf_with(4, 4, 16, vec![0; strip_len], Some(&[2, 0, 2, 0, 0, 1, 1, 2]));
            assert!(matches!(crate::decode(&bytes), Err(RawError::Unsupported(why)) if why == "Olympus compressed ORF"));
            assert!(matches!(crate::probe_info(&bytes), Err(RawError::Unsupported(why)) if why == "Olympus compressed ORF"));
        }
    }

    /// Strip byte counts that add up past `u64::MAX` (LONG8 `[u64::MAX, 1]`) are an error or a picture, not an overflow.
    #[test]
    fn strip_byte_counts_past_u64_max_do_not_overflow() {
        let file = crate::shared::tests::huge_strip_counts(orf(4, 2, 16, vec![0; 16]));
        for mode in [Mode::Header, Mode::Full] {
            let _ = super::decode(&file, mode, &Limits::default());
        }
    }

    /// An old-style little-endian Olympus maker note (`OLYMP\0`, version 1, then the IFD) of SHORT (type 3) or
    /// UNDEFINED (type 7) entries. Values longer than four bytes follow the IFD; their offsets count from the TIFF
    /// header, so `at` is where the note sits in the file.
    fn olymp_note(at: usize, entries: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
        let ifd_end = 8 + 2 + 12 * entries.len() + 4;
        let (mut b, mut extra) = (b"OLYMP\0\x01\0".to_vec(), Vec::new());
        b.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, typ, value) in entries {
            b.extend_from_slice(&tag.to_le_bytes());
            b.extend_from_slice(&typ.to_le_bytes());
            b.extend_from_slice(&(if *typ == 3 { value.len() / 2 } else { value.len() } as u32).to_le_bytes());
            if value.len() <= 4 {
                let mut v = value.clone();
                v.resize(4, 0);
                b.extend_from_slice(&v);
            } else {
                b.extend_from_slice(&((at + ifd_end + extra.len()) as u32).to_le_bytes());
                extra.extend_from_slice(value);
            }
        }
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&extra);
        b
    }

    /// SHORT values, little-endian.
    fn shorts(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// A 16 × 4 ORF of 12-bit samples in 16-bit words with an [`olymp_note`] of `entries`, decoded.
    fn decode_noted(entries: &[(u16, u16, Vec<u8>)]) -> RawImage {
        let (w, h) = (16u32, 4u32);
        let words: Vec<u8> = (0..w * h).flat_map(|i| (((200 + i * 37) % 4096) as u16).to_le_bytes()).collect();
        let file = |at| orf_with_note(w, h, 16, words.clone(), Some(RGGB), Some(olymp_note(at, entries)));
        let at = file(0).windows(6).position(|b| b == b"OLYMP\0").unwrap();
        let bytes = file(at);
        assert_eq!(bytes.windows(6).position(|b| b == b"OLYMP\0"), Some(at));
        crate::decode(&bytes).unwrap()
    }

    /// An ImageProcessing sub-directory stored inline (old-style note) with one unrelated entry and no levels.
    fn bare_image_processing() -> (u16, u16, Vec<u8>) {
        let mut ifd = 1u16.to_le_bytes().to_vec();
        ifd.extend_from_slice(&[0, 0, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
        ifd.extend_from_slice(&0u32.to_le_bytes());
        (IMAGE_PROCESSING, 7, ifd)
    }

    /// An ImageProcessing sub-directory stored inline (old-style note) of SHORT entries of at most two values.
    fn image_processing(entries: &[(u16, &[u16])]) -> (u16, u16, Vec<u8>) {
        let mut ifd = (entries.len() as u16).to_le_bytes().to_vec();
        for (tag, v) in entries {
            let mut value = shorts(v);
            value.resize(4, 0);
            ifd.extend([&tag.to_le_bytes()[..], &3u16.to_le_bytes(), &(v.len() as u32).to_le_bytes(), &value].concat());
        }
        ifd.extend_from_slice(&0u32.to_le_bytes());
        (IMAGE_PROCESSING, 7, ifd)
    }

    /// The red and blue levels are relative to WB_GLevel (`0x011f`), 256 when it is absent or zero (as in
    /// PhotoCraft's reader).
    #[test]
    fn as_shot_white_balance_is_relative_to_the_green_level() {
        let wb = |g: Option<u16>| {
            let mut e = vec![(WB_RB, &[512u16, 384][..])];
            let g = g.map(|g| [g]);
            e.extend(g.as_ref().map(|g| (WB_G, &g[..])));
            decode_noted(&[image_processing(&e)]).wb_multipliers
        };
        assert_eq!(wb(None), Some([2.0, 1.0, 1.5]));
        assert_eq!(wb(Some(256)), Some([2.0, 1.0, 1.5]));
        assert_eq!(wb(Some(512)), Some([1.0, 1.0, 0.75]));
        assert_eq!(wb(Some(0)), Some([2.0, 1.0, 1.5]));
    }

    /// The CameraSettings preview (`0x0101` start, relative to the note's base, and `0x0102` length) is the file's
    /// embedded preview; a length past the end is clamped to the file.
    #[test]
    fn camera_settings_preview_is_the_embedded_preview() {
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 8, 0, 1, 0, 1, 1, 1, 0x11, 0];
        jpeg.extend([0x55; 40]);
        jpeg.extend([0xff, 0xd9]);
        let long = |v: u32| v.to_le_bytes().to_vec();
        // the note holds CameraSettings (30 bytes, inline) and the JPEG, both after its 38-byte IFD
        let file = |at: usize, len: u32| {
            let mut cs = 2u16.to_le_bytes().to_vec();
            cs.extend([&PREVIEW_START.to_le_bytes()[..], &4u16.to_le_bytes(), &1u32.to_le_bytes(), &long((at + 38 + 30) as u32)].concat());
            cs.extend([&PREVIEW_LENGTH.to_le_bytes()[..], &4u16.to_le_bytes(), &1u32.to_le_bytes(), &long(len)].concat());
            cs.extend(0u32.to_le_bytes());
            let note = olymp_note(at, &[(CAMERA_SETTINGS, 7, cs), (0xfff0, 7, jpeg.clone())]);
            orf_with_note(16, 4, 16, vec![0; 128], Some(RGGB), Some(note))
        };
        let at = file(0, 0).windows(6).position(|b| b == b"OLYMP\0").unwrap();
        let bytes = file(at, jpeg.len() as u32);
        assert_eq!(preview(&bytes), Some(&jpeg[..]));
        assert_eq!(crate::embedded_preview(&bytes), Some(jpeg.clone()));
        let past = file(at, u32::MAX);
        assert!(preview(&past).is_some_and(|p| p.starts_with(&jpeg) && p.len() < past.len()));
    }

    /// storytold/lightcraft#1273: the E-10 (16-bit words, no ImageProcessing sub-directory) keeps its red and blue gains in
    /// maker-note entries `0x1017`/`0x1018`, as the two-field compacts do.
    #[test]
    fn as_shot_white_balance_from_the_main_maker_note() {
        let wb = |red: u16, blue: Option<u16>| {
            let mut e = vec![(MN_WB_RED, 3, shorts(&[red, 64]))];
            e.extend(blue.map(|b| (MN_WB_BLUE, 3, shorts(&[b, 64]))));
            decode_noted(&e).wb_multipliers
        };
        assert_eq!(wb(384, Some(406)), Some([1.5, 1.0, 406.0 / 256.0]));
        // both gains are needed, within 0.1 to 10 (25.6 to 2560 in 256 units)
        assert_eq!(wb(384, None), None);
        assert_eq!(wb(0, Some(406)), None);
        assert_eq!(wb(25, Some(406)), None);
        assert_eq!(wb(384, Some(2560)), None);
        assert_eq!(wb(27, Some(2559)), Some([27.0 / 256.0, 1.0, 2559.0 / 256.0]));
        assert_eq!(decode_noted(&[]).wb_multipliers, None);
        // a body with ImageProcessing but without its levels doesn't fall back to these entries
        let e = [bare_image_processing(), (MN_WB_RED, 3, shorts(&[384, 64])), (MN_WB_BLUE, 3, shorts(&[406, 64]))];
        assert_eq!(decode_noted(&e).wb_multipliers, None);
    }

    /// storytold/lightcraft#1273: a body without ImageProcessing states its black level in the main note, `0x1012` (four
    /// values) in `0x102c` ValidBits units: the E-10's 32 at 10 bits is 128 on its 12-bit samples. The two-field
    /// compacts state 0 at 12 bits.
    #[test]
    fn black_level_from_the_main_maker_note() {
        let black = |level: u16, valid: Option<u16>| {
            let mut e = vec![(MN_BLACK, 3, shorts(&[level; 4]))];
            e.extend(valid.map(|v| (MN_VALID_BITS, 3, shorts(&[v, 0]))));
            let r = decode_noted(&e);
            assert_eq!(r.bits, 12);
            r.black.values
        };
        assert_eq!(black(32, Some(10)), vec![128.0]);
        assert_eq!(black(0, Some(12)), vec![0.0]);
        assert_eq!(black(32, Some(12)), vec![32.0]);
        // the four values are averaged
        let e = [(MN_BLACK, 3, shorts(&[30, 32, 32, 34])), (MN_VALID_BITS, 3, shorts(&[10, 0]))];
        assert_eq!(decode_noted(&e).black.values, vec![128.0]);
        // no ValidBits, ValidBits above the samples' depth, a level of a quarter of the range or more: no black
        assert_eq!(black(32, None), vec![0.0]);
        assert_eq!(black(32, Some(14)), vec![0.0]);
        assert_eq!(black(256, Some(10)), vec![0.0]);
        // ImageProcessing present (without BlackLevel2): the main note isn't read
        let e = [bare_image_processing(), (MN_BLACK, 3, shorts(&[32; 4])), (MN_VALID_BITS, 3, shorts(&[10, 0]))];
        assert_eq!(decode_noted(&e).black.values, vec![0.0]);
    }

    /// A 12-bit mosaic stored as 16-bit words: `site(cx, cy)` gives the four values of the 2×2 cell at (cx, cy).
    fn mosaic(w: usize, h: usize, site: impl Fn(usize, usize) -> [u16; 4]) -> Vec<u8> {
        (0..w * h).flat_map(|i| site(i % w / 2, i / w / 2)[((i / w) & 1) * 2 + ((i % w) & 1)].to_le_bytes()).collect()
    }

    /// Smooth scene texture shared by the four sites of a cell.
    fn shade(cx: usize, cy: usize) -> u16 {
        ((cx * 7 + cy * 13) % 23) as u16
    }

    const BGGR: &[u8] = &[2, 0, 2, 0, 2, 1, 1, 0];
    const GRBG: &[u8] = &[2, 0, 2, 0, 1, 0, 2, 1];
    // repeat counts in the other byte order
    const RGGB: &[u8] = &[0, 2, 0, 2, 0, 1, 1, 2];
    const GBRG: &[u8] = &[0, 2, 0, 2, 1, 2, 0, 1];

    fn layout(bytes: &[u8]) -> String {
        let r = crate::decode(bytes).unwrap();
        assert_eq!(crate::probe_info(bytes).unwrap(), r.info());
        r.cfa.unwrap().name()
    }

    #[test]
    fn exif_cfa_pattern_states_the_layout() {
        let (w, h) = (160usize, 144usize);
        // samples whose green diagonal alone would say RGGB
        let px = mosaic(w, h, |cx, cy| [600, 1000, 1000, 300].map(|v| v + shade(cx, cy)));
        for (tag, name) in [(BGGR, "BGGR"), (GRBG, "GRBG"), (RGGB, "RGGB"), (GBRG, "GBRG")] {
            assert_eq!(layout(&orf_with(w as u32, h as u32, 16, px.clone(), Some(tag))), name);
        }
        // not a 2×2 Bayer cell (four greens, greens side by side, a fourth colour), other repeat counts, wrong
        // lengths: the samples decide
        let unusable: [&[u8]; 8] = [
            &[2, 0, 2, 0, 1, 1, 1, 1],
            &[2, 0, 2, 0, 1, 1, 0, 2],
            &[2, 0, 2, 0, 0, 1, 1, 3],
            &[3, 0, 3, 0, 0, 1, 1, 2],
            &[2, 2, 2, 0, 0, 1, 1, 2],
            &[2, 0, 2, 0, 1, 0],
            &[2, 0, 2, 0, 2, 1, 1, 0, 0],
            &[],
        ];
        for (site, name) in [([600, 1000, 1000, 300], "RGGB"), ([1000, 600, 300, 1000], "GRBG")] {
            let px = mosaic(w, h, |cx, cy| site.map(|v| v + shade(cx, cy)));
            assert_eq!(layout(&orf(w as u32, h as u32, 16, px.clone())), name);
            for tag in unusable {
                assert_eq!(layout(&orf_with(w as u32, h as u32, 16, px.clone(), Some(tag))), name, "{tag:?}");
            }
        }
    }

    #[test]
    fn packed_12_bit_probe_agrees_with_decode() {
        let (w, h) = (16usize, 4usize);
        let strip: Vec<u8> = (0..w * h * 3 / 2).map(|i| (i * 37 % 251) as u8).collect();
        for tag in [Some(BGGR), None] {
            let bytes = orf_with(w as u32, h as u32, 12, strip.clone(), tag);
            let r = crate::decode(&bytes).unwrap();
            assert_eq!((r.bits, r.data.len()), (12, w * h));
            assert_eq!(r.cfa.as_ref().unwrap().name(), if tag.is_some() { "BGGR" } else { "RGGB" });
            assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
        }
    }

    #[test]
    fn data_fallback_looks_past_fine_texture() {
        let (w, h) = (160usize, 144usize);
        // greens on the main diagonal, red and blue close; a checkerboard of detail makes the two greens of a cell
        // differ by far more than red differs from blue
        let detail = |cx: usize, cy: usize| if (cx + cy) & 1 == 0 { 150 } else { -150i32 };
        let site = |cx: usize, cy: usize| {
            let (g, d) = (1000 + shade(cx, cy) as i32, detail(cx, cy));
            [(g + d) as u16, 500, 520, (g - d) as u16]
        };
        assert_eq!(layout(&orf(w as u32, h as u32, 16, mosaic(w, h, site))), "GRBG");
        // the same scene with the greens on the other diagonal
        let px = mosaic(w, h, |cx, cy| {
            let [g0, r, b, g1] = site(cx, cy);
            [r, g0, g1, b]
        });
        assert_eq!(layout(&orf(w as u32, h as u32, 16, px)), "RGGB");
    }

    #[test]
    fn data_fallback_survives_any_area() {
        let d = vec![100u16; 64 * 64];
        let big = usize::MAX;
        for a in [
            Rect::new(0, 0, 64, 64),
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 31, 31),
            Rect::new(63, 63, 1, 1),
            Rect::new(0, 0, 4096, 4096),
            Rect::new(big, big, big, big),
            Rect::new(0, big - 40, 64, 40),
        ] {
            for w in [64, 0, 1, big] {
                assert_eq!(cfa_from_data(&d, w, a).name(), "RGGB", "{a:?} width {w}");
                assert_eq!(cfa_from_data(&[], w, a).name(), "RGGB", "{a:?} width {w}");
            }
        }
    }

    /// Output sample (x, y) of the two-field tests: distinct per site, all below 4096.
    fn field_px(x: usize, y: usize) -> u16 {
        ((y * 331 + x * 17 + 5) % 4096) as u16
    }

    /// 12 bits MSB-first, two samples per three bytes, for output row `y`.
    fn packed_row(w: usize, y: usize) -> Vec<u8> {
        (0..w)
            .step_by(2)
            .flat_map(|x| {
                let (a, b) = (field_px(x, y), field_px(x + 1, y));
                [(a >> 4) as u8, ((a & 15) << 4 | b >> 8) as u8, b as u8]
            })
            .collect()
    }

    /// A two-field file: strips A1 (two rows), A2 (one row), a gap, B (all but the last row of the second field),
    /// with the last row of the second field right after B, as the strip table of the old compacts has it (width 8,
    /// height 5). `table_gap` keeps the gap in the table (else the strips are contiguous and the table is wrong).
    fn two_field(table_gap: bool) -> Vec<u8> {
        let (w, h) = (8usize, 5usize);
        let h0 = h.div_ceil(2);
        // stored rows: the even output rows, then the odd ones (the last of them is past the picture)
        let stored: Vec<Vec<u8>> = (0..h0).map(|k| packed_row(w, 2 * k)).chain((0..h0).map(|k| packed_row(w, 2 * k + 1))).collect();
        let a1 = stored[..2].concat();
        let a2 = stored[2].clone();
        let gap = vec![0xffu8; 20];
        let b = stored[3].clone();
        let b2 = stored[4].clone();
        let last = stored[5].clone();
        let mut ifd = IfdBuilder::new();
        ifd.set(t::IMAGE_WIDTH, Value::Long(vec![w as u32]));
        ifd.set(t::IMAGE_LENGTH, Value::Long(vec![h as u32]));
        ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![12]));
        ifd.set(t::COMPRESSION, Value::Short(vec![1]));
        ifd.set(t::PHOTOMETRIC, Value::Short(vec![2]));
        ifd.set(t::MAKE, Value::Ascii("OLYMPUS OPTICAL CO.,LTD".into()));
        ifd.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![a1, a2, gap, b, b2, last] });
        let mut exif = IfdBuilder::new();
        exif.set(EXIF_CFA_PATTERN, Value::Undefined(BGGR.to_vec()));
        ifd.set_child(t::EXIF_IFD, exif);
        let mut b = TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).unwrap();
        b[2] = b'R';
        b[3] = b'S';
        // keep strips A1, A2, B1 and B2 in the table (drop the gap and the last row): four strips for a grid of three
        let le = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let at0 = le(&b, 4) as usize;
        let n = u16::from_le_bytes([b[at0], b[at0 + 1]]) as usize;
        for e in (0..n).map(|i| at0 + 2 + 12 * i) {
            let tag = u16::from_le_bytes([b[e], b[e + 1]]);
            if tag != t::STRIP_OFFSETS && tag != t::STRIP_BYTE_COUNTS {
                continue;
            }
            let at = le(&b, e + 8) as usize;
            let all: Vec<u32> = (0..6).map(|i| le(&b, at + 4 * i)).collect();
            let keep: Vec<u32> = if table_gap { vec![all[0], all[1], all[3], all[4]] } else { vec![all[0], all[1], all[2], all[3]] };
            b[e + 4..e + 8].copy_from_slice(&(keep.len() as u32).to_le_bytes());
            for (i, v) in keep.iter().enumerate() {
                b[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn two_field_12_bit_layout() {
        let bytes = two_field(true);
        let r = crate::decode(&bytes).unwrap();
        let want: Vec<u16> = (0..5).flat_map(|y| (0..8).map(move |x| field_px(x, y))).collect();
        assert_eq!((r.data.clone(), r.bits, r.width, r.height), (RawData::U16(want), 12, 8, 5));
        assert_eq!(r.cfa.as_ref().unwrap().name(), "BGGR");
        assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
        // the table without the gap: sizes no longer add up
        let e = crate::decode(&two_field(false)).map(|_| ());
        assert!(matches!(e, Err(RawError::Unsupported(_))), "{e:?}");
    }

    #[test]
    fn two_field_layout_needs_every_condition() {
        let chunk = |_index: usize, offset: u64, rows: u64| (offset, rows * 12);
        // width 8, height 5: rows 0..3 are the first field, the gap, rows 3..5, the last row after them
        let good = [chunk(0, 100, 2), chunk(1, 124, 1), chunk(2, 200, 2)];
        let rows = field_rows(&good, 8, 5, 236).unwrap();
        assert_eq!(rows, [100, 112, 124, 200, 212, 224]);
        // the last row must be inside the file
        assert!(field_rows(&good, 8, 5, 235).is_none());
        // the gap is the field seam: not after two rows, not absent, not twice
        assert!(field_rows(&[chunk(0, 100, 2), chunk(1, 140, 1), chunk(2, 164, 2)], 8, 5, 300).is_none());
        assert!(field_rows(&[chunk(0, 100, 2), chunk(1, 124, 1), chunk(2, 136, 2)], 8, 5, 300).is_none());
        assert!(field_rows(&[chunk(0, 100, 2), chunk(1, 124, 1), chunk(2, 200, 1), chunk(3, 300, 1)], 8, 5, 400).is_none());
        // overlapping strips, sizes that don't add up, even height, odd width, a single strip
        assert!(field_rows(&[chunk(0, 100, 2), chunk(1, 120, 1), chunk(2, 200, 2)], 8, 5, 300).is_none());
        assert!(field_rows(&[chunk(0, 100, 2), chunk(1, 124, 1), chunk(2, 200, 1)], 8, 5, 300).is_none());
        assert!(field_rows(&good, 8, 4, 300).is_none());
        assert!(field_rows(&good, 7, 5, 300).is_none());
        assert!(field_rows(&good[..1], 8, 5, 300).is_none());
    }

    #[test]
    fn word16_shifted_and_packed12() {
        let (w, h) = (16usize, 4usize);
        let px: Vec<u16> = (0..w * h).map(|i| ((i * 211) % 4096) as u16).collect();
        let words: Vec<u8> = px.iter().flat_map(|v| (v << 4).to_le_bytes()).collect();
        let bytes = orf(w as u32, h as u32, 16, words);
        assert_eq!(crate::probe(&bytes), Some(RawFormat::Orf));
        let r = crate::decode(&bytes).unwrap();
        assert_eq!((r.data.clone(), r.bits), (RawData::U16(px.clone()), 12));
        // 12-bit: MSB-first stream, every 32-bit word byte-swapped
        let mut be = Vec::new();
        for p in px.chunks(2) {
            let v = (p[0] as u32) << 12 | p[1] as u32;
            be.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        }
        let le: Vec<u8> = be.chunks(4).flat_map(|c| [c[3], c[2], c[1], c[0]]).collect();
        let bytes = orf(w as u32, h as u32, 12, le);
        assert_eq!(crate::decode(&bytes).unwrap().data, RawData::U16(px));
        let bytes = orf(w as u32, h as u32, 16, vec![0; 40]);
        assert!(matches!(crate::decode(&bytes), Err(RawError::Unsupported(_))));
    }

    /// Pack samples into 16-byte blocks: ten little-endian 12-bit fields, then a zero pad byte.
    fn pack_blocks(samples: &[u16]) -> Vec<u8> {
        samples
            .chunks(10)
            .flat_map(|c| {
                let mut v = 0u128;
                for (i, s) in c.iter().enumerate() {
                    v |= (*s as u128 & 0xfff) << (12 * i);
                }
                let mut b = v.to_le_bytes();
                b[15] = 0;
                b
            })
            .collect()
    }

    #[test]
    fn block16_round_trip() {
        let (w, h) = (20usize, 4usize);
        let samples: Vec<u16> = (0..w * h).map(|i| ((i * 509 + 17) % 4096) as u16).collect();
        let strip = pack_blocks(&samples);
        assert_eq!(strip.len(), w * h * 16 / 10);
        let mut out = vec![0u16; w * h];
        for (y, row) in out.chunks_mut(w).enumerate() {
            unpack_row_blocks16(&strip[y * 32..(y + 1) * 32], row).unwrap();
        }
        assert_eq!(out, samples);
        // a known block: s0 = b0 | (b1 & 15) << 8, s1 = b1 >> 4 | b2 << 4
        let mut px = [0u16; 10];
        let mut blk = [0u8; 16];
        blk[..3].copy_from_slice(&[0x21, 0x43, 0x65]);
        unpack_row_blocks16(&blk, &mut px).unwrap();
        assert_eq!(&px[..2], &[0x321, 0x654]);
        // the whole file through the decoder
        let bytes = orf_with(w as u32, h as u32, 16, strip, Some(RGGB));
        let r = crate::decode(&bytes).unwrap();
        assert_eq!(r.bits, 12);
        let RawData::U16(d) = &r.data else { panic!() };
        assert_eq!(d, &samples);
    }

    #[test]
    fn block16_routing() {
        assert!(is_block16(3360, 3360 * 2504, 13461504, 1));
        assert!(!is_block16(3360, 3360 * 2504, 3360 * 2504 * 12 / 8, 1)); // XZ-2: exactly 12.0 bpp
        assert!(!is_block16(3360, 3360 * 2504, 13461504, 2));
        assert!(!is_block16(3365, 3365 * 100, 3365 * 100 * 16 / 10, 1));
        // the 12.0 bpp layout still decodes through its own branch
        let bytes = orf_with(8, 2, 12, vec![0u8; 8 * 2 * 12 / 8], Some(RGGB));
        assert!(crate::decode(&bytes).is_ok());
    }

    #[test]
    fn block16_short_input_is_an_error() {
        let mut px = [0u16; 10];
        assert!(unpack_row_blocks16(&[0u8; 15], &mut px).is_err());
        assert!(unpack_row_blocks16(&[0u8; 32], &mut px).is_err());
        assert!(unpack_row_blocks16(&[], &mut px).is_err());
        let mut odd = [0u16; 7];
        assert!(unpack_row_blocks16(&[0u8; 16], &mut odd).is_err());
        // a truncated file never panics
        let bytes = orf_with(20, 4, 16, pack_blocks(&[5u16; 80]), Some(RGGB));
        for cut in [bytes.len() - 1, bytes.len() - 40, bytes.len() / 2, 100] {
            let _ = crate::decode(&bytes[..cut]);
        }
    }
}
