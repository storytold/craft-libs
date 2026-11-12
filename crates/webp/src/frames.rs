//! Every frame of a WebP file, composited, with bounded memory (feature `frames`, through the
//! `image` crate's decoder). Each frame is a full RGBA8 picture of the canvas with its duration;
//! a still is one frame. The apps apply their own rules for short durations and for what a stop
//! means.
//!
//! The file is hostile: the canvas must fit [`FrameLimits::max_canvas_bytes`] before anything is
//! decoded; decoding then stops at [`FrameLimits::max_frames`] frames or [`FrameLimits::max_bytes`]
//! of kept pictures, or at the first damaged frame after the first, and [`Animation::stop`] says
//! which.
//!
//! shortcut: the limit and result types mirror `craft_gif::frames` rather than sharing one crate
//! (one crate per format); unify them if a third animated format needs the same loop.

use std::io::Cursor;

use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, ImageDecoder};

/// Caps on a frame decode. `#[non_exhaustive]`: start from [`FrameLimits::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameLimits {
    /// Most frames kept. Default 100,000.
    pub max_frames: usize,
    /// Most bytes of kept pictures (4 bytes a pixel each). Default 1 GiB.
    pub max_bytes: u64,
    /// Largest canvas (logical screen) in bytes at 4 bytes a pixel. Default 512 MiB.
    pub max_canvas_bytes: u64,
    /// Merge a frame identical to the one before into it (its delay added). Default off.
    pub merge_identical: bool,
}

impl Default for FrameLimits {
    fn default() -> Self {
        FrameLimits { max_frames: 100_000, max_bytes: 1 << 30, max_canvas_bytes: 512 << 20, merge_identical: false }
    }
}

/// One composited frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimFrame {
    /// Straight-alpha RGBA8 of the whole canvas, top row first.
    pub rgba: Vec<u8>,
    /// On-screen time in milliseconds as stored (0 for a still). With merging, the sum of the
    /// merged frames' delays.
    pub delay_ms: u32,
}

/// Why decoding stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// Every frame was decoded.
    End,
    /// [`FrameLimits::max_frames`] frames were kept and more followed.
    FrameLimit,
    /// The next frame would have passed [`FrameLimits::max_bytes`].
    ByteLimit,
    /// A frame after the first could not be decoded (the message says why).
    Damaged(String),
}

/// A decoded animation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    pub width: u32,
    pub height: u32,
    /// At least one frame.
    pub frames: Vec<AnimFrame>,
    pub stop: Stop,
}

