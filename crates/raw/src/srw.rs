//! Samsung SRW: the variants that store the sensor samples uncompressed.
//!
//! Sources: TIFF 6.0 (container: a big- or little-endian TIFF whose IFD0 points at sub-IFDs, one of them the raw
//! image), Exif 2.3 (`CFAPattern`, `PixelXDimension`, `PixelYDimension`; maker note `0x927c`), the ExifTool Samsung
//! tag-name documentation (`SensorAreas` `0xa010`, `EncryptionKey` `0xa020`, `WB_RGGBLevelsUncorrected` `0xa021`,
//! `WB_RGGBLevelsBlack` `0xa028`, `RawData` `0xa048`) and black-box analysis of CC0 samples from raw.pixls.us
//! (NX5, NX10, NX11, NX20, NX200, NX210, NX1000, NX1100, EX1, WB2000; every number below was measured on them):
//!
//! - The raw image is the IFD whose `Compression` is a private value (32769..=32773); its `CFAPattern` (`0x828e`)
//!   names the 2x2 layout, anchored at the first sample of the stored array. Checked on the files above: the colours
//!   of the decoded mosaic follow the camera's JPEG best with exactly that layout (`corpus_samsung_srw`).
//! - Compression 32769 (EX1, WB2000): one 16-bit little-endian word per sample (the TIFF is big-endian, the words are
//!   not); the strip holds exactly `width * height * 2` bytes.
//! - Compression 32770 with exactly `width * height * 12 / 8` strip bytes (NX5, NX10, NX11, NX20, NX200, NX210, NX1000,
//!   NX1100): 12-bit samples, two in three bytes, rows without padding. The NX5/NX10/NX11 files read them MSB-first,
//!   the others LSB-first; the file doesn't say which, the image does: in the wrong order neighbouring same-colour
//!   samples differ 5 to 11 times more (10 files, every one decided by that margin), see [`lsb_is_smoother`].
//! - Every other shape of those and of the other private values (32770 with fewer bytes: NX30, NX300, NX2000,
//!   EK-GN120; 32772: NX mini, NX3000, NX3300; 32773: NX1, NX500) is a compressed coding: reported as unsupported.
//! - The maker note is a plain IFD whose value offsets count from the start of the note, not of the TIFF. Several
//!   vectors are stored offset by `EncryptionKey`, 11 integers in every file (element `i` of a vector uses key
//!   `i mod 11`): `WB_RGGBLevelsUncorrected` and `WB_RGGBLevelsBlack` hold `value + key`, `RawData` holds
//!   `value - key`, found by trying both signs and keeping the one that gives plausible numbers (green levels
//!   4096 in all but one file, saturation levels 4095 or 16383, black levels 0 to 3, the dark-column level).
//! - `SensorAreas`: the second rectangle (left, top, right, bottom) is the camera JPEG's framing (its size equals the
//!   JPEG's in every file but WB2000's), used as the active area. Registered against the JPEG its centre agrees to
//!   within 2 pixels (NX10 family; the NX20 family's JPEG is lens-corrected, edges differ).
//! - The Exif picture size is smaller than the framing in one direction only on the WB2000 (16:9 from a 4:3 sensor):
//!   the JPEG is the centre of the framing (registration: top offset 366 = framing top 22 + (2736 - 2048) / 2).
//! - `RawData`: four black levels, then saturation levels (R, G1, G2, B twice). The EX1 saturates at 15881, not at its
//!   stated 16383 (85786 samples sit on the plateau), so a plateau below the stated level wins.
//! - Black: `WB_RGGBLevelsBlack` when it states one (NX5, NX10, NX11: 2 to 3, the right-hand dark columns measure
//!   3.5 to 4.5); otherwise the dark rows or columns outside the framing (EX1: eight rows at 2.3); otherwise 0.
//! - `WB_RGGBLevelsUncorrected` divided by its mean green is the as-shot gain: the renders of the 12 files match the
//!   camera JPEG at median ΔE76 2.2 to 7.6 (10.7 on the backlit WB2000 sample, 5.7 after registration).
use crate::shared::{black_from_columns_sourced, white_from_data_sourced};
use crate::unpack::{read_u16s, unpack_lsb, unpack_msb};
use crate::{BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result};
use craft_tiff::image::chunk_bytes;
use craft_tiff::{ByteOrder, Ifd, Tiff, tags as t};
use rayon::prelude::*;

