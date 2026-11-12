//! Kodak DCS520C raw TIFF: the CFA sub-IFD holds one lossless JPEG (SOF3) stream of two components.
//!
//! Sources: TIFF 6.0 (container; `GrayResponseCurve` `0x123`; the CFA tags `CFARepeatPatternDim` and `CFAPattern`),
//! ITU T.81 (lossless JPEG, process SOF3), the lossless-JPEG decoder in [`crate::ljpeg`], the file's `KodakIFD`
//! (`0x8290`, a plain IFD with absolute offsets) and black-box analysis of the one CC0 sample from
//! raw.pixls.us (2573, DCS520C; every number below was measured on it):
//!
//! - The raw IFD is `Compression` 7, photometric 32803 (CFA), 12 bits, one strip, `ImageWidth` x `ImageLength` =
//!   1736 x 1160. Its stream is a 12-bit lossless JPEG of 868 x 1160 with two components; the decoder's interleaved
//!   output (component 0, component 1, per pixel) is the mosaic row by row, so component 0 is the even columns and
//!   component 1 the odd ones. The stream ends exactly at the strip's end. Recognised structurally: a Kodak make
//!   and a stream whose width times its component count is the IFD's `ImageWidth`.
//! - `CFAPattern` bytes `01 00 02 01` (anchored at the first sample) are GRBG; the decoded colours follow the IFD0
//!   RGB thumbnail (156 x 104) with that layout.
//! - The samples span only 191..=622 because they are stored companded: IFD `GrayResponseCurve` (`0x123`, 4096
//!   entries) is a non-decreasing table that is the identity below 216 and expands the rest to 12-bit linear
//!   values (623 distinct values, stored 622 gives 4095 and so does every higher code). Applying it makes the
//!   frame linear with 12-bit white 4095.
//! - No masked border: all four edges are picture. Black differs per colour: the darkest 0.01% of each CFA
//!   position's samples sit at 220 (G), 223 (R) and 197 (B) after the table (scene shadows end at the sensor's
//!   black), so black is measured per position from the samples.
//! - White balance: five rational triples in the `KodakIFD` look like neutral sensor values (`0x848`, `0x849`,
//!   `0x84a`, `0x84c`, `0x84d`). Against the thumbnail's neutral pixels (370 pixels with R, G and B within 6 of
//!   each other) the raw G/R is 0.94 and G/B 1.96; `0x848` (1913, 1834, 921) gives 0.96 and 1.99, the others
//!   0.77..1.25 and 2.08..2.96, so its green over red and over blue are the gains.
use crate::{
    BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result, ljpeg,
};
use craft_tiff::image::chunk_bytes;
use craft_tiff::{Ifd, Tiff, tags as t};

/// The vendor IFD pointer (`KodakIFD`).
const KODAK_IFD: u16 = 0x8290;
/// Rational triple (red, green, blue) of neutral sensor values, see the module notes.
const NEUTRAL: u16 = 0x848;
/// TIFF `GrayResponseCurve`.
const RESPONSE_CURVE: u16 = 0x123;
/// Entries of the response curve: one per 12-bit code.
const CURVE_LEN: usize = 4096;

