//! Decoded pictures as compositor frames: [`compositor_frame`] takes the codec-agnostic picture
//! a `craft-video` decoder returns (`craft_video::frame::VideoFrame`) and builds the
//! [`VideoFrame`] a [`crate::MediaSource`] returns.

use std::sync::Arc;

use craft_color::{ColorInfo, Matrix, Primaries, Range, Transfer, to_linear};
use craft_frame::{Chroma, DEFAULT_MAX_FRAME_BYTES, PixelData, VideoFrame};
use craft_time::Tick;
use craft_video::frame::{ColorFamily, ColorTags, Plane, Samples, Subsampling, VideoFrame as DecodedFrame};

use crate::{MediaError, Result, allocate};

/// [`compositor_frame_within`] with buffers bounded by [`DEFAULT_MAX_FRAME_BYTES`].
pub fn compositor_frame(frame: DecodedFrame, pts: Tick) -> Result<VideoFrame> {
    compositor_frame_within(frame, pts, DEFAULT_MAX_FRAME_BYTES)
}

/// The compositor [`VideoFrame`] at `pts` for a picture a `craft-video` decoder returned. Every
/// buffer it builds is at most `max_bytes`; a larger one is [`MediaError::Io`].
///
/// - Y'CbCr and gray pictures become planar Y'CbCr: [`PixelData::Yuv8`] when no plane, alpha
///   included, is deeper than 8 bits, otherwise [`PixelData::Yuv16`] at the deepest plane's bit
///   depth. Shallower color planes are shifted up to that depth; a shallower alpha plane is
///   rescaled so that its maximum stays opaque. 4:2:0, 4:2:2 and 4:4:4 keep their subsampling,
///   and any other subsampling (4:4:0, and FFV1's shifts of 2 and 3) is repeated out to 4:4:4.
///   A gray picture gets neutral 4:2:0 chroma.
/// - RGB pictures become [`PixelData::Rgba8`] (straight alpha) when every plane, alpha included,
///   is 8 bits and the transfer is BT.709 or sRGB, otherwise [`PixelData::RgbaF32`] (linear
///   light, premultiplied), tagged full range. The samples of an RGB picture tagged limited range
///   are expanded to full range from black at 16 and white at 235, scaled to the plane's depth.
///   Alpha is read as full range.
/// - The ITU-T H.273 color tags map onto [`ColorInfo`]. A matrix, transfer or primaries code
///   that is unspecified or has no [`ColorInfo`] value takes BT.709, except that pictures under
///   720 lines take the BT.601 matrix and BT.601 primaries (625-line primaries at 288 and 576
///   lines, 525-line ones otherwise).
/// - The sample aspect ratio becomes the pixel aspect ratio, square when the picture has none.
///   The field order and the draft flag are not kept.
pub fn compositor_frame_within(frame: DecodedFrame, pts: Tick, max_bytes: usize) -> Result<VideoFrame> {
    let (width, height) = (frame.width(), frame.height());
    let family = frame.family();
    let subsampling = frame.subsampling();
    let par = frame.sar.filter(|&(n, d)| n > 0 && d > 0).unwrap_or((1, 1));
    let tags = frame.color;
    let mut color = color_info(tags, height);
    let (planes, alpha) = frame.into_planes();
    let data = match family {
        ColorFamily::Rgb => {
            color.range = Range::Full;
            let (data, transfer) = rgb(planes, alpha, color.transfer, !tags.full_range, max_bytes)?;
            color.transfer = transfer;
            data
        }
        ColorFamily::YCbCr | ColorFamily::Gray => planar(planes, alpha, subsampling, max_bytes)?,
    };
    let out = VideoFrame { width, height, data, color, par, pts };
    out.validate().map_err(|e| MediaError::Decode(format!("decoded picture: {e}")))?;
    Ok(out)
}

/// The H.273 code points as [`ColorInfo`], with the defaults [`compositor_frame_within`] names.
fn color_info(tags: ColorTags, height: u32) -> ColorInfo {
    let sd = height < 720;
    let matrix = Matrix::from_code(tags.matrix).unwrap_or(if sd { Matrix::Bt601 } else { Matrix::Bt709 });
    let transfer = Transfer::from_code(tags.transfer).unwrap_or(Transfer::Bt709);
    let primaries = primaries_from_code(tags.primaries).unwrap_or(match height {
        288 | 576 => Primaries::Bt601_625,
        _ if sd => Primaries::Bt601_525,
        _ => Primaries::Bt709,
    });
    let range = if tags.full_range { Range::Full } else { Range::Limited };
    ColorInfo { matrix, transfer, primaries, range }
}

