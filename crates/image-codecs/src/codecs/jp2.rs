//! JPEG 2000 (JP2 container and raw J2K codestream) decoding via `hayro-jpeg2000`.
//!
//! Decodes within `Limits`, rejecting unsupported JPX multi-layer/composition boxes and
//! checking declared SIZ/tile/palette/channel-definition bounds before pixel allocation.

use hayro_jpeg2000::{ColorSpace, DecodeSettings, DecoderContext, Image as Jp2Image};

use crate::Format;
use crate::error::CodecError;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::Jp2;
const JP2_SIGNATURE: &[u8] = b"\0\0\0\x0cjP  \r\n\x87\n";
const MAX_PROFILE: usize = 4 * 1024 * 1024;
const MAX_TILES: u64 = 65_535;
const TILE_COMPONENT_BYTES: u64 = 256;

/// Boxes at one JP2 nesting level. Invalid lengths stop the walk at that level.
fn boxes(b: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let header = b.get(at..at.checked_add(8)?)?;
        let size = u32::from_be_bytes(header.get(..4)?.try_into().ok()?);
        let (length, head) = match size {
            0 => (b.len().checked_sub(at)?, 8usize),
            1 => (usize::try_from(u64::from_be_bytes(b.get(at.checked_add(8)?..at.checked_add(16)?)?.try_into().ok()?)).ok()?, 16usize),
            n => (n as usize, 8usize),
        };
        if length < head {
            return None;
        }
        let end = at.checked_add(length)?;
        let data = b.get(at.checked_add(head)?..end)?;
        let ty = header.get(4..8)?;
        at = end;
        Some((ty, data))
    })
}

fn unsupported_jpx_content(bytes: &[u8]) -> bool {
    fn association_has_content(data: &[u8], depth: usize) -> bool {
        if depth >= 16 {
            return true;
        }
        boxes(data).any(|(ty, child)| {
            matches!(ty, b"jp2c" | b"jpch" | b"jplh" | b"comp" | b"ftbl") || (ty == b"asoc" && association_has_content(child, depth + 1))
        })
    }
    let mut codestreams = 0u8;
    for (ty, data) in boxes(bytes) {
        match ty {
            b"jp2c" => {
                codestreams = codestreams.saturating_add(1);
                if codestreams > 1 {
                    return true;
                }
            }
            b"jpch" | b"jplh" | b"comp" | b"ftbl" => return true,
            b"asoc" if association_has_content(data, 0) => return true,
            _ => {}
        }
    }
    false
}

fn codestream_size(bytes: &[u8]) -> Option<(u64, u32, u32, u64)> {
    let stream = if bytes.starts_with(b"\xff\x4f") { bytes } else { boxes(bytes).find_map(|(ty, data)| (ty == b"jp2c").then_some(data))? };
    if stream.get(..4)? != b"\xff\x4f\xff\x51" {
        return None;
    }
    let length = u16::from_be_bytes(stream.get(4..6)?.try_into().ok()?) as usize;
    let count = u16::from_be_bytes(stream.get(40..42)?.try_into().ok()?) as usize;
    if count == 0 || length != 38usize.checked_add(count.checked_mul(3)?)? || stream.get(4..4usize.checked_add(length)?)?.len() != length {
        return None;
    }
    let be32 = |range: std::ops::Range<usize>| stream.get(range).and_then(|v| v.try_into().ok()).map(u32::from_be_bytes);
    let width = be32(8..12)?.checked_sub(be32(16..20)?)?;
    let height = be32(12..16)?.checked_sub(be32(20..24)?)?;
    let tiles = |grid: u32, offset: u32, tile: u32| match (grid.checked_sub(offset), tile) {
        (Some(span), 1..) => Some(u64::from(span).div_ceil(u64::from(tile))),
        _ => None,
    };
    let across = tiles(be32(8..12)?, be32(32..36)?, be32(24..28)?)?;
    let down = tiles(be32(12..16)?, be32(36..40)?, be32(28..32)?)?;
    Some((count as u64, width, height, across.checked_mul(down)?))
}

fn single_header_boxes(bytes: &[u8]) -> bool {
    let mut headers = boxes(bytes).filter(|(ty, _)| *ty == b"jp2h");
    let (Some((_, header)), None) = (headers.next(), headers.next()) else {
        return false;
    };
    [b"pclr", b"cmap", b"cdef"].iter().all(|kind| boxes(header).filter(|(ty, _)| ty == kind).count() <= 1)
}