/// The raw sub-IFD of a Kodak DCS TIFF with this layout: the CFA image whose single lossless JPEG stream has two
/// components, `ImageWidth / 2` columns and `ImageLength` rows, 12 bits.
pub(crate) fn raw_ifd<'a>(tiff: &'a Tiff, bytes: &[u8]) -> Option<&'a Ifd> {
    let make = tiff.find(t::MAKE).and_then(|e| e.value.as_str())?;
    if !make.to_ascii_uppercase().starts_with("KODAK") {
        return None;
    }
    tiff.all_ifds().into_iter().find(|i| {
        if i.u16(t::PHOTOMETRIC) != Some(t::photometric::CFA) || i.u16(t::COMPRESSION) != Some(t::compression::JPEG) {
            return false;
        }
        let Ok(info) = i.image() else { return false };
        let chunks = info.chunks(bytes.len() as u64);
        let [chunk] = chunks.as_slice() else { return false };
        let Some(src) = chunk_bytes(bytes, chunk) else { return false };
        matches!(
            ljpeg::frame_info(src),
            Ok((w, h, 2, 12)) if w.checked_mul(2) == Some(info.width as usize) && h == info.height as usize
        ) && i.u64s(RESPONSE_CURVE).is_some_and(|c| c.len() == CURVE_LEN)
    })
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("Kodak TIFF without IFDs".into()))?;
    let raw = raw_ifd(&tiff, bytes).ok_or_else(|| RawError::Unsupported("Kodak raw image not found".into()))?;
    let info = raw.image()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let n = w.checked_mul(h).filter(|n| *n > 0 && *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let cfa = match (raw.u64s(t::CFA_REPEAT_PATTERN_DIM).as_deref(), raw.bytes(t::CFA_PATTERN_EP)) {
        (Some([2, 2]), Some(p)) if p.len() == 4 && p.iter().all(|&c| c <= 2) => Cfa { width: 2, height: 2, pattern: p.to_vec() },
        _ => return Err(RawError::Unsupported("Kodak raw without a 2x2 CFAPattern".into())),
    };
    let curve: Vec<u16> = raw
        .u64s(RESPONSE_CURVE)
        .filter(|c| c.len() == CURVE_LEN && c.windows(2).all(|p| p[0] <= p[1]) && c.iter().all(|&v| v < CURVE_LEN as u64))
        .ok_or_else(|| RawError::Unsupported("Kodak raw without a usable response curve".into()))?
        .into_iter()
        .map(|v| v as u16)
        .collect();
    let (black, data) = if mode == Mode::Full {
        let chunks = info.chunks(bytes.len() as u64);
        let src = chunks.first().and_then(|c| chunk_bytes(bytes, c)).ok_or_else(|| RawError::Corrupt("Kodak strip outside file".into()))?;
        let f = ljpeg::decode(src, n)?;
        if f.components != 2 || f.width * 2 != w || f.height != h || f.data.len() != n {
            return Err(RawError::Corrupt("Kodak lossless JPEG does not match the image size".into()));
        }
        let mut d = f.data;
        for v in &mut d {
            *v = curve.get(usize::from(*v)).copied().unwrap_or(CURVE_LEN as u16 - 1);
        }
        (measured_black(&d, w), RawData::U16(d))
    } else {
        (BlackLevel::uniform(0.0), RawData::U16(Vec::new()))
    };
    let wb = kodak_ifd(bytes, &tiff, ifd0)
        .and_then(|k| k.f64s(NEUTRAL))
        .filter(|v| v.len() == 3 && v.iter().all(|&x| x > 0.0))
        .map(|v| [(v[1] / v[0]) as f32, 1.0, (v[1] / v[2]) as f32]);
    let img = RawImage {
        format: RawFormat::KodakDcs,
        width: w,
        height: h,
        cpp: 1,
        data,
        cfa: Some(cfa),
        bits: 12,
        black,
        white: vec![(CURVE_LEN - 1) as f32],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: ColorData::default(),
        wb_multipliers: wb,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    };
    img.validate_for(mode)?;
    Ok(img)
}

/// The vendor IFD (`KodakIFD`): a plain IFD with absolute value offsets.
fn kodak_ifd(bytes: &[u8], tiff: &Tiff, ifd0: &Ifd) -> Option<Ifd> {
    let off = u64::from(ifd0.u32(KODAK_IFD)?);
    let opts = craft_tiff::ParseOptions { max_ifds: 4, max_depth: 1, follow_children: false, ..Default::default() };
    craft_tiff::parse_ifd_at(bytes, off, tiff.order, 0, false, &opts).ok().map(|(i, _)| i)
}

