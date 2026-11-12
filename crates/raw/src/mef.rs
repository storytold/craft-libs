//! Mamiya MEF (ZD): a big-endian TIFF whose first `SubIFDs` child is an uncompressed 12-bit packed CFA image.
//!
//! Sources: TIFF 6.0 (container, MSB-first bit packing, `CFAPattern`) and black-box analysis of the one CC0
//! MEF sample on raw.pixls.us (Mamiya ZD); every number below was measured on it.
//!
//! - IFD0 is a 144 x 192 RGB thumbnail with three `SubIFDs`: the raw (4016 x 5344, 12 bits, `Compression` 1,
//!   photometric CFA, one strip of exactly 1.5 bytes per pixel, two samples in three bytes most significant bits
//!   first, as TIFF packs them), a 992 x 1328 and a 240 x 320 RGB preview, all uncompressed and 8 bits. No JPEG.
//! - `CFAPattern` `00 01 01 02`: RGGB; the two green sites agree (means 254 and 256 against 235 and 165).
//! - No masked border and no stored black level: the darkest 0.01% of each site's samples sit at 3..6 and the
//!   minimum is 0, so black is 0. White: the saturation plateau, else 4095.
//! - The 992 x 1328 preview shows a 3968 x 5312 window (4 sensor pixels per preview pixel) that starts at column 24,
//!   row 12: that offset gives the best correlation of the preview's luminance with the box-filtered mosaic (0.965,
//!   against 0.83 for the full frame). It is the default crop.
//! - Orientation is the raw IFD's own `Orientation` (6 on the sample, a portrait shot).
//! - No white-balance tag was found (the maker note holds an ICC-like profile), so `wb_multipliers` is `None`: a
//!   developer estimates the gains (LightCraft measures them from that preview's neutral pixels).
use crate::{BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result};
use craft_tiff::image::chunk_bytes;
use craft_tiff::{Ifd, Tiff, tags as t};

/// Where the preview's window starts in the sensor frame, and how many sensor pixels one preview pixel covers.
const CROP_ORIGIN: (usize, usize) = (24, 12);
const PREVIEW_SCALE: usize = 4;

fn is_mamiya(tiff: &Tiff) -> bool {
    tiff.find(t::MAKE).and_then(|e| e.value.as_str()).is_some_and(|m| m.to_ascii_uppercase().starts_with("MAMIYA"))
}

/// The raw IFD of a Mamiya MEF: CFA, 12 bits, uncompressed, one strip of 1.5 bytes per pixel.
pub(crate) fn raw_ifd(tiff: &Tiff, len: usize) -> Option<&Ifd> {
    if !is_mamiya(tiff) {
        return None;
    }
    tiff.all_ifds().into_iter().find(|i| {
        let Ok(info) = i.image() else { return false };
        let (w, h) = (info.width as u64, info.height as u64);
        i.u16(t::PHOTOMETRIC) == Some(t::photometric::CFA)
            && i.u16(t::COMPRESSION).unwrap_or(1) == 1
            && i.u16(t::BITS_PER_SAMPLE) == Some(12)
            && w > 0
            && w % 2 == 0
            && matches!(info.chunks(len as u64).as_slice(), [c] if Some(c.len) == (w * 3 / 2).checked_mul(h))
    })
}