fn channel_definitions(bytes: &[u8]) -> Option<u64> {
    if bytes.starts_with(b"\xff\x4f") {
        return None;
    }
    let header = boxes(bytes).find_map(|(ty, data)| (ty == b"jp2h").then_some(data))?;
    let cdef = boxes(header).find_map(|(ty, data)| (ty == b"cdef").then_some(data))?;
    let count = cdef.get(..2).and_then(|n| n.try_into().ok()).map_or(0, |n: [u8; 2]| usize::from(u16::from_be_bytes(n)));
    let Some(entries) = cdef.get(2..).filter(|rest| count > 0 && rest.len() == count * 6) else {
        return Some(0);
    };
    let mut defs: Vec<[u16; 3]> =
        entries.as_chunks::<6>().0.iter().map(|&[a, b, c, d, e, f]| [[a, b], [c, d], [e, f]].map(u16::from_be_bytes)).collect();
    defs.sort_unstable();
    let opacity = defs.last().is_some_and(|&[_, kind, association]| kind == 1 && association == 0);
    let colors = defs.len() - usize::from(opacity);
    let mut seen = vec![false; colors];
    let layout_ok = defs.iter().enumerate().all(|(i, &[channel, kind, association])| {
        usize::from(channel) == i
            && (i >= colors
                || (kind == 0
                    && usize::from(association).checked_sub(1).and_then(|slot| seen.get_mut(slot)).is_some_and(|s| !std::mem::replace(s, true))))
    });
    Some(if layout_ok { defs.len() as u64 } else { 0 })
}

