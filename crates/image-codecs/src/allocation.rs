//! Predecode bounds for raster containers whose frames may exceed their declared canvas.

use crate::error::CodecError;
use crate::format::Format;

/// The largest width and height any decoded frame can declare, without allocating pixel data.
/// Animated GIF and WebP frames are inspected individually, including frames larger than the
/// animation canvas. Other recognized raster formats use their container's image dimensions.
pub fn probe_allocation_size(bytes: &[u8]) -> Result<(u32, u32), CodecError> {
    let format = crate::detect(bytes).ok_or(CodecError::UnknownFormat)?;
    let (width, height) = match format {
        Format::Png => png_bounds(bytes)?,
        Format::Jpeg => jpeg_bounds(bytes)?,
        Format::Gif => gif_bounds(bytes)?,
        Format::WebP => webp_bounds(bytes)?,
        _ => {
            if let Some(info) = crate::inspect_raster(bytes)? {
                (info.width, info.height)
            } else {
                crate::probe_size(bytes)?
            }
        }
    };
    // This is an allocation-size probe, not a decode policy. Callers need the coded extents even
    // when they exceed this crate's default decode limits so they can report/refuse against their
    // own budgets before passing the bytes to a decoder.
    Ok((width, height))
}

fn malformed(format: Format, message: &'static str) -> CodecError {
    CodecError::malformed(format, message)
}

fn png_bounds(bytes: &[u8]) -> Result<(u32, u32), CodecError> {
    let bad = || malformed(Format::Png, "truncated or malformed PNG header");
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") || bytes.get(8..16) != Some(b"\0\0\0\rIHDR") {
        return Err(bad());
    }
    let width = u32::from_be_bytes(bytes.get(16..20).ok_or_else(bad)?.try_into().map_err(|_| bad())?);
    let height = u32::from_be_bytes(bytes.get(20..24).ok_or_else(bad)?.try_into().map_err(|_| bad())?);
    if width == 0 || height == 0 {
        return Err(bad());
    }
    Ok((width, height))
}

fn jpeg_bounds(bytes: &[u8]) -> Result<(u32, u32), CodecError> {
    let bad = || malformed(Format::Jpeg, "truncated or malformed JPEG frame header");
    if bytes.get(..2) != Some(&[0xff, 0xd8]) {
        return Err(bad());
    }
    let mut at = 2usize;
    while at < bytes.len() {
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at).ok_or_else(bad)?;
        at += 1;
        if matches!(marker, 0xd8 | 0xd9 | 0x01 | 0xd0..=0xd7) {
            continue;
        }
        let length =
            usize::from(u16::from_be_bytes(bytes.get(at..at.checked_add(2).ok_or_else(bad)?).ok_or_else(bad)?.try_into().map_err(|_| bad())?));
        if length < 2 {
            return Err(bad());
        }
        let end = at.checked_add(length).ok_or_else(bad)?;
        let segment = bytes.get(at..end).ok_or_else(bad)?;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if segment.len() < 7 {
                return Err(bad());
            }
            let height = u32::from(u16::from_be_bytes([segment[3], segment[4]]));
            let width = u32::from(u16::from_be_bytes([segment[5], segment[6]]));
            if width == 0 || height == 0 {
                return Err(bad());
            }
            return Ok((width, height));
        }
        at = end;
    }
    Err(bad())
}

fn u16_le(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn gif_bounds(bytes: &[u8]) -> Result<(u32, u32), CodecError> {
    let bad = || malformed(Format::Gif, "truncated or malformed GIF frame table");
    if !bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a") {
        return Err(bad());
    }
    let packed = *bytes.get(10).ok_or_else(bad)?;
    let (mut width, mut height) = (u32::from(u16_le(bytes, 6).ok_or_else(bad)?), u32::from(u16_le(bytes, 8).ok_or_else(bad)?));
    let table_len =
        if packed & 0x80 != 0 { 3usize.checked_mul(1usize.checked_shl(u32::from((packed & 7) + 1)).ok_or_else(bad)?).ok_or_else(bad)? } else { 0 };
    let mut at = 13usize.checked_add(table_len).ok_or_else(bad)?;
    if bytes.get(..at).is_none() || width == 0 || height == 0 {
        return Err(bad());
    }
    let mut frames = 0usize;
    loop {
        match bytes.get(at).copied().ok_or_else(bad)? {
            0x3b => break,
            0x21 => {
                at = at.checked_add(2).ok_or_else(bad)?;
                loop {
                    let length = usize::from(*bytes.get(at).ok_or_else(bad)?);
                    at = at.checked_add(1).ok_or_else(bad)?;
                    if length == 0 {
                        break;
                    }
                    at = at.checked_add(length).ok_or_else(bad)?;
                    if bytes.get(..at).is_none() {
                        return Err(bad());
                    }
                }
            }
            0x2c => {
                let left = u32::from(u16_le(bytes, at + 1).ok_or_else(bad)?);
                let top = u32::from(u16_le(bytes, at + 3).ok_or_else(bad)?);
                let frame_width = u32::from(u16_le(bytes, at + 5).ok_or_else(bad)?);
                let frame_height = u32::from(u16_le(bytes, at + 7).ok_or_else(bad)?);
                let descriptor = bytes.get(at..at.checked_add(10).ok_or_else(bad)?).ok_or_else(bad)?;
                let flags = descriptor[9];
                // The frame rectangle determines its decoded allocation even when the later
                // compressed payload is truncated. Report known dimensions so callers can
                // refuse an oversized frame before asking a decoder to inspect its pixels.
                width = width.max(left.saturating_add(frame_width));
                height = height.max(top.saturating_add(frame_height));
                let mut body = at.checked_add(10).ok_or_else(bad)?;
                if flags & 0x80 != 0 {
                    let table = 3usize.checked_mul(1usize.checked_shl(u32::from((flags & 7) + 1)).ok_or_else(bad)?).ok_or_else(bad)?;
                    body = body.checked_add(table).ok_or_else(bad)?;
                }
                body = body.checked_add(1).ok_or_else(bad)?; // LZW minimum code size.
                if frame_width == 0 || frame_height == 0 {
                    return Err(bad());
                }
                if bytes.get(..body).is_none() {
                    return Ok((width, height));
                }
                at = body;
                frames = frames.saturating_add(1);
                loop {
                    let Some(&length) = bytes.get(at) else { return Ok((width, height)) };
                    let length = usize::from(length);
                    at = at.checked_add(1).ok_or_else(bad)?;
                    if length == 0 {
                        break;
                    }
                    at = at.checked_add(length).ok_or_else(bad)?;
                    if bytes.get(..at).is_none() {
                        return Err(bad());
                    }
                }
            }
            _ => return Err(bad()),
        }
    }
    if frames == 0 {
        return Err(bad());
    }
    Ok((width, height))
}

