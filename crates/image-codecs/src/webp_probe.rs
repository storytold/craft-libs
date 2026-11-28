//! Header-only WebP inspection with image-chunk completeness checks.

use crate::{CodecError, Format};

/// Stored width, height and EXIF orientation without decoding pixels. The image payload must
/// fit in the input, but its compressed contents are not validated. Uses the default decode limits.
pub fn header(bytes: &[u8]) -> Result<(u32, u32, u16), CodecError> {
    let err = |s: &str| CodecError::malformed(Format::WebP, s);
    let mut p = 12usize;
    let mut has_image = false;
    let mut exif = None;
    while let Some(h) = bytes.get(p..p.saturating_add(8)) {
        let size = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
        let end = p.saturating_add(8).saturating_add(size);
        if matches!(&h[..4], b"VP8 " | b"VP8L" | b"ANMF") {
            if end > bytes.len() {
                return Err(err("truncated image data"));
            }
            has_image = true;
        } else if &h[..4] == b"EXIF" && end <= bytes.len() && exif.is_none() {
            exif = bytes.get(p + 8..end);
        }
        p = end.saturating_add(size & 1);
    }
    if !has_image {
        return Err(err("no image data"));
    }
    // Read stored dimensions directly. Undoing probe_size with the compatibility EXIF
    // orientation is incorrect when its lenient parser disagrees with the strict one.
    let (w, h, _) = crate::codecs::webp::probe(bytes, &crate::Limits::default())?;
    let orientation = exif.map_or(1, craft_metadata::compat_orientation::orientation);
    Ok((w, h, orientation))
}

/// Whether a WebP carries transparency, from the first chunk only (nothing is decoded):
/// `VP8X` flag bit 0x10, `VP8L` `alpha_is_used`, plain `VP8` never.
pub fn has_alpha(bytes: &[u8]) -> Result<bool, CodecError> {
    let err = |s: &str| CodecError::malformed(Format::WebP, s);
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return Err(err("bad signature"));
    }
    match bytes.get(12..16) {
        Some(b"VP8X") => Ok(bytes.get(20).ok_or_else(|| err("truncated VP8X"))? & 0x10 != 0),
        // 0x2f signature byte, then 14+14 size bits, then the alpha_is_used bit (bit 28)
        Some(b"VP8L") => match bytes.get(20..25) {
            Some([0x2f, a, b, c, d]) => Ok(u32::from_le_bytes([*a, *b, *c, *d]) >> 28 & 1 != 0),
            _ => Err(err("bad VP8L header")),
        },
        Some(b"VP8 ") => Ok(false),
        _ => Err(err("no image data")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_stored_dimensions_with_exif_orientation() {
        let mut image = crate::Image::from_u8(2, 3, crate::ChannelLayout::Rgb, vec![42; 18]).unwrap();
        image.meta.exif = Some(b"II\x2a\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00\x06\x00\x00\x00\x00\x00\x00\x00".to_vec());
        let mut bytes = crate::encode(&image, Format::WebP, &crate::EncodeOptions::default()).unwrap();
        // The writer correctly makes exported pixels upright; simulate a camera file carrying orientation 6.
        let exif = bytes.windows(4).position(|v| v == b"EXIF").unwrap();
        let tiff = exif + 8;
        bytes[tiff + 18] = 6;
        assert_eq!(header(&bytes).unwrap(), (2, 3, 6));
    }

    #[test]
    fn malformed_exif_does_not_swap_stored_dimensions() {
        let mut image = crate::Image::from_u8(2, 3, crate::ChannelLayout::Rgb, vec![42; 18]).unwrap();
        image.meta.exif = Some(b"II\x2a\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00\x06\x00\x00\x00\x00\x00\x00\x00".to_vec());
        let mut bytes = crate::encode(&image, Format::WebP, &crate::EncodeOptions::default()).unwrap();
        let exif = bytes.windows(4).position(|v| v == b"EXIF").unwrap() + 8;
        bytes[exif + 18] = 6;
        // Compatibility metadata preserves orientation despite a malformed count;
        // the strict orientation used by probe_size rejects it. Stored size is neither.
        bytes[exif + 14..exif + 18].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(crate::probe_size(&bytes).unwrap(), (2, 3));
        assert_eq!(header(&bytes).unwrap(), (2, 3, 6));
    }

    #[test]
    fn rejects_missing_and_truncated_image_chunks() {
        let mut bytes = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        assert!(header(&bytes).is_err());
        bytes.extend_from_slice(b"VP8 ");
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.push(0);
        assert!(header(&bytes).is_err());
    }

    #[test]
    fn has_alpha_reads_vp8x_vp8l_and_plain_vp8() {
        for (layout, want) in [(crate::ChannelLayout::Rgb, false), (crate::ChannelLayout::Rgba, true)] {
            let n = layout.channels();
            let image = crate::Image::from_u8(2, 3, layout, vec![42; 6 * n]).unwrap();
            let bytes = crate::encode(&image, Format::WebP, &crate::EncodeOptions::default()).unwrap();
            assert_eq!(has_alpha(&bytes).unwrap(), want);
        }
        let riff = |chunk: &[u8; 4], body: &[u8]| {
            let mut v = b"RIFF\0\0\0\0WEBP".to_vec();
            v.extend_from_slice(chunk);
            v.extend_from_slice(&(body.len() as u32).to_le_bytes());
            v.extend_from_slice(body);
            v
        };
        let mut x = [0u8; 10];
        assert!(!has_alpha(&riff(b"VP8X", &x)).unwrap());
        x[0] = 0x10;
        assert!(has_alpha(&riff(b"VP8X", &x)).unwrap());
        assert!(!has_alpha(&riff(b"VP8 ", &[0; 10])).unwrap());
        assert!(has_alpha(&riff(b"VP8L", &[0x2f, 0, 0, 0, 0x10])).unwrap());
        assert!(!has_alpha(&riff(b"VP8L", &[0x2f, 0, 0, 0, 0])).unwrap());
    }

    #[test]
    fn has_alpha_rejects_malformed_prefixes() {
        let full = [b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0".as_slice(), &[0x10; 10]].concat();
        for n in 0..20 {
            assert!(has_alpha(&full[..n]).is_err(), "prefix {n}");
        }
        assert!(has_alpha(&full[..21]).is_ok());
        assert!(has_alpha(b"RIFF\0\0\0\0WEBPVP8L\x05\0\0\0\x2e").is_err());
        assert!(has_alpha(b"junk").is_err());
    }
}
