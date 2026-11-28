//! JPEG 2000 (JP2 container and raw J2K codestream): decoding via `hayro-jpeg2000`, with samples
//! deeper than 8 bits decoded by `j2k`, and lossless encoding via `j2k`.
//!
//! Decodes within `Limits`, rejecting unsupported JPX multi-layer/composition boxes and
//! checking declared SIZ/tile/palette/channel-definition bounds before pixel allocation.

use hayro_jpeg2000::{ColorSpace, DecodeSettings, DecoderContext, Image as Jp2Image};
use j2k::{
    J2kChannelAssociation, J2kChannelDefinition, J2kChannelType, J2kDecoder, J2kFileBoxMetadata, J2kFileColorSpec, J2kFileWrapOptions,
    J2kLosslessEncodeOptions, J2kLosslessSamples, encode_j2k_lossless, wrap_j2k_codestream,
};
use j2k_core::Colorspace;

use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::{EncodeOptions, Limits};

const F: Format = Format::Jp2;
const JP2_SIGNATURE: &[u8] = b"\0\0\0\x0cjP  \r\n\x87\n";
const MAX_PROFILE: usize = 4 * 1024 * 1024;
// Method, precedence and approximation bytes before a colr box's profile
const COLR_HEADER_BYTES: usize = 3;
const MAX_TILES: u64 = 65_535;
const TILE_COMPONENT_BYTES: u64 = 256;
// hayro-jpeg2000 decodes each component to an f32 plane
const PLANE_SAMPLE_BYTES: u64 = 4;
// Decoder scratch: two more plane-sized buffers and 8 bytes per pixel
const SCRATCH_PLANE_COPIES: u64 = 2;
const SCRATCH_PIXEL_BYTES: u64 = 8;
// hayro-jpeg2000 holds each palette row as a `Vec<u64>`: a 24-byte header and 8 bytes per column
const PALETTE_ROW_BYTES: u64 = 24;
const PALETTE_ENTRY_BYTES: u64 = 8;

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

/// The codestream of a JP2 file, or `bytes` when they are one.
fn codestream(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.starts_with(b"\xff\x4f") { Some(bytes) } else { boxes(bytes).find_map(|(ty, data)| (ty == b"jp2c").then_some(data)) }
}

fn codestream_size(bytes: &[u8]) -> Option<(u64, u32, u32, u64)> {
    let stream = codestream(bytes)?;
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
    let declared_plane_bytes = declared_pixels.and_then(|n| n.checked_mul(codestream_channels)).and_then(|n| n.checked_mul(PLANE_SAMPLE_BYTES));
    if declared_w == 0
        || declared_h == 0
        || declared_w > limits.max_width
        || declared_h > limits.max_height
        || declared_pixels.is_none_or(|n| n > limits.max_pixels)
        || declared_plane_bytes.is_none_or(|n| n > max_alloc)
        || tile_count.checked_mul(codestream_channels).and_then(|n| n.checked_mul(TILE_COMPONENT_BYTES)).is_none_or(|n| n > max_alloc)
    {
        return Err(CodecError::LimitExceeded(format!("dimensions {declared_w}x{declared_h} exceed decode limits")));
    }

    let header = boxes(bytes).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)).unwrap_or_default();
    if boxes(header).any(|(ty, data)| ty == b"colr" && data.len() > MAX_PROFILE + COLR_HEADER_BYTES) {
        return Err(CodecError::unsupported(F, "ICC profile exceeds 4 MiB limit"));
    }
    let palette = palette_columns(bytes, codestream_channels).map_err(|()| malformed("invalid JP2 palette"))?;
    let palette_rows = boxes(header).find_map(|(ty, data)| (ty == b"pclr").then(|| data.get(..2)).flatten());
    if let (Some(columns), Some(&[r0, r1])) = (palette, palette_rows)
        && u64::from(u16::from_be_bytes([r0, r1])) * (columns * PALETTE_ENTRY_BYTES + PALETTE_ROW_BYTES) > max_alloc
    {
        return Err(CodecError::LimitExceeded("JP2 palette exceeds max_alloc".into()));
    }

    let image = Jp2Image::new(bytes, &DecodeSettings::default()).map_err(|e| malformed(&format!("{e:?}")))?;
    let (w, h) = (image.width(), image.height());
    let (channels, icc) = match image.color_space() {
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

    // hayro-jpeg2000 returns 8-bit samples; deeper ones go through j2k when it yields the same channels
    let colors = match layout {
        ChannelLayout::Gray | ChannelLayout::GrayA => Some(1),
        ChannelLayout::Rgb | ChannelLayout::Rgba => Some(3),
        _ => None,
    };
    let deep = image.original_bit_depth() > 8 && colors.is_some_and(|colors| j2k_reads_as_hayro(bytes, colors, has_alpha));
    let plane_channels = codestream_channels.max(stride).max(palette.unwrap_or(0));
    let fit = |sample| sample_count_within_limits(limits, (w, h), layout, plane_channels, sample);
    // A deep image that fits max_alloc only at 8 bits is read at 8 rather than refused
    let (sample, len) = match deep.then(|| fit(SampleType::U16)) {
        Some(Ok(len)) => (SampleType::U16, len),
        _ => (SampleType::U8, fit(SampleType::U8)?),
    };

    let deep_samples = if sample == SampleType::U16 { decode_u16(bytes, (w, h), len)? } else { None };
    let mut out = match deep_samples {
        Some(samples) => Image::from_raw(w, h, layout, SampleType::U16, samples)?,
        None => {
            let mut samples = crate::options::alloc_zeroed::<u8>(len)?;
            image.decode_into(&mut samples, &mut DecoderContext::default()).map_err(|e| malformed(&format!("{e:?}")))?;
            Image::from_raw(w, h, layout, SampleType::U8, samples)?
        }
    };
    out.icc = icc;
    Ok(out)
}