/// Why nothing could be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Not a WebP file, or the first frame does not decode.
    Malformed(String),
    /// The canvas is over [`FrameLimits::max_canvas_bytes`].
    LimitExceeded(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Malformed(m) => write!(f, "WebP: {m}"),
            FrameError::LimitExceeded(m) => write!(f, "WebP exceeds a limit: {m}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Decodes the frames of a WebP file: every frame of an animation, composited, or the picture of
/// a still as one frame.
pub fn read_frames(bytes: &[u8], limits: &FrameLimits) -> Result<Animation, FrameError> {
    let info = crate::scan(bytes, 0).ok_or_else(|| FrameError::Malformed("not a WebP file".into()))?;
    let canvas = u64::from(info.width) * u64::from(info.height) * 4;
    if canvas > limits.max_canvas_bytes {
        return Err(FrameError::LimitExceeded(format!("a {}x{} canvas is over {} bytes", info.width, info.height, limits.max_canvas_bytes)));
    }
    let malformed = |e: image::ImageError| FrameError::Malformed(e.to_string());
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(malformed)?;
    let mut il = image::Limits::default();
    il.max_alloc = Some(limits.max_canvas_bytes.saturating_mul(2));
    decoder.set_limits(il).map_err(malformed)?;
    let (width, height) = decoder.dimensions();
    if !decoder.has_animation() {
        let mut rgba = vec![0u8; usize::try_from(canvas).map_err(|_| FrameError::LimitExceeded("canvas".into()))?];
        let alpha = decoder.color_type().has_alpha();
        if alpha {
            decoder.read_image(&mut rgba).map_err(malformed)?;
        } else {
            let mut rgb = vec![0u8; rgba.len() / 4 * 3];
            decoder.read_image(&mut rgb).map_err(malformed)?;
            for (d, s) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(rgb.as_chunks::<3>().0) {
                *d = [s[0], s[1], s[2], 255];
            }
        }
        return Ok(Animation { width, height, frames: vec![AnimFrame { rgba, delay_ms: 0 }], stop: Stop::End });
    }
    collect(width, height, decoder.into_frames(), limits)
}

/// Keeps the frames `frames` yields under `limits`.
fn collect(width: u32, height: u32, frames: image::Frames<'_>, limits: &FrameLimits) -> Result<Animation, FrameError> {
    let frame_bytes = u64::from(width) * u64::from(height) * 4;
    let mut out: Vec<AnimFrame> = Vec::new();
    let mut stop = Stop::End;
    for frame in frames {
        let frame = match frame {
            Ok(f) => f,
            Err(e) if out.is_empty() => return Err(FrameError::Malformed(e.to_string())),
            Err(e) => {
                stop = Stop::Damaged(e.to_string());
                break;
            }
        };
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = numer.checked_div(denom).unwrap_or(0);
        let rgba = frame.into_buffer().into_raw();
        if limits.merge_identical
            && let Some(last) = out.last_mut()
            && last.rgba == rgba
        {
            last.delay_ms = last.delay_ms.saturating_add(delay_ms);
            continue;
        }
        if out.len() >= limits.max_frames {
            stop = Stop::FrameLimit;
            break;
        }
        if frame_bytes.saturating_mul(out.len() as u64 + 1) > limits.max_bytes {
            stop = Stop::ByteLimit;
            break;
        }
        out.push(AnimFrame { rgba, delay_ms });
    }
    if out.is_empty() {
        return Err(FrameError::Malformed("no frame".into()));
    }
    Ok(Animation { width, height, frames: out, stop })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use image::{ExtendedColorType, ImageEncoder, codecs::webp::WebPEncoder};

    fn vp8l(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut file = Vec::new();
        WebPEncoder::new_lossless(&mut file).write_image(&rgba.repeat((w * h) as usize), w, h, ExtendedColorType::Rgba8).unwrap();
        let at = file.windows(4).position(|c| c == b"VP8L").unwrap();
        let size = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
        file[at + 8..at + 8 + size].to_vec()
    }

    fn chunk(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = fourcc.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// An animation of solid 3 × 2 lossless frames: (red level, duration in ms).
    fn anim(frames: &[(u8, u32)]) -> Vec<u8> {
        let u24 = |v: u32| [v as u8, (v >> 8) as u8, (v >> 16) as u8];
        let mut body = b"WEBP".to_vec();
        body.extend(chunk(b"VP8X", &[0x12, 0, 0, 0, 2, 0, 0, 1, 0, 0]));
        body.extend(chunk(b"ANIM", &[0; 6]));
        for &(r, ms) in frames {
            let mut anmf = vec![0; 6];
            anmf.extend(u24(2));
            anmf.extend(u24(1));
            anmf.extend(u24(ms));
            anmf.push(0x02);
            anmf.extend(chunk(b"VP8L", &vp8l(3, 2, [r, 0, 0, 255])));
            body.extend(chunk(b"ANMF", &anmf));
        }
        let mut out = b"RIFF".to_vec();
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    fn reds(a: &Animation) -> Vec<(u8, u32)> {
        a.frames.iter().map(|f| (f.rgba[0], f.delay_ms)).collect()
    }

    #[test]
    fn every_frame_with_its_duration() {
        let a = read_frames(&anim(&[(10, 50), (200, 100), (10, 0)]), &FrameLimits::default()).unwrap();
        assert_eq!((a.width, a.height, a.stop.clone()), (3, 2, Stop::End));
        assert_eq!(reds(&a), [(10, 50), (200, 100), (10, 0)]);
    }

    #[test]
    fn a_still_is_one_frame() {
        let mut file = Vec::new();
        WebPEncoder::new_lossless(&mut file).write_image(&[1, 2, 3, 255].repeat(6), 3, 2, ExtendedColorType::Rgba8).unwrap();
        let a = read_frames(&file, &FrameLimits::default()).unwrap();
        assert_eq!((a.frames.len(), a.frames[0].delay_ms, &a.frames[0].rgba[..4]), (1, 0, &[1, 2, 3, 255][..]));
        let mut rgb = Vec::new();
        WebPEncoder::new_lossless(&mut rgb).write_image(&[9, 8, 7].repeat(6), 3, 2, ExtendedColorType::Rgb8).unwrap();
        assert_eq!(&read_frames(&rgb, &FrameLimits::default()).unwrap().frames[0].rgba[..8], &[9, 8, 7, 255, 9, 8, 7, 255]);
    }

    #[test]
    fn identical_frames_merge_when_asked() {
        let bytes = anim(&[(10, 50), (10, 70), (200, 10), (200, 10), (10, 20)]);
        let merged = read_frames(&bytes, &FrameLimits { merge_identical: true, ..FrameLimits::default() }).unwrap();
        assert_eq!(reds(&merged), [(10, 120), (200, 20), (10, 20)]);
    }

    #[test]
    fn limits_stop_and_say_why() {
        let bytes = anim(&[(1, 10), (2, 10), (3, 10)]);
        let a = read_frames(&bytes, &FrameLimits { max_frames: 2, ..FrameLimits::default() }).unwrap();
        assert_eq!((a.frames.len(), a.stop), (2, Stop::FrameLimit));
        let a = read_frames(&bytes, &FrameLimits { max_bytes: 48, ..FrameLimits::default() }).unwrap();
        assert_eq!((a.frames.len(), a.stop), (2, Stop::ByteLimit));
        assert!(matches!(read_frames(&bytes, &FrameLimits { max_canvas_bytes: 23, ..FrameLimits::default() }), Err(FrameError::LimitExceeded(_))));
        assert!(read_frames(&bytes, &FrameLimits { max_canvas_bytes: 24, ..FrameLimits::default() }).is_ok());
    }

    #[test]
    fn damage_and_hostile_input() {
        let bytes = anim(&[(1, 10), (2, 10), (3, 10)]);
        // The third frame's picture cut short: the two before it are kept.
        let a = read_frames(&bytes[..bytes.len() - 4], &FrameLimits::default());
        let a = a.unwrap();
        assert_eq!(reds(&a), [(1, 10), (2, 10)]);
        assert!(matches!(a.stop, Stop::Damaged(_)), "{:?}", a.stop);
        for n in 0..bytes.len() {
            let _ = read_frames(&bytes[..n], &FrameLimits::default());
        }
        assert!(matches!(read_frames(b"RIFF\0\0\0\0WAVE", &FrameLimits::default()), Err(FrameError::Malformed(_))));
        // A huge canvas declared over a few bytes is refused before decoding.
        let mut big = bytes.clone();
        big[24..27].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        big[27..30].copy_from_slice(&[0xFF, 0xFF, 0x0F]);
        assert!(matches!(read_frames(&big, &FrameLimits::default()), Err(FrameError::LimitExceeded(_))));
    }
}
