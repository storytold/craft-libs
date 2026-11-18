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
    Ok((w, h, exif.map_or(1, craft_metadata::compat_orientation::orientation)))
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

/// Most chunks [`pixel_model`] walks before the image data; each chunk is at least 12 bytes.
const MAX_PROBE_CHUNKS: usize = 1 << 16;

/// Pixel model of a PNG, from `IHDR` and the chunk headers before the first `IDAT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelModel {
    pub width: u32,
    pub height: u32,
    /// `IHDR` bit depth (1, 2, 4, 8 or 16).
    pub bit_depth: u8,
    /// `IHDR` colour type (0 grey, 2 RGB, 3 palette, 4 grey+alpha, 6 RGBA).
    pub color_type: u8,
    /// A `tRNS` chunk precedes the image data; it adds alpha to grey, RGB and palette images.
    pub has_trns: bool,
}

impl PixelModel {
    /// Channels of the decoded buffer (palette expands to RGB; `tRNS` adds an alpha channel).
    pub fn channels(&self) -> u8 {
        let base = match self.color_type {
            0 => 1,
            2 | 3 => 3,
            4 => 2,
            _ => 4,
        };
        base + u8::from(self.has_trns && matches!(self.color_type, 0 | 2 | 3))
    }

    /// Bytes per pixel of the buffer a decoder that expands sub-byte samples to 8 bits gives:
    /// `channels * (2 if bit_depth == 16 else 1)`.
    pub fn bytes_per_pixel(&self) -> u8 {
        self.channels() * if self.bit_depth == 16 { 2 } else { 1 }
    }

    /// Whether the image has transparency (alpha colour type or `tRNS`).
    pub fn has_alpha(&self) -> bool {
        matches!(self.color_type, 4 | 6) || self.has_trns
    }
}