/// Black level per CFA position (the mosaic's first sample is position 0): the value below which only 0.01% of that
/// position's samples fall, see the module notes. Uniform 0 for an empty frame.
fn measured_black(d: &[u16], w: usize) -> BlackLevel {
    let mut hist = vec![[0u32; CURVE_LEN]; 4];
    let mut count = [0u64; 4];
    for (y, row) in d.chunks(w.max(1)).enumerate() {
        for (x, &v) in row.iter().enumerate() {
            let p = (y & 1) * 2 + (x & 1);
            hist[p][usize::from(v).min(CURVE_LEN - 1)] += 1;
            count[p] += 1;
        }
    }
    if count.contains(&0) {
        return BlackLevel::uniform(0.0);
    }
    let values = (0..4)
        .map(|p| {
            let limit = count[p].div_ceil(10_000);
            let mut seen = 0u64;
            let level = hist[p]
                .iter()
                .position(|&c| {
                    seen += u64::from(c);
                    seen >= limit
                })
                .unwrap_or(0);
            level as f32
        })
        .collect();
    BlackLevel { repeat_rows: 2, repeat_cols: 2, values, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, probe, probe_info};
    use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};

    const W: usize = 16;
    const H: usize = 8;

    /// A response curve: the identity below 100, then steeper, saturating at 4095.
    fn curve() -> Vec<u16> {
        (0..CURVE_LEN as u32).map(|i| if i < 100 { i as u16 } else { (100 + (i - 100) * 7).min(4095) as u16 }).collect()
    }

    /// Stored mosaic: values 60..=300, in the range the curve expands.
    fn stored() -> Vec<u16> {
        (0..W * H).map(|i| 60 + ((i * 37) % 241) as u16).collect()
    }

    /// A big-endian IFD of `(tag, SRATIONAL triple)` entries at `at` in the file.
    fn kodak_ifd_bytes(at: usize, tag: u16, v: [i32; 3]) -> Vec<u8> {
        let mut b = vec![0, 1];
        b.extend(tag.to_be_bytes());
        b.extend(10u16.to_be_bytes());
        b.extend(3u32.to_be_bytes());
        b.extend(((at + 2 + 12 + 4) as u32).to_be_bytes());
        b.extend([0; 4]);
        for x in v {
            b.extend(x.to_be_bytes());
            b.extend(1i32.to_be_bytes());
        }
        b
    }

    fn file(make: &str, stream: Vec<u8>, width: u32, with_curve: bool, wb: Option<[i32; 3]>) -> Vec<u8> {
        let mut raw = IfdBuilder::new();
        raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        raw.set(t::IMAGE_WIDTH, Value::Short(vec![width as u16]));
        raw.set(t::IMAGE_LENGTH, Value::Short(vec![H as u16]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![12]));
        raw.set(t::COMPRESSION, Value::Short(vec![7]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![32803]));
        raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        if with_curve {
            raw.set(RESPONSE_CURVE, Value::Short(curve()));
        }
        raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
        raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![1, 0, 2, 1]));
        raw.set_image(ImageData::Strips { rows_per_strip: H as u32, strips: vec![stream] });
        let build = |ptr: u32| {
            let mut ifd0 = IfdBuilder::new();
            ifd0.set(t::MAKE, Value::Ascii(make.into()));
            ifd0.set(t::MODEL, Value::Ascii("DCS520C".into()));
            if wb.is_some() {
                ifd0.set(KODAK_IFD, Value::Long(vec![ptr]));
            }
            ifd0.add_sub_ifd(raw.clone());
            TiffWriter::new(ByteOrder::Big, false).write(&[ifd0]).unwrap()
        };
        let mut bytes = build(0);
        if let Some(v) = wb {
            let at = bytes.len().next_multiple_of(2);
            bytes = build(at as u32);
            bytes.resize(at, 0);
            bytes.extend(kodak_ifd_bytes(at, NEUTRAL, v));
        }
        bytes
    }

    fn stream() -> Vec<u8> {
        ljpeg::encode(&stored(), W / 2, H, 2, 12, 1, 0)
    }

    /// Known answer: two interleaved components are the even and odd columns, the response curve linearises them,
    /// the layout is GRBG, white balance comes from the vendor IFD, black is measured per CFA position.
    #[test]
    fn two_component_stream_decodes_through_the_response_curve() {
        let bytes = file("Kodak", stream(), W as u32, true, Some([1900, 1800, 900]));
        assert_eq!(probe(&bytes), Some(RawFormat::KodakDcs));
        let r = decode(&bytes).unwrap();
        assert_eq!((r.format, r.width, r.height, r.bits), (RawFormat::KodakDcs, W, H, 12));
        let want: Vec<u16> = stored().iter().map(|&v| curve()[usize::from(v)]).collect();
        assert_eq!(r.data, RawData::U16(want.clone()));
        assert_eq!(r.cfa.as_ref().unwrap().name(), "GRBG");
        assert_eq!(r.white, vec![4095.0]);
        assert_eq!(r.wb_multipliers, Some([1800.0 / 1900.0, 1.0, 2.0]));
        // 0.01% of 32 samples is the smallest one of each position
        let w = &want;
        let min = |y0: usize, x0: usize| (y0..H).step_by(2).flat_map(|y| (x0..W).step_by(2).map(move |x| w[y * W + x])).min().unwrap() as f32;
        assert_eq!(r.black.values, vec![min(0, 0), min(0, 1), min(1, 0), min(1, 1)]);
        let i = probe_info(&bytes).unwrap();
        assert_eq!((i.format, i.width, i.height, i.wb_multipliers), (RawFormat::KodakDcs, W, H, r.wb_multipliers));
    }

    /// Without the vendor IFD there is no white balance; the layout is recognised from the structure alone, so
    /// another make, one component, a stream of another width or a missing curve change nothing for other files.
    #[test]
    fn only_this_layout_is_claimed() {
        assert_eq!(decode(&file("Kodak", stream(), W as u32, true, None)).unwrap().wb_multipliers, None);
        assert_ne!(probe(&file("Canon", stream(), W as u32, true, None)), Some(RawFormat::KodakDcs));
        assert_ne!(probe(&file("Kodak", stream(), W as u32 + 2, true, None)), Some(RawFormat::KodakDcs));
        assert_ne!(probe(&file("Kodak", stream(), W as u32, false, None)), Some(RawFormat::KodakDcs));
        let one = ljpeg::encode(&stored()[..W * H / 2], W, H / 2, 1, 12, 1, 0);
        assert_ne!(probe(&file("Kodak", one, W as u32, true, None)), Some(RawFormat::KodakDcs));
    }

    /// Hostile input: truncated streams and tables report errors instead of panicking.
    #[test]
    fn damaged_files_are_errors() {
        let s = stream();
        for cut in [s.len() / 2, 40, 0] {
            let bytes = file("Kodak", s[..cut].to_vec(), W as u32, true, Some([1, 1, 1]));
            if probe(&bytes) == Some(RawFormat::KodakDcs) {
                assert!(decode(&bytes).is_err() || cut == s.len());
            }
        }
        let bytes = file("Kodak", s, W as u32, true, Some([0, 5, 0]));
        assert_eq!(decode(&bytes).unwrap().wb_multipliers, None);
    }
}
