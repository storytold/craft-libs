//! Bounded-shape GIF animation writing from full-canvas RGBA8 frames.
//!
//! Timing and frame production remain the caller's choice. This module owns the GIF-specific
//! encoding rules: palette quantization, loop extension, transparency and disposal metadata.

/// Disposal behavior to write for every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposal {
    /// Leave the encoded disposal method unspecified (`0`), as the encoder default does.
    Unspecified,
    /// Restore the logical screen to the background after each frame.
    Background,
}

/// GIF encoding options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Loop indefinitely. `false` uses `gif`'s `Finite(0)` setting (no repeat extension).
    pub loop_forever: bool,
    /// Disposal metadata to attach to each frame.
    pub disposal: Disposal,
    /// `gif` encoder palette quality (`1` is best, `30` is fastest). Default 10.
    pub speed: i32,
}

impl Default for Options {
    fn default() -> Self {
        Self { loop_forever: true, disposal: Disposal::Unspecified, speed: 10 }
    }
}

/// A full-canvas, straight-alpha RGBA8 frame. Delays are centiseconds, as stored in GIF.
#[derive(Debug, Clone, Copy)]
pub struct RgbaFrame<'a> {
    pub pixels: &'a [u8],
    pub delay_cs: u16,
}

/// Why the requested animation could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Dimensions { width: u32, height: u32 },
    InvalidSpeed(i32),
    FrameLength { index: usize, actual: usize, expected: usize },
    NoFrames,
    Encode(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dimensions { width, height } => write!(f, "GIF dimensions {width}x{height} exceed the 65535 px limit"),
            Self::InvalidSpeed(speed) => write!(f, "GIF quantization speed {speed} is outside 1–30"),
            Self::FrameLength { index, actual, expected } => write!(f, "GIF frame {index} has {actual} bytes; expected {expected} RGBA bytes"),
            Self::NoFrames => f.write_str("GIF animation has no frames"),
            Self::Encode(message) => write!(f, "GIF encode failed: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<gif::EncodingError> for Error {
    fn from(error: gif::EncodingError) -> Self {
        Self::Encode(error.to_string())
    }
}

/// Encode an animation into a complete GIF file.
///
/// Every input frame must cover the whole canvas and contain exactly `width * height * 4` bytes.
/// At most one mutable frame copy is made at a time for the quantizer. This is intentionally a
/// streaming writer so callers can render and encode frames incrementally.
pub fn animation(width: u32, height: u32, frames: &[RgbaFrame<'_>], options: Options) -> Result<Vec<u8>, Error> {
    if width == 0 || height == 0 || width > u32::from(u16::MAX) || height > u32::from(u16::MAX) {
        return Err(Error::Dimensions { width, height });
    }
    if frames.is_empty() {
        return Err(Error::NoFrames);
    }
    if !(1..=30).contains(&options.speed) {
        return Err(Error::InvalidSpeed(options.speed));
    }
    let expected = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Error::Dimensions { width, height })?;
    for (index, frame) in frames.iter().enumerate() {
        if frame.pixels.len() != expected {
            return Err(Error::FrameLength { index, actual: frame.pixels.len(), expected });
        }
    }

    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, width as u16, height as u16, &[]).map_err(Error::from)?;
        encoder.set_repeat(if options.loop_forever { gif::Repeat::Infinite } else { gif::Repeat::Finite(0) }).map_err(Error::from)?;
        for frame in frames {
            let mut pixels = frame.pixels.to_vec();
            let mut encoded = gif::Frame::from_rgba_speed(width as u16, height as u16, &mut pixels, options.speed);
            encoded.delay = frame.delay_cs;
            encoded.dispose = match options.disposal {
                Disposal::Unspecified => gif::DisposalMethod::Any,
                Disposal::Background => gif::DisposalMethod::Background,
            };
            encoder.write_frame(&encoded).map_err(Error::from)?;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Disposal, Options, RgbaFrame, animation};

    #[test]
    fn writes_repeat_delays_disposal_and_dimensions() {
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let bytes = animation(
            1,
            1,
            &[RgbaFrame { pixels: &red, delay_cs: 4 }, RgbaFrame { pixels: &blue, delay_cs: 7 }],
            Options { loop_forever: false, disposal: Disposal::Background, speed: 10 },
        )
        .unwrap();
        let info = crate::scan(&bytes, 4).unwrap();
        assert_eq!((info.width, info.height), (1, 1));
        assert_eq!(info.loop_count, None);
        assert_eq!(info.frames.iter().map(|f| f.delay_cs).collect::<Vec<_>>(), [4, 7]);
        assert!(info.frames.iter().all(|f| f.disposal == 2));
    }

    #[test]
    fn rejects_dimensions_empty_animation_and_wrong_pixel_lengths() {
        assert!(matches!(animation(0, 1, &[RgbaFrame { pixels: &[0; 4], delay_cs: 1 }], Options::default()), Err(super::Error::Dimensions { .. })));
        assert!(matches!(animation(1, 1, &[], Options::default()), Err(super::Error::NoFrames)));
        assert!(matches!(animation(1, 1, &[RgbaFrame { pixels: &[0; 3], delay_cs: 1 }], Options::default()), Err(super::Error::FrameLength { .. })));
        assert!(matches!(
            animation(1, 1, &[RgbaFrame { pixels: &[0; 4], delay_cs: 1 }], Options { speed: 0, ..Options::default() }),
            Err(super::Error::InvalidSpeed(0))
        ));
    }

    #[cfg(feature = "frames")]
    #[test]
    fn encoded_frames_decode_to_the_requested_colors() {
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let bytes =
            animation(1, 1, &[RgbaFrame { pixels: &red, delay_cs: 1 }, RgbaFrame { pixels: &blue, delay_cs: 2 }], Options::default()).unwrap();
        let decoded = crate::frames::read_frames(&bytes, &crate::frames::FrameLimits::default()).unwrap();
        assert_eq!(decoded.frames[0].rgba, red);
        assert_eq!(decoded.frames[1].rgba, blue);
    }
}
