//! PdfCraft patch (48): `JXLDecode`, JPEG XL image data (ISO/IEC 18181) in an image XObject.
//!
//! ISO 32000-2 defines no JPEG XL filter yet (the PDF Association has announced it as the
//! preferred new image format for HDR in PDF and invites implementers to join:
//! <https://pdfa.org/pdf-errata-makeover/>, 17 December 2025), so this reads the name
//! `JXLDecode`, the placeholder other implementations use (for example the PDFium design doc). Only the first frame
//! is drawn (a PDF image is static), as 8-bit grey or RGB with an optional alpha channel;
//! CMYK, spot colours and other layouts are refused (the image is then not drawn).
//!
//! Two things are settled before jxl-oxide sees the bytes or trusts its answer. A container's
//! Brotli-compressed metadata boxes (`brob`, Exif and XMP) are dropped unread: jxl-oxide inflates
//! them into a buffer its allocation tracker does not count, so a few KB could expand into
//! gigabytes before any size check ran, and a PDF image has no use for them. And the extra
//! channels the codestream declares must be none, or one alpha channel that the pixel format
//! reports: a spot colour, a depth map or a second alpha would otherwise be composited or ignored
//! behind the same channel count.

use crate::filter::FilterResult;
use crate::object::stream::{ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;
use jxl_oxide::{EnumColourEncoding, ExtraChannelType, JxlImage, PixelFormat, RenderingIntent};

/// The signature box every JPEG XL container starts with. A bare codestream does not.
const SIGNATURE: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";

/// Decode a JPEG XL image for an image XObject described by `image_params`, refusing images over
/// `max_pixels` pixels or whose decoded raster would take more than `limit` bytes (1 to 4 bytes
/// for each pixel, before the alpha channel is split off); `None` when it can't be decoded.
pub(crate) fn decode(data: &[u8], image_params: &ImageDecodeParams, limit: usize, max_pixels: u64) -> Option<FilterResult<'static>> {
    let data = without_brob_boxes(data)?;
    let mut image = JxlImage::builder().read(std::io::Cursor::new(data.as_ref())).ok()?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > max_pixels {
        return None;
    }
    // A declared size that disagrees with the codestream is not trusted (as for the other codecs).
    if (image_params.width != 0 && image_params.width != width) || (image_params.height != 0 && image_params.height != height) {
        return None;
    }
    let (color_space, has_alpha, channels) = match image.pixel_format() {
        PixelFormat::Gray => (ImageColorSpace::Gray, false, 1usize),
        PixelFormat::Graya => (ImageColorSpace::Gray, true, 2),
        PixelFormat::Rgb => (ImageColorSpace::Rgb, false, 3),
        PixelFormat::Rgba => (ImageColorSpace::Rgb, true, 4),
        _ => return None,
    };
    let extra: Vec<ExtraChannelType> = image.image_header().metadata.ec_info.iter().map(|info| info.ty).collect();
    if !extra_channels_supported(&extra, has_alpha) {
        return None;
    }
    let pixels = (width as usize).checked_mul(height as usize)?;
    let bytes = pixels.checked_mul(channels)?;
    if bytes > limit {
        return None;
    }
    // The samples go on as `DeviceGray` or `DeviceRGB`, which are sRGB-like, so the codestream's own
    // encoding (linear, a wide gamut, HDR transfer) is converted to sRGB first; jxl-oxide would
    // otherwise hand out native samples. An indexed image keeps its raw samples (they are indices).
    if !image_params.is_indexed {
        let intent = RenderingIntent::Relative;
        image.request_color_encoding(if matches!(color_space, ImageColorSpace::Gray) {
            EnumColourEncoding::gray_srgb(intent)
        } else {
            EnumColourEncoding::srgb(intent)
        });
    }
    let associated = image.image_header().metadata.ec_info.iter().any(|info| info.alpha_associated() == Some(true));
    let render = image.render_frame(0).ok()?;
    let mut stream = render.stream();
    if stream.channels() as usize != channels {
        return None;
    }
    let mut buf = vec![0u8; bytes];
    if associated && has_alpha {
        // Premultiplied colour is divided by alpha while it is still a float, then quantised: the
        // interpreter composites straight colour with the alpha it is given, so leaving it
        // premultiplied would attenuate it twice.
        let mut floats = vec![0f32; bytes];
        if stream.write_to_buffer::<f32>(&mut floats) != floats.len() {
            return None;
        }
        for (px, out) in floats.chunks_exact_mut(channels).zip(buf.chunks_exact_mut(channels)) {
            unpremultiply(px);
            for (v, o) in px.iter().zip(out.iter_mut()) {
                *o = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    } else if stream.write_to_buffer::<u8>(&mut buf) != buf.len() {
        return None;
    }
    let colors = channels - usize::from(has_alpha);
    let (data, alpha) = if has_alpha {
        let mut color: Vec<u8> = Vec::with_capacity(pixels.checked_mul(colors)?);
        let mut alpha: Vec<u8> = Vec::with_capacity(pixels);
        for px in buf.chunks_exact(channels) {
            let (a, c) = px.split_last()?;
            alpha.push(*a);
            color.extend_from_slice(c);
        }
        (color, Some(alpha))
    } else {
        (buf, None)
    };
    Some(FilterResult {
        data: Cow::Owned(data),
        image_data: Some(ImageData { alpha, color_space: Some(color_space), bits_per_component: 8, width, height }),
    })
}

/// Turns one premultiplied pixel (colour samples, then alpha last) into straight colour. A zero
/// alpha leaves nothing to recover, so its colour becomes 0.
fn unpremultiply(px: &mut [f32]) {
    if let Some((alpha, colors)) = px.split_last_mut() {
        for c in colors {
            *c = if *alpha > 0.0 { *c / *alpha } else { 0.0 };
        }
    }
}

/// Whether the extra channels a codestream declares are ones this filter draws: none when the
/// pixel format has no alpha channel, exactly one alpha channel when it has. Anything else (a spot
/// colour, a black or depth channel, a second alpha) is refused.
fn extra_channels_supported(declared: &[ExtraChannelType], has_alpha: bool) -> bool {
    match declared {
        [] => !has_alpha,
        [ExtraChannelType::Alpha { .. }] => has_alpha,
        _ => false,
    }
}

/// `data` without the `brob` boxes of a JPEG XL container, in the same order otherwise. A bare
/// codestream, and a container with no `brob` box, come back as they are, uncopied. `None` when a
/// box header is cut off or its size cannot be right (below its own header), since the file then
/// cannot be walked box by box.
fn without_brob_boxes(data: &[u8]) -> Option<Cow<'_, [u8]>> {
    if !data.starts_with(SIGNATURE) {
        return Some(Cow::Borrowed(data));
    }
    let mut kept: Option<Vec<u8>> = None;
    let mut at = 0usize;
    while at < data.len() {
        let head = data.get(at..at.checked_add(8)?)?;
        let size = u64::from(u32::from_be_bytes(head.get(..4)?.try_into().ok()?));
        let (header, total) = match size {
            0 => (8u64, (data.len() - at) as u64),
            1 => (16, u64::from_be_bytes(data.get(at + 8..at.checked_add(16)?)?.try_into().ok()?)),
            n => (8, n),
        };
        if total < header {
            return None;
        }
        let end = at.checked_add(usize::try_from(total).ok()?)?.min(data.len());
        if head.get(4..8)? == b"brob" {
            kept.get_or_insert_with(|| data[..at].to_vec());
        } else if let Some(out) = kept.as_mut() {
            out.extend_from_slice(&data[at..end]);
        }
        at = end;
    }
    Some(match kept {
        Some(out) => Cow::Owned(out),
        None => Cow::Borrowed(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 256 x 128 colour gradient with a dark bar and a checker block, written by libjxl's `cjxl` losslessly
    /// (`-d 0`). Nothing in it is third-party.
    const LOSSLESS: &[u8] = include_bytes!("jxl_lossless.jxl");
    const W: usize = 256;
    const H: usize = 128;

    fn pixel(x: usize, y: usize) -> [u8; 3] {
        let (mut r, mut g, mut b) = (x as u8, (y * 2) as u8, (255 - x) as u8);
        if (40..56).contains(&y) {
            (r, g, b) = (20, 20, 20);
        }
        if x >= 192 && y >= 64 && (x / 8 + y / 8).is_multiple_of(2) {
            (r, g, b) = (250, 250, 250);
        }
        [r, g, b]
    }

    fn expected() -> Vec<u8> {
        (0..H).flat_map(|y| (0..W).flat_map(move |x| pixel(x, y))).collect()
    }

    fn params(width: u32, height: u32) -> ImageDecodeParams {
        ImageDecodeParams { width, height, ..ImageDecodeParams::default() }
    }

    /// One ISO BMFF box.
    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    /// The lossless codestream in a container, with a `brob` box before it when `brob` is given.
    fn container(brob: Option<&[u8]>) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        out.extend(bx(b"ftyp", b"jxl \0\0\0\0jxl "));
        if let Some(payload) = brob {
            out.extend(bx(b"brob", payload));
        }
        out.extend(bx(b"jxlc", LOSSLESS));
        out
    }

    #[test]
    fn a_lossless_codestream_decodes_to_its_pixels() {
        let result = decode(LOSSLESS, &params(W as u32, H as u32), usize::MAX, 1 << 28).unwrap();
        let image = result.image_data.unwrap();
        assert!(matches!(image.color_space, Some(ImageColorSpace::Rgb)));
        assert_eq!((image.width, image.height, image.bits_per_component), (W as u32, H as u32, 8));
        assert!(image.alpha.is_none());
        assert_eq!(result.data.as_ref(), expected().as_slice());
    }

    #[test]
    fn premultiplied_colour_is_divided_by_alpha_and_zero_alpha_gives_black() {
        let mut half = [0.25, 0.5, 0.1, 0.5];
        unpremultiply(&mut half);
        assert_eq!(half, [0.5, 1.0, 0.2, 0.5]);
        let mut none = [0.3, 0.3, 0.3, 0.0];
        unpremultiply(&mut none);
        assert_eq!(none, [0.0, 0.0, 0.0, 0.0]);
        let mut opaque = [0.2, 0.4, 0.6, 1.0];
        unpremultiply(&mut opaque);
        assert_eq!(opaque, [0.2, 0.4, 0.6, 1.0]);
    }

    #[test]
    fn an_srgb_codestream_keeps_its_samples_when_the_output_encoding_is_requested() {
        // The fixture is tagged sRGB, so asking for sRGB output must not change a sample; and an
        // indexed image skips the request altogether.
        let result = decode(LOSSLESS, &params(0, 0), usize::MAX, 1 << 28).unwrap();
        assert_eq!(result.data.as_ref(), expected().as_slice());
        let indexed = ImageDecodeParams { is_indexed: true, ..params(0, 0) };
        assert_eq!(decode(LOSSLESS, &indexed, usize::MAX, 1 << 28).unwrap().data.as_ref(), expected().as_slice());
    }

    #[test]
    fn a_truncated_or_empty_stream_is_refused() {
        for data in [&LOSSLESS[..LOSSLESS.len() / 2], &[0xff, 0x0a, 0x00][..], &[][..]] {
            assert!(decode(data, &params(W as u32, H as u32), usize::MAX, 1 << 28).is_none());
        }
    }

    #[test]
    fn a_declared_size_that_disagrees_with_the_codestream_is_refused() {
        assert!(decode(LOSSLESS, &params(W as u32 - 1, H as u32), usize::MAX, 1 << 28).is_none());
        assert!(decode(LOSSLESS, &params(W as u32, H as u32 + 1), usize::MAX, 1 << 28).is_none());
        // No declared size at all is accepted: the codestream's own is used.
        assert!(decode(LOSSLESS, &params(0, 0), usize::MAX, 1 << 28).is_some());
    }

    #[test]
    fn an_image_over_the_byte_limit_or_the_pixel_cap_is_refused() {
        let p = params(W as u32, H as u32);
        assert!(decode(LOSSLESS, &p, W * H * 3, 1 << 28).is_some(), "exactly at the limit is allowed");
        assert!(decode(LOSSLESS, &p, W * H * 3 - 1, 1 << 28).is_none());
        assert!(decode(LOSSLESS, &p, usize::MAX, (W * H) as u64).is_some());
        assert!(decode(LOSSLESS, &p, usize::MAX, (W * H) as u64 - 1).is_none());
    }

    #[test]
    fn a_container_decodes_like_the_bare_codestream() {
        let result = decode(&container(None), &params(W as u32, H as u32), usize::MAX, 1 << 28).unwrap();
        assert_eq!(result.data.as_ref(), expected().as_slice());
    }

    #[test]
    fn a_brob_box_is_dropped_unread() {
        // Where a Brotli stream would be there is garbage: left in, jxl-oxide would try to inflate it and the image
        // would fail; dropped before it sees the file, the pixels decode.
        let data = container(Some(b"Exif\xff\xff\xff\xff\xff\xff\xff\xff"));
        let stripped = without_brob_boxes(&data).unwrap();
        assert!(matches!(stripped, Cow::Owned(_)));
        assert!(!stripped.windows(4).any(|w| w == b"brob"), "no brob box is left");
        let result = decode(&data, &params(W as u32, H as u32), usize::MAX, 1 << 28).unwrap();
        assert_eq!(result.data.as_ref(), expected().as_slice());
    }

    #[test]
    fn a_stream_without_brob_boxes_is_not_copied() {
        for data in [LOSSLESS.to_vec(), container(None)] {
            assert!(matches!(without_brob_boxes(&data), Some(Cow::Borrowed(_))));
        }
    }

    #[test]
    fn a_container_with_an_unreadable_box_is_refused() {
        // A box whose size is below its own header, and a header cut off.
        let mut small = SIGNATURE.to_vec();
        small.extend_from_slice(&[0, 0, 0, 4, b'f', b't', b'y', b'p']);
        let mut cut = SIGNATURE.to_vec();
        cut.extend_from_slice(&[0, 0, 0]);
        for data in [small, cut] {
            assert!(without_brob_boxes(&data).is_none());
            assert!(decode(&data, &params(0, 0), usize::MAX, 1 << 28).is_none());
        }
    }

    #[test]
    fn only_no_extra_channel_or_one_alpha_is_accepted() {
        const ALPHA: ExtraChannelType = ExtraChannelType::Alpha { alpha_associated: false };
        const ASSOCIATED: ExtraChannelType = ExtraChannelType::Alpha { alpha_associated: true };
        const SPOT: ExtraChannelType = ExtraChannelType::SpotColour { red: 1.0, green: 0.0, blue: 0.0, solidity: 1.0 };
        assert!(extra_channels_supported(&[], false));
        assert!(extra_channels_supported(&[ALPHA], true));
        assert!(extra_channels_supported(&[ASSOCIATED], true));
        assert!(!extra_channels_supported(&[], true), "an alpha format with no alpha channel declared");
        assert!(!extra_channels_supported(&[ALPHA], false), "an alpha channel the format does not report");
        for other in [SPOT, ExtraChannelType::Black, ExtraChannelType::Depth] {
            assert!(!extra_channels_supported(&[other], false), "{other:?} without alpha");
            assert!(!extra_channels_supported(&[other], true));
            assert!(!extra_channels_supported(&[ALPHA, other], true), "{other:?} beside an alpha");
        }
        assert!(!extra_channels_supported(&[ALPHA, ALPHA], true), "a second alpha");
    }
}
