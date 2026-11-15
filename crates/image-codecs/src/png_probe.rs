//! Header-only PNG inspection without inflating image data.
use crate::{CodecError, Format};
fn err(message: &str) -> CodecError {
    CodecError::malformed(Format::Png, message)
}

/// Dimensions declared in a PNG IHDR prefix, without checking the rest of the file.
/// Useful only for an early allocation limit; use [`header`] for structural inspection.
pub fn declared_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    if !b.starts_with(b"\x89PNG\r\n\x1a\n") || b.get(12..16) != Some(b"IHDR") {
        return None;
    }
    let w = u32::from_be_bytes(b.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(b.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

/// Stored dimensions and EXIF orientation. This does not validate CRCs or compressed pixels.
pub fn header(b: &[u8]) -> Result<(u32, u32, u16), CodecError> {
    header_with_max_pixels(b, u64::MAX)
}

/// As [`header`], refusing images over `max_pixels` before scanning their chunks.
pub fn header_with_max_pixels(b: &[u8], max_pixels: u64) -> Result<(u32, u32, u16), CodecError> {
    if !b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(err("bad signature"));
    }
    let be32 = |p: usize| b.get(p..p.checked_add(4)?).map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]));
    if be32(8) != Some(13) || b.get(12..16) != Some(b"IHDR") {
        return Err(err("missing IHDR"));
    }
    let ihdr = b.get(16..29).ok_or_else(|| err("truncated IHDR"))?;
    let (w, h) = (u32::from_be_bytes([ihdr[0], ihdr[1], ihdr[2], ihdr[3]]), u32::from_be_bytes([ihdr[4], ihdr[5], ihdr[6], ihdr[7]]));
    let (depth, color, compression, filter, interlace) = (ihdr[8], ihdr[9], ihdr[10], ihdr[11], ihdr[12]);
    if w > i32::MAX as u32 || h > i32::MAX as u32 {
        return Err(err("dimensions out of range"));
    }
    if w == 0 || h == 0 {
        return Err(err("zero dimension"));
    }
    if u64::from(w) * u64::from(h) > max_pixels {
        return Err(CodecError::LimitExceeded("PNG pixel count".into()));
    }
    let depth_ok = match color {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(depth, 8 | 16),
        _ => false,
    };
    if !depth_ok || compression != 0 || filter != 0 || interlace > 1 {
        return Err(err("invalid IHDR"));
    }
    let (mut p, mut palette, mut idat, mut after_idat, mut exif) = (8usize, false, false, false, None);
    while let Some(len) = be32(p) {
        let Some(kind) = p.checked_add(4).and_then(|k| b.get(k..k.checked_add(4)?)) else { break };
        if idat && kind != b"IDAT" {
            // a chunk follows the image data: the IDAT run is complete
            after_idat = true;
        }
        if kind == b"IEND" {
            break;
        }
        let start = p.saturating_add(8);
        let data = (len <= i32::MAX as u32).then(|| start.checked_add(len as usize).and_then(|end| b.get(start..end))).flatten();
        let Some(data) = data else {
            if after_idat {
                break; // a damaged chunk after the image data: a decode ignores it too
            }
            return Err(err("truncated"));
        };
        match kind {
            b"PLTE" => palette = true,
            b"IDAT" if after_idat => {} // a stray IDAT after other chunks: the first run is the image
            b"IDAT" => {
                if color == 3 && !palette {
                    return Err(err("indexed image without a palette"));
                }
                idat = true;
            }
            b"eXIf" if exif.is_none() => exif = Some(data.strip_prefix(b"Exif\0\0").unwrap_or(data)),
            _ => {}
        }
        p = start.saturating_add(data.len()).saturating_add(4);
    }
    if !idat {
        return Err(err("no image data"));
    }
    if !after_idat {
        return Err(err("truncated image data"));
    }
    Ok((w, h, exif.map_or(1, crate::exif_orientation)))
}

/// Container color declarations preceding image data. Raw values leave color policy to callers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorChunks {
    pub srgb: bool,
    pub gamma: Option<f32>,
    pub chromaticities: Option<[f64; 8]>,
}

/// Read bounded color chunks before image data; malformed tails are ignored.
pub fn color_chunks(b: &[u8]) -> Option<ColorChunks> {
    if !b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut result = ColorChunks::default();
    let mut p = 8usize;
    while let Some(length) = b.get(p..p.checked_add(4)?) {
        let len = u32::from_be_bytes(length.try_into().ok()?) as usize;
        let start = p.checked_add(8)?;
        let Some(kind) = b.get(p.checked_add(4)?..start) else { break };
        let Some(payload) = start.checked_add(len).and_then(|end| b.get(start..end)) else { break };
        match kind {
            b"sRGB" => result.srgb = true,
            b"gAMA" if payload.len() == 4 => {
                let v = u32::from_be_bytes(payload.try_into().ok()?) as f32 / 100_000.0;
                if v > 0.0 && v.is_finite() {
                    result.gamma = Some(v);
                }
            }
            b"cHRM" if payload.len() == 32 => {
                let mut vals = [0.0; 8];
                for (i, dst) in vals.iter_mut().enumerate() {
                    let raw = i32::from_be_bytes(payload[i * 4..i * 4 + 4].try_into().ok()?);
                    *dst = f64::from(raw) / 100_000.0;
                }
                result.chromaticities = Some(vals);
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        let Some(next) = start.checked_add(len).and_then(|end| end.checked_add(4)) else { break };
        p = next;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0; 4]); // Header inspection deliberately does not validate CRCs.
    }

    fn png(color: u8) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&3u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, color, 0, 0, 0]);
        chunk(&mut bytes, b"IHDR", &ihdr);
        bytes
    }

    #[test]
    fn header_checks_structure_and_reads_late_orientation_without_crc_validation() {
        let mut bytes = png(2);
        chunk(&mut bytes, b"IDAT", &[1]);
        let exif = b"II\x2a\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00\x06\x00\x00\x00\x00\x00\x00\x00";
        chunk(&mut bytes, b"eXIf", exif);
        chunk(&mut bytes, b"IEND", &[]);
        assert_eq!(header(&bytes).ok(), Some((2, 3, 6)));
        assert!(matches!(header_with_max_pixels(&bytes, 5), Err(CodecError::LimitExceeded(_))));
        assert_eq!(declared_dimensions(&bytes), Some((2, 3)));
        assert!(header(&bytes[..34]).is_err());
        let mut indexed = png(3);
        chunk(&mut indexed, b"IDAT", &[1]);
        chunk(&mut indexed, b"IEND", &[]);
        assert!(header(&indexed).is_err());
        indexed[25] = 7;
        assert!(header(&indexed).is_err());
    }

    #[test]
    fn reads_color_chunks_only_before_image_data() {
        let mut bytes = png(2);
        chunk(&mut bytes, b"gAMA", &45455u32.to_be_bytes());
        chunk(&mut bytes, b"IDAT", &[1]);
        chunk(&mut bytes, b"sRGB", &[0]);
        let color = color_chunks(&bytes).unwrap();
        assert_eq!(color.gamma, Some(0.45455));
        assert!(!color.srgb);
        assert!(color_chunks(b"not png").is_none());
    }
}
