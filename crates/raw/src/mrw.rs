//! Minolta / Konica Minolta MRW: a block container around a 12-bit Bayer mosaic.
//!
//! Sources: the public prose description of the MRW layout (Dalibor Jelínek's MRW notes) and black-box
//! analysis of the eight CC0 MRW samples on raw.pixls.us (DiMAGE 7, 7Hi, A1, A2, A200, Dynax / Maxxum / ALPHA-7
//! Digital); every number below was measured on them.
//!
//! - `PRD` (24 bytes, big-endian): 8 ASCII version bytes, sensor rows and columns (`u16` each), image rows and
//!   columns, data size (12 or 16 bits per stored sample), pixel size (12), storage method (`0x59` = 12-bit
//!   packed: two samples in three bytes, most significant bits first; `0x52` = one 12-bit sample per big-endian
//!   16-bit word), two unknown bytes and the Bayer layout as a `u16` (`1` = RGGB, `4` = GBRG). The sensor data
//!   starts after the header and is exactly sensor rows x columns samples in every sample.
//! - The sensor frame is 8 columns and 6 to 8 rows larger than the image size, but every edge holds picture (no
//!   masked border), so the whole frame is the active area and the image size is the default crop.
//! - Black is 0: the darkest samples are 0 in every packed file and 13..34 in the unpacked ones, with no offset
//!   left to subtract. White is the saturation plateau where the frame has one (3964 on the A1, 3965 on a clipped
//!   DiMAGE 7, 4020 on the A200, 4076 on the 7D), else 4095.
//! - The layout code was checked against the embedded preview: with `1` the two greens are the equal (0,1) and
//!   (1,0) positions; in the A200 (`4`) the greens are (0,0) and (1,1), and only red at (1,0) reproduces the
//!   preview's red-dominated colour.
//! - `WBG` (12 bytes): four scale bytes, then four big-endian `u16` coefficients in the order of the Bayer cell
//!   read row by row (RGGB: R G G B, GBRG: G B R G). Red over green and blue over green are the as-shot gains;
//!   applied to the raw channel means they reproduce the preview's colour balance within the colour matrix's
//!   reach on all eight samples.
//! - `TTW` is a TIFF with IFD0 (orientation), Exif and the maker note; the preview JPEG sits in it with its first
//!   byte overwritten (see `preview::scan_for_jpeg`).
//!
//! [`parse`] reads the block container on its own, for a caller that wants a block (the `TTW` Exif, say)
//! without decoding the mosaic.
use crate::{
    BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result, unpack,
};
use craft_tiff::{Tiff, tags as t};

/// Storage method codes in `PRD`.
const PACKED_12: u8 = 0x59;
const WORDS_16: u8 = 0x52;

/// What `PRD` says about the sensor data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Prd {
    sensor: (usize, usize),
    image: (usize, usize),
    packed: bool,
    cfa: &'static str,
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}

fn prd(b: &[u8]) -> Result<Prd> {
    let u = |at| be16(b, at).map(usize::from).ok_or_else(|| RawError::Corrupt("Mrw PRD block too short".into()));
    let (sh, sw, ih, iw) = (u(8)?, u(10)?, u(12)?, u(14)?);
    let (size, storage) = (b.get(16).copied(), b.get(18).copied());
    let packed = match (size, storage) {
        (Some(12), Some(PACKED_12)) => true,
        (Some(16), Some(WORDS_16)) => false,
        _ => return Err(RawError::Unsupported(format!("Mrw storage {size:?}/{storage:?} is not decoded yet"))),
    };
    let cfa = match u(22)? {
        1 => "RGGB",
        4 => "GBRG",
        other => return Err(RawError::Unsupported(format!("Mrw Bayer layout {other}"))),
    };
    if sw == 0 || sh == 0 || (packed && sw % 2 != 0) {
        return Err(RawError::Corrupt("Mrw sensor size".into()));
    }
    Ok(Prd { sensor: (sw, sh), image: (iw.min(sw), ih.min(sh)), packed, cfa })
}

