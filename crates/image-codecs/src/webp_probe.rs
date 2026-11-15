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
    let (w, h) = crate::probe_size(bytes)?;
    let orientation = exif.map_or(1, crate::exif_orientation);
    let (w, h) = if (5..=8).contains(&orientation) { (h, w) } else { (w, h) };
    Ok((w, h, orientation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_and_truncated_image_chunks() {
        let mut bytes = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        assert!(header(&bytes).is_err());
        bytes.extend_from_slice(b"VP8 ");
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.push(0);
        assert!(header(&bytes).is_err());
    }
}
