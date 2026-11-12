//! Epson ERF (R-D1, R-D1s, R-D1x): a big-endian TIFF whose `SubIFDs` child is a 12-bit CFA image in 16-byte blocks.
//!
//! Sources: TIFF 6.0 (container, `CFARepeatPatternDim` / `CFAPattern`) and black-box analysis of the three
//! CC0 ERF samples on raw.pixls.us (one per body); every number below was measured on them.
//!
//! - IFD0 is a 160 x 120 RGB thumbnail; its `SubIFDs` child is the raw: 3040 x 2024, 12 bits, `Compression` 32769,
//!   photometric CFA, one strip of exactly `width * 8 / 5` bytes per row. Byte 15 of every 16 is zero in all rows:
//!   a row is 16-byte blocks, each 15 bytes of ten 12-bit samples, most significant bits first (`b0 << 4 | b1 >> 4`,
//!   `(b1 & 15) << 8 | b2`), then a pad byte. Read the other way (least significant first, as the Olympus blocks are)
//!   the frame is noise: the mean step between same-colour neighbours is 150..199 codes against 11..18.
//! - `CFAPattern` `00 01 01 02`: RGGB.
//! - The maker note (`EPSON\0\x01\0`, then a big-endian IFD with offsets from the file start) holds four black levels
//!   as `u32` in tag `0x0401` (61..65; in Bayer-cell order), above which the darkest 0.01% of samples sit; `0x020b` x
//!   `0x020c` is the picture size, 3008 x 2000. Registered against the 640 x 424 preview JPEG (correlation of the
//!   green channel over every even offset), that picture starts at column 20, row 12 in all three files.
//! - White: the saturation plateau (4095 on the clipped R-D1x frame), else 4095.
//! - No maker-note tag tracks the as-shot white balance across the three files, so `wb_multipliers` is `None`:
//!   a developer estimates it (LightCraft measures it from the preview's neutral pixels; on the R-D1s sample that
//!   gives red 2.68 and blue 1.31). An estimate from picture content is not the file-format layer's.
//! - The preview is the maker note's tag `0x0280`, a JPEG whose first byte is stored as `ee` (found by the
//!   structural scan in `preview::scan_for_jpeg`).
use crate::{BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result};
use craft_tiff::image::chunk_bytes;
use craft_tiff::{Ifd, Tiff, tags as t};

/// Epson's private compression code for the 16-byte block layout.
const BLOCKS16: u16 = 32769;
/// Maker-note tags.
const MN_BLACK: u16 = 0x0401;
const MN_WIDTH: u16 = 0x020b;
const MN_HEIGHT: u16 = 0x020c;
/// Where the picture starts in the sensor frame (measured, see the module notes).
const CROP_ORIGIN: (usize, usize) = (20, 12);

/// The raw IFD of an Epson ERF: CFA, 12 bits, compression 32769, one strip of 16-byte blocks of ten samples.
pub(crate) fn raw_ifd(tiff: &Tiff, len: usize) -> Option<&Ifd> {
    let make = tiff.find(t::MAKE).and_then(|e| e.value.as_str())?;
    if !make.to_ascii_uppercase().contains("EPSON") {
        return None;
    }
    tiff.all_ifds().into_iter().find(|i| {
        let Ok(info) = i.image() else { return false };
        let (w, h) = (info.width as u64, info.height as u64);
        i.u16(t::PHOTOMETRIC) == Some(t::photometric::CFA)
            && i.u16(t::COMPRESSION) == Some(BLOCKS16)
            && i.u16(t::BITS_PER_SAMPLE) == Some(12)
            && w > 0
            && w % 10 == 0
            && matches!(info.chunks(len as u64).as_slice(), [c] if Some(c.len) == (w * 8 / 5).checked_mul(h))
    })
}

/// The maker-note IFD (`EPSON\0\x01\0` then an IFD in the file's byte order, offsets from the file start).
fn maker_note(bytes: &[u8], tiff: &Tiff) -> Option<Ifd> {
    let e = tiff.exif()?.get(t::MAKER_NOTE)?;
    let start = usize::try_from(e.offset).ok()?;
    if !bytes.get(start..)?.starts_with(b"EPSON\0") {
        return None;
    }
    let opts = craft_tiff::ParseOptions { max_ifds: 1, max_depth: 0, follow_children: false, ..Default::default() };
    craft_tiff::parse_ifd_at(bytes, e.offset.checked_add(8)?, tiff.order, 0, false, &opts).ok().map(|(i, _)| i)
}

