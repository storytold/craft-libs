//! Every frame of an animated GIF, composited, with bounded memory (feature `frames`, through the
//! `image` crate's decoder). Each frame is a full RGBA8 picture of the logical screen with its
//! delay; the apps apply their own rules for short delays and for what a stop means.
//!
//! The file is hostile: before anything is decoded, the logical screen must fit
//! [`FrameLimits::max_canvas_bytes`] and be plausible for the file's size ([`crate::plausible`]);
//! decoding then stops at [`FrameLimits::max_frames`] frames or [`FrameLimits::max_bytes`] of kept
//! pictures, or at the first damaged frame after the first, and [`Animation::stop`] says which.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
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
    /// On-screen time in milliseconds as stored (a GIF counts hundredths, so a multiple of 10;
    /// 0 when the file gives none). With merging, the sum of the merged frames' delays.
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
    /// Not a GIF, or the first frame does not decode.
    Malformed(String),
    /// The canvas is over [`FrameLimits::max_canvas_bytes`] or more than the file can hold.
    LimitExceeded(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Malformed(m) => write!(f, "GIF: {m}"),
            FrameError::LimitExceeded(m) => write!(f, "GIF exceeds a limit: {m}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Decodes the frames of a GIF.
pub fn read_frames(bytes: &[u8], limits: &FrameLimits) -> Result<Animation, FrameError> {
    let info = crate::scan(bytes, 1).ok_or_else(|| FrameError::Malformed("not a GIF".into()))?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    let canvas = u64::from(width) * u64::from(height) * 4;
    if canvas > limits.max_canvas_bytes {
        return Err(FrameError::LimitExceeded(format!("a {width}x{height} canvas is over {} bytes", limits.max_canvas_bytes)));
    }
    // The decoder sets up the whole logical screen before the first frame: the file must back it.
    if !crate::plausible(info.bytes_to_back(4, 1 << 28), bytes.len(), 1 << 14) {
        return Err(FrameError::LimitExceeded(format!("a {} byte file cannot hold a {width}x{height} picture", bytes.len())));
    }
    let malformed = |e: image::ImageError| FrameError::Malformed(e.to_string());
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(malformed)?;
    let mut il = image::Limits::default();
    il.max_alloc = Some(limits.max_canvas_bytes.saturating_mul(2));
    decoder.set_limits(il).map_err(malformed)?;
    let (width, height) = decoder.dimensions();
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

    /// An animated GIF of solid 3 × 2 frames, written by `image`: (red level, delay in hundredths).
    fn gif(frames: &[(u8, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            for &(r, cs) in frames {
                let buf = image::RgbaImage::from_pixel(3, 2, image::Rgba([r, 0, 0, 255]));
                enc.encode_frame(image::Frame::from_parts(buf, 0, 0, image::Delay::from_numer_denom_ms(cs * 10, 1))).unwrap();
            }
        }
        out
    }

    fn reds(a: &Animation) -> Vec<(u8, u32)> {
        a.frames.iter().map(|f| (f.rgba[0], f.delay_ms)).collect()
    }

    #[test]
    fn every_frame_with_its_delay() {
        let a = read_frames(&gif(&[(10, 5), (200, 10), (10, 0)]), &FrameLimits::default()).unwrap();
        assert_eq!((a.width, a.height, a.stop.clone()), (3, 2, Stop::End));
        assert_eq!(reds(&a), [(10, 50), (200, 100), (10, 0)]);
        assert!(a.frames.iter().all(|f| f.rgba.len() == 24));
    }

    #[test]
    fn identical_frames_merge_when_asked() {
        let bytes = gif(&[(10, 5), (10, 7), (200, 1), (200, 1), (10, 2)]);
        let merged = read_frames(&bytes, &FrameLimits { merge_identical: true, ..FrameLimits::default() }).unwrap();
        assert_eq!(reds(&merged), [(10, 120), (200, 20), (10, 20)]);
        assert_eq!(read_frames(&bytes, &FrameLimits::default()).unwrap().frames.len(), 5);
    }

    #[test]
    fn limits_stop_and_say_why() {
        let bytes = gif(&[(1, 1), (2, 1), (3, 1)]);
        let a = read_frames(&bytes, &FrameLimits { max_frames: 2, ..FrameLimits::default() }).unwrap();
        assert_eq!((a.frames.len(), a.stop), (2, Stop::FrameLimit));
        assert_eq!(read_frames(&bytes, &FrameLimits { max_frames: 3, ..FrameLimits::default() }).unwrap().stop, Stop::End);
        // 24 bytes a frame: room for two, not three.
        let a = read_frames(&bytes, &FrameLimits { max_bytes: 48, ..FrameLimits::default() }).unwrap();
        assert_eq!((a.frames.len(), a.stop), (2, Stop::ByteLimit));
        assert!(matches!(read_frames(&bytes, &FrameLimits { max_canvas_bytes: 23, ..FrameLimits::default() }), Err(FrameError::LimitExceeded(_))));
        assert!(read_frames(&bytes, &FrameLimits { max_canvas_bytes: 24, ..FrameLimits::default() }).is_ok());
    }

    #[test]
    fn damage_after_the_first_frame_keeps_what_came_before() {
        let bytes = gif(&[(1, 1), (2, 1), (3, 1)]);
        let info = crate::scan(&bytes, 10).unwrap();
        // Cut inside the third frame's image data.
        let cut = &bytes[..info.frames[2].offset + 14];
        let a = read_frames(cut, &FrameLimits::default()).unwrap();
        assert_eq!(a.frames.len(), 2);
        assert!(matches!(a.stop, Stop::Damaged(_)), "{:?}", a.stop);
        // Nothing decodes: an error.
        assert!(matches!(read_frames(&bytes[..info.frames[0].offset + 12], &FrameLimits::default()), Err(FrameError::Malformed(_))));
        assert!(matches!(read_frames(b"not a gif", &FrameLimits::default()), Err(FrameError::Malformed(_))));
    }

    #[test]
    fn a_huge_screen_over_a_tiny_file_is_refused_before_decoding() {
        let mut b = gif(&[(1, 1)]);
        b[6..10].copy_from_slice(&[0xFF, 0x7F, 0xFF, 0x7F]);
        let limits = FrameLimits { max_canvas_bytes: u64::MAX, ..FrameLimits::default() };
        assert!(matches!(read_frames(&b, &limits), Err(FrameError::LimitExceeded(m)) if m.contains("cannot hold")));
    }

    #[test]
    fn truncation_never_panics() {
        let bytes = gif(&[(1, 1), (2, 1)]);
        for n in 0..bytes.len() {
            let _ = read_frames(&bytes[..n], &FrameLimits::default());
        }
    }
}