fn webp_bounds(bytes: &[u8]) -> Result<(u32, u32), CodecError> {
    let bad = || malformed(Format::WebP, "truncated or malformed WebP frame table");
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return Err(bad());
    }
    let riff_len = usize::try_from(u32::from_le_bytes(bytes.get(4..8).ok_or_else(bad)?.try_into().map_err(|_| bad())?)).map_err(|_| bad())?;
    let end = 8usize.checked_add(riff_len).ok_or_else(bad)?;
    if end > bytes.len() || end < 12 {
        return Err(bad());
    }
    let mut max_width = 0u32;
    let mut max_height = 0u32;
    webp_chunks(bytes, 12, end, false, &mut max_width, &mut max_height)?;
    if max_width == 0 || max_height == 0 {
        return Err(bad());
    }
    Ok((max_width, max_height))
}

fn webp_chunks(bytes: &[u8], mut at: usize, end: usize, nested: bool, max_width: &mut u32, max_height: &mut u32) -> Result<(), CodecError> {
    let bad = || malformed(Format::WebP, "truncated or malformed WebP frame table");
    while at < end {
        let header_end = at.checked_add(8).ok_or_else(bad)?;
        let header = bytes.get(at..header_end).filter(|_| header_end <= end).ok_or_else(bad)?;
        let size = usize::try_from(u32::from_le_bytes(header.get(4..8).ok_or_else(bad)?.try_into().map_err(|_| bad())?)).map_err(|_| bad())?;
        let from = header_end;
        let to = from.checked_add(size).ok_or_else(bad)?;
        let payload = bytes.get(from..to).filter(|_| to <= end).ok_or_else(bad)?;
        match header.get(..4).ok_or_else(bad)? {
            b"VP8 " => {
                if payload.get(3..6) != Some(&[0x9d, 0x01, 0x2a]) || payload.len() < 10 {
                    return Err(bad());
                }
                let w = u32::from(u16::from_le_bytes([payload[6], payload[7]]) & 0x3fff);
                let h = u32::from(u16::from_le_bytes([payload[8], payload[9]]) & 0x3fff);
                *max_width = (*max_width).max(w);
                *max_height = (*max_height).max(h);
            }
            b"VP8L" => {
                if payload.len() < 5 || payload[0] != 0x2f {
                    return Err(bad());
                }
                let bits = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
                *max_width = (*max_width).max((bits & 0x3fff) + 1);
                *max_height = (*max_height).max(((bits >> 14) & 0x3fff) + 1);
            }
            b"VP8X" => {
                if nested || payload.len() < 10 {
                    return Err(bad());
                }
                *max_width = (*max_width).max(read_le24(payload, 4).ok_or_else(bad)?.saturating_add(1));
                *max_height = (*max_height).max(read_le24(payload, 7).ok_or_else(bad)?.saturating_add(1));
            }
            b"ANMF" => {
                if nested || payload.len() < 16 {
                    return Err(bad());
                }
                let x = read_le24(payload, 0).ok_or_else(bad)?.saturating_mul(2);
                let y = read_le24(payload, 3).ok_or_else(bad)?.saturating_mul(2);
                let w = read_le24(payload, 6).ok_or_else(bad)?.saturating_add(1);
                let h = read_le24(payload, 9).ok_or_else(bad)?.saturating_add(1);
                *max_width = (*max_width).max(x.saturating_add(w));
                *max_height = (*max_height).max(y.saturating_add(h));
                webp_chunks(payload, 16, payload.len(), true, max_width, max_height)?;
            }
            _ => {}
        }
        at = to.checked_add(size & 1).ok_or_else(bad)?;
        if at > end {
            return Err(bad());
        }
    }
    if at != end {
        return Err(bad());
    }
    Ok(())
}

fn read_le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(3)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], 0]))
}
