//! Write plain 8-bit and 16-bit PNGs from packed samples.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::Write;

/// Stored samples per pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Gray,
    Rgb,
    Rgba,
}

impl Layout {
    fn color_and_channels(self) -> (png::ColorType, usize) {
        match self {
            Layout::Gray => (png::ColorType::Grayscale, 1),
            Layout::Rgb => (png::ColorType::Rgb, 3),
            Layout::Rgba => (png::ColorType::Rgba, 4),
        }
    }
}

/// PNG encoding or sample validation failure.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("invalid PNG dimensions {0}×{1}")]
    Dimensions(u32, u32),
    #[error("PNG needs {expected} sample bytes, got {actual}")]
    SampleLength { expected: usize, actual: usize },
    #[error(transparent)]
    Png(#[from] png::EncodingError),
}

fn validate_dimensions(width: u32, height: u32, channels: usize, sample_bytes: usize) -> Result<(usize, usize), EncodeError> {
    const MAX_DIMENSION: u32 = i32::MAX as u32;
    let invalid_dimensions = || EncodeError::Dimensions(width, height);
    if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
        return Err(invalid_dimensions());
    }
    let bytes_per_pixel = channels.checked_mul(sample_bytes).ok_or_else(invalid_dimensions)?;
    let row_bytes = (width as usize).checked_mul(bytes_per_pixel).ok_or_else(invalid_dimensions)?;
    // Include each scanline's filter byte in the encoder's uncompressed size.
    let with_filters = row_bytes.checked_add(1).and_then(|stride| stride.checked_mul(height as usize)).filter(|&size| size <= isize::MAX as usize);
    let expected_bytes = with_filters.ok_or_else(invalid_dimensions)? - height as usize;
    Ok((row_bytes, expected_bytes))
}

fn write_png(output: impl Write, width: u32, height: u32, color: png::ColorType, depth: png::BitDepth, bytes: &[u8]) -> Result<(), EncodeError> {
    let mut encoder = png::Encoder::new(output, width, height);
    encoder.set_color(color);
    encoder.set_depth(depth);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::Filter::Adaptive);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(bytes)?;
    writer.finish()?;
    Ok(())
}

/// Write an 8-bit PNG with fast compression and adaptive row filtering.
///
/// Samples are packed without padding in top-to-bottom row order. The dimensions and
/// exact input length are checked before a header is written. Success includes IEND
/// and flushing the sink.
pub fn write_8bit(output: impl Write, width: u32, height: u32, layout: Layout, samples: &[u8]) -> Result<(), EncodeError> {
    let (color, channels) = layout.color_and_channels();
    let (_, expected) = validate_dimensions(width, height, channels, 1)?;
    if samples.len() != expected {
        return Err(EncodeError::SampleLength { expected, actual: samples.len() });
    }
    write_png(output, width, height, color, png::BitDepth::Eight, samples)
}

