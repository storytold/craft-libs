//! Bounded-shape GIF animation writing from full-canvas RGBA8 frames.
//!
//! Timing and frame production remain the caller's choice. This module owns the GIF-specific
//! encoding rules: palette quantization, loop extension, transparency and disposal metadata.

use std::io::Write;

/// Disposal behavior to write for every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposal {
    /// Leave the encoded disposal method unspecified (`0`).
    Unspecified,
    /// Do not dispose of the current frame (`1`), matching `gif::Frame`'s default.
    Keep,
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
        Self { loop_forever: true, disposal: Disposal::Keep, speed: 10 }
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
    PreparedDimensions { actual_width: u16, actual_height: u16, expected_width: u16, expected_height: u16 },
    NoFrames,
    Encode(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dimensions { width, height } => write!(f, "GIF dimensions {width}x{height} exceed the 65535 px limit"),
            Self::InvalidSpeed(speed) => write!(f, "GIF quantization speed {speed} is outside 1–30"),
            Self::FrameLength { index, actual, expected } => write!(f, "GIF frame {index} has {actual} bytes; expected {expected} RGBA bytes"),
            Self::PreparedDimensions { actual_width, actual_height, expected_width, expected_height } => {
                write!(f, "prepared GIF frame is {actual_width}x{actual_height}; expected {expected_width}x{expected_height}")
            }
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

/// A palette-quantized GIF frame ready for a writer.
///
/// Prepare these inside a parallel render batch to keep only indexed frame data in the batch,
/// rather than retaining every source RGBA buffer until the writer reaches it.
pub struct PreparedFrame {
    width: u16,
    height: u16,
    frame: gif::Frame<'static>,
}

/// Quantize one owned full-canvas RGBA8 frame using the GIF encoder's speed setting.
pub fn prepare_frame(width: u32, height: u32, mut pixels: Vec<u8>, speed: i32) -> Result<PreparedFrame, Error> {
    let (width, height) = dimensions(width, height)?;
    if !(1..=30).contains(&speed) {
        return Err(Error::InvalidSpeed(speed));
    }
    let expected = rgba_len(width, height)?;
    if pixels.len() != expected {
        return Err(Error::FrameLength { index: 0, actual: pixels.len(), expected });
    }
    let frame = gif::Frame::from_rgba_speed(width, height, &mut pixels, speed);
    Ok(PreparedFrame { width, height, frame })
}

fn rgba_len(width: u16, height: u16) -> Result<usize, Error> {
    usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Error::Dimensions { width: u32::from(width), height: u32::from(height) })
}

fn dimensions(width: u32, height: u32) -> Result<(u16, u16), Error> {
    if width == 0 || height == 0 || width > u32::from(u16::MAX) || height > u32::from(u16::MAX) {
        return Err(Error::Dimensions { width, height });
    }
    Ok((width as u16, height as u16))
}

/// A writer that appends encoded frames to an output incrementally.
pub struct AnimationWriter<W: Write> {
    encoder: gif::Encoder<W>,
    width: u16,
    height: u16,
    options: Options,
    frames_written: usize,
}

impl<W: Write> AnimationWriter<W> {
    /// Start an animation on `output`; subsequent calls write one frame at a time.
    pub fn new(output: W, width: u32, height: u32, options: Options) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > u32::from(u16::MAX) || height > u32::from(u16::MAX) {
            return Err(Error::Dimensions { width, height });
        }
        if !(1..=30).contains(&options.speed) {
            return Err(Error::InvalidSpeed(options.speed));
        }
        let width = width as u16;
        let height = height as u16;
        let mut encoder = gif::Encoder::new(output, width, height, &[]).map_err(Error::from)?;
        encoder.set_repeat(if options.loop_forever { gif::Repeat::Infinite } else { gif::Repeat::Finite(0) }).map_err(Error::from)?;
        Ok(Self { encoder, width, height, options, frames_written: 0 })
    }

    /// Encode and append one full-canvas RGBA8 frame.
    pub fn write_frame(&mut self, frame: RgbaFrame<'_>) -> Result<(), Error> {
        let expected = rgba_len(self.width, self.height)?;
        if frame.pixels.len() != expected {
            return Err(Error::FrameLength { index: self.frames_written, actual: frame.pixels.len(), expected });
        }
        let prepared =
            prepare_frame(u32::from(self.width), u32::from(self.height), frame.pixels.to_vec(), self.options.speed).map_err(|error| match error {
                Error::FrameLength { actual, expected, .. } => Error::FrameLength { index: self.frames_written, actual, expected },
                error => error,
            })?;
        self.write_prepared_frame(prepared, frame.delay_cs)
    }

    /// Append a previously quantized frame without copying its pixel buffer.
    pub fn write_prepared_frame(&mut self, mut prepared: PreparedFrame, delay_cs: u16) -> Result<(), Error> {
        if prepared.width != self.width || prepared.height != self.height {
            return Err(Error::PreparedDimensions {
                actual_width: prepared.width,
                actual_height: prepared.height,
                expected_width: self.width,
                expected_height: self.height,
            });
        }
        prepared.frame.delay = delay_cs;
        prepared.frame.dispose = match self.options.disposal {
            Disposal::Unspecified => gif::DisposalMethod::Any,
            Disposal::Keep => gif::DisposalMethod::Keep,
            Disposal::Background => gif::DisposalMethod::Background,
        };
        self.encoder.write_frame(&prepared.frame).map_err(Error::from)?;
        self.frames_written = self.frames_written.saturating_add(1);
        Ok(())
    }

    /// Finish the GIF and return the underlying output.
    pub fn finish(self) -> Result<W, Error> {
        self.encoder.into_inner().map_err(Error::from)
    }
}