/// Unpack a strip of 16-byte blocks (15 bytes of ten MSB-first 12-bit samples, one pad byte).
fn unpack(src: &[u8], out: &mut [u16]) {
    let (blocks, _) = src.as_chunks::<16>();
    let (pixels, _) = out.as_chunks_mut::<10>();
    for (b, px) in blocks.iter().zip(pixels) {
        crate::unpack::unpack_msb(&b[..15], 12, px);
    }
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("Erf TIFF without IFDs".into()))?;
    let raw = raw_ifd(&tiff, bytes.len()).ok_or_else(|| RawError::Unsupported("Erf raw image not found".into()))?;
    let info = raw.image()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let n = w.checked_mul(h).filter(|n| *n > 0 && *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let cfa = match (raw.u64s(t::CFA_REPEAT_PATTERN_DIM).as_deref(), raw.bytes(t::CFA_PATTERN_EP)) {
        (Some([2, 2]), Some(p)) if p.len() == 4 && p.iter().all(|&c| c <= 2) => Cfa { width: 2, height: 2, pattern: p.to_vec() },
        _ => return Err(RawError::Unsupported("Erf raw without a 2x2 CFAPattern".into())),
    };
    let mn = maker_note(bytes, &tiff);
    let (black, black_source) = match mn.as_ref().and_then(|m| m.u64s(MN_BLACK)).as_deref() {
        Some(v @ [_, _, _, _]) if v.iter().all(|&b| b < 4095) => (
            BlackLevel { repeat_rows: 2, repeat_cols: 2, values: v.iter().map(|&b| b as f32).collect(), delta_h: vec![], delta_v: vec![] },
            crate::LevelSource::Metadata,
        ),
        _ => (BlackLevel::uniform(0.0), crate::LevelSource::Fallback),
    };
    let size = mn.as_ref().and_then(|m| Some((m.u32(MN_WIDTH)? as usize, m.u32(MN_HEIGHT)? as usize)));
    let crop = match size {
        Some((cw, ch)) if cw > 0 && ch > 0 && CROP_ORIGIN.0 + cw <= w && CROP_ORIGIN.1 + ch <= h => Rect::new(CROP_ORIGIN.0, CROP_ORIGIN.1, cw, ch),
        _ => Rect::new(0, 0, w, h),
    };
    // the white balance is measured from the samples, so the header probe reads them too
    // the strip length is the tag value, so a short file must fail before the sample buffer is sized from it
    let chunks = info.chunks(bytes.len() as u64);
    let src = chunks
        .first()
        .and_then(|c| chunk_bytes(bytes, c).filter(|s| s.len() as u64 == c.len))
        .ok_or_else(|| RawError::Corrupt("Erf strip outside file".into()))?;
    let mut d = vec![0u16; n];
    unpack(src, &mut d);
    let (white, white_source) = crate::shared::white_from_data_sourced(&d, 12);
    let data = if mode == Mode::Full { RawData::U16(d) } else { RawData::U16(Vec::new()) };
    let img = RawImage {
        format: RawFormat::Erf,
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
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: ColorData::default(),
        wb_multipliers: None,
        linearized: false,
        opcodes: OpcodeLists::default(),
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: crate::LevelSources { black: black_source, white: white_source },
        pixel_aspect: 1.0,
    };
    img.validate_for(mode)?;
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::{IfdBuilder, ImageData, TiffWriter, Value};

    /// Pack ten-sample groups into 16-byte blocks, MSB first, zero pad.
    fn pack(px: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        for g in px.chunks(10) {
            for p in g.chunks(2) {
                let (a, b) = (p[0], p[1]);
                out.extend_from_slice(&[(a >> 4) as u8, ((a & 15) << 4) as u8 | (b >> 8) as u8, b as u8]);
            }
            out.push(0);
        }
        out
    }

    fn sample(w: u32, h: u32, px: &[u16]) -> Vec<u8> {
        let mut raw = IfdBuilder::new();
        raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![12]));
        raw.set(t::COMPRESSION, Value::Short(vec![BLOCKS16]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
        raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
        raw.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![pack(px)] });
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("SEIKO EPSON CORP.".into()));
        ifd0.set(t::MODEL, Value::Ascii("R-D1".into()));
        ifd0.set(t::ORIENTATION, Value::Short(vec![1]));
        ifd0.add_sub_ifd(raw);
        TiffWriter::new(craft_tiff::ByteOrder::Big, false).write(&[ifd0]).unwrap()
    }

    #[test]
    fn decodes_the_block_layout() {
        let (w, h) = (20u32, 6u32);
        let px: Vec<u16> = (0..w * h).map(|i| (i * 37 % 4096) as u16).collect();
        let f = sample(w, h, &px);
        assert_eq!(crate::probe(&f), Some(RawFormat::Erf));
        let r = crate::decode(&f).unwrap();
        assert_eq!(r.data, RawData::U16(px));
        assert_eq!(r.cfa.as_ref().map(Cfa::name).as_deref(), Some("RGGB"));
        assert_eq!(r.wb_multipliers, None, "the file states none");
        // no maker note: no stated black; no plateau
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Fallback, white: crate::LevelSource::Fallback });
        assert_eq!(crate::probe_info(&f).unwrap(), r.info());
    }

    #[test]
    fn malformed_files_never_panic() {
        let px: Vec<u16> = (0..200).map(|i| (i * 13 % 4096) as u16).collect();
        let f = sample(20, 10, &px);
        for n in (0..f.len()).step_by(3) {
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
        declare(&mut f, 2000, 32 * 2000);
        assert_eq!(crate::probe(&f), Some(RawFormat::Erf));
        assert!(matches!(crate::probe_info(&f), Err(RawError::Corrupt(_))));
        assert!(matches!(crate::decode(&f), Err(RawError::Corrupt(_))));
    }
}