/// H.273 `ColourPrimaries` (table 2): 1 BT.709, 5 BT.601 625-line, 6 and 7 the 525-line
/// primaries of SMPTE 170M and 240M, 9 BT.2020, 12 P3 with the D65 white point.
fn primaries_from_code(c: u8) -> Option<Primaries> {
    match c {
        1 => Some(Primaries::Bt709),
        5 => Some(Primaries::Bt601_625),
        6 | 7 => Some(Primaries::Bt601_525),
        9 => Some(Primaries::Bt2020),
        12 => Some(Primaries::P3D65),
        _ => None,
    }
}

fn decode_error(what: &str) -> MediaError {
    MediaError::Decode(format!("decoded picture: {what}"))
}

/// The largest sample value a plane of `depth` bits holds.
fn depth_max(depth: u8) -> u16 {
    u16::MAX.checked_shr(u32::from(16u8.saturating_sub(depth.min(16)))).unwrap_or(0)
}

fn plane_size(p: &Plane) -> Result<(usize, usize)> {
    let w = usize::try_from(p.width()).map_err(|_| decode_error("plane width"))?;
    let h = usize::try_from(p.height()).map_err(|_| decode_error("plane height"))?;
    Ok((w, h))
}

/// The plane's rows packed one after another, each sample passed through `f`.
fn pack<T: Default + Clone>(p: &Plane, max_bytes: usize, f: impl Fn(u16) -> T) -> Result<Vec<T>> {
    let (w, h) = plane_size(p)?;
    let len = w.checked_mul(h).ok_or_else(|| decode_error("plane size overflows"))?;
    let mut out = allocate::<T>(len, max_bytes)?;
    if len == 0 {
        return Ok(out);
    }
    for (y, row) in out.chunks_exact_mut(w).enumerate() {
        let y = u32::try_from(y).map_err(|_| decode_error("plane height"))?;
        match p.samples() {
            Samples::U8(_) => {
                let src = p.row_u8(y).ok_or_else(|| decode_error("short plane"))?;
                for (o, &s) in row.iter_mut().zip(src) {
                    *o = f(u16::from(s));
                }
            }
            Samples::U16(_) => {
                let src = p.row_u16(y).ok_or_else(|| decode_error("short plane"))?;
                for (o, &s) in row.iter_mut().zip(src) {
                    *o = f(s);
                }
            }
        }
    }
    Ok(out)
}

/// Whether the plane's buffer can be used as it is: rows packed, `storage_bits` storage, and no
/// rescaling to do.
fn reusable(p: &Plane, storage_bits: u8, depth: u8, max_bytes: usize) -> bool {
    let stored = match p.samples() {
        Samples::U8(v) => (8, v.len()),
        Samples::U16(v) => (16, v.len().saturating_mul(2)),
    };
    usize::try_from(p.width()).is_ok_and(|w| w == p.stride()) && stored.0 == storage_bits && p.bit_depth() == depth && stored.1 <= max_bytes
}

/// The plane as packed 8-bit samples, shifted up from a shallower depth.
fn plane_u8(p: Plane, max_bytes: usize) -> Result<Vec<u8>> {
    let (w, h) = plane_size(&p)?;
    if reusable(&p, 8, 8, max_bytes) {
        return match p.into_samples() {
            Samples::U8(mut v) => {
                v.truncate(w.saturating_mul(h));
                Ok(v)
            }
            Samples::U16(_) => Err(decode_error("sample storage")),
        };
    }
    let max = depth_max(p.bit_depth().min(8));
    let shift = 8 - p.bit_depth().min(8);
    pack(&p, max_bytes, |s| u8::try_from(s.min(max) << shift).unwrap_or(u8::MAX))
}

/// The plane as packed 16-bit samples at `bits`, shifted up from a shallower depth. Samples
/// above the plane's depth are clamped to its maximum.
fn plane_u16(p: Plane, bits: u8, max_bytes: usize) -> Result<Vec<u16>> {
    let (w, h) = plane_size(&p)?;
    if reusable(&p, 16, bits, max_bytes) {
        let max = depth_max(bits);
        return match p.into_samples() {
            Samples::U16(mut v) => {
                v.truncate(w.saturating_mul(h));
                if max < u16::MAX {
                    v.iter_mut().for_each(|s| *s = (*s).min(max));
                }
                Ok(v)
            }
            Samples::U8(_) => Err(decode_error("sample storage")),
        };
    }
    let depth = p.bit_depth().min(bits);
    let max = depth_max(depth);
    let shift = bits - depth;
    pack(&p, max_bytes, |s| s.min(max) << shift)
}