// --- the block container: the file starts with `\0MRM` and a big-endian `u32` length; the sensor data begins right
// after that many bytes (at `8 + length`). In between is a sequence of blocks, each a four-byte name (`\0PRD`,
// `\0TTW`, `\0WBG`, `\0RIF`, `\0PAD`), a big-endian `u32` length and that many bytes. `\0TTW` holds a complete
// big-endian TIFF (IFD0, Exif, maker note) whose offsets are relative to the block.

/// The blocks and the offset of the sensor data in an MRW file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mrw<'a> {
    /// `(name without the leading NUL, contents)`, in file order.
    pub blocks: Vec<([u8; 3], &'a [u8])>,
    /// Absolute offset of the sensor data.
    pub data_offset: usize,
}

impl<'a> Mrw<'a> {
    /// The first block called `name` (e.g. `b"PRD"`).
    pub fn block(&self, name: &[u8; 3]) -> Option<&'a [u8]> {
        self.blocks.iter().find(|(n, _)| n == name).map(|(_, d)| *d)
    }
}

/// At most this many blocks are walked (real files have four or five).
pub const MAX_BLOCKS: usize = 64;

fn be32(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// Parse the block list of an MRW file. `None` when the magic is missing or the header length points past the
/// file; a block that runs past the header is dropped together with everything after it, and at most
/// [`MAX_BLOCKS`] blocks are read. Nothing is decoded and nothing is copied.
pub fn parse(bytes: &[u8]) -> Option<Mrw<'_>> {
    if !bytes.starts_with(b"\0MRM") {
        return None;
    }
    let data_offset = be32(bytes, 4)?.checked_add(8).filter(|&o| o <= bytes.len())?;
    let mut blocks = Vec::new();
    let mut at = 8usize;
    while blocks.len() < MAX_BLOCKS && at.saturating_add(8) <= data_offset {
        let (Some(name), Some(len)) = (bytes.get(at..at + 4), be32(bytes, at + 4)) else { break };
        let start = at + 8;
        let Some(end) = start.checked_add(len).filter(|&e| e <= data_offset) else { break };
        let Some(data) = bytes.get(start..end) else { break };
        if name[0] == 0 {
            blocks.push(([name[1], name[2], name[3]], data));
        }
        at = end;
    }
    Some(Mrw { blocks, data_offset })
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let m = parse(bytes).ok_or_else(|| RawError::Corrupt("Mrw block header".into()))?;
    let p = prd(m.block(b"PRD").ok_or_else(|| RawError::Unsupported("Mrw file without a PRD block".into()))?)?;
    let (w, h) = p.sensor;
    let n = w.checked_mul(h).filter(|n| *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let need = if p.packed { n / 2 * 3 } else { n * 2 };
    let src = bytes
        .get(m.data_offset..)
        .and_then(|s| s.get(..need))
        .ok_or_else(|| RawError::Corrupt(format!("Mrw sensor data truncated (needs {need} bytes)")))?;
    let cfa = Cfa::bayer_static(p.cfa);
    let (data, white, white_source) = if mode == Mode::Full {
        let mut d = vec![0u16; n];
        if p.packed {
            unpack::unpack_msb(src, 12, &mut d);
        } else {
            unpack::read_u16s(src, craft_tiff::ByteOrder::Big, &mut d);
            for v in &mut d {
                *v = (*v).min(4095);
            }
        }
        let (white, white_source) = crate::shared::white_from_data_sourced(&d, 12);
        (RawData::U16(d), white, white_source)
    } else {
        (RawData::U16(Vec::new()), 4095.0, crate::LevelSource::Unknown)
    };
    let wb = m.block(b"WBG").and_then(|b| wb_from(b, &cfa));
    let tiff = m.block(b"TTW").and_then(|b| Tiff::parse(b).ok());
    let orientation = tiff.as_ref().and_then(|t| t.ifds.first()).and_then(|i| i.u16(t::ORIENTATION)).unwrap_or(1);
    let img = RawImage {
        format: RawFormat::Mrw,
        width: w,
        height: h,
        cpp: 1,
        data,
        cfa: Some(cfa),
        bits: 12,
        black: BlackLevel::uniform(0.0),
        white: vec![white],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, p.image.0, p.image.1),
        orientation: Orientation::from_exif(orientation),
        color: ColorData::default(),
        wb_multipliers: wb,
        linearized: false,
        opcodes: OpcodeLists::default(),
        camera: tiff.as_ref().and_then(|t| t.ifds.first()).map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: crate::LevelSources { black: crate::LevelSource::Fallback, white: white_source },
        pixel_aspect: 1.0,
    };
    img.validate_for(mode)?;
    Ok(img)
}

