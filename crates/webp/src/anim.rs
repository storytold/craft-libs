//! Animated WebP writing: the `VP8X`, `ANIM` and `ANMF` chunks around frames another encoder
//! made ([`animation`]), and, with feature `vp8`, whole lossy animations from RGBA frames through
//! `craft-video`'s VP8 encoder ([`lossy_animation`]).

use crate::{Error, MAX_CANVAS, MAX_VP8_DIMENSION, chunk};

/// The encoded picture of one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload<'a> {
    /// A VP8 key frame and, optionally, an alpha plane of one byte a pixel (stored uncompressed).
    Lossy { vp8: &'a [u8], alpha: Option<&'a [u8]> },
    /// A VP8L (lossless) bitstream, signature byte included.
    Lossless { vp8l: &'a [u8] },
}

/// One frame of an animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimFrame<'a> {
    /// Position on the canvas; both must be even.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// How long the frame is shown, in milliseconds (below 2^24).
    pub duration_ms: u32,
    /// Alpha-blend onto the canvas (false: replace the frame's rectangle).
    pub blend: bool,
    /// Clear the frame's rectangle to the background before the next frame.
    pub dispose_to_background: bool,
    pub payload: Payload<'a>,
}

/// Animation-wide settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnimOptions<'a> {
    /// Times to play: 0 loops forever.
    pub loop_count: u16,
    /// Background colour as stored in the file: blue, green, red, alpha. A hint players may ignore.
    pub background_bgra: [u8; 4],
    /// Set the alpha flag even when no lossy frame carries an alpha plane (lossless frames with alpha).
    pub alpha: bool,
    pub icc: Option<&'a [u8]>,
    pub exif: Option<&'a [u8]>,
    pub xmp: Option<&'a [u8]>,
}

fn u24(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes()[..3]);
}

