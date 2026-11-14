use std::panic::{AssertUnwindSafe, catch_unwind};

use jxl_oxide::color::{ColourEncoding, ColourSpace, EnumColourEncoding, Primaries, RenderingIntent, TransferFunction, WhitePoint};
use jxl_oxide::{AllocTracker, AuxBoxData, ExtraChannelType, InitializeResult, JxlImage};

use crate::{DecodeLimits, Decoded, Error, Layout, Metadata, OutputColor, Result};

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
    decode_with_color(bytes, limits, OutputColor::DecoderDefault)
}

/// Decode the first image frame using an explicit rendered-pixel color target.
pub fn decode_with_color(bytes: &[u8], limits: DecodeLimits, output_color: OutputColor) -> Result<Decoded> {
    catch_unwind(AssertUnwindSafe(|| decode_inner(bytes, limits, output_color)))
        .unwrap_or_else(|_| Err(Error::Malformed("decoder failed while reading this file".into())))
}

fn decode_inner(bytes: &[u8], limits: DecodeLimits, output_color: OutputColor) -> Result<Decoded> {
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
        return Err(Error::PixelLimitExceeded { width, height, max_pixels: limits.max_pixels });
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

    let (bit_depth, float_samples, grayscale, source_orientation, xyb_encoded, extra_channels, colour_encoding) = {
        let metadata = &image.image_header().metadata;
        (
            metadata.bit_depth.bits_per_sample() as u8,
            matches!(metadata.bit_depth, jxl_oxide::image::BitDepth::FloatSample { .. }),
            metadata.grayscale(),
            metadata.orientation,
            metadata.xyb_encoded,
            metadata.ec_info.iter().map(|info| info.ty).collect::<Vec<_>>(),
            metadata.colour_encoding.clone(),
        )
    };
    let source_icc = image.original_icc().map(ToOwned::to_owned);
    let has_source_icc = source_icc.is_some();
    if has_black_channel(&extra_channels) || image.pixel_format().has_black() {
        return Err(Error::Unsupported("CMYK JPEG XL rendering requires an external color-management transform".into()));
    }
    let alpha_channel = first_alpha(&extra_channels);
    let alpha_associated = alpha_channel.is_some_and(|(_, associated)| associated);
    let via_srgb = output_color == OutputColor::LinearRec2020 && source_icc.is_none() && !grayscale && xyb_encoded;
    // Associated samples must be divided while still in their coded nonlinear space. Leave these
    // frames in their own encoding so the caller can apply its color transform after unpremultiplying.
    // XYB and unsupported transfer curves stay on the renderer's linear target path; those cases
    // are premultiplied after decoding instead.
    let enum_source_has_supported_curve = matches!(
        &colour_encoding,
        ColourEncoding::Enum(encoding) if enum_encoding_has_supported_curve(encoding)
    );
    let preserve_source_encoding =
        output_color == OutputColor::LinearRec2020 && alpha_associated && !xyb_encoded && (source_icc.is_some() || enum_source_has_supported_curve);
    if output_color == OutputColor::LinearRec2020 && source_icc.is_none() && !preserve_source_encoding {
        image.request_color_encoding(EnumColourEncoding {
            colour_space: if grayscale { ColourSpace::Grey } else { ColourSpace::Rgb },
            white_point: WhitePoint::D65,
            primaries: if via_srgb { Primaries::Srgb } else { Primaries::Bt2100 },
            tf: TransferFunction::Linear,
            rendering_intent: RenderingIntent::Relative,
        });
    }
    let colors = if grayscale { 1 } else { 3 };
    let pixel_format = image.pixel_format();
    if pixel_format.has_black() {
        return Err(Error::Unsupported("CMYK JPEG XL rendering requires an external color-management transform".into()));
    }
    let has_alpha = alpha_channel.is_some();
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
    let color_channels = channels
        .checked_sub(extra_channels.len())
        .filter(|&count| count >= colors)
        .ok_or_else(|| Error::Malformed("decoded frame has too few color channels".into()))?;
    let alpha_index = alpha_channel.map(|(index, _)| color_channels + index);
    let (rendered_width, rendered_height) = (
        u32::try_from(buffer.width()).map_err(|_| Error::LimitExceeded("rendered width exceeds 32-bit dimensions".into()))?,
        u32::try_from(buffer.height()).map_err(|_| Error::LimitExceeded("rendered height exceeds 32-bit dimensions".into()))?,
    );
    let frame_pixels = u128::from(rendered_width) * u128::from(rendered_height);
    if frame_pixels != u128::from(pixels) {
        return Err(Error::Malformed("decoded frame dimensions do not match the image header".into()));
    }
    let sample_count = usize::try_from(frame_pixels.checked_mul(keep as u128).ok_or_else(|| Error::LimitExceeded("sample count overflows".into()))?)
        .map_err(|_| Error::LimitExceeded("sample count exceeds addressable memory".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(sample_count).map_err(|e| Error::LimitExceeded(format!("cannot reserve decoded pixels: {e}")))?;
    for pixel in buffer.buf().chunks_exact(channels) {
        let start = out.len();
        out.extend_from_slice(&pixel[..colors]);
        if let Some(index) = alpha_index {
            let alpha_value = pixel.get(index).copied().ok_or_else(|| Error::Malformed("decoded frame is missing its alpha channel".into()))?;
            if preserve_source_encoding && alpha_associated {
                let divisor = alpha_value.clamp(0.0, 1.0);
                if divisor > 0.0 {
                    for channel in &mut out[start..] {
                        *channel /= divisor;
                    }
                }
            }
            out.push(alpha_value);
        }
        if via_srgb {
            let [r, g, b] = [out[start], out[start + 1], out[start + 2]];
            out[start] = 0.627_403_9 * r + 0.329_283_04 * g + 0.043_313_067 * b;
            out[start + 1] = 0.069_097_29 * r + 0.919_540_4 * g + 0.011_362_316 * b;
            out[start + 2] = 0.016_391_44 * r + 0.088_013_31 * g + 0.895_595_25 * b;
        }
    }

    let metadata = Metadata {
        icc: source_icc,
        exif: match image.aux_boxes().first_exif() {
            Ok(AuxBoxData::Data(exif)) => {
                usize::try_from(exif.tiff_header_offset()).ok().and_then(|offset| exif.payload().get(offset..)).map(ToOwned::to_owned)
            }
            _ => None,
        },
        xmp: match image.aux_boxes().first_xml() {
            AuxBoxData::Data(xmp) => Some(xmp.to_vec()),
            _ => None,
        },
    };
    let pixels_linear_rec2020 = output_color == OutputColor::LinearRec2020 && !has_source_icc && !preserve_source_encoding;
    Ok(Decoded {
        width: rendered_width,
        height: rendered_height,
        layout,
        bit_depth,
        float_samples,
        pixels: out,
        metadata,
        source_orientation,
        rendered_cicp: image.rendered_cicp(),
        rendered_icc: image.rendered_icc(),
        xyb_encoded,
        pixels_linear_rec2020,
        pixels_premultiplied: alpha_associated && !preserve_source_encoding,
    })
}

fn first_alpha(channels: &[ExtraChannelType]) -> Option<(usize, bool)> {
    channels.iter().enumerate().find_map(|(index, channel)| match channel {
        ExtraChannelType::Alpha { alpha_associated } => Some((index, *alpha_associated)),
        _ => None,
    })
}

fn has_black_channel(channels: &[ExtraChannelType]) -> bool {
    channels.iter().any(|channel| matches!(channel, ExtraChannelType::Black))
}

fn enum_encoding_has_supported_curve(encoding: &EnumColourEncoding) -> bool {
    matches!(encoding.colour_space, ColourSpace::Rgb | ColourSpace::Grey)
        && match &encoding.tf {
            TransferFunction::Linear | TransferFunction::Srgb | TransferFunction::Bt709 | TransferFunction::Dci => true,
            TransferFunction::Gamma { g, .. } => *g > 0,
            _ => false,
        }
}

fn decode_error(error: impl std::fmt::Display) -> Error {
    Error::Malformed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRAIGHT: ExtraChannelType = ExtraChannelType::Alpha { alpha_associated: false };
    const ASSOCIATED: ExtraChannelType = ExtraChannelType::Alpha { alpha_associated: true };
    const SPOT: ExtraChannelType = ExtraChannelType::SpotColour { red: 1.0, green: 0.0, blue: 0.0, solidity: 1.0 };

    #[test]
    fn alpha_is_selected_by_type_and_order() {
        assert_eq!(first_alpha(&[SPOT, STRAIGHT]), Some((1, false)));
        assert_eq!(first_alpha(&[STRAIGHT, SPOT]), Some((0, false)));
        assert_eq!(first_alpha(&[ExtraChannelType::Depth, ASSOCIATED, STRAIGHT]), Some((1, true)));
        assert_eq!(first_alpha(&[ExtraChannelType::Black, SPOT, ExtraChannelType::Depth]), None);
    }

    #[test]
    fn black_extra_channel_is_detected_by_type() {
        assert!(has_black_channel(&[SPOT, ExtraChannelType::Black]));
        assert!(!has_black_channel(&[SPOT, STRAIGHT]));
    }

    #[test]
    fn only_modeled_nonlinear_enum_encodings_are_preserved_for_unpremultiplication() {
        let mut encoding = EnumColourEncoding::srgb(RenderingIntent::Relative);
        assert!(enum_encoding_has_supported_curve(&encoding));
        encoding.tf = TransferFunction::Pq;
        assert!(!enum_encoding_has_supported_curve(&encoding));
        encoding.tf = TransferFunction::Linear;
        assert!(enum_encoding_has_supported_curve(&encoding));
        encoding.colour_space = ColourSpace::Xyb;
        assert!(!enum_encoding_has_supported_curve(&encoding));
    }
}