/// As-shot gains from `WBG`: four coefficients in Bayer-cell order after four scale bytes.
fn wb_from(b: &[u8], cfa: &Cfa) -> Option<[f32; 3]> {
    let mut sum = [0f32; 3];
    let mut count = [0f32; 3];
    for (i, &c) in cfa.pattern.iter().enumerate().take(4) {
        let v = f32::from(be16(b, 4 + 2 * i)?);
        let k = usize::from(c).min(2);
        sum[k] += v;
        count[k] += 1.0;
    }
    if count.contains(&0.0) || sum.iter().any(|&s| s <= 0.0) {
        return None;
    }
    let mean = |k: usize| sum[k] / count[k];
    Some([mean(0) / mean(1), 1.0, mean(2) / mean(1)])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic MRW: `PRD` with the given storage, `WBG`, a `TTW` TIFF with orientation 6, then the samples.
    pub(crate) fn sample(w: u16, h: u16, packed: bool, layout: u16, wbg: [u16; 4], samples: &[u16]) -> Vec<u8> {
        let mut prd = b"21810002".to_vec();
        for v in [h, w, h - 2, w - 2] {
            prd.extend_from_slice(&v.to_be_bytes());
        }
        prd.extend_from_slice(&[if packed { 12 } else { 16 }, 12, if packed { PACKED_12 } else { WORDS_16 }, 0, 0, 0]);
        prd.extend_from_slice(&layout.to_be_bytes());
        let mut wb = vec![2u8; 4];
        for v in wbg {
            wb.extend_from_slice(&v.to_be_bytes());
        }
        let mut ifd0 = craft_tiff::IfdBuilder::new();
        ifd0.set(t::MAKE, craft_tiff::Value::Ascii("Minolta Co., Ltd.".into()));
        ifd0.set(t::ORIENTATION, craft_tiff::Value::Short(vec![6]));
        let ttw = craft_tiff::TiffWriter::default().write(&[ifd0]).unwrap();
        let mut body = Vec::new();
        for (name, data) in [(b"PRD", &prd), (b"WBG", &wb), (b"TTW", &ttw)] {
            body.push(0);
            body.extend_from_slice(name);
            body.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(data);
        }
        let mut f = b"\0MRM".to_vec();
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend_from_slice(&body);
        if packed {
            for p in samples.chunks(2) {
                let (a, b) = (p[0], p.get(1).copied().unwrap_or(0));
                f.extend_from_slice(&[(a >> 4) as u8, ((a & 15) << 4) as u8 | (b >> 8) as u8, b as u8]);
            }
        } else {
            for v in samples {
                f.extend_from_slice(&v.to_be_bytes());
            }
        }
        f
    }

    #[test]
    fn decodes_packed_and_word_storage() {
        let (w, h) = (8u16, 6u16);
        let px: Vec<u16> = (0..48).map(|i| (i * 85) as u16 % 4096).collect();
        for packed in [true, false] {
            let f = sample(w, h, packed, 1, [408, 256, 256, 450], &px);
            assert_eq!(crate::probe(&f), Some(RawFormat::Mrw));
            let r = crate::decode(&f).unwrap();
            assert_eq!((r.width, r.height, r.bits), (8, 6, 12));
            assert_eq!(r.data, RawData::U16(px.clone()), "packed {packed}");
            assert_eq!(r.cfa.as_ref().map(Cfa::name).as_deref(), Some("RGGB"));
            assert_eq!(r.crop, Rect::new(0, 0, 6, 4));
            assert_eq!(r.orientation, Orientation::from_exif(6));
            assert_eq!(r.camera.make.as_deref(), Some("Minolta Co., Ltd."));
            let wb = r.wb_multipliers.unwrap();
            assert!((wb[0] - 408.0 / 256.0).abs() < 1e-6 && (wb[2] - 450.0 / 256.0).abs() < 1e-6, "{wb:?}");
            assert_eq!(
                r.level_sources,
                crate::LevelSources { black: crate::LevelSource::Fallback, white: crate::LevelSource::Fallback },
                "packed {packed}"
            );
            assert_eq!(crate::probe_info(&f).unwrap(), r.info());
        }
    }

    /// GBRG (layout 4): the coefficients follow the cell, so the second is blue and the third red.
    #[test]
    fn gbrg_white_balance_follows_the_cell() {
        let f = sample(4, 4, true, 4, [255, 416, 480, 255], &[100; 16]);
        let r = crate::decode(&f).unwrap();
        assert_eq!(r.cfa.as_ref().map(Cfa::name).as_deref(), Some("GBRG"));
        let wb = r.wb_multipliers.unwrap();
        assert!((wb[0] - 480.0 / 255.0).abs() < 1e-6 && (wb[2] - 416.0 / 255.0).abs() < 1e-6, "{wb:?}");
    }

    /// Truncations and corrupted header bytes give errors, never panics.
    #[test]
    fn malformed_files_never_panic() {
        let f = sample(8, 6, true, 1, [408, 256, 256, 450], &[7; 48]);
        for n in 0..f.len() {
            assert!(crate::decode(&f[..n]).is_err() || n == f.len());
        }
        for i in 8..f.len().min(120) {
            for v in [0u8, 0xff, 0x52, 0x59] {
                let mut g = f.clone();
                g[i] = v;
                let _ = crate::decode(&g);
                let _ = crate::probe_info(&g);
            }
        }
        assert!(matches!(crate::decode(b"\0MRM\0\x01\0\0"), Err(RawError::Corrupt(_))));
        assert!(matches!(crate::decode(b"\0MRM\0\0\0\0"), Err(RawError::Unsupported(w)) if w.contains("Mrw")));
    }
}