/// Whether j2k's planes, interleaved in codestream order, are the channels hayro-jpeg2000 reads from `bytes`, only deeper.
fn j2k_reads_as_hayro(bytes: &[u8], colors: u16, has_alpha: bool) -> bool {
    // SIZ, already checked whole: each component's signedness and depth, then its subsampling
    let Some(stream) = codestream(bytes) else {
        return false;
    };

    let count = stream.get(40..42).and_then(|n| n.try_into().ok()).map_or(0, |n| usize::from(u16::from_be_bytes(n)));
    let sizes = stream.get(42..42 + 3 * count).unwrap_or_default();
    if sizes.is_empty() || !sizes.as_chunks::<3>().0.iter().all(|&[depth, dx, dy]| depth & 0x80 == 0 && dx == 1 && dy == 1) {
        return false;
    }
    if bytes.starts_with(b"\xff\x4f") {
        return true;
    }

    let Some(header) = boxes(bytes).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)) else {
        return false;
    };

    // Channel definitions, when present, must name the colours in order and then opacity for the whole image
    let components = colors + u16::from(has_alpha);
    let mut definitions = components.to_be_bytes().to_vec();
    for channel in 0..components {
        let (kind, association) = if channel < colors { (0u16, channel + 1) } else { (1, 0) };
        definitions.extend([channel, kind, association].iter().flat_map(|v| v.to_be_bytes()));
    }

    // One colour box, enumerated sRGB or grayscale or an ICC profile; a palette or any other box reads at 8 bits
    let enumerated: u32 = if colors == 3 { 16 } else { 17 };
    let mut colour_boxes = 0;
    let known = boxes(header).all(|(ty, data)| match ty {
        b"ihdr" | b"res " => true,
        b"colr" => {
            colour_boxes += 1;
            matches!(data, [1, _, _, space @ ..] if *space == enumerated.to_be_bytes()) || data.first() == Some(&2)
        }
        b"cdef" => data == definitions,
        _ => false,
    });
    known && colour_boxes == 1
}

/// The interleaved sample count of a `width`×`height` decode to `sample`, once it and the
/// decoder's planes and scratch fit `limits`.
fn sample_count_within_limits(
    limits: &Limits,
    (width, height): (u32, u32),
    layout: ChannelLayout,
    plane_channels: u64,
    sample: SampleType,
) -> Result<usize, CodecError> {
    limits.check(width, height, layout, sample)?;

    let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or_else(|| CodecError::LimitExceeded("pixel count overflow".into()))?;
    let size = pixels.checked_mul(layout.channels() as u64).ok_or_else(|| CodecError::LimitExceeded("sample count overflow".into()))?;
    let plane_bytes = pixels
        .checked_mul(plane_channels)
        .and_then(|n| n.checked_mul(PLANE_SAMPLE_BYTES))
        .ok_or_else(|| CodecError::LimitExceeded("plane overflow".into()))?;
    let scratch_bytes = plane_bytes
        .checked_mul(SCRATCH_PLANE_COPIES)
        .and_then(|n| n.checked_add(pixels.checked_mul(SCRATCH_PIXEL_BYTES)?))
        .ok_or_else(|| CodecError::LimitExceeded("scratch overflow".into()))?;
    let working = size
        .checked_mul(sample.bytes() as u64)
        .and_then(|n| n.checked_add(plane_bytes))
        .and_then(|n| n.checked_add(scratch_bytes))
        .ok_or_else(|| CodecError::LimitExceeded("memory overflow".into()))?;
    if working > limits.max_alloc {
        return Err(CodecError::LimitExceeded(format!("working set {working} exceeds max_alloc {}", limits.max_alloc)));
    }

    usize::try_from(size).map_err(|_| CodecError::LimitExceeded("buffer does not fit in memory".into()))
}