/// An alpha plane rescaled from its depth to `bits`, the maximum staying the maximum.
fn alpha_scaled(p: &Plane, bits: u8, max_bytes: usize) -> Result<Vec<u16>> {
    let from = u32::from(depth_max(p.bit_depth())).max(1);
    let to = u32::from(depth_max(bits));
    pack(p, max_bytes, |s| u16::try_from((u32::from(s).min(from) * to + from / 2) / from).unwrap_or(u16::MAX))
}

fn alpha_u8(p: Plane, max_bytes: usize) -> Result<Vec<u8>> {
    if p.bit_depth() == 8 {
        return plane_u8(p, max_bytes);
    }
    let v = alpha_scaled(&p, 8, max_bytes)?;
    Ok(v.into_iter().map(|s| u8::try_from(s).unwrap_or(u8::MAX)).collect())
}

fn alpha_u16(p: Plane, bits: u8, max_bytes: usize) -> Result<Vec<u16>> {
    if p.bit_depth() == bits {
        return plane_u16(p, bits, max_bytes);
    }
    alpha_scaled(&p, bits, max_bytes)
}

/// A packed chroma plane of `cw` by `ch` repeated out to `w` by `h`.
fn repeat<T: Default + Clone + Copy>(c: &[T], cw: usize, (sx, sy): (u8, u8), (w, h): (usize, usize), max_bytes: usize) -> Result<Vec<T>> {
    let len = w.checked_mul(h).ok_or_else(|| decode_error("plane size overflows"))?;
    let mut out = allocate::<T>(len, max_bytes)?;
    if len == 0 {
        return Ok(out);
    }
    for (y, row) in out.chunks_exact_mut(w).enumerate() {
        let start = (y >> sy).checked_mul(cw).ok_or_else(|| decode_error("plane size overflows"))?;
        let src = c.get(start..start.saturating_add(cw)).ok_or_else(|| decode_error("short chroma plane"))?;
        for (x, o) in row.iter_mut().enumerate() {
            *o = *src.get(x >> sx).ok_or_else(|| decode_error("short chroma plane"))?;
        }
    }
    Ok(out)
}

/// What [`planar`] does with the chroma planes.
enum ChromaPlan {
    Keep(Chroma),
    /// Repeat out to 4:4:4 from these shifts.
    Repeat(u8, u8),
    /// A gray picture: neutral 4:2:0 chroma.
    Neutral,
}