#[cfg(test)]
mod container_tests {
    use super::*;

    fn block(name: &[u8; 3], data: &[u8]) -> Vec<u8> {
        let mut v = vec![0];
        v.extend_from_slice(name);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn walks_blocks_and_finds_the_data() {
        let mut body = block(b"PRD", &[1, 2, 3]);
        body.extend(block(b"WBG", &[4; 12]));
        let mut f = b"\0MRM".to_vec();
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend_from_slice(&body);
        f.extend_from_slice(&[9, 9]);
        let m = parse(&f).unwrap();
        assert_eq!(m.block(b"PRD"), Some(&[1u8, 2, 3][..]));
        assert_eq!(m.block(b"WBG").map(<[u8]>::len), Some(12));
        assert_eq!(m.block(b"TTW"), None);
        assert_eq!(m.data_offset, f.len() - 2);
    }

    #[test]
    fn hostile_lengths_never_panic() {
        assert_eq!(parse(b"\0MRM"), None);
        assert_eq!(parse(b"\0MRM\xff\xff\xff\xff"), None);
        // a block claiming more than the header holds is dropped
        let f = b"\0MRM\0\0\0\x0c\0PRD\xff\xff\xff\xff\0\0\0\0";
        assert_eq!(parse(f).map(|m| m.blocks.len()), Some(0));
        for n in 0..f.len() {
            let _ = parse(&f[..n]);
        }
    }

    #[test]
    fn at_most_max_blocks_are_read() {
        let body: Vec<u8> = (0..MAX_BLOCKS + 6).flat_map(|_| block(b"PAD", &[0])).collect();
        let mut f = b"\0MRM".to_vec();
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend_from_slice(&body);
        let m = parse(&f).unwrap();
        assert_eq!(m.blocks.len(), MAX_BLOCKS);
        assert_eq!(m.data_offset, f.len());
    }
}