/// The `len` native-endian 16-bit samples of equal 9 to 16-bit unsigned full-size components, widened; `None` reads at 8 bits.
fn decode_u16(bytes: &[u8], (width, height): (u32, u32), len: usize) -> Result<Option<Vec<u8>>, CodecError> {
    let Ok(decoded) = J2kDecoder::new(bytes).and_then(|mut decoder| decoder.decode_native_components()) else {
        return Ok(None);
    };

    let planes = decoded.planes();
    let Some(first) = planes.first() else {
        return Ok(None);
    };

    let depth = first.bit_depth();
    let pixels = width as usize * height as usize;
    let usable = |plane: &j2k::J2kNativeComponentPlane| {
        plane.bit_depth() == depth && !plane.signed() && plane.dimensions() == (width, height) && plane.data().len() == pixels * size_of::<u16>()
    };
    if !(9..=16).contains(&depth) || planes.len() * pixels != len || !planes.iter().all(usable) {
        return Ok(None);
    }

    let max = (1u32 << depth) - 1;
    let full_range = u32::from(u16::MAX);
    let byte_len = len.checked_mul(size_of::<u16>()).ok_or_else(|| CodecError::LimitExceeded("buffer does not fit in memory".into()))?;
    let mut samples = crate::options::alloc_zeroed::<u8>(byte_len)?;
    for (index, plane) in planes.iter().enumerate() {
        let values = plane.data().as_chunks::<2>().0.iter().map(|&pair| u32::from(u16::from_le_bytes(pair)).min(max));
        for (sample, value) in samples.as_chunks_mut::<2>().0.iter_mut().skip(index).step_by(planes.len()).zip(values) {
            *sample = (((value * full_range + max / 2) / max) as u16).to_ne_bytes();
        }
    }
    Ok(Some(samples))
}

/// Encode `src` losslessly (the reversible 5/3 wavelet) at 8 or 16 bits, as gray or RGB with or
/// without alpha. A JP2 file records the image's ICC profile when `opts.embed_icc` is set and
/// one is attached, otherwise sRGB or grayscale, and marks alpha as opacity for the whole image.
/// With `opts.jp2_codestream` the raw codestream is returned, with no colour information.
pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let image = src.converted(plan.layout, plan.sample);
    let (color_count, has_alpha) = match image.layout() {
        ChannelLayout::Gray => (1u16, false),
        ChannelLayout::GrayA => (1, true),
        ChannelLayout::Rgb => (3, false),
        ChannelLayout::Rgba => (3, true),
        layout => return Err(CodecError::unsupported(F, format!("JPEG 2000 is written as gray or RGB, not {layout:?}"))),
    };
    let components = color_count + u16::from(has_alpha);

    // j2k takes 16-bit samples little-endian, which native ones already are on most targets
    let (bit_depth, data) = match image.sample_type() {
        SampleType::U8 => (8, std::borrow::Cow::Borrowed(image.data())),
        SampleType::U16 if cfg!(target_endian = "little") => (16, std::borrow::Cow::Borrowed(image.data())),
        SampleType::U16 => {
            (16, std::borrow::Cow::Owned(image.data().as_chunks::<2>().0.iter().flat_map(|&pair| u16::from_ne_bytes(pair).to_le_bytes()).collect()))
        }
        sample => return Err(CodecError::unsupported(F, format!("JPEG 2000 is written with 8 or 16 bits per sample, not {sample:?}"))),
    };
    let (width, height) = image.dimensions();
    let samples = J2kLosslessSamples { data: &data, width, height, components, bit_depth, signed: false };
    let codestream = encode_j2k_lossless(samples, &J2kLosslessEncodeOptions::default()).map_err(|e| CodecError::encode(F, e))?.codestream;
    // Free the little-endian copy before the JP2 wrap allocates
    drop(data);
    if opts.jp2_codestream {
        return Ok(codestream);
    }

    let mut wrap = J2kFileWrapOptions::jp2();
    wrap.color = match image.icc.as_deref().filter(|_| opts.embed_icc) {
        Some(profile) => J2kFileColorSpec::IccProfile(profile),
        None if color_count == 3 => J2kFileColorSpec::Enumerated(Colorspace::SRgb),
        None => J2kFileColorSpec::Enumerated(Colorspace::Grayscale),
    };
    let definitions: Vec<J2kChannelDefinition>;
    if has_alpha {
        let colors = (0..color_count).map(|channel_index| J2kChannelDefinition {
            channel_index,
            channel_type: J2kChannelType::Color,
            association: J2kChannelAssociation::Color { index: channel_index + 1 },
        });
        let opacity = J2kChannelDefinition {
            channel_index: color_count,
            channel_type: J2kChannelType::Opacity,
            association: J2kChannelAssociation::WholeImage,
        };
        definitions = colors.chain([opacity]).collect();
        wrap.metadata = J2kFileBoxMetadata { palette: None, component_mappings: &[], channel_definitions: &definitions };
    }

    wrap_j2k_codestream(&codestream, wrap).map_err(|e| CodecError::encode(F, e))
}
