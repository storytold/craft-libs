use std::panic::{AssertUnwindSafe, catch_unwind};

use jxl_oxide::{AllocTracker, AuxBoxData, InitializeResult, JxlImage};

use crate::{DecodeLimits, Decoded, Error, Layout, Metadata, Result};

const INPUT_CHUNK: usize = 4096;
const TRACKED_BYTES_PER_PIXEL: u64 = 4 * 4 * 8;
const MIN_TRACKED_BYTES: u64 = 64 << 20;

/// Decode the first frame, keeping the decoder's tracked allocations within `limits`.
///
/// The allocator budget applies to allocations registered with `jxl-oxide`'s `AllocTracker`.
/// The decoder may also make untracked allocations, including the input buffer supplied by the
/// caller. Header data is fed in small chunks so the dimensions are checked before the rest of
/// the codestream is loaded.
pub fn decode(bytes: &[u8], limits: DecodeLimits) -> Result<Decoded> {
    catch_unwind(AssertUnwindSafe(|| decode_inner(bytes, limits)))
        .unwrap_or_else(|_| Err(Error::Malformed("decoder failed while reading this file".into())))
}

fn decode_inner(bytes: &[u8], limits: DecodeLimits) -> Result<Decoded> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::LimitExceeded(format!("{} input bytes exceed the {} byte input limit", bytes.len(), limits.max_input_bytes)));
    }
    let budget = limits.max_tracked_bytes.unwrap_or_else(|| limits.max_pixels.saturating_mul(TRACKED_BYTES_PER_PIXEL).max(MIN_TRACKED_BYTES));
    let tracker = AllocTracker::with_limit(usize::try_from(budget).unwrap_or(usize::MAX));
    let mut uninit = JxlImage::builder().alloc_tracker(tracker).build_uninit();
    let mut rest = bytes;
    let mut window = INPUT_CHUNK;
    let mut image = loop {
        let feed = rest.get(..window).unwrap_or(rest);
        let consumed = uninit.feed_bytes(feed).map_err(decode_error)?;
        rest = rest.get(consumed..).unwrap_or_default();
        match uninit.try_init().map_err(decode_error)? {
            InitializeResult::Initialized(image) => break image,
            InitializeResult::NeedMoreData(next) => uninit = next,
        }
        if consumed == 0 {
            if window >= rest.len() {
                return Err(Error::Malformed("truncated JPEG XL header".into()));
            }
            window = window.saturating_mul(2);
        }
    };

    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err(Error::Malformed("zero image dimension".into()));
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels > limits.max_pixels {
        return Err(Error::LimitExceeded(format!("{width}×{height} image exceeds the {} pixel limit", limits.max_pixels)));
    }

    // `JxlImageBuilder::read` feeds the full input. Continue in bounded chunks so trailing frame
    // data and boxes are not copied into an untracked buffer all at once.
    while !image.is_loading_done() && !rest.is_empty() {
        let consumed = image.feed_bytes(rest.get(..INPUT_CHUNK).unwrap_or(rest)).map_err(decode_error)?;
        if consumed == 0 {
            break;
        }
        rest = rest.get(consumed..).unwrap_or_default();
    }
    image.finalize().map_err(decode_error)?;

    let header = image.image_header();
    let bit_depth = header.metadata.bit_depth.bits_per_sample() as u8;
    let float_samples = matches!(header.metadata.bit_depth, jxl_oxide::image::BitDepth::FloatSample { .. });
    let grayscale = header.metadata.grayscale();
    let colors = if grayscale { 1 } else { 3 };
    let pixel_format = image.pixel_format();
    if pixel_format.has_black() {
        return Err(Error::Unsupported("CMYK JPEG XL rendering requires an external color-management transform".into()));
    }
    let has_alpha = pixel_format.has_alpha();
    let layout = match (grayscale, has_alpha) {
        (true, false) => Layout::Gray,
        (true, true) => Layout::GrayAlpha,
        (false, false) => Layout::Rgb,
        (false, true) => Layout::Rgba,
    };
    let keep = colors + usize::from(has_alpha);
    let frame = image.render_frame(0).map_err(decode_error)?;
    let buffer = frame.image_all_channels();
    let channels = buffer.channels();
    if channels < colors || (has_alpha && channels < keep) {
        return Err(Error::Malformed("decoded frame has too few channels".into()));
    }
    let frame_pixels = (buffer.width() as u128) * (buffer.height() as u128);
    if frame_pixels != u128::from(pixels) {
        return Err(Error::Malformed("decoded frame dimensions do not match the image header".into()));
    }
    let sample_count = usize::try_from(frame_pixels.checked_mul(keep as u128).ok_or_else(|| Error::LimitExceeded("sample count overflows".into()))?)
        .map_err(|_| Error::LimitExceeded("sample count exceeds addressable memory".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(sample_count).map_err(|e| Error::LimitExceeded(format!("cannot reserve decoded pixels: {e}")))?;
    for pixel in buffer.buf().chunks_exact(channels) {
        out.extend_from_slice(&pixel[..keep]);
    }

    let metadata = Metadata {
        icc: image.original_icc().map(ToOwned::to_owned),
        exif: match image.aux_boxes().first_exif() {
            Ok(AuxBoxData::Data(exif)) => Some(exif.payload().to_vec()),
            _ => None,
        },
        xmp: match image.aux_boxes().first_xml() {
            AuxBoxData::Data(xmp) => Some(xmp.to_vec()),
            _ => None,
        },
    };
    Ok(Decoded {
        width,
        height,
        layout,
        bit_depth,
        float_samples,
        pixels: out,
        metadata,
        source_orientation: header.metadata.orientation,
        rendered_cicp: image.rendered_cicp(),
        rendered_icc: image.rendered_icc(),
        xyb_encoded: header.metadata.xyb_encoded,
    })
}

fn decode_error(error: impl std::fmt::Display) -> Error {
    Error::Malformed(error.to_string())
}
