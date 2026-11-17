//! First-directory TIFF inspection without decoding pixels.

/// A header inspection failure, distinguishing caller limits from unsupported decoder layouts.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Dimensions exceed the caller's pixel limit.
    #[error("TIFF dimensions {width}x{height} exceed the pixel limit")]
    TooLarge {
        /// Stored width.
        width: u32,
        /// Stored height.
        height: u32,
    },
    /// The shared TIFF decoder does not support this layout.
    #[error("unsupported TIFF: {0}")]
    Unsupported(&'static str),
    /// The file's header, directory or image-data bounds are malformed.
    #[error("malformed TIFF: {0}")]
    Malformed(String),
}

/// First-directory values needed to preflight decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeInfo {
    /// Stored width.
    pub width: u32,
    /// Stored height.
    pub height: u32,
    /// First sample's bit depth, defaulting to eight.
    pub bit_depth: u8,
    /// Raw orientation tag, if present.
    pub orientation: Option<u16>,
}

fn first_directory(bytes: &[u8], max_pixels: u64) -> Result<(craft_tiff::Ifd, craft_tiff::image::ImageInfo), Error> {
    let (tiff, complete) = craft_tiff::Tiff::parse_with_status(bytes, &Default::default()).map_err(|e| Error::Malformed(e.to_string()))?;
    if !complete {
        return Err(Error::Malformed("truncated TIFF".into()));
    }
    let ifd = tiff.ifds.into_iter().next().ok_or_else(|| Error::Malformed("no IFD".into()))?;
    let info = ifd.image().map_err(|e| Error::Malformed(e.to_string()))?;
    if info.width == 0 || info.height == 0 {
        return Err(Error::Malformed("zero dimension".into()));
    }
    if u64::from(info.width) * u64::from(info.height) > max_pixels {
        return Err(Error::TooLarge { width: info.width, height: info.height });
    }
    Ok((ifd, info))
}

/// Parse the first directory and enforce the pixel limit before decoding.
/// Unlike [`header`], this does not impose layout or strip checks ahead of the decoder.
pub fn decode_info(bytes: &[u8], max_pixels: u64) -> Result<DecodeInfo, Error> {
    let (ifd, info) = first_directory(bytes, max_pixels)?;
    Ok(DecodeInfo {
        width: info.width,
        height: info.height,
        bit_depth: info.bits_per_sample.first().copied().unwrap_or(8) as u8,
        orientation: ifd.u16(craft_tiff::tags::ORIENTATION),
    })
}

/// Stored dimensions and orientation of the first directory, after checking the layouts and
/// image-data bounds required by the TIFF decoder. Compressed strips are not decompressed.
pub fn header(bytes: &[u8], max_pixels: u64) -> Result<(u32, u32, u16), Error> {
    let (ifd, info) = first_directory(bytes, max_pixels)?;
    let bits = info.bits_per_sample.first().copied().unwrap_or(1);
    if bits < 8 && info.planar == 2 && info.samples_per_pixel > 1 {
        return Err(Error::Unsupported("planar TIFF with samples narrower than a byte"));
    }
    if !(matches!(info.photometric, 0 | 1 | 2 | 3 | 5) || info.photometric == 6 && info.compression == 7) {
        return Err(Error::Unsupported("colour type"));
    }
    if !matches!(info.compression, 1 | 4 | 5 | 7 | 8 | 32773 | 32946) {
        return Err(Error::Unsupported("compression method"));
    }
    let len = bytes.len() as u64;
    if info.offsets.is_empty()
        || info.offsets.iter().zip(&info.byte_counts).any(|(&o, &c)| c > 0 && (o >= len || (info.compression == 1 && o.saturating_add(c) > len)))
    {
        return Err(Error::Malformed("truncated: image data past the end of the file".into()));
    }
    let orientation = ifd.u16(craft_tiff::tags::ORIENTATION).filter(|o| (1..=8).contains(o)).unwrap_or(1);
    Ok((info.width, info.height, orientation))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_first_directory_and_enforces_limits_and_bounds() {
        let image = crate::Image::from_u8(2, 3, crate::ChannelLayout::Rgb, vec![42; 18]).unwrap();
        let bytes = crate::encode(&image, crate::Format::Tiff, &crate::EncodeOptions::default()).unwrap();
        assert_eq!(header(&bytes, 6).unwrap(), (2, 3, 1));
        let info = decode_info(&bytes, 6).unwrap();
        assert_eq!((info.width, info.height, info.bit_depth), (2, 3, 8));
        assert!(matches!(decode_info(&bytes, 5), Err(Error::TooLarge { width: 2, height: 3 })));
        assert!(matches!(header(&bytes, 5), Err(Error::TooLarge { width: 2, height: 3 })));
        assert!(header(&bytes[..8], 6).is_err());
    }
}