/// Writes an animated WebP of a `width × height` canvas with these frames, in order.
pub fn animation(width: u32, height: u32, frames: &[AnimFrame], opts: &AnimOptions) -> Result<Vec<u8>, Error> {
    if width == 0 || height == 0 || width > MAX_CANVAS || height > MAX_CANVAS || u64::from(width) * u64::from(height) > u64::from(u32::MAX) {
        return Err(Error::Encode("the canvas is 1 to 2^24 pixels a side and at most 2^32 pixels"));
    }
    if frames.is_empty() {
        return Err(Error::Encode("an animation needs a frame"));
    }
    let mut anmf = Vec::new();
    let mut any_alpha = opts.alpha;
    for f in frames {
        if f.width == 0 || f.height == 0 || f.x % 2 != 0 || f.y % 2 != 0 {
            return Err(Error::Encode("a frame is empty or not at an even position"));
        }
        if f.x.checked_add(f.width).is_none_or(|r| r > width) || f.y.checked_add(f.height).is_none_or(|b| b > height) {
            return Err(Error::Encode("a frame reaches past the canvas"));
        }
        if f.duration_ms >= 1 << 24 {
            return Err(Error::Encode("a frame duration is 2^24 ms or more"));
        }
        let mut body = Vec::new();
        u24(&mut body, f.x / 2);
        u24(&mut body, f.y / 2);
        u24(&mut body, f.width - 1);
        u24(&mut body, f.height - 1);
        u24(&mut body, f.duration_ms);
        // Bit 1: do not blend; bit 0: dispose to the background.
        body.push(u8::from(!f.blend) << 1 | u8::from(f.dispose_to_background));
        match f.payload {
            Payload::Lossy { vp8, alpha } => {
                if f.width > MAX_VP8_DIMENSION || f.height > MAX_VP8_DIMENSION {
                    return Err(Error::Encode("a lossy frame is at most 16383 pixels a side"));
                }
                if let Some(a) = alpha {
                    if a.len() as u64 != u64::from(f.width) * u64::from(f.height) {
                        return Err(Error::Encode("an alpha plane does not match its frame"));
                    }
                    any_alpha = true;
                    let mut alph = Vec::with_capacity(a.len() + 1);
                    // Header byte: no pre-processing, no filtering, no compression.
                    alph.push(0);
                    alph.extend_from_slice(a);
                    chunk(&mut body, b"ALPH", &alph)?;
                }
                chunk(&mut body, b"VP8 ", vp8)?;
            }
            Payload::Lossless { vp8l } => chunk(&mut body, b"VP8L", vp8l)?,
        }
        chunk(&mut anmf, b"ANMF", &body)?;
    }
    let mut body = b"WEBP".to_vec();
    let flags = [(opts.icc.is_some(), 0x20u8), (any_alpha, 0x10), (opts.exif.is_some(), 0x08), (opts.xmp.is_some(), 0x04), (true, 0x02)]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |f, (_, bit)| f | bit);
    let mut vp8x = vec![flags, 0, 0, 0];
    u24(&mut vp8x, width - 1);
    u24(&mut vp8x, height - 1);
    chunk(&mut body, b"VP8X", &vp8x)?;
    if let Some(icc) = opts.icc {
        chunk(&mut body, b"ICCP", icc)?;
    }
    let mut anim = opts.background_bgra.to_vec();
    anim.extend_from_slice(&opts.loop_count.to_le_bytes());
    chunk(&mut body, b"ANIM", &anim)?;
    body.extend_from_slice(&anmf);
    if let Some(exif) = opts.exif {
        chunk(&mut body, b"EXIF", exif)?;
    }
    if let Some(xmp) = opts.xmp {
        chunk(&mut body, b"XMP ", xmp)?;
    }
    let size = u32::try_from(body.len()).map_err(|_| Error::Encode("the file is larger than 4 GiB"))?;
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Writes a lossy animated WebP of full-canvas RGBA8 frames (`width × height × 4` bytes each,
/// straight alpha) with these durations, each frame a VP8 key frame at `quality` (0–100). A frame
/// whose pixels are all opaque gets no alpha plane. Frames replace the canvas (no blending).
#[cfg(feature = "vp8")]
pub fn lossy_animation(width: u32, height: u32, frames: &[(&[u8], u32)], quality: u8, opts: &AnimOptions) -> Result<Vec<u8>, Error> {
    use craft_video::vp8::{Vp8Error, Vp8Params, encode_rgba};
    let params = Vp8Params { quality, loop_filter: None };
    let mut encoded = Vec::with_capacity(frames.len());
    for (rgba, duration_ms) in frames {
        let vp8 = encode_rgba(rgba, width, height, &params)
            .map_err(|e| match e {
                Vp8Error::Size(..) => Error::Encode("a lossy frame is 1 to 16383 pixels a side"),
                Vp8Error::Buffer(..) => Error::Encode("a frame is not width × height × 4 bytes of RGBA"),
                Vp8Error::PartitionTooLarge(_) => Error::Encode("a lossy frame's first partition is over the 19-bit limit"),
            })?
            .data;
        let alpha: Vec<u8> = rgba.iter().skip(3).step_by(4).copied().collect();
        let alpha = (!alpha.iter().all(|&a| a == 255)).then_some(alpha);
        encoded.push((vp8, alpha, *duration_ms));
    }
    let list: Vec<AnimFrame> = encoded
        .iter()
        .map(|(vp8, alpha, duration_ms)| AnimFrame {
            x: 0,
            y: 0,
            width,
            height,
            duration_ms: *duration_ms,
            blend: false,
            dispose_to_background: false,
            payload: Payload::Lossy { vp8, alpha: alpha.as_deref() },
        })
        .collect();
    animation(width, height, &list, opts)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use image::{AnimationDecoder, ExtendedColorType, ImageEncoder, codecs::webp::WebPDecoder, codecs::webp::WebPEncoder};

    use super::*;
    use crate::scan;

    /// The `VP8L` chunk payload of a lossless still of one colour, written by `image`.
    fn vp8l(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut file = Vec::new();
        WebPEncoder::new_lossless(&mut file).write_image(&rgba.repeat((w * h) as usize), w, h, ExtendedColorType::Rgba8).unwrap();
        let at = file.windows(4).position(|c| c == b"VP8L").unwrap();
        let size = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
        file[at + 8..at + 8 + size].to_vec()
    }

    fn frame(payload: Payload<'_>, x: u32, y: u32, w: u32, h: u32, ms: u32) -> AnimFrame<'_> {
        AnimFrame { x, y, width: w, height: h, duration_ms: ms, blend: false, dispose_to_background: false, payload }
    }

    #[test]
    fn lossless_frames_round_trip_through_scan_and_a_decoder() {
        let (red, green) = (vp8l(4, 2, [255, 0, 0, 255]), vp8l(2, 2, [0, 255, 0, 128]));
        let frames = [frame(Payload::Lossless { vp8l: &red }, 0, 0, 4, 2, 40), frame(Payload::Lossless { vp8l: &green }, 2, 0, 2, 2, 120)];
        let mut opts = AnimOptions { loop_count: 3, alpha: true, ..AnimOptions::default() };
        opts.xmp = Some(b"<x/>");
        let file = animation(4, 2, &frames, &opts).unwrap();
        let info = scan(&file, 10).unwrap();
        assert!(info.is_animation() && info.complete && info.alpha);
        assert_eq!((info.width, info.height, info.loop_count), (4, 2, Some(3)));
        assert_eq!(info.frames.iter().map(|f| (f.x, f.width, f.duration_ms, f.blend)).collect::<Vec<_>>(), [(0, 4, 40, false), (2, 2, 120, false)]);
        // The `image` crate composites it: the second frame replaces the right half.
        let d = WebPDecoder::new(std::io::Cursor::new(&file)).unwrap();
        let decoded: Vec<_> = d.into_frames().collect::<Result<_, _>>().unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].buffer().get_pixel(3, 1).0, [255, 0, 0, 255]);
        assert_eq!(decoded[1].buffer().get_pixel(3, 1).0, [0, 255, 0, 128]);
        assert_eq!(decoded[1].buffer().get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(decoded[1].delay().numer_denom_ms(), (120, 1));
    }

    #[test]
    fn frame_flags_and_chunk_order() {
        let px = vp8l(2, 2, [1, 2, 3, 255]);
        let mut f = frame(Payload::Lossless { vp8l: &px }, 0, 0, 2, 2, 10);
        f.blend = true;
        f.dispose_to_background = true;
        let file =
            animation(2, 2, &[f], &AnimOptions { icc: Some(b"icc"), exif: Some(b"e"), background_bgra: [1, 2, 3, 4], ..AnimOptions::default() })
                .unwrap();
        let info = scan(&file, 10).unwrap();
        assert_eq!((info.frames[0].blend, info.frames[0].dispose_to_background, info.alpha, info.animated), (true, true, false, true));
        let mut at = 12;
        let mut seen = Vec::new();
        while at < file.len() {
            let size = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
            seen.push(String::from_utf8_lossy(&file[at..at + 4]).into_owned());
            at += 8 + size + (size & 1);
        }
        assert_eq!(seen, ["VP8X", "ICCP", "ANIM", "ANMF", "EXIF"]);
        // ICC, EXIF and animation flags; the background colour and loop count in ANIM.
        assert_eq!(file[20], 0x20 | 0x08 | 0x02);
        let anim = file.windows(4).position(|c| c == b"ANIM").unwrap();
        assert_eq!(&file[anim + 8..anim + 14], &[1, 2, 3, 4, 0, 0]);
    }

    #[test]
    fn what_does_not_fit_is_refused() {
        let px = vp8l(2, 2, [0; 4]);
        let ok = frame(Payload::Lossless { vp8l: &px }, 0, 0, 2, 2, 10);
        let o = AnimOptions::default();
        assert!(animation(2, 2, &[ok], &o).is_ok());
        assert!(animation(2, 2, &[], &o).is_err());
        assert!(animation(0, 2, &[ok], &o).is_err());
        assert!(animation(MAX_CANVAS + 1, 1, &[ok], &o).is_err());
        assert!(animation(2, 2, &[AnimFrame { x: 1, ..ok }], &o).is_err());
        assert!(animation(2, 2, &[AnimFrame { x: 2, ..ok }], &o).is_err());
        assert!(animation(2, 2, &[AnimFrame { width: 0, ..ok }], &o).is_err());
        assert!(animation(2, 2, &[AnimFrame { duration_ms: (1 << 24) - 1, ..ok }], &o).is_ok());
        assert!(animation(2, 2, &[AnimFrame { duration_ms: 1 << 24, ..ok }], &o).is_err());
        let lossy = AnimFrame { payload: Payload::Lossy { vp8: &[0; 10], alpha: Some(&[0; 3]) }, ..ok };
        assert!(animation(2, 2, &[lossy], &o).is_err());
        let wide = AnimFrame { width: MAX_VP8_DIMENSION + 1, payload: Payload::Lossy { vp8: &[0; 10], alpha: None }, ..ok };
        assert!(animation(MAX_VP8_DIMENSION + 1, 2, &[wide], &o).is_err());
    }

    #[cfg(feature = "vp8")]
    #[test]
    fn lossy_animations_decode_with_their_colours_alpha_and_timing() {
        let (w, h) = (24u32, 16u32);
        let solid = |c: [u8; 4]| c.repeat((w * h) as usize);
        let (red, half_blue) = (solid([220, 30, 30, 255]), solid([30, 30, 220, 128]));
        let file = lossy_animation(w, h, &[(&red, 100), (&half_blue, 50)], 90, &AnimOptions::default()).unwrap();
        let info = scan(&file, 10).unwrap();
        assert!(info.is_animation() && info.alpha && info.complete);
        assert_eq!(info.frames.iter().map(|f| f.duration_ms).collect::<Vec<_>>(), [100, 50]);
        let d = WebPDecoder::new(std::io::Cursor::new(&file)).unwrap();
        let decoded: Vec<_> = d.into_frames().collect::<Result<_, _>>().unwrap();
        assert_eq!(decoded.len(), 2);
        let close = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 6);
        assert!(close(decoded[0].buffer().get_pixel(5, 5).0, [220, 30, 30, 255]), "{:?}", decoded[0].buffer().get_pixel(5, 5));
        assert!(close(decoded[1].buffer().get_pixel(5, 5).0, [30, 30, 220, 128]), "{:?}", decoded[1].buffer().get_pixel(5, 5));
        // An opaque frame carries no alpha plane; a buffer of the wrong size is refused.
        let opaque = lossy_animation(w, h, &[(&red, 10), (&red, 10)], 50, &AnimOptions::default()).unwrap();
        assert!(!scan(&opaque, 10).unwrap().alpha);
        assert!(lossy_animation(w, h, &[(&red[..10], 10)], 50, &AnimOptions::default()).is_err());
    }
}
