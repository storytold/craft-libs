//! Explicit verbatim EXIF and JPEG APP metadata helpers for callers that preserve stored orientation.
//! Normal encoder APIs still normalize metadata for upright pixels; callers opt in to these helpers.
use crate::{CodecError, Format};

/// Borrowed metadata used to construct JPEG APP segments.
#[derive(Clone, Copy, Default)]
pub struct JpegMetadata<'a> {
    /// TIFF-structured EXIF bytes.
    pub exif: Option<&'a [u8]>,
    /// XMP packet.
    pub xmp: Option<&'a str>,
    /// ICC profile bytes.
    pub icc: Option<&'a [u8]>,
    /// Pixels per inch, if nonzero.
    pub ppi: Option<u16>,
}

fn error(format: Format, message: &str) -> CodecError {
    CodecError::encode(format, message)
}

/// Build JFIF, EXIF, XMP and ICC APP payloads in that order, preserving bytes and segment limits.
pub fn jpeg_segments(meta: JpegMetadata<'_>) -> Result<Vec<(u8, Vec<u8>)>, CodecError> {
    let mut segments = Vec::new();
    let density = meta.ppi.filter(|p| *p > 0).map_or([0, 0, 1, 0, 1], |p| {
        let [hi, lo] = p.to_be_bytes();
        [1, hi, lo, hi, lo]
    });
    segments.push((0xe0, [&b"JFIF\0\x01\x02"[..], &density, &[0, 0]].concat()));
    if let Some(exif) = meta.exif {
        if exif.len() > 65_527 {
            return Err(error(Format::Jpeg, "EXIF larger than one APP1 segment (64 KiB)"));
        }
        segments.push((0xe1, [&b"Exif\0\0"[..], exif].concat()));
    }
    if let Some(xmp) = meta.xmp {
        segments.push((0xe1, jpeg_xmp(xmp)?));
    }
    if let Some(icc) = meta.icc {
        let chunks: Vec<_> = icc.chunks(65_519).collect();
        if chunks.len() > 255 {
            return Err(error(Format::Jpeg, "ICC profile too large for JPEG"));
        }
        for (i, chunk) in chunks.iter().enumerate() {
            segments.push((0xe2, [&b"ICC_PROFILE\0"[..], &[i as u8 + 1, chunks.len() as u8], chunk].concat()));
        }
    }
    Ok(segments)
}

/// Build a single standard XMP APP1 payload, rejecting extended-XMP-sized packets.
pub fn jpeg_xmp(xmp: &str) -> Result<Vec<u8>, CodecError> {
    let payload = [&b"http://ns.adobe.com/xap/1.0/\0"[..], xmp.as_bytes()].concat();
    if payload.len() > 65_533 {
        return Err(error(Format::Jpeg, "XMP larger than one APP1 segment (extended XMP not supported)"));
    }
    Ok(payload)
}

/// Restore bytes in the first existing JPEG EXIF payload when it has room. No segment is inserted.
pub fn replace_jpeg_exif(jpeg: &mut [u8], exif: &[u8]) {
    if let Some(pos) = jpeg.windows(6).position(|w| w == b"Exif\0\0")
        && let Some(end) = pos.checked_add(6).and_then(|v| v.checked_add(exif.len()))
        && let Some(dst) = jpeg.get_mut(pos + 6..end)
    {
        dst.copy_from_slice(exif);
    }
}