/// Write a 16-bit PNG with fast compression and adaptive row filtering.
///
/// Samples are native `u16` values packed without padding in top-to-bottom row order
/// and written in big-endian PNG byte order. The dimensions and exact input length are
/// checked before a header is written. Success includes IEND and flushing the sink.
pub fn write_16bit(output: impl Write, width: u32, height: u32, layout: Layout, samples: &[u16]) -> Result<(), EncodeError> {
    let (color, channels) = layout.color_and_channels();
    let (_, expected_bytes) = validate_dimensions(width, height, channels, 2)?;
    let expected_samples = expected_bytes / 2;
    let actual_bytes = samples.len().saturating_mul(2);
    if samples.len() != expected_samples {
        return Err(EncodeError::SampleLength { expected: expected_bytes, actual: actual_bytes });
    }
    let mut big_endian = Vec::with_capacity(expected_bytes);
    for &sample in samples {
        big_endian.extend_from_slice(&sample.to_be_bytes());
    }
    write_png(output, width, height, color, png::BitDepth::Sixteen, &big_endian)
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor};

    use super::{EncodeError, Layout, Write, write_8bit, write_16bit};

    #[test]
    fn decodes_exact_samples_in_each_layout() -> Result<(), Box<dyn std::error::Error>> {
        let (width, height) = (513, 131);
        for (layout, color, channels) in
            [(Layout::Gray, png::ColorType::Grayscale, 1), (Layout::Rgb, png::ColorType::Rgb, 3), (Layout::Rgba, png::ColorType::Rgba, 4)]
        {
            let expected: Vec<u8> = (0..width * height * channels).map(|i| (i * 17 + i / (width * channels)) as u8).collect();
            let mut encoded = Vec::new();
            write_8bit(&mut encoded, width, height, layout, &expected)?;

            assert!(encoded.ends_with(b"\0\0\0\0IEND\xaeB\x60\x82"));
            let mut reader = png::Decoder::new(Cursor::new(encoded)).read_info()?;
            let mut decoded = vec![0; reader.output_buffer_size().ok_or("output size")?];
            let info = reader.next_frame(&mut decoded)?;
            assert_eq!((info.width, info.height, info.color_type, info.bit_depth), (width, height, color, png::BitDepth::Eight));
            assert_eq!(decoded, expected);
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_dimensions_and_sample_lengths_before_output() {
        let mut encoded = Vec::new();
        for (width, height) in [(0, 1), (1, 0), (u32::MAX, 1), (i32::MAX as u32, i32::MAX as u32)] {
            assert!(matches!(write_8bit(&mut encoded, width, height, Layout::Rgba, &[]), Err(EncodeError::Dimensions(..))));
        }
        for actual in [7, 9] {
            assert!(matches!(
                write_8bit(&mut encoded, 2, 1, Layout::Rgba, &[0; 9][..actual]),
                Err(EncodeError::SampleLength { expected: 8, actual: n }) if n == actual
            ));
        }
        assert!(encoded.is_empty());
    }

    #[test]
    fn returns_an_error_when_the_sink_cannot_write_or_flush() {
        struct FlushError;

        impl Write for FlushError {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }

        let mut short = [0; 20];
        assert!(matches!(write_8bit(short.as_mut_slice(), 1, 1, Layout::Gray, &[19]), Err(EncodeError::Png(_))));
        assert!(matches!(write_8bit(FlushError, 1, 1, Layout::Gray, &[19]), Err(EncodeError::Png(_))));
    }

    #[test]
    fn decodes_exact_16bit_samples_in_each_layout() -> Result<(), Box<dyn std::error::Error>> {
        let (width, height) = (37, 19);
        for (layout, color, channels) in
            [(Layout::Gray, png::ColorType::Grayscale, 1), (Layout::Rgb, png::ColorType::Rgb, 3), (Layout::Rgba, png::ColorType::Rgba, 4)]
        {
            let expected: Vec<u16> = (0..width * height * channels).map(|i| ((i * 257 + 13) & 0xFFFF) as u16).collect();
            let mut encoded = Vec::new();
            write_16bit(&mut encoded, width, height, layout, &expected)?;

            assert!(encoded.ends_with(b"\0\0\0\0IEND\xaeB\x60\x82"));
            let mut reader = png::Decoder::new(Cursor::new(encoded)).read_info()?;
            let mut decoded = vec![0u8; reader.output_buffer_size().ok_or("output size")?];
            let info = reader.next_frame(&mut decoded)?;
            assert_eq!((info.width, info.height, info.color_type, info.bit_depth), (width, height, color, png::BitDepth::Sixteen));
            let decoded_u16: Vec<u16> = decoded.as_chunks::<2>().0.iter().copied().map(u16::from_be_bytes).collect();
            assert_eq!(decoded_u16, expected);
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_16bit_dimensions_and_sample_lengths_before_output() {
        let mut encoded = Vec::new();
        for (width, height) in [(0, 1), (1, 0), (u32::MAX, 1), (i32::MAX as u32, i32::MAX as u32)] {
            assert!(matches!(write_16bit(&mut encoded, width, height, Layout::Rgba, &[]), Err(EncodeError::Dimensions(..))));
        }
        for actual in [3, 5] {
            assert!(matches!(
                write_16bit(&mut encoded, 1, 1, Layout::Rgba, &[0u16; 5][..actual]),
                Err(EncodeError::SampleLength { expected: 8, actual: n }) if n == actual * 2
            ));
        }
        assert!(encoded.is_empty());
    }
}