/// The largest uncompressed 8-bit RGB sub-IFD (the preview): its pixels, width and height.
fn rgb_preview<'a>(tiff: &Tiff, bytes: &'a [u8]) -> Option<(&'a [u8], usize, usize)> {
    tiff.all_ifds()
        .into_iter()
        .filter_map(|i| {
            let info = i.image().ok()?;
            let ok = i.u16(t::PHOTOMETRIC) == Some(2)
                && i.u16(t::COMPRESSION).unwrap_or(1) == 1
                && i.u16(t::SAMPLES_PER_PIXEL) == Some(3)
                && i.u64s(t::BITS_PER_SAMPLE).is_some_and(|b| b.iter().all(|&v| v == 8));
            let (w, h) = (info.width as usize, info.height as usize);
            let chunks = info.chunks(bytes.len() as u64);
            let [c] = chunks.as_slice() else { return None };
            let px = chunk_bytes(bytes, c)?;
            (ok && px.len() >= w.checked_mul(h)?.checked_mul(3)?).then_some((px, w, h))
        })
        .max_by_key(|(_, w, h)| w * h)
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let raw = raw_ifd(&tiff, bytes.len()).ok_or_else(|| RawError::Unsupported("Mef raw image not found".into()))?;
    let info = raw.image()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let n = w.checked_mul(h).filter(|n| *n > 0 && *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let cfa = match (raw.u64s(t::CFA_REPEAT_PATTERN_DIM).as_deref(), raw.bytes(t::CFA_PATTERN_EP)) {
        (Some([2, 2]), Some(p)) if p.len() == 4 && p.iter().all(|&c| c <= 2) => Cfa { width: 2, height: 2, pattern: p.to_vec() },
        _ => return Err(RawError::Unsupported("Mef raw without a 2x2 CFAPattern".into())),
    };
    let preview = rgb_preview(&tiff, bytes);
    let crop = match preview {
        Some((_, pw, ph)) => {
            let (cw, ch) = (pw.saturating_mul(PREVIEW_SCALE), ph.saturating_mul(PREVIEW_SCALE));
            if cw > 0 && ch > 0 && CROP_ORIGIN.0 + cw <= w && CROP_ORIGIN.1 + ch <= h {
                Rect::new(CROP_ORIGIN.0, CROP_ORIGIN.1, cw, ch)
            } else {
                Rect::new(0, 0, w, h)
            }
        }
        None => Rect::new(0, 0, w, h),
    };
    // the white balance is measured from the samples, so the header probe reads them too
    // the strip length is the tag value, so a short file must fail before the sample buffer is sized from it
    let chunks = info.chunks(bytes.len() as u64);
    let src = chunks
        .first()
        .and_then(|c| chunk_bytes(bytes, c).filter(|s| s.len() as u64 == c.len))
        .ok_or_else(|| RawError::Corrupt("Mef strip outside file".into()))?;
    // 12-bit MSB packing takes 3 bytes per 2 samples; a strip that cannot hold the declared
    // samples is a truncated file, not a picture — unpacking it would run past the real bytes
    // through every output sample (a short strip asks for the whole allocation and more).
    let need = n.div_ceil(2).saturating_mul(3);
    if src.len() < need {
        return Err(RawError::Corrupt(format!("a Mef strip of {} bytes cannot hold the declared {n} samples", src.len())));
    }
    let mut d = vec![0u16; n];
    crate::unpack::unpack_msb(src, 12, &mut d);
    let (white, white_source) = crate::shared::white_from_data_sourced(&d, 12);
    let black = BlackLevel::uniform(0.0);
    let data = if mode == Mode::Full { RawData::U16(d) } else { RawData::U16(Vec::new()) };
    let orientation = raw.u16(t::ORIENTATION).or_else(|| tiff.ifds.first().and_then(|i| i.u16(t::ORIENTATION))).unwrap_or(1);
    let img = RawImage {
        format: RawFormat::Mef,
        width: w,
        height: h,
        cpp: 1,
        data,
        cfa: Some(cfa),
        bits: 12,
        black,
        white: vec![white],
        active_area: Rect::new(0, 0, w, h),
        crop,
        orientation: Orientation::from_exif(orientation),
        color: ColorData::default(),
        wb_multipliers: None,
        linearized: false,
        opcodes: OpcodeLists::default(),
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: crate::LevelSources { black: crate::LevelSource::Fallback, white: white_source },
        pixel_aspect: 1.0,
    };
    img.validate_for(mode)?;
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};

    fn pack(px: &[u16]) -> Vec<u8> {
        px.chunks(2).flat_map(|p| [(p[0] >> 4) as u8, ((p[0] & 15) << 4) as u8 | (p[1] >> 8) as u8, p[1] as u8]).collect()
    }

    fn sample(w: u32, h: u32, px: &[u16]) -> Vec<u8> {
        sample_strip(w, h, &pack(px))
    }

    fn sample_strip(w: u32, h: u32, strip: &[u8]) -> Vec<u8> {
        let mut raw = IfdBuilder::new();
        raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![12]));
        raw.set(t::COMPRESSION, Value::Short(vec![1]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        raw.set(t::ORIENTATION, Value::Short(vec![6]));
        raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
        raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
        raw.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![strip.to_vec()] });
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("Mamiya-OP Co.,Ltd.".into()));
        ifd0.set(t::MODEL, Value::Ascii("MAMIYA ZD".into()));
        ifd0.add_sub_ifd(raw);
        TiffWriter::new(ByteOrder::Big, false).write(&[ifd0]).unwrap()
    }

    /// A strip that cannot hold the declared samples is a truncated file, refused as corrupt
    /// instead of unpacked: `unpack_msb` would otherwise run `n` samples through the few real
    /// bytes, filling the rest from nothing.
    #[test]
    fn a_strip_too_short_for_the_declared_samples_is_refused() {
        let px: Vec<u16> = (0..16 * 6).map(|i| (i * 41 % 4096) as u16).collect();
        let f = sample(16, 6, &px);
        assert!(crate::decode(&f).is_ok());
        // A truncated file shaped like a real one: IFD tables first, strip last — only its first
        // 20 packed bytes remain, while the IFD still declares the full 1.5 B/sample length. The
        // probe accepts the file on the declared length; before the byte gate below, `unpack_msb`
        // ran the dimension-sized allocation through those 20 bytes, filling it from nothing.
        let u32be = |b: &[u8], a: usize| u32::from_be_bytes([b[a], b[a + 1], b[a + 2], b[a + 3]]);
        let full = pack(&px);
        let at = f.windows(full.len()).position(|w| w == full).expect("strip bytes in the file");
        let after_strip = at + full.len();
        let moved = after_strip - (at + 20);
        let mut g = f[..8].to_vec();
        g.extend(std::iter::repeat_n(0u8, at + 20 - 8));
        g.extend_from_slice(&f[after_strip..]);
        // repoint the first-IFD pointer and ifd0's MAKE / MODEL / SubIfds offsets (the tables now
        // sit `moved` earlier); the raw sub-IFD's strip offset and byte count are patched below
        let poke = |b: &mut [u8], a: usize| {
            let v = u32::from_be_bytes([b[a], b[a + 1], b[a + 2], b[a + 3]]);
            if v as usize >= after_strip {
                b[a..a + 4].copy_from_slice(&(v - moved as u32).to_be_bytes());
            }
        };
        let ifd0_at = u32be(&g, 4) as usize - moved;
        g[4..8].copy_from_slice(&(ifd0_at as u32).to_be_bytes());
        let n = u16::from_be_bytes([g[ifd0_at], g[ifd0_at + 1]]) as usize;
        for i in 0..n {
            poke(&mut g, ifd0_at + 2 + i * 12 + 8);
        }
        // the raw sub-IFD: entries follow its count; tag 273 (StripOffsets) is the seventh, held
        // inline — repoint it at the strip's new home, and append the truncated strip for real
        let sub_at = u32be(&g, ifd0_at + 2 + 2 * 12 + 8) as usize;
        let e_offsets = sub_at + 2 + 6 * 12 + 8;
        let strip_at = g.len() as u32;
        g[e_offsets..e_offsets + 4].copy_from_slice(&strip_at.to_be_bytes());
        g.extend_from_slice(&full[..20]);
        assert_eq!(crate::probe(&g), Some(RawFormat::Mef));
        let r = crate::decode(&g);
        assert!(matches!(r, Err(RawError::Corrupt(_))), "{r:?}");
    }

    #[test]
    fn decodes_packed_12_bit() {
        let px: Vec<u16> = (0..16 * 6).map(|i| (i * 41 % 4096) as u16).collect();
        let f = sample(16, 6, &px);
        assert_eq!(crate::probe(&f), Some(RawFormat::Mef));
        let r = crate::decode(&f).unwrap();
        assert_eq!(r.data, RawData::U16(px));
        assert_eq!(r.orientation, Orientation::from_exif(6));
        assert_eq!(r.crop, Rect::new(0, 0, 16, 6));
        // the format's black is a constant 0; no plateau
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Fallback, white: crate::LevelSource::Fallback });
        assert_eq!(crate::probe_info(&f).unwrap(), r.info());
    }

    #[test]
    fn malformed_files_never_panic() {
        let px: Vec<u16> = (0..16 * 6).map(|i| (i * 7 % 4096) as u16).collect();
        let f = sample(16, 6, &px);
        for n in 0..f.len() {
            let _ = crate::decode(&f[..n]);
        }
        for i in 0..f.len() {
            for v in [0u8, 0xff] {
                let mut g = f.clone();
                g[i] = v;
                let _ = crate::decode(&g);
                let _ = crate::probe_info(&g);
            }
        }
    }

    /// Rewrite the raw IFD's height, rows per strip and strip length in place (the strip data stays as written).
    fn declare(f: &mut [u8], rows: u32, strip_len: u32) {
        let tiff = Tiff::parse(f).unwrap();
        let raw = &tiff.ifds[0].sub_ifds[0];
        let spots: Vec<_> = [(t::IMAGE_LENGTH, rows), (t::ROWS_PER_STRIP, rows), (t::STRIP_BYTE_COUNTS, strip_len)]
            .into_iter()
            .map(|(tag, v)| {
                let e = raw.entries.iter().find(|e| e.tag == tag).unwrap();
                (e.offset as usize, matches!(e.value, Value::Short(_)), v)
            })
            .collect();
        for (at, short, v) in spots {
            if short {
                f[at..at + 2].copy_from_slice(&u16::try_from(v).unwrap().to_be_bytes());
            } else {
                f[at..at + 4].copy_from_slice(&v.to_be_bytes());
            }
        }
    }

    #[test]
    fn a_strip_longer_than_the_file_is_an_error() {
        // storytold/lightcraft#2119: the declared strip length matched the size, so a few bytes of data were decoded as a full frame
        let px: Vec<u16> = (0..40).map(|i| (i * 13 % 4096) as u16).collect();
        let mut f = sample(20, 2, &px);
        declare(&mut f, 2000, 30 * 2000);
        assert_eq!(crate::probe(&f), Some(RawFormat::Mef));
        assert!(matches!(crate::probe_info(&f), Err(RawError::Corrupt(_))));
        assert!(matches!(crate::decode(&f), Err(RawError::Corrupt(_))));
    }
}