fn palette_columns(bytes: &[u8], components: u64) -> Result<Option<u64>, ()> {
    if bytes.starts_with(b"\xff\x4f") {
        return Ok(None);
    }
    let Some(header) = boxes(bytes).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)) else {
        return Ok(None);
    };
    let Some(palette) = boxes(header).find_map(|(ty, data)| (ty == b"pclr").then_some(data)) else {
        return Ok(None);
    };
    let columns = palette.get(2).copied().ok_or(())?;
    let depths = palette.get(3..3 + usize::from(columns)).ok_or(())?;
    if columns == 0 || depths.iter().any(|&d| d & 0x80 != 0 || (d & 0x7f) + 1 > 31) {
        return Err(());
    }
    let Some(mapping) = boxes(header).find_map(|(ty, data)| (ty == b"cmap").then_some(data)) else {
        return Ok(Some(u64::from(columns)));
    };
    if mapping.len() != usize::from(columns) * 4 {
        return Err(());
    }
    for &[c0, c1, kind, column] in mapping.as_chunks::<4>().0 {
        let valid = u64::from(u16::from_be_bytes([c0, c1])) < components
            && match kind {
                0 => true,
                1 => column < columns,
                _ => false,
            };
        if !valid {
            return Err(());
        }
    }
    Ok(Some(u64::from(columns)))
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let malformed = |msg: &str| CodecError::malformed(F, msg);
    if bytes.starts_with(JP2_SIGNATURE) && unsupported_jpx_content(bytes) {
        return Err(CodecError::unsupported(F, "unsupported color space or JPX content"));
    }
    if bytes.starts_with(JP2_SIGNATURE) && !single_header_boxes(bytes) {
        return Err(malformed("invalid JP2 header boxes"));
    }
    let (codestream_channels, declared_w, declared_h, tile_count) = codestream_size(bytes).ok_or_else(|| malformed("invalid codestream SIZ"))?;
    if tile_count == 0 || tile_count > MAX_TILES {
        return Err(malformed("invalid tile count"));
    }
    let max_alloc = limits.max_alloc;
    let declared_pixels = u64::from(declared_w).checked_mul(u64::from(declared_h));
    if declared_w == 0
        || declared_h == 0
        || declared_w > limits.max_width
        || declared_h > limits.max_height
        || declared_pixels.is_none_or(|n| n > limits.max_pixels)
        || declared_pixels.and_then(|n| n.checked_mul(codestream_channels)).and_then(|n| n.checked_mul(4)).is_none_or(|n| n > max_alloc)
        || tile_count.checked_mul(codestream_channels).and_then(|n| n.checked_mul(TILE_COMPONENT_BYTES)).is_none_or(|n| n > max_alloc)
    {
        return Err(CodecError::LimitExceeded(format!("dimensions {declared_w}x{declared_h} exceed decode limits")));
    }
    let header = boxes(bytes).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)).unwrap_or_default();
    if boxes(header).any(|(ty, data)| ty == b"colr" && data.len() > MAX_PROFILE + 3) {
        return Err(CodecError::unsupported(F, "ICC profile exceeds 4 MiB limit"));
    }
    let palette = palette_columns(bytes, codestream_channels).map_err(|()| malformed("invalid JP2 palette"))?;
    let palette_rows = boxes(header).find_map(|(ty, data)| (ty == b"pclr").then(|| data.get(..2)).flatten());
    if let (Some(columns), Some(&[r0, r1])) = (palette, palette_rows)
        && u64::from(u16::from_be_bytes([r0, r1])) * (columns * 8 + 24) > max_alloc
    {
        return Err(CodecError::LimitExceeded("JP2 palette exceeds max_alloc".into()));
    }
    let image = Jp2Image::new(bytes, &DecodeSettings::default()).map_err(|e| malformed(&format!("{e:?}")))?;
    let (w, h) = (image.width(), image.height());
    let color = image.color_space();
    let (channels, icc) = match color {
        ColorSpace::Gray => (1u8, None),
        ColorSpace::RGB => (3u8, None),
        ColorSpace::CMYK => (4u8, None),
        ColorSpace::Icc { num_channels: ch @ (1 | 3 | 4), profile } if profile.len() <= MAX_PROFILE => (*ch, Some(profile.to_vec())),
        _ => return Err(CodecError::unsupported(F, "unsupported JPEG 2000 color space")),
    };
    let has_alpha = image.has_alpha();
    let stride = u64::from(channels) + u64::from(has_alpha);
    if palette.is_some_and(|columns| columns != stride) {
        return Err(malformed("JP2 palette columns do not match output channels"));
    }
    if channel_definitions(bytes).is_some_and(|count| count != stride || count != palette.unwrap_or(codestream_channels)) {
        return Err(malformed("JP2 channel definitions do not match output channels"));
    }
    let layout = match (channels, has_alpha) {
        (1, false) => ChannelLayout::Gray,
        (1, true) => ChannelLayout::GrayA,
        (3, false) => ChannelLayout::Rgb,
        (3, true) => ChannelLayout::Rgba,
        (4, false) => ChannelLayout::Cmyk,
        (4, true) => ChannelLayout::CmykA,
        _ => return Err(CodecError::unsupported(F, "unsupported channel count")),
    };
    limits.check(w, h, layout, SampleType::U8)?;
    let plane_channels = codestream_channels.max(stride).max(palette.unwrap_or(0));
    let pixels = u64::from(w).checked_mul(u64::from(h)).ok_or_else(|| CodecError::LimitExceeded("pixel count overflow".into()))?;
    let size = pixels.checked_mul(stride).ok_or_else(|| CodecError::LimitExceeded("sample count overflow".into()))?;
    let plane_bytes =
        pixels.checked_mul(plane_channels).and_then(|n| n.checked_mul(4)).ok_or_else(|| CodecError::LimitExceeded("plane overflow".into()))?;
    let scratch_bytes = plane_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(pixels.checked_mul(8)?))
        .ok_or_else(|| CodecError::LimitExceeded("scratch overflow".into()))?;
    let working = size
        .checked_add(plane_bytes)
        .and_then(|n| n.checked_add(scratch_bytes))
        .ok_or_else(|| CodecError::LimitExceeded("memory overflow".into()))?;
    if working > max_alloc {
        return Err(CodecError::LimitExceeded(format!("working set {working} exceeds max_alloc {max_alloc}")));
    }
    let len = usize::try_from(size).map_err(|_| CodecError::LimitExceeded("buffer does not fit in memory".into()))?;
    let mut samples = vec![0u8; len];
    image.decode_into(&mut samples, &mut DecoderContext::default()).map_err(|e| malformed(&format!("{e:?}")))?;
    let mut out = Image::from_raw(w, h, layout, SampleType::U8, samples)?;
    out.icc = icc;
    Ok(out)
}