const SENSOR_AREAS: u16 = 0xa010;
const ENCRYPTION_KEY: u16 = 0xa020;
const WB_UNCORRECTED: u16 = 0xa021;
const WB_BLACK: u16 = 0xa028;
const RAW_DATA: u16 = 0xa048;

/// The private `Compression` values of the raw image IFD.
const COMPRESSIONS: std::ops::RangeInclusive<u16> = 32769..=32773;

/// The maker-note IFD (offsets relative to the note's own start).
fn maker_note(bytes: &[u8], tiff: &Tiff) -> Option<Ifd> {
    let e = tiff.exif()?.get(t::MAKER_NOTE)?;
    let opts = craft_tiff::ParseOptions { max_ifds: 4, max_depth: 1, follow_children: false, ..Default::default() };
    craft_tiff::parse_ifd_at(bytes, e.offset, tiff.order, e.offset, false, &opts).ok().map(|(i, _)| i)
}

/// A key-offset vector: the entries of `tag` shifted by the maker note's key (`sign` is +1 or -1 times the key).
fn keyed(mn: &Ifd, tag: u16, sign: i64) -> Option<Vec<i64>> {
    let key = mn.u64s(ENCRYPTION_KEY).filter(|k| k.len() == 11)?;
    let vals = mn.f64s(tag)?;
    // 32-bit entries: the stored value wraps modulo 2^32 (a negative number stored as a LONG)
    // (a key value past i64::MAX must not overflow the multiply: only the low 32 bits are kept)
    Some(vals.iter().enumerate().map(|(i, &v)| i64::from((v as i64).wrapping_add((key[i % 11] as i64).wrapping_mul(sign)) as i32)).collect())
}

/// The raw image's IFD: the one with a private compression value.
fn raw_ifd(tiff: &Tiff) -> Option<&Ifd> {
    tiff.all_ifds().into_iter().find(|i| i.u16(t::COMPRESSION).is_some_and(|c| COMPRESSIONS.contains(&c)) && i.contains(t::IMAGE_WIDTH))
}

/// How the samples of an IFD are stored, when uncompressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coding {
    /// One little-endian 16-bit word per sample.
    Words,
    /// 12-bit samples, two in three bytes: read MSB-first (the NX5, NX10 and NX11 files) or LSB-first (the later
    /// bodies' files); the file does not say which, the smoother image does ([`lsb_is_smoother`]).
    Packed12,
}