/// Encode an animation into a complete GIF file.
///
/// Every input frame must cover the whole canvas and contain exactly `width * height * 4` bytes.
/// At most one mutable frame copy is made at a time for the quantizer. This is intentionally a
/// streaming writer so callers can render and encode frames incrementally.
pub fn animation(width: u32, height: u32, frames: &[RgbaFrame<'_>], options: Options) -> Result<Vec<u8>, Error> {
    if frames.is_empty() {
        return Err(Error::NoFrames);
    }
    let mut writer = AnimationWriter::new(Vec::new(), width, height, options)?;
    for frame in frames {
        writer.write_frame(*frame)?;
    }
    writer.finish()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Disposal, Options, RgbaFrame, animation, prepare_frame};

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
    fn default_disposal_matches_gif_frame_default() {
        let bytes = animation(1, 1, &[RgbaFrame { pixels: &[1, 2, 3, 255], delay_cs: 1 }], Options::default()).unwrap();
        let info = crate::scan(&bytes, 1).unwrap();
        assert_eq!(info.frames[0].disposal, 1);
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
        assert!(matches!(
            animation(1, 1, &[RgbaFrame { pixels: &[0; 4], delay_cs: 1 }, RgbaFrame { pixels: &[0; 3], delay_cs: 1 }], Options::default()),
            Err(super::Error::FrameLength { index: 1, .. })
        ));
    }

    #[test]
    fn streaming_writer_preserves_empty_output_behavior() {
        let bytes = super::AnimationWriter::new(Vec::new(), 1, 1, Options::default()).unwrap().finish().unwrap();
        assert!(bytes.starts_with(b"GIF89a"));
        assert_eq!(bytes.last(), Some(&b';'));
    }

    #[cfg(feature = "frames")]
    #[test]
    fn prepared_frames_stream_without_changing_decoded_pixels() {
        let red = vec![255, 0, 0, 255];
        let blue = vec![0, 0, 255, 255];
        let mut writer = super::AnimationWriter::new(Vec::new(), 1, 1, Options::default()).unwrap();
        writer.write_prepared_frame(prepare_frame(1, 1, red, 10).unwrap(), 3).unwrap();
        writer.write_prepared_frame(prepare_frame(1, 1, blue, 10).unwrap(), 5).unwrap();
        let bytes = writer.finish().unwrap();
        let expected = animation(
            1,
            1,
            &[RgbaFrame { pixels: &[255, 0, 0, 255], delay_cs: 3 }, RgbaFrame { pixels: &[0, 0, 255, 255], delay_cs: 5 }],
            Options::default(),
        )
        .unwrap();
        assert_eq!(bytes, expected);
        let decoded = crate::frames::read_frames(&bytes, &crate::frames::FrameLimits::default()).unwrap();
        assert_eq!(decoded.frames[0].rgba, [255, 0, 0, 255]);
        assert_eq!(decoded.frames[1].rgba, [0, 0, 255, 255]);
        assert_eq!(decoded.frames.iter().map(|frame| frame.delay_ms).collect::<Vec<_>>(), [30, 50]);
    }

    #[test]
    fn rejects_prepared_frames_with_other_dimensions() {
        let prepared = prepare_frame(2, 1, vec![0; 8], 10).unwrap();
        let mut writer = super::AnimationWriter::new(Vec::new(), 1, 1, Options::default()).unwrap();
        assert!(matches!(writer.write_prepared_frame(prepared, 1), Err(super::Error::PreparedDimensions { .. })));
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