fn planar(planes: Vec<Plane>, alpha: Option<Plane>, subsampling: Subsampling, max_bytes: usize) -> Result<PixelData> {
    let plan = match (planes.len(), subsampling) {
        (1, _) => ChromaPlan::Neutral,
        (_, Subsampling::S420) => ChromaPlan::Keep(Chroma::C420),
        (_, Subsampling::S422) => ChromaPlan::Keep(Chroma::C422),
        (_, Subsampling::S444) => ChromaPlan::Keep(Chroma::C444),
        (_, s) => ChromaPlan::Repeat(s.x, s.y),
    };
    let size = planes.first().map(plane_size).transpose()?.ok_or_else(|| decode_error("no planes"))?;
    let chroma = match plan {
        ChromaPlan::Keep(c) => c,
        ChromaPlan::Repeat(..) => Chroma::C444,
        ChromaPlan::Neutral => Chroma::C420,
    };
    let (sx, sy) = chroma.shifts();
    let neutral_len = size.0.div_ceil(1 << sx).checked_mul(size.1.div_ceil(1 << sy)).ok_or_else(|| decode_error("plane size overflows"))?;
    let chroma_size = planes.get(1).map(plane_size).transpose()?.map_or(0, |(w, _)| w);
    let bits = planes.iter().chain(alpha.as_ref()).map(Plane::bit_depth).max().unwrap_or(8);
    let mut it = planes.into_iter();
    let mut next = || it.next().ok_or_else(|| decode_error("plane count"));
    if bits <= 8 {
        let y = plane_u8(next()?, max_bytes)?;
        let (cb, cr) = match plan {
            ChromaPlan::Neutral => {
                let mut c = allocate::<u8>(neutral_len, max_bytes)?;
                c.fill(128);
                (c.clone(), c)
            }
            ChromaPlan::Keep(_) => (plane_u8(next()?, max_bytes)?, plane_u8(next()?, max_bytes)?),
            ChromaPlan::Repeat(rx, ry) => {
                let (cb, cr) = (plane_u8(next()?, max_bytes)?, plane_u8(next()?, max_bytes)?);
                (repeat(&cb, chroma_size, (rx, ry), size, max_bytes)?, repeat(&cr, chroma_size, (rx, ry), size, max_bytes)?)
            }
        };
        let alpha = alpha.map(|a| alpha_u8(a, max_bytes)).transpose()?;
        return Ok(PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(cb), Arc::new(cr)], chroma, alpha: alpha.map(Arc::new) });
    }
    let y = plane_u16(next()?, bits, max_bytes)?;
    let (cb, cr) = match plan {
        ChromaPlan::Neutral => {
            let mut c = allocate::<u16>(neutral_len, max_bytes)?;
            c.fill(1 << (bits - 1));
            (c.clone(), c)
        }
        ChromaPlan::Keep(_) => (plane_u16(next()?, bits, max_bytes)?, plane_u16(next()?, bits, max_bytes)?),
        ChromaPlan::Repeat(rx, ry) => {
            let (cb, cr) = (plane_u16(next()?, bits, max_bytes)?, plane_u16(next()?, bits, max_bytes)?);
            (repeat(&cb, chroma_size, (rx, ry), size, max_bytes)?, repeat(&cr, chroma_size, (rx, ry), size, max_bytes)?)
        }
    };
    let alpha = alpha.map(|a| alpha_u16(a, bits, max_bytes)).transpose()?;
    Ok(PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(cb), Arc::new(cr)], chroma, bits: u32::from(bits), alpha: alpha.map(Arc::new) })
}

/// An 8-bit limited-range sample (black 16, white 235) at full range, rounded.
fn full_range_u8(s: u8) -> u8 {
    let v = (i32::from(s) - 16).clamp(0, 219);
    u8::try_from((v * 255 + 109) / 219).unwrap_or(u8::MAX)
}