fn coding(compression: u16, w: usize, h: usize, strip: u64) -> Option<Coding> {
    let n = (w as u64).checked_mul(h as u64)?;
    match compression {
        32769 if strip == n.checked_mul(2)? => Some(Coding::Words),
        32770 if w.is_multiple_of(2) && strip == n.checked_mul(12)? / 8 => Some(Coding::Packed12),
        _ => None,
    }
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("SRW without IFDs".into()))?;
    let raw = raw_ifd(&tiff).ok_or_else(|| RawError::Unsupported("Samsung raw image not found".into()))?;
    let info = raw.image()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let n = w.checked_mul(h).filter(|n| *n > 0 && *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let chunks = info.chunks(bytes.len() as u64);
    let strip = chunks.iter().fold(0u64, |a, c| a.saturating_add(c.len));
    let Some(coding) = coding(info.compression, w, h, strip).filter(|_| chunks.len() == 1) else {
        return Err(RawError::Unsupported(format!(
            "Samsung compressed raw (compression {}, {:.2} stored bits per sample)",
            info.compression,
            strip as f64 * 8.0 / n as f64
        )));
    };
    let data = if mode == Mode::Full {
        let src = chunk_bytes(bytes, &chunks[0]).ok_or_else(|| RawError::Corrupt("SRW strip outside file".into()))?;
        if src.len() as u64 != strip {
            return Err(RawError::Corrupt("SRW strip shorter than its rows".into()));
        }
        let mut d = vec![0u16; n];
        match coding {
            Coding::Words => read_u16s(src, ByteOrder::Little, &mut d),
            Coding::Packed12 => {
                let stride = w / 2 * 3;
                let unpack: fn(&[u8], u32, &mut [u16]) = if lsb_is_smoother(src, w, h) { unpack_lsb } else { unpack_msb };
                d.par_chunks_mut(w).zip(src.par_chunks(stride)).for_each(|(row, s)| unpack(s, 12, row));
            }
        }
        RawData::U16(d)
    } else {
        RawData::U16(Vec::new())
    };

    let mn = maker_note(bytes, &tiff);
    // the camera JPEG's framing (left, top, right, bottom), when it lies inside the stored array
    let active = mn
        .as_ref()
        .and_then(|m| m.u64s(SENSOR_AREAS))
        .filter(|v| v.len() == 8)
        .map(|v| (v[4] as usize, v[5] as usize, v[6] as usize, v[7] as usize))
        .filter(|&(l, tp, r, b)| l < r && tp < b && r <= w && b <= h)
        .map_or(Rect::new(0, 0, w, h), |(l, tp, r, b)| Rect::new(l, tp, r - l, b - tp));
    let cfa = match (raw.u64s(t::CFA_REPEAT_PATTERN_DIM).as_deref(), raw.bytes(t::CFA_PATTERN_EP)) {
        (Some([2, 2]), Some(p)) if p.len() == 4 && p.iter().all(|&c| c <= 2) => Cfa { width: 2, height: 2, pattern: p.to_vec() },
        _ => return Err(RawError::Unsupported("Samsung raw without a 2x2 CFAPattern".into())),
    };
    // saturation levels and black levels (R, G1, G2, B), in the file's own words
    let levels = mn.as_ref().and_then(|m| keyed(m, RAW_DATA, 1)).filter(|v| v.len() == 12);
    let RawData::U16(samples) = &data else { return Err(RawError::Unsupported("float SRW".into())) };
    // saturation: the file's level, or lower where the samples pile up (EX1 clips at 15882 of its stated 16383)
    let stated = levels.as_ref().filter(|v| v[4..].iter().all(|&x| x == v[4]) && (255..=65535).contains(&v[4])).map(|v| v[4] as f32);
    let bits = stated.map_or((info.bits() as u32).clamp(8, 16), |s| 32 - (s as u32).leading_zeros());
    let plateau = (!samples.is_empty()).then(|| white_from_data_sourced(&active_rows(samples, w, active), bits));
    let (white, white_source) = match (stated, plateau) {
        (Some(s), Some((p, p_source))) if p < s => (p, p_source),
        (Some(s), _) => (s, crate::LevelSource::Metadata),
        (None, Some(p)) => p,
        (None, None) => (4095.0, crate::LevelSource::Fallback),
    };
    let tagged_black =
        mn.as_ref().and_then(|m| keyed(m, WB_BLACK, -1)).filter(|v| v.len() == 4 && v.iter().all(|&x| (0..=u16::MAX as i64).contains(&x)));
    let (black, black_source) = match tagged_black {
        Some(v) if v.iter().any(|&x| x > 0) => {
            // RGGB-ordered levels -> per CFA position at the active area origin
            let a = cfa.shifted(active.x, active.y);
            let values = a.pattern.iter().map(|&c| [v[0], (v[1] + v[2]) / 2, v[3]][c as usize] as f32).collect();
            (BlackLevel { repeat_rows: 2, repeat_cols: 2, values, ..Default::default() }, crate::LevelSource::Metadata)
        }
        // the file states no black level: measure the masked band around the active area, when there is one
        _ => masked_black(samples, w, h, active, white),
    };
    // as-shot white balance: gains with the greens near 4096 (unity)
    let wb = mn.as_ref().and_then(|m| keyed(m, WB_UNCORRECTED, -1)).filter(|v| v.len() == 4 && v.iter().all(|&x| x > 0)).map(|v| {
        let g = (v[1] + v[2]) as f64 / 2.0;
        [(v[0] as f64 / g) as f32, 1.0, (v[3] as f64 / g) as f32]
    });
    let crop = picture_crop(&tiff, active);
    let img = RawImage {
        format: RawFormat::Srw,
        width: w,
        height: h,
        cpp: 1,
        data,
        cfa: Some(cfa),
        bits,
        black,
        white: vec![white],
        active_area: active,
        crop,
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

/// Which bit order unpacks 12-bit samples of `w` x `h` (in `src`, `w / 2 * 3` bytes per row) into the image: in
/// the right order neighbouring samples of one colour differ by little, in the wrong one by about the whole range.
/// Compares the summed absolute differences of samples two apart over 64 rows from the middle of the frame.
fn lsb_is_smoother(src: &[u8], w: usize, h: usize) -> bool {
    let stride = w / 2 * 3;
    let (mut msb, mut lsb) = (0u64, 0u64);
    let (mut a, mut b) = (vec![0u16; w], vec![0u16; w]);
    for y in (h / 5..h * 4 / 5).step_by((h * 3 / 5 / 64).max(1)) {
        let Some(row) = y.checked_mul(stride).and_then(|s| src.get(s..s + stride)) else { break };
        unpack_msb(row, 12, &mut a);
        unpack_lsb(row, 12, &mut b);
        let spread = |v: &[u16]| v.windows(3).map(|p| u64::from(p[0].abs_diff(p[2]))).sum::<u64>();
        msb += spread(&a);
        lsb += spread(&b);
    }
    lsb < msb
}

/// The default crop: the whole framing, or, when the Exif picture size is the framing cut in one direction only
/// (a 16:9 picture from a 4:3 sensor), the centred part of that size.
fn picture_crop(tiff: &Tiff, active: Rect) -> Rect {
    let whole = Rect::new(0, 0, active.width, active.height);
    let Some(exif) = tiff.exif() else { return whole };
    let (Some(pw), Some(ph)) = (exif.u32(t::PIXEL_X_DIMENSION), exif.u32(t::PIXEL_Y_DIMENSION)) else { return whole };
    let (pw, ph) = (pw as usize, ph as usize);
    if pw == 0 || ph == 0 || pw > active.width || ph > active.height || (pw < active.width && ph < active.height) {
        return whole;
    }
    Rect::new((active.width - pw) / 2, (active.height - ph) / 2, pw, ph)
}

/// Every third row of the active area (the saturation statistics need no more).
fn active_rows(d: &[u16], w: usize, a: Rect) -> Vec<u16> {
    (a.y..a.y + a.height).step_by(3).filter_map(|y| d.get(y * w + a.x..y * w + a.x + a.width)).flatten().copied().collect()
}

/// Black level from rows or columns outside the active area that are dark (mean below 1.5% of `white`): the
/// longest run of dark lines on the right, else the left, below, above; at least four lines, the two outermost
/// rows and columns of the array left out. Uniform 0 when there is no such run.
fn masked_black(d: &[u16], w: usize, h: usize, a: Rect, white: f32) -> (BlackLevel, crate::LevelSource) {
    if d.is_empty() {
        return (BlackLevel::uniform(0.0), crate::LevelSource::Fallback);
    }
    let limit = f64::from(white) * 0.015;
    let col_mean = |x: usize| {
        (a.y..a.y + a.height).step_by(7).filter_map(|y| d.get(y * w + x)).map(|&v| f64::from(v)).sum::<f64>() / a.height.div_ceil(7) as f64
    };
    let row_mean =
        |y: usize| (a.x..a.x + a.width).step_by(7).filter_map(|x| d.get(y * w + x)).map(|&v| f64::from(v)).sum::<f64>() / a.width.div_ceil(7) as f64;
    // the longest run of consecutive dark lines within `lines`
    let longest = |lines: std::ops::Range<usize>, mean: &dyn Fn(usize) -> f64| {
        let (mut best, mut start) = (0..0, lines.start);
        for i in lines.clone() {
            if mean(i) >= limit {
                start = i + 1;
            } else if i + 1 - start > best.len() {
                best = start..i + 1;
            }
        }
        best
    };
    let cols = a.y..a.y + a.height;
    let rows = a.x..a.x + a.width;
    let right = longest(a.x + a.width..w.saturating_sub(2), &col_mean);
    let left = longest(2..a.x, &col_mean);
    let below = longest(a.y + a.height..h.saturating_sub(2), &row_mean);
    let above = longest(2..a.y, &row_mean);
    if right.len() >= 4 {
        black_from_columns_sourced(d, w, right, cols, a)
    } else if left.len() >= 4 {
        black_from_columns_sourced(d, w, left, cols, a)
    } else if below.len() >= 4 {
        black_from_columns_sourced(d, w, rows, below, a)
    } else if above.len() >= 4 {
        black_from_columns_sourced(d, w, rows, above, a)
    } else {
        (BlackLevel::uniform(0.0), crate::LevelSource::Fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, probe, probe_info};
    use craft_tiff::{IfdBuilder, ImageData, TiffWriter, Value};

    const KEY: [u32; 11] = [305, 72, 737, 456, 282, 307, 519, 724, 13, 505, 193];

    /// A maker note in the Samsung layout: a big-endian IFD of LONG vectors, offsets relative to the note's start.
    fn note(entries: &[(u16, Vec<u32>)]) -> Vec<u8> {
        let typed: Vec<_> = entries.iter().map(|(tag, v)| (*tag, 4, v.len() as u32, v.iter().flat_map(|x| x.to_be_bytes()).collect())).collect();
        note_typed(&typed)
    }

    /// [`note`] with any entry types: `(tag, type, count, big-endian value bytes)`.
    fn note_typed(entries: &[(u16, u16, u32, Vec<u8>)]) -> Vec<u8> {
        let mut head = Vec::new();
        let mut tail = Vec::new();
        head.extend_from_slice(&(entries.len() as u16).to_be_bytes());
        let values_at = 2 + 12 * entries.len() + 4;
        for (tag, kind, count, v) in entries {
            head.extend_from_slice(&tag.to_be_bytes());
            head.extend_from_slice(&kind.to_be_bytes());
            head.extend_from_slice(&count.to_be_bytes());
            if v.len() <= 4 {
                head.extend(v.iter().copied().chain(std::iter::repeat(0)).take(4));
            } else {
                head.extend_from_slice(&((values_at + tail.len()) as u32).to_be_bytes());
                tail.extend_from_slice(v);
            }
        }
        head.extend_from_slice(&[0; 4]);
        head.extend(tail);
        head
    }

    /// Stored form of a vector the way the file keeps it: `plain + key` (`sign` 1) or `plain - key` (`sign` -1).
    fn stored(plain: &[i64], sign: i64) -> Vec<u32> {
        plain.iter().enumerate().map(|(i, &p)| (p + sign * i64::from(KEY[i % 11])) as u32).collect()
    }

    fn pack12(v: &[u16]) -> Vec<u8> {
        v.chunks(2).flat_map(|p| [(p[0] >> 4) as u8, ((p[0] & 15) << 4) as u8 | (p[1] >> 8) as u8, p[1] as u8]).collect()
    }

    fn le_words(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    struct Spec {
        w: u32,
        h: u32,
        compression: u16,
        strip: Vec<u8>,
        pattern: Option<[u8; 4]>,
        note: Option<Vec<u8>>,
        picture: Option<(u32, u32)>,
    }

    fn srw(s: Spec) -> Vec<u8> {
        let mut raw = IfdBuilder::new();
        raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![s.w]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![s.h]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![12]));
        raw.set(t::COMPRESSION, Value::Short(vec![s.compression]));
        raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        if let Some(p) = s.pattern {
            raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
            raw.set(t::CFA_PATTERN_EP, Value::Byte(p.to_vec()));
        }
        raw.set_image(ImageData::Strips { rows_per_strip: s.h, strips: vec![s.strip] });
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("SAMSUNG".into()));
        ifd0.set(t::MODEL, Value::Ascii("NX0".into()));
        let mut exif = IfdBuilder::new();
        if let Some(n) = s.note {
            exif.set(t::MAKER_NOTE, Value::Undefined(n));
        }
        if let Some((pw, ph)) = s.picture {
            exif.set(t::PIXEL_X_DIMENSION, Value::Long(vec![pw]));
            exif.set(t::PIXEL_Y_DIMENSION, Value::Long(vec![ph]));
        }
        ifd0.set_child(t::EXIF_IFD, exif);
        ifd0.add_sub_ifd(raw);
        TiffWriter::new(craft_tiff::ByteOrder::Big, false).write(&[ifd0]).unwrap()
    }

    /// Strip byte counts that add up past `u64::MAX` (LONG8 `[u64::MAX, 1]`) are an error or a picture, not an overflow.
    #[test]
    fn strip_byte_counts_past_u64_max_do_not_overflow() {
        let file = crate::shared::tests::huge_strip_counts(srw(Spec {
            w: 4,
            h: 2,
            compression: 32769,
            strip: vec![0; 16],
            pattern: None,
            note: None,
            picture: None,
        }));
        for mode in [Mode::Header, Mode::Full] {
            let _ = super::decode(&file, mode, &Limits::default());
        }
    }

    /// The camera-stated description every test file carries: JPEG framing, white balance, levels.
    fn full_note(area: [u32; 4], white: i64, black: i64) -> Vec<u8> {
        let mut levels = vec![0i64; 4];
        levels.extend([white; 8]);
        note(&[
            (SENSOR_AREAS, vec![0, 0, 64, 32, area[0], area[1], area[2], area[3]]),
            (ENCRYPTION_KEY, KEY.to_vec()),
            (WB_UNCORRECTED, stored(&[8192, 4096, 4096, 6144], 1)),
            (WB_BLACK, stored(&[black; 4], 1)),
            (RAW_DATA, stored(&levels, -1)),
        ])
    }

    /// A smooth gradient with a little texture, `max` the first value that is not reached.
    fn ramp(w: u32, h: u32, max: u32) -> Vec<u16> {
        (0..w * h).map(|i| ((1000 + (i % w) * 7 + (i / w) * 5 + (i * 13) % 9) % max) as u16).collect()
    }

    fn pack12_lsb(v: &[u16]) -> Vec<u8> {
        v.chunks(2).flat_map(|p| [p[0] as u8, ((p[0] >> 8) as u8) | ((p[1] & 15) << 4) as u8, (p[1] >> 4) as u8]).collect()
    }

    /// Known answer: the samples, layout, framing, levels and white balance all come from the file's own words.
    #[test]
    fn packed_12_bit_samples_decode_with_the_maker_note() {
        let v = ramp(64, 32, 4096);
        let bytes = srw(Spec {
            w: 64,
            h: 32,
            compression: 32770,
            strip: pack12(&v),
            pattern: Some([2, 1, 1, 0]),
            note: Some(full_note([6, 4, 58, 30], 4095, 3)),
            picture: None,
        });
        assert_eq!(probe(&bytes), Some(RawFormat::Srw));
        let r = decode(&bytes).unwrap();
        assert_eq!((r.format, r.width, r.height, r.bits), (RawFormat::Srw, 64, 32, 12));
        assert_eq!(r.data, RawData::U16(v));
        assert_eq!(r.cfa.as_ref().unwrap().name(), "BGGR");
        assert_eq!(r.active_area, Rect::new(6, 4, 52, 26));
        assert_eq!(r.crop, Rect::new(0, 0, 52, 26));
        assert_eq!((r.black.mean(), r.white.clone()), (3.0, vec![4095.0]));
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Metadata, white: crate::LevelSource::Metadata });
        assert_eq!(r.wb_multipliers, Some([2.0, 1.0, 1.5]));
        // headers only: everything but the samples
        let i = probe_info(&bytes).unwrap();
        assert_eq!((i.active_area, i.wb_multipliers, i.bits), (Rect::new(6, 4, 52, 26), Some([2.0, 1.0, 1.5]), 12));
    }

    /// The later bodies pack the same 12-bit samples LSB-first; which of the two orders a file uses is found from
    /// the samples (the wrong order makes a noisy image), not from the model.
    #[test]
    fn lsb_first_packing_is_found_from_the_samples() {
        let v = ramp(64, 32, 4096);
        for (strip, name) in [(pack12(&v), "msb"), (pack12_lsb(&v), "lsb")] {
            let bytes = srw(Spec { w: 64, h: 32, compression: 32770, strip, pattern: Some([1, 0, 2, 1]), note: None, picture: None });
            let r = decode(&bytes).unwrap();
            assert_eq!(r.data, RawData::U16(v.clone()), "{name}");
            // no maker note, no dark border, no plateau
            assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Fallback, white: crate::LevelSource::Fallback }, "{name}");
        }
    }

    /// A picture size narrower than the framing in one direction only (16:9 from a 4:3 sensor) is the framing's
    /// centre; any other size is ignored.
    #[test]
    fn the_exif_picture_size_centres_a_one_direction_crop() {
        let v = ramp(64, 32, 4096);
        let file = |picture| {
            srw(Spec {
                w: 64,
                h: 32,
                compression: 32770,
                strip: pack12(&v),
                pattern: Some([2, 1, 1, 0]),
                note: Some(full_note([6, 4, 58, 30], 4095, 0)),
                picture,
            })
        };
        // framing 52 x 26
        assert_eq!(decode(&file(Some((52, 18)))).unwrap().crop, Rect::new(0, 4, 52, 18));
        assert_eq!(decode(&file(Some((40, 26)))).unwrap().crop, Rect::new(6, 0, 40, 26));
        let r = decode(&file(Some((52, 18)))).unwrap();
        assert_eq!(r.info().developed_size(), (52, 18));
        for other in [None, Some((52, 26)), Some((40, 18)), Some((60, 18)), Some((52, 30)), Some((0, 18))] {
            assert_eq!(decode(&file(other)).unwrap().crop, Rect::new(0, 0, 52, 26), "{other:?}");
        }
    }

    /// 16-bit words are little-endian although the file is big-endian; a saturation plateau below the stated
    /// level wins; dark rows below the framing give the black level when the file states none.
    #[test]
    fn word_samples_decode_and_measure_what_the_file_does_not_say() {
        let (w, h) = (64u32, 40u32);
        let mut v: Vec<u16> = (0..w * h).map(|i| 3000 + (i * 91 % 5000) as u16).collect();
        // saturation: a plateau at 15882 inside the framing (rows 4..28), two rows of picture beyond it, dark rows 30..40
        for i in 0..150usize {
            v[(4 + i % 24) * 64 + 8 + i / 24] = 15882;
        }
        for x in 0..64 * 10 {
            v[30 * 64 + x] = 80;
        }
        let bytes = srw(Spec {
            w,
            h,
            compression: 32769,
            strip: le_words(&v),
            pattern: Some([0, 1, 1, 2]),
            note: Some(full_note([0, 4, 64, 28], 16383, 0)),
            picture: None,
        });
        let r = decode(&bytes).unwrap();
        assert_eq!(r.data, RawData::U16(v));
        assert_eq!(r.bits, 14);
        assert!((15800.0..=15882.0).contains(&r.white[0]), "{:?}", r.white);
        assert_eq!(r.black.mean(), 80.0);
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::MaskedPixels, white: crate::LevelSource::Inferred });
        assert_eq!(r.cfa.unwrap().name(), "RGGB");
    }

    /// Files whose strip is not exactly the uncompressed size (or that use another private value) are compressed
    /// codings: reported as such, still recognised as SRW.
    #[test]
    fn compressed_codings_are_reported_not_misread() {
        let plain = |compression, strip: Vec<u8>| {
            srw(Spec { w: 64, h: 32, compression, strip, pattern: Some([2, 1, 1, 0]), note: Some(full_note([0, 0, 64, 32], 4095, 0)), picture: None })
        };
        for bytes in [
            plain(32770, vec![7; 64 * 32 * 12 / 8 - 3]),
            plain(32772, vec![7; 4000]),
            plain(32773, pack12(&ramp(64, 32, 4096))),
            plain(32769, vec![0; 64 * 32]),
        ] {
            assert_eq!(probe(&bytes), Some(RawFormat::Srw));
            let Err(RawError::Unsupported(why)) = decode(&bytes) else { panic!("expected Unsupported") };
            assert!(why.contains("compressed"), "{why}");
            assert!(probe_info(&bytes).is_err());
        }
    }

    /// Without a maker note the samples still decode: whole array, no white balance, depth from the TIFF tags.
    #[test]
    fn a_file_without_a_maker_note_decodes_with_defaults() {
        let v = ramp(64, 32, 4096);
        let bytes = srw(Spec { w: 64, h: 32, compression: 32770, strip: pack12(&v), pattern: Some([1, 0, 2, 1]), note: None, picture: None });
        let r = decode(&bytes).unwrap();
        assert_eq!(r.data, RawData::U16(v));
        assert_eq!((r.active_area, r.wb_multipliers, r.black.mean()), (Rect::new(0, 0, 64, 32), None, 0.0));
        assert_eq!(r.cfa.unwrap().name(), "GRBG");
        assert_eq!(r.bits, 12);
    }

    /// A raw IFD that does not say how its mosaic is laid out is not guessed at.
    #[test]
    fn a_file_without_a_cfa_pattern_is_unsupported() {
        let bytes = srw(Spec { w: 64, h: 32, compression: 32770, strip: pack12(&ramp(64, 32, 4096)), pattern: None, note: None, picture: None });
        assert!(matches!(decode(&bytes), Err(RawError::Unsupported(w)) if w.contains("CFAPattern")));
    }

    /// An encryption key value past i64::MAX (LONG8) overflowed `-1 * key` in debug builds.
    #[test]
    fn a_huge_encryption_key_does_not_panic() {
        let key: Vec<u8> = std::iter::repeat_n((1u64 << 63).to_be_bytes(), 11).flatten().collect();
        let note = note_typed(&[(ENCRYPTION_KEY, 16, 11, key), (WB_UNCORRECTED, 4, 4, [0u8; 16].to_vec()), (WB_BLACK, 4, 4, [0u8; 16].to_vec())]);
        let bytes = srw(Spec {
            w: 64,
            h: 32,
            compression: 32770,
            strip: pack12(&ramp(64, 32, 4096)),
            pattern: Some([2, 1, 1, 0]),
            note: Some(note),
            picture: None,
        });
        let mn = maker_note(&bytes, &Tiff::parse(&bytes).unwrap()).unwrap();
        assert_eq!(mn.u64s(ENCRYPTION_KEY).map(|k| k[0]), Some(1 << 63), "the key reaches keyed()");
        assert_eq!(keyed(&mn, WB_BLACK, -1), Some(vec![0; 4]), "(2^63 mod 2^32 is 0)");
        assert!(decode(&bytes).is_ok());
        assert!(probe_info(&bytes).is_ok());
    }

    /// A framing rectangle outside the array, or reversed, is ignored; truncated files fail without panicking.
    #[test]
    fn hostile_framing_and_truncation_do_not_panic() {
        let v = ramp(64, 32, 4096);
        for area in [[0, 0, 65, 32], [10, 0, 10, 32], [0, 40, 64, 20]] {
            let bytes = srw(Spec {
                w: 64,
                h: 32,
                compression: 32770,
                strip: pack12(&v),
                pattern: Some([2, 1, 1, 0]),
                note: Some(full_note(area, 4095, 0)),
                picture: None,
            });
            assert_eq!(decode(&bytes).unwrap().active_area, Rect::new(0, 0, 64, 32));
        }
        let bytes = srw(Spec {
            w: 64,
            h: 32,
            compression: 32770,
            strip: pack12(&v),
            pattern: Some([2, 1, 1, 0]),
            note: Some(full_note([0, 0, 64, 32], 4095, 0)),
            picture: None,
        });
        for cut in [10, bytes.len() / 2, bytes.len() - 20] {
            let _ = decode(&bytes[..cut]);
            let _ = probe_info(&bytes[..cut]);
        }
    }

    /// storytold/lightcraft#2561: a small file whose strip declares a large frame is refused before the full decode allocates the frame.
    #[test]
    fn a_strip_cut_short_is_refused_before_allocating() {
        let (w, h) = (3000u32, 2000u32);
        for (compression, declared) in [(32769, w * h * 2), (32770, w * h / 2 * 3)] {
            let mut f = srw(Spec { w, h, compression, strip: vec![0u8; 12], pattern: Some([0, 1, 1, 2]), note: None, picture: None });
            crate::shared::declare_strip_len(&mut f, 12, declared);
            assert_eq!(probe(&f), Some(crate::RawFormat::Srw));
            assert!(matches!(decode(&f), Err(RawError::Corrupt(_))), "compression {compression}");
        }
    }
}