/// Replace the existing PNG eXIf chunk, preserving the other chunks and recomputing its CRC.
/// No chunk is inserted when one is absent.
pub fn replace_png_exif(png: &mut Vec<u8>, exif: &[u8]) -> Result<(), CodecError> {
    let len = u32::try_from(exif.len()).map_err(|_| error(Format::Png, "EXIF too large"))?;
    let mut p = 8usize;
    while let Some(bytes) = p.checked_add(4).and_then(|end| png.get(p..end)) {
        let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let Some(end) = p.checked_add(12).and_then(|v| v.checked_add(size)).filter(|end| *end <= png.len()) else { break };
        if png.get(p + 4..p + 8) == Some(b"eXIf") {
            let mut chunk = Vec::new();
            chunk.extend_from_slice(&len.to_be_bytes());
            chunk.extend_from_slice(b"eXIf");
            chunk.extend_from_slice(exif);
            let crc = crc32fast::hash(&chunk[4..]);
            chunk.extend_from_slice(&crc.to_be_bytes());
            png.splice(p..end, chunk);
            break;
        }
        p = end;
    }
    Ok(())
}

/// Replace the existing WebP EXIF chunk, including padding and the RIFF size. No chunk is inserted
/// when one is absent. The input must be an encoded WebP as emitted by this crate.
pub fn replace_webp_exif(webp: &mut Vec<u8>, exif: &[u8]) -> Result<(), CodecError> {
    let len = u32::try_from(exif.len()).map_err(|_| error(Format::WebP, "EXIF too large"))?;
    let mut p = 12usize;
    while let Some(h) = webp.get(p..p.saturating_add(8)) {
        let size = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
        let end = p.saturating_add(8).saturating_add(size).saturating_add(size & 1);
        if &h[..4] == b"EXIF" && end <= webp.len() {
            let new_len = webp
                .len()
                .checked_sub(end - p)
                .and_then(|v| v.checked_add(8))
                .and_then(|v| v.checked_add(exif.len()))
                .and_then(|v| v.checked_add(exif.len() & 1))
                .ok_or_else(|| error(Format::WebP, "RIFF too large"))?;
            let riff_len = u32::try_from(new_len.saturating_sub(8)).map_err(|_| error(Format::WebP, "RIFF too large"))?;
            let mut chunk = Vec::new();
            chunk.extend_from_slice(b"EXIF");
            chunk.extend_from_slice(&len.to_le_bytes());
            chunk.extend_from_slice(exif);
            if exif.len() & 1 == 1 {
                chunk.push(0);
            }
            webp.splice(p..end, chunk);
            webp[4..8].copy_from_slice(&riff_len.to_le_bytes());
            break;
        }
        p = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXIF: &[u8] = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    #[test]
    fn explicit_replacements_preserve_raw_orientation_in_real_encodings() {
        let mut image = crate::Image::from_u8(2, 2, crate::ChannelLayout::Rgb, vec![80; 12]).unwrap();
        image.meta.exif = Some(EXIF.to_vec());
        for format in [Format::Jpeg, Format::Png, Format::WebP] {
            let mut bytes = crate::encode(&image, format, &crate::EncodeOptions::default()).unwrap();
            match format {
                Format::Jpeg => replace_jpeg_exif(&mut bytes, EXIF),
                Format::Png => replace_png_exif(&mut bytes, EXIF).unwrap(),
                Format::WebP => replace_webp_exif(&mut bytes, EXIF).unwrap(),
                _ => {}
            }
            let decoded = crate::decode_as_with(format, &bytes, &crate::DecodeOptions { keep_orientation: true, ..Default::default() }).unwrap();
            assert_eq!(decoded.meta.exif.as_deref().map(crate::exif_orientation), Some(6), "{format:?}");
        }
    }
    #[test]
    fn jpeg_segments_keep_order_and_enforce_limits() {
        let parts = jpeg_segments(JpegMetadata { exif: Some(EXIF), xmp: Some("<x/>"), icc: Some(&[1, 2]), ppi: Some(300) }).unwrap();
        assert_eq!(parts.iter().map(|v| v.0).collect::<Vec<_>>(), [0xe0, 0xe1, 0xe1, 0xe2]);
        assert!(jpeg_segments(JpegMetadata { exif: Some(&vec![0; 65_528]), ..Default::default() }).is_err());
        assert!(jpeg_xmp(&"x".repeat(65_533)).is_err());
    }
}
