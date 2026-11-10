//! JPEG images, as compiled graphs store them: baseline greyscale ones by [`baseline_gray`] (for its inverse DCT, which
//! is nearer to the reference's), anything else by `zune-jpeg`. The result is 8-bit samples, greyscale or RGB.

mod baseline_gray;

use std::fmt;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

/// Decoded 8-bit samples, row by row, `channels` per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegImage {
    pub width: usize,
    pub height: usize,
    /// 1 (greyscale), 3 (RGB) or, from [`decode_with_alpha`], 4 (RGBA).
    pub channels: usize,
    pub samples: Vec<u8>,
}

/// Why a JPEG could not be decoded, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegError(pub String);

impl fmt::Display for JpegError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JpegError {}

/// The JPEG's samples, keeping its channel count: greyscale stays greyscale, anything else becomes RGB.
pub fn decode(jpeg: &[u8]) -> Result<JpegImage, JpegError> {
    if let Some((width, height, samples)) = baseline_gray::decode(jpeg) {
        return Ok(JpegImage { width, height, channels: 1, samples });
    }
    let mut probe = JpegDecoder::new(ZCursor::new(jpeg));
    probe.decode_headers().map_err(|e| JpegError(e.to_string()))?;
    let info = probe.info().ok_or_else(|| JpegError("no image header".into()))?;
    let (space, channels) = if info.components == 1 { (ColorSpace::Luma, 1) } else { (ColorSpace::RGB, 3) };
    let options = DecoderOptions::default().jpeg_set_out_colorspace(space);
    let samples = JpegDecoder::new_with_options(ZCursor::new(jpeg), options).decode().map_err(|e| JpegError(e.to_string()))?;
    let (width, height) = (usize::from(info.width), usize::from(info.height));
    if Some(samples.len()) != width.checked_mul(height).and_then(|n| n.checked_mul(channels)) {
        return Err(JpegError(format!("decoded {} bytes for a {width}x{height} image", samples.len())));
    }
    Ok(JpegImage { width, height, channels, samples })
}

/// A colour (or greyscale) JPEG with a greyscale JPEG of the same size as its alpha, as RGBA (a grey image in all three
/// colour channels; the alpha from the first channel of the second image).
pub fn decode_with_alpha(jpeg: &[u8], alpha: &[u8]) -> Result<JpegImage, JpegError> {
    let colour = decode(jpeg)?;
    let a = decode(alpha)?;
    let (w, h) = (colour.width, colour.height);
    if (a.width, a.height) != (w, h) {
        return Err(JpegError(format!("alpha {}x{} for a {w}x{h} image", a.width, a.height)));
    }
    let samples: Vec<u8> = colour
        .samples
        .chunks_exact(colour.channels)
        .zip(a.samples.chunks_exact(a.channels))
        .flat_map(|(c, a)| {
            let first = a.first().copied().unwrap_or(0);
            match *c {
                [r, g, b] => [r, g, b, first],
                [v, ..] => [v, v, v, first],
                [] => [0, 0, 0, first],
            }
        })
        .collect();
    Ok(JpegImage { width: w, height: h, channels: 4, samples })
}