/// Dimensions, bit depth, colour type and `tRNS` presence without inflating anything (an `iCCP`
/// profile is skipped, not decompressed). Walks at most 65,536 chunk headers and stops at `IDAT`;
/// `IEND` or the end of the input before `IDAT` is an error. Does not validate CRCs.
/// A caller gets its `(width, height, bytes_per_pixel, alpha)` as
/// `(m.width, m.height, u64::from(m.bytes_per_pixel()), m.has_alpha())`.
pub fn pixel_model(b: &[u8]) -> Result<PixelModel, CodecError> {
    if !b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(err("bad signature"));
    }
    let be32 = |p: usize| b.get(p..p.checked_add(4)?).and_then(|v| <[u8; 4]>::try_from(v).ok()).map(u32::from_be_bytes);
    if be32(8) != Some(13) || b.get(12..16) != Some(b"IHDR") {
        return Err(err("missing IHDR"));
    }
    let ihdr = b.get(16..29).ok_or_else(|| err("truncated IHDR"))?;
    let (width, height) = (be32(16).unwrap_or(0), be32(20).unwrap_or(0));
    let (bit_depth, color_type) = (ihdr[8], ihdr[9]);
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err(err("dimensions out of range"));
    }
    let depth_ok = match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(bit_depth, 8 | 16),
        _ => false,
    };
    if !depth_ok {
        return Err(err("invalid IHDR"));
    }
    let mut has_trns = false;
    let mut p = 8usize;
    for _ in 0..MAX_PROBE_CHUNKS {
        let len = be32(p).ok_or_else(|| err("truncated"))?;
        let kind = p.checked_add(4).and_then(|k| b.get(k..k.checked_add(4)?)).ok_or_else(|| err("truncated"))?;
        match kind {
            b"IDAT" => return Ok(PixelModel { width, height, bit_depth, color_type, has_trns }),
            b"IEND" => return Err(err("no image data")),
            b"tRNS" => has_trns = true,
            _ => {}
        }
        if len > i32::MAX as u32 {
            return Err(err("chunk length out of range"));
        }
        p = p.checked_add(12).and_then(|q| q.checked_add(len as usize)).ok_or_else(|| err("truncated"))?;
    }
    Err(CodecError::LimitExceeded("PNG chunk count".into()))
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

    /// WordCraft's `png_chunks` (crates/geom/src/picture.rs) as the differential oracle.
    fn oracle(bytes: &[u8]) -> Option<(u32, u32, u64, bool)> {
        let be = |at: usize| bytes.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_be_bytes);
        if bytes.get(12..16) != Some(b"IHDR".as_slice()) {
            return None;
        }
        let (width, height) = (be(16)?, be(20)?);
        let (depth, color) = (*bytes.get(24)?, *bytes.get(25)?);
        let mut trns = false;
        let mut at = 8usize;
        loop {
            let len = usize::try_from(be(at)?).ok()?;
            match bytes.get(at.checked_add(4)?..at.checked_add(8)?)? {
                b"IDAT" => break,
                b"IEND" => return None,
                b"tRNS" => trns = true,
                _ => {}
            }
            at = at.checked_add(12)?.checked_add(len)?;
        }
        let (channels, alpha) = match color {
            0 => (1 + u64::from(trns), trns),
            2 | 3 => (3 + u64::from(trns), trns),
            4 => (2, true),
            6 => (4, true),
            _ => return None,
        };
        Some((width, height, channels * if depth == 16 { 2 } else { 1 }, alpha))
    }

    fn tuple(b: &[u8]) -> Option<(u32, u32, u64, bool)> {
        pixel_model(b).ok().map(|m| (m.width, m.height, u64::from(m.bytes_per_pixel()), m.has_alpha()))
    }

    fn png_with(depth: u8, color: u8, before_idat: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&5u32.to_be_bytes());
        ihdr.extend_from_slice(&7u32.to_be_bytes());
        ihdr.extend_from_slice(&[depth, color, 0, 0, 0]);
        chunk(&mut bytes, b"IHDR", &ihdr);
        for (k, d) in before_idat {
            chunk(&mut bytes, k, d);
        }
        chunk(&mut bytes, b"IDAT", &[1, 2, 3]);
        chunk(&mut bytes, b"IEND", &[]);
        bytes
    }

    #[test]
    fn pixel_model_matches_oracle_for_every_color_type_depth_and_trns() {
        let valid: [(u8, &[u8]); 5] = [(0, &[1, 2, 4, 8, 16]), (2, &[8, 16]), (3, &[1, 2, 4, 8]), (4, &[8, 16]), (6, &[8, 16])];
        for (color, depths) in valid {
            for &depth in depths {
                for trns in [false, true] {
                    let extra: Vec<(&[u8; 4], Vec<u8>)> = if trns { vec![(b"tRNS", vec![0, 0])] } else { vec![] };
                    let bytes = png_with(depth, color, &extra);
                    assert!(tuple(&bytes).is_some(), "color {color} depth {depth}");
                    assert_eq!(tuple(&bytes), oracle(&bytes), "color {color} depth {depth} trns {trns}");
                }
            }
        }
    }

    #[test]
    fn pixel_model_known_values() {
        let m = pixel_model(&png_with(16, 2, &[(b"tRNS", vec![0; 6])])).unwrap();
        assert_eq!((m.width, m.height, m.channels(), m.bytes_per_pixel(), m.has_alpha()), (5, 7, 4, 8, true));
        let m = pixel_model(&png_with(4, 3, &[])).unwrap();
        assert_eq!((m.bytes_per_pixel(), m.has_alpha()), (3, false));
    }

    #[test]
    fn pixel_model_skips_a_huge_iccp_without_inflating() {
        let junk = vec![0xAB; 20 << 20];
        let bytes = png_with(8, 2, &[(b"iCCP", junk), (b"tRNS", vec![0; 6])]);
        assert_eq!(tuple(&bytes), oracle(&bytes));
        assert_eq!(tuple(&bytes), Some((5, 7, 4, true)));
    }

    #[test]
    fn pixel_model_malformed_inputs_agree_with_oracle_and_never_panic() {
        let full = png_with(8, 6, &[(b"tRNS", vec![0]), (b"gAMA", vec![0; 4])]);
        for n in 0..=full.len() {
            let cut = &full[..n];
            let got = tuple(cut);
            // an Err where the oracle answers can only be a stricter header check
            if got.is_some() {
                assert_eq!(got, oracle(cut), "prefix {n}");
            }
        }
        assert!(pixel_model(&full[..33]).is_err());
        // missing IHDR
        let mut bad = full.clone();
        bad[12..16].copy_from_slice(b"IHDX");
        assert!(pixel_model(&bad).is_err() && oracle(&bad).is_none());
        // chunk length overflow: a length past the input, then one past i32::MAX
        for len in [u32::MAX, i32::MAX as u32, 0x7fff_fff0] {
            let mut o = full.clone();
            o[33..37].copy_from_slice(&len.to_be_bytes());
            assert!(pixel_model(&o).is_err(), "len {len:#x}");
            assert_eq!(tuple(&o), oracle(&o));
        }
        // IEND before IDAT, unknown colour type, zero size, bad signature
        let mut iend = png_with(8, 2, &[]);
        let at = iend.windows(4).position(|w| w == b"IDAT").unwrap();
        iend[at..at + 4].copy_from_slice(b"IEND");
        assert!(pixel_model(&iend).is_err() && oracle(&iend).is_none());
        assert!(pixel_model(&png_with(8, 5, &[])).is_err());
        let mut zero = png_with(8, 2, &[]);
        zero[16..20].copy_from_slice(&0u32.to_be_bytes());
        assert!(pixel_model(&zero).is_err());
        assert!(pixel_model(b"not a png").is_err());
        // deterministic pseudo-random corruption
        let mut s = 0x9e37_79b9u32;
        for _ in 0..2000 {
            let mut f = full.clone();
            for _ in 0..3 {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let i = (s >> 8) as usize % f.len();
                f[i] = (s >> 24) as u8;
            }
            let _ = pixel_model(&f);
        }
    }

    #[test]
    fn pixel_model_many_tiny_chunks_and_the_chunk_cap() {
        let many: Vec<(&[u8; 4], Vec<u8>)> = (0..10_000).map(|_| (b"tEXt", Vec::new())).chain([(b"tRNS", vec![0])]).collect();
        let bytes = png_with(8, 0, &many);
        assert_eq!(tuple(&bytes), oracle(&bytes));
        assert_eq!(tuple(&bytes), Some((5, 7, 2, true)));
        let over: Vec<(&[u8; 4], Vec<u8>)> = (0..MAX_PROBE_CHUNKS + 1).map(|_| (b"tEXt", Vec::new())).collect();
        assert!(matches!(pixel_model(&png_with(8, 0, &over)), Err(CodecError::LimitExceeded(_))));
    }
}
