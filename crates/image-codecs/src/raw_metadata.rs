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
///
/// The EXIF APP1 is found by walking the marker segments before the first scan, so an `Exif\0\0`
/// inside another segment (a comment, say) is never taken for it, and the copy must fit inside
/// the APP1's own declared length, not merely inside the file.
pub fn replace_jpeg_exif(jpeg: &mut [u8], exif: &[u8]) {
    if let Some(payload) = jpeg_exif_payload(jpeg)
        && let Some(dst) = jpeg.get_mut(payload).and_then(|p| p.get_mut(..exif.len()))
    {
        dst.copy_from_slice(exif);
    }
}

/// The byte range of the TIFF data in the first EXIF APP1 (after `Exif\0\0`), up to the end of
/// that segment; `None` when the header segments end, are damaged, or hold no EXIF APP1.
fn jpeg_exif_payload(jpeg: &[u8]) -> Option<std::ops::Range<usize>> {
    if jpeg.get(..2) != Some(&[0xff, 0xd8][..]) {
        return None;
    }
    let mut p = 2usize;
    loop {
        // fill bytes (0xff) may precede a marker
        while jpeg.get(p) == Some(&0xff) && jpeg.get(p + 1) == Some(&0xff) {
            p += 1;
        }
        let (&0xff, &marker) = (jpeg.get(p)?, jpeg.get(p + 1)?) else { return None };
        match marker {
            // TEM and RSTn carry no length
            0x01 | 0xd0..=0xd7 => p += 2,
            // start of scan, end of image: no header segment follows
            0xda | 0xd9 | 0x00 => return None,
            _ => {
                let len = usize::from(u16::from_be_bytes([*jpeg.get(p + 2)?, *jpeg.get(p + 3)?]));
                let end = p.checked_add(2)?.checked_add(len).filter(|&e| len >= 2 && e <= jpeg.len())?;
                if marker == 0xe1 && jpeg.get(p + 4..end)?.starts_with(b"Exif\0\0") {
                    return Some(p + 10..end);
                }
                p = end;
            }
        }
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
    fn replacements_match_previous_code_on_mutated_containers() {
        fn old_png(png: &mut Vec<u8>, exif: &[u8]) {
            let mut p = 8usize;
            while let Some(b) = png.get(p..p + 4) {
                let size = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
                let end = p.saturating_add(12).saturating_add(size);
                if end > png.len() {
                    break;
                }
                if png.get(p + 4..p + 8) == Some(b"eXIf") {
                    let mut chunk = (exif.len() as u32).to_be_bytes().to_vec();
                    chunk.extend_from_slice(b"eXIf");
                    chunk.extend_from_slice(exif);
                    let mut crc = 0xffff_ffffu32;
                    for &v in &chunk[4..] {
                        crc ^= u32::from(v);
                        for _ in 0..8 {
                            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
                        }
                    }
                    chunk.extend_from_slice(&(!crc).to_be_bytes());
                    png.splice(p..end, chunk);
                    return;
                }
                p = end;
            }
        }
        fn old_webp(webp: &mut Vec<u8>, exif: &[u8]) {
            let mut p = 12usize;
            while let Some(h) = webp.get(p..p.saturating_add(8)) {
                let size = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
                let end = p.saturating_add(8).saturating_add(size).saturating_add(size & 1);
                if &h[..4] == b"EXIF" && end <= webp.len() {
                    let mut chunk = b"EXIF".to_vec();
                    chunk.extend_from_slice(&(exif.len() as u32).to_le_bytes());
                    chunk.extend_from_slice(exif);
                    if exif.len() & 1 == 1 {
                        chunk.push(0);
                    }
                    webp.splice(p..end, chunk);
                    let size = (webp.len().saturating_sub(8)) as u32;
                    webp[4..8].copy_from_slice(&size.to_le_bytes());
                    return;
                }
                p = end;
            }
        }
        let mut image = crate::Image::from_u8(2, 2, crate::ChannelLayout::Rgb, vec![80; 12]).unwrap();
        image.meta.exif = Some(EXIF.to_vec());
        for format in [Format::Jpeg, Format::Png, Format::WebP] {
            let encoded = crate::encode(&image, format, &crate::EncodeOptions::default()).unwrap();
            for at in 0..encoded.len() {
                for value in [0, 255] {
                    let mut original = encoded.clone();
                    original[at] = value;
                    let mut shared = original.clone();
                    if format == Format::Jpeg {
                        // the old byte search; the segment walk writes where it did or, when the
                        // mutation broke the segment structure or the APP1 length, not at all
                        let untouched = original.clone();
                        if let Some(pos) = original.windows(6).position(|w| w == b"Exif\0\0")
                            && let Some(dst) = original.get_mut(pos + 6..pos + 6 + EXIF.len())
                        {
                            dst.copy_from_slice(EXIF);
                        }
                        replace_jpeg_exif(&mut shared, EXIF);
                        assert!(shared == original || shared == untouched, "Jpeg: byte {at} value {value}");
                        if at == 0 && value == 255 {
                            // SOI kept: a real replacement
                            assert_eq!(shared, original);
                        }
                        continue;
                    } else if format == Format::Png {
                        old_png(&mut original, EXIF);
                        replace_png_exif(&mut shared, EXIF).unwrap();
                    } else {
                        old_webp(&mut original, EXIF);
                        replace_webp_exif(&mut shared, EXIF).unwrap();
                    }
                    assert_eq!(shared, original, "{format:?}: byte {at} value {value}");
                }
            }
        }
    }

    #[test]
    fn explicit_replacements_preserve_raw_orientation_in_real_encodings() {
        let mut image = crate::Image::from_u8(2, 2, crate::ChannelLayout::Rgb, vec![80; 12]).unwrap();
        image.meta.exif = Some(EXIF.to_vec());
        for format in [Format::Jpeg, Format::Png, Format::WebP] {
            let mut bytes = crate::encode(&image, format, &crate::EncodeOptions::default()).unwrap();
            let normal = crate::decode_as_with(format, &bytes, &crate::DecodeOptions { keep_orientation: true, ..Default::default() }).unwrap();
            assert_eq!(normal.meta.exif.as_deref().map(crate::exif_orientation), Some(1), "default {format:?}");
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
    fn jpeg_exif_is_found_by_segment_not_by_signature() {
        fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
            [&[0xff, marker][..], &((payload.len() + 2) as u16).to_be_bytes(), payload].concat()
        }
        let tail = [0xff, 0xda, 0, 2, 0xff, 0xd9];
        let exif_app1 = segment(0xe1, &[&b"Exif\0\0"[..], &[0; 26]].concat());
        // a comment carrying the signature comes first: only the APP1 is rewritten
        let comment = segment(0xfe, &[&b"Exif\0\0"[..], &[7; 40]].concat());
        let mut jpeg = [&[0xff, 0xd8, 0xff][..], &comment, &exif_app1, &tail].concat();
        let before = jpeg.clone();
        replace_jpeg_exif(&mut jpeg, &[9; 26]);
        let app1_data = 3 + comment.len() + 10;
        assert_eq!(&jpeg[..app1_data], &before[..app1_data]);
        assert_eq!(&jpeg[app1_data..app1_data + 26], &[9; 26]);
        assert_eq!(&jpeg[app1_data + 26..], &before[app1_data + 26..]);
        // no EXIF APP1, only the comment: nothing changes
        let mut only_comment = [&[0xff, 0xd8][..], &comment, &tail].concat();
        let before = only_comment.clone();
        replace_jpeg_exif(&mut only_comment, &[9; 26]);
        assert_eq!(only_comment, before);
        // a replacement larger than the APP1 is refused even though the file continues
        let mut small = [&[0xff, 0xd8][..], &exif_app1, &segment(0xe2, &[1; 64]), &tail].concat();
        let before = small.clone();
        replace_jpeg_exif(&mut small, &[9; 28]);
        assert_eq!(small, before);
        // markers without a length, damaged lengths, a scan before any APP1, and a non-JPEG
        let mut tem = [&[0xff, 0xd8, 0xff, 0x01][..], &exif_app1, &tail].concat();
        replace_jpeg_exif(&mut tem, &[9; 26]);
        assert_eq!(&tem[14..40], &[9; 26]);
        for broken in [vec![0xff, 0xd8, 0xff, 0xe1, 0, 1], vec![0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff, 0], tail.to_vec(), vec![0xff, 0xd8, 0xff, 0xda]] {
            let mut b = broken.clone();
            replace_jpeg_exif(&mut b, &[9; 2]);
            assert_eq!(b, broken);
        }
        let mut no_marker = [&[0xff, 0xd8, 0x12][..], &exif_app1].concat();
        let before = no_marker.clone();
        replace_jpeg_exif(&mut no_marker, &[9; 26]);
        assert_eq!(no_marker, before);
    }

    #[test]
    fn jpeg_segments_keep_order_and_enforce_limits() {
        let parts = jpeg_segments(JpegMetadata { exif: Some(EXIF), xmp: Some("<x/>"), icc: Some(&[1, 2]), ppi: Some(300) }).unwrap();
        assert_eq!(parts.iter().map(|v| v.0).collect::<Vec<_>>(), [0xe0, 0xe1, 0xe1, 0xe2]);
        assert!(jpeg_segments(JpegMetadata { exif: Some(&vec![0; 65_528]), ..Default::default() }).is_err());
        assert!(jpeg_xmp(&"x".repeat(65_533)).is_err());
        let profile = vec![42; 65_520];
        let chunks = jpeg_segments(JpegMetadata { icc: Some(&profile), ..Default::default() }).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(&chunks[1].1[12..14], &[1, 2]);
        assert_eq!(&chunks[2].1[12..14], &[2, 2]);
    }
}