/// RGB planes as RGBA, and the transfer the result is tagged with. `limited` color samples are
/// expanded to full range.
fn rgb(planes: Vec<Plane>, alpha: Option<Plane>, transfer: Transfer, limited: bool, max_bytes: usize) -> Result<(PixelData, Transfer)> {
    let [r, g, b]: [Plane; 3] = planes.try_into().map_err(|_| decode_error("plane count"))?;
    let (w, h) = plane_size(&r)?;
    let len = w.checked_mul(h).and_then(|n| n.checked_mul(4)).ok_or_else(|| decode_error("plane size overflows"))?;
    let encoded = matches!(transfer, Transfer::Bt709 | Transfer::Srgb);
    if encoded && [Some(&r), Some(&g), Some(&b), alpha.as_ref()].into_iter().flatten().all(|p| p.bit_depth() == 8) {
        let (mut r, mut g, mut b) = (plane_u8(r, max_bytes)?, plane_u8(g, max_bytes)?, plane_u8(b, max_bytes)?);
        if limited {
            r.iter_mut().chain(g.iter_mut()).chain(b.iter_mut()).for_each(|s| *s = full_range_u8(*s));
        }
        let a = alpha.map(|a| alpha_u8(a, max_bytes)).transpose()?;
        let mut out = allocate::<u8>(len, max_bytes)?;
        for (i, o) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let at = |v: &[u8]| v.get(i).copied().unwrap_or(0);
            *o = [at(&r), at(&g), at(&b), a.as_deref().map_or(u8::MAX, at)];
        }
        return Ok((PixelData::Rgba8(Arc::new(out)), transfer));
    }
    // Linear light through a table per plane (65 536 entries for 16-bit samples), or sample by
    // sample when the picture has fewer samples than the table or the table exceeds the limit.
    let linear = |p: &Plane| -> Result<Vec<f32>> {
        let max = depth_max(p.bit_depth());
        let scale = f32::from(max.max(1));
        // A limited-range code value is 2^(depth - 8) times the 8-bit one.
        let unit = (f32::from(max) + 1.0) / 256.0;
        let level = |s: u16| {
            let v = f32::from(s.min(max));
            if limited { ((v / unit - 16.0) / 219.0).clamp(0.0, 1.0) } else { v / scale }
        };
        let entries = usize::from(max) + 1;
        if entries > w.saturating_mul(h) || entries.saturating_mul(4) > max_bytes {
            return pack(p, max_bytes, |s| to_linear(level(s), transfer));
        }
        let mut table = allocate::<f32>(entries, max_bytes)?;
        for (v, t) in (0..=max).zip(table.iter_mut()) {
            *t = to_linear(level(v), transfer);
        }
        pack(p, max_bytes, |s| table.get(usize::from(s.min(max))).copied().unwrap_or(0.0))
    };
    let (r, g, b) = (linear(&r)?, linear(&g)?, linear(&b)?);
    let a = alpha
        .map(|a| {
            let max = f32::from(depth_max(a.bit_depth()));
            pack(&a, max_bytes, |s| (f32::from(s) / max).min(1.0))
        })
        .transpose()?;
    let mut out = allocate::<f32>(len, max_bytes)?;
    for (i, o) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let at = |v: &[f32]| v.get(i).copied().unwrap_or(0.0);
        let alpha = a.as_deref().map_or(1.0, at);
        *o = [at(&r) * alpha, at(&g) * alpha, at(&b) * alpha, alpha];
    }
    Ok((PixelData::RgbaF32(Arc::new(out)), Transfer::Linear))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_video::ffv1;

    /// An FFV1 round trip of `frame`: the picture the decoder returns, as the shared form.
    fn ffv1_round_trip(frame: &ffv1::Frame) -> DecodedFrame {
        let enc = ffv1::Encoder::new(frame, ffv1::EncoderConfig::default()).unwrap();
        let packet = enc.encode(frame).unwrap();
        let dec = ffv1::Decoder::new(&enc.config_record(), frame.width, frame.height).unwrap();
        DecodedFrame::try_from(dec.decode(&packet).unwrap()).unwrap()
    }

    fn ramp(len: usize, step: usize, max: u16) -> Vec<u16> {
        (0..len).map(|i| ((i * step) % (usize::from(max) + 1)) as u16).collect()
    }

    #[test]
    fn an_ffv1_420_picture_keeps_its_samples_in_8_bit_planes() {
        // FFV1 returns 16-bit storage even at 8 bits; the frame still lands in Yuv8.
        let (w, h) = (17u32, 9u32);
        let (cw, ch) = (9usize, 5usize);
        let y = ramp(17 * 9, 7, 255);
        let cb = ramp(cw * ch, 11, 255);
        let cr = ramp(cw * ch, 13, 255);
        let src = ffv1::Frame {
            width: w,
            height: h,
            colorspace: ffv1::Colorspace::YCbCr,
            bit_depth: 8,
            chroma_shift: (1, 1),
            planes: vec![y.clone(), cb.clone(), cr.clone()],
        };
        let pts = Tick::from_seconds_f64(2.5);
        let out = compositor_frame(ffv1_round_trip(&src), pts).unwrap();
        assert_eq!((out.width, out.height, out.pts, out.par), (w, h, pts, (1, 1)));
        let PixelData::Yuv8 { planes, chroma, alpha } = &out.data else { panic!("expected Yuv8, got {}", out.format_label()) };
        assert_eq!(*chroma, Chroma::C420);
        assert!(alpha.is_none());
        let wide = |v: &[u8]| v.iter().map(|&s| u16::from(s)).collect::<Vec<_>>();
        assert_eq!(wide(&planes[0]), y);
        assert_eq!(wide(&planes[1]), cb);
        assert_eq!(wide(&planes[2]), cr);
        // FFV1 signals no color: a 9-line picture takes the BT.601 defaults.
        assert_eq!(out.color.matrix, Matrix::Bt601);
        assert_eq!(out.color.range, Range::Limited);
    }

    #[test]
    fn mixed_depths_shift_up_to_the_deepest_plane_and_alpha_stays_opaque_at_its_maximum() {
        let (w, h) = (4u32, 2u32);
        let luma = Plane::packed(w, h, 10, Samples::U16(vec![0, 1, 512, 1023, 64, 940, 100, 200])).unwrap();
        let cb = Plane::packed(2, 2, 8, Samples::U8(vec![16, 128, 240, 255])).unwrap();
        let cr = Plane::packed(2, 2, 8, Samples::U8(vec![128, 64, 32, 0])).unwrap();
        let alpha = Plane::packed(w, h, 8, Samples::U8(vec![0, 255, 128, 255, 255, 1, 254, 255])).unwrap();
        let frame = DecodedFrame::new(w, h, ColorFamily::YCbCr, Subsampling::S422, vec![luma, cb, cr], Some(alpha)).unwrap();
        let out = compositor_frame(frame, Tick::ZERO).unwrap();
        let PixelData::Yuv16 { planes, chroma, bits, alpha } = &out.data else { panic!("expected Yuv16, got {}", out.format_label()) };
        assert_eq!((*chroma, *bits), (Chroma::C422, 10));
        assert_eq!(planes[0].as_slice(), &[0, 1, 512, 1023, 64, 940, 100, 200]);
        // 8-bit chroma is the same code value at 10 bits.
        assert_eq!(planes[1].as_slice(), &[16 << 2, 128 << 2, 240 << 2, 255 << 2]);
        let alpha = alpha.as_ref().unwrap();
        assert_eq!((alpha[0], alpha[1], alpha[3]), (0, 1023, 1023));
        assert!(alpha[2] > 500 && alpha[2] < 520);
    }

    #[test]
    fn samples_above_the_plane_depth_clamp_the_same_packed_or_strided() {
        let chroma = || Plane::packed(1, 1, 10, Samples::U16(vec![512])).unwrap();
        let convert = |luma: Plane| {
            let frame = DecodedFrame::new(2, 1, ColorFamily::YCbCr, Subsampling::S420, vec![luma, chroma(), chroma()], None).unwrap();
            let PixelData::Yuv16 { planes, .. } = compositor_frame(frame, Tick::ZERO).unwrap().data else { panic!("expected Yuv16") };
            planes[0].as_slice().to_vec()
        };
        let packed = convert(Plane::packed(2, 1, 10, Samples::U16(vec![65535, 100])).unwrap());
        let strided = convert(Plane::new(2, 1, 4, 10, Samples::U16(vec![65535, 100, 0, 0])).unwrap());
        assert_eq!(packed, [1023, 100]);
        assert_eq!(strided, packed);
    }

    #[test]
    fn a_deeper_alpha_plane_keeps_its_precision() {
        let alpha = || Plane::packed(2, 1, 10, Samples::U16(vec![1, 1022])).unwrap();
        let plane = |v: Vec<u8>| Plane::packed(2, 1, 8, Samples::U8(v)).unwrap();
        let yuv = DecodedFrame::new(
            2,
            1,
            ColorFamily::YCbCr,
            Subsampling::S444,
            vec![plane(vec![16, 235]), plane(vec![128; 2]), plane(vec![128; 2])],
            Some(alpha()),
        )
        .unwrap();
        let PixelData::Yuv16 { bits, alpha: Some(a), planes, .. } = &compositor_frame(yuv, Tick::ZERO).unwrap().data else {
            panic!("expected Yuv16 with alpha")
        };
        assert_eq!((*bits, a.as_slice(), planes[0].as_slice()), (10, &[1, 1022][..], &[16 << 2, 235 << 2][..]));
        let mut rgb = DecodedFrame::new(
            2,
            1,
            ColorFamily::Rgb,
            Subsampling::S444,
            vec![plane(vec![255; 2]), plane(vec![255; 2]), plane(vec![255; 2])],
            Some(alpha()),
        )
        .unwrap();
        rgb.color.transfer = 13;
        let PixelData::RgbaF32(px) = &compositor_frame(rgb, Tick::ZERO).unwrap().data else { panic!("expected RgbaF32") };
        assert!(px[3] > 0.0 && px[3] < 1.0 / 255.0);
    }

    #[test]
    fn strided_rows_are_packed_and_odd_subsampling_is_repeated_out_to_444() {
        // 5x3 luma with a stride of 8; 4:1:0 chroma (shifts 2, 2) is 2x1.
        let mut stored = vec![0xEE; 8 * 3];
        for y in 0..3 {
            for x in 0..5 {
                stored[y * 8 + x] = (y * 5 + x) as u8;
            }
        }
        let luma = Plane::new(5, 3, 8, 8, Samples::U8(stored)).unwrap();
        let cb = Plane::packed(2, 1, 8, Samples::U8(vec![10, 20])).unwrap();
        let cr = Plane::packed(2, 1, 8, Samples::U8(vec![30, 40])).unwrap();
        let frame = DecodedFrame::new(5, 3, ColorFamily::YCbCr, Subsampling { x: 2, y: 2 }, vec![luma, cb, cr], None).unwrap();
        let out = compositor_frame(frame, Tick::ZERO).unwrap();
        let PixelData::Yuv8 { planes, chroma, .. } = &out.data else { panic!("expected Yuv8") };
        assert_eq!(*chroma, Chroma::C444);
        assert_eq!(planes[0].as_slice(), (0..15u8).collect::<Vec<_>>().as_slice());
        // Columns 0 to 3 share the first chroma sample and column 4 takes the second.
        for row in planes[1].chunks(5) {
            assert_eq!(row, &[10, 10, 10, 10, 20]);
        }
        for row in planes[2].chunks(5) {
            assert_eq!(row, &[30, 30, 30, 30, 40]);
        }
    }

    #[test]
    fn a_gray_picture_gets_neutral_chroma_and_draws_gray() {
        let luma = Plane::packed(3, 3, 8, Samples::U8(vec![16, 50, 100, 150, 200, 235, 60, 70, 80])).unwrap();
        let frame = DecodedFrame::new(3, 3, ColorFamily::Gray, Subsampling::S444, vec![luma], None).unwrap();
        let out = compositor_frame(frame, Tick::ZERO).unwrap();
        let rgba = out.to_rgba8().unwrap();
        for p in rgba.as_chunks::<4>().0 {
            assert!(p[0].abs_diff(p[1]) <= 1 && p[1].abs_diff(p[2]) <= 1, "{p:?} is not gray");
        }
        // The rows run from dark to light.
        assert!(rgba[0] < rgba[4 * 3] && rgba[4 * 3] < rgba[4 * 5]);
    }

    #[test]
    fn rgb_pictures_become_rgba_and_deep_ones_linear_premultiplied() {
        // 8-bit sRGB keeps its code values.
        let plane = |v: Vec<u8>| Plane::packed(2, 1, 8, Samples::U8(v)).unwrap();
        let mut frame =
            DecodedFrame::new(2, 1, ColorFamily::Rgb, Subsampling::S444, vec![plane(vec![255, 1]), plane(vec![0, 2]), plane(vec![9, 3])], None)
                .unwrap();
        frame.color = ColorTags { primaries: 1, transfer: 13, matrix: 0, full_range: true };
        let out = compositor_frame(frame, Tick::ZERO).unwrap();
        let PixelData::Rgba8(px) = &out.data else { panic!("expected Rgba8, got {}", out.format_label()) };
        assert_eq!(px.as_slice(), &[255, 0, 9, 255, 1, 2, 3, 255]);
        assert_eq!(out.color.range, Range::Full);

        // 10-bit RGB with transparency through FFV1: linear light, multiplied by alpha.
        let src = ffv1::Frame {
            width: 2,
            height: 2,
            colorspace: ffv1::Colorspace::Rgb,
            bit_depth: 10,
            chroma_shift: (0, 0),
            planes: vec![vec![1023, 512, 0, 1023], vec![0, 512, 1023, 1023], vec![0, 512, 0, 1023], vec![1023, 1023, 1023, 0]],
        };
        let out = compositor_frame(ffv1_round_trip(&src), Tick::ZERO).unwrap();
        let PixelData::RgbaF32(px) = &out.data else { panic!("expected RgbaF32, got {}", out.format_label()) };
        assert_eq!(out.color.transfer, Transfer::Linear);
        let px = px.as_chunks::<4>().0;
        assert_eq!(px[0], [1.0, 0.0, 0.0, 1.0]);
        // Half the code range is well under half the light.
        assert!(px[1][0] > 0.1 && px[1][0] < 0.4 && px[1][0] == px[1][1]);
        // A transparent pixel premultiplies to nothing.
        assert_eq!(px[3], [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn limited_range_rgb_is_expanded_to_full_range() {
        let rgb = |bits: u8, samples: [u16; 3], color: ColorTags| {
            let plane = |s: u16| match bits {
                8 => Plane::packed(1, 1, 8, Samples::U8(vec![u8::try_from(s).unwrap()])).unwrap(),
                _ => Plane::packed(1, 1, bits, Samples::U16(vec![s])).unwrap(),
            };
            let mut frame = DecodedFrame::new(1, 1, ColorFamily::Rgb, Subsampling::S444, samples.map(plane).to_vec(), None).unwrap();
            frame.color = color;
            compositor_frame(frame, Tick::ZERO).unwrap()
        };
        let limited = ColorTags { matrix: 0, ..ColorTags::UNSPECIFIED };
        // 8-bit black, white and a middle value.
        let out = rgb(8, [16, 235, 126], limited);
        let PixelData::Rgba8(px) = &out.data else { panic!("expected Rgba8, got {}", out.format_label()) };
        assert_eq!((px.as_slice(), out.color.range), (&[0, 255, 128, 255][..], Range::Full));
        // Codes outside 16 to 235 clamp.
        let PixelData::Rgba8(px) = &rgb(8, [0, 255, 16], limited).data else { panic!("expected Rgba8") };
        assert_eq!(px.as_slice(), &[0, 255, 0, 255]);
        // 10-bit black and white are 64 and 940.
        let PixelData::RgbaF32(px) = &rgb(10, [64, 940, 64], limited).data else { panic!("expected RgbaF32") };
        assert!(px[0] == 0.0 && (px[1] - 1.0).abs() < 1e-5 && px[2] == 0.0 && px[3] == 1.0, "{:?}", px.as_slice());
        // Full-range codes stay as they are.
        let full = ColorTags { full_range: true, ..limited };
        let PixelData::Rgba8(px) = &rgb(8, [16, 235, 126], full).data else { panic!("expected Rgba8") };
        assert_eq!(px.as_slice(), &[16, 235, 126, 255]);
        // An RGB frame whose range nobody set starts full range.
        let plane = |s: u8| Plane::packed(1, 1, 8, Samples::U8(vec![s])).unwrap();
        let untouched = DecodedFrame::new(1, 1, ColorFamily::Rgb, Subsampling::S444, vec![plane(16), plane(235), plane(126)], None).unwrap();
        let PixelData::Rgba8(px) = &compositor_frame(untouched, Tick::ZERO).unwrap().data else { panic!("expected Rgba8") };
        assert_eq!(px.as_slice(), &[16, 235, 126, 255]);
    }

    #[test]
    fn h273_code_points_map_and_unspecified_ones_follow_the_picture_height() {
        let tags = |primaries, transfer, matrix, full_range| ColorTags { primaries, transfer, matrix, full_range };
        let info = color_info(tags(9, 16, 9, false), 2160);
        assert_eq!((info.primaries, info.transfer, info.matrix, info.range), (Primaries::Bt2020, Transfer::Pq, Matrix::Bt2020Ncl, Range::Limited));
        let info = color_info(tags(12, 13, 1, true), 1080);
        assert_eq!((info.primaries, info.transfer, info.matrix, info.range), (Primaries::P3D65, Transfer::Srgb, Matrix::Bt709, Range::Full));
        let info = color_info(ColorTags::UNSPECIFIED, 480);
        assert_eq!((info.primaries, info.matrix, info.transfer), (Primaries::Bt601_525, Matrix::Bt601, Transfer::Bt709));
        assert_eq!(color_info(ColorTags::UNSPECIFIED, 576).primaries, Primaries::Bt601_625);
        let info = color_info(ColorTags::UNSPECIFIED, 1080);
        assert_eq!((info.primaries, info.matrix), (Primaries::Bt709, Matrix::Bt709));
        assert_eq!(color_info(tags(6, 1, 6, false), 1080).primaries, Primaries::Bt601_525);
    }

    #[test]
    fn the_sample_aspect_ratio_carries_over_and_buffers_respect_the_limit() {
        let luma = Plane::packed(64, 64, 8, Samples::U8(vec![0; 64 * 64])).unwrap();
        let mut frame = DecodedFrame::new(64, 64, ColorFamily::Gray, Subsampling::S444, vec![luma], None).unwrap();
        frame.sar = Some((10, 11));
        assert_eq!(compositor_frame(frame.clone(), Tick::ZERO).unwrap().par, (10, 11));
        assert!(compositor_frame_within(frame.clone(), Tick::ZERO, 64 * 64 - 1).is_err());
        frame.sar = Some((0, 1));
        assert_eq!(compositor_frame(frame, Tick::ZERO).unwrap().par, (1, 1));
    }

    #[test]
    fn an_empty_picture_converts_without_rows() {
        let plane = |w, h| Plane::packed(w, h, 8, Samples::U8(Vec::new())).unwrap();
        let frame = DecodedFrame::new(0, 0, ColorFamily::YCbCr, Subsampling::S420, vec![plane(0, 0), plane(0, 0), plane(0, 0)], None).unwrap();
        let out = compositor_frame(frame, Tick::ZERO).unwrap();
        assert_eq!((out.width, out.height), (0, 0));
    }
}
