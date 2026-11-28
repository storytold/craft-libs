//! PdfCraft patch (48): `JXLDecode`, JPEG XL image data (ISO/IEC 18181) in an image XObject.
//!
//! ISO 32000-2 defines no JPEG XL filter yet (the PDF Association has announced it as the
//! preferred new image format for HDR in PDF and invites implementers to join:
//! <https://pdfa.org/pdf-errata-makeover/>, 17 December 2025), so this reads the name
//! `JXLDecode`, the placeholder other implementations use (for example the PDFium design doc). Only the first frame
//! is drawn (a PDF image is static), as 8-bit grey or RGB with an optional alpha channel;
//! CMYK, spot colours and other layouts are refused (the image is then not drawn).

use crate::filter::FilterResult;
use crate::object::stream::{ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;
use jxl_oxide::{JxlImage, PixelFormat};

/// Decode a JPEG XL image for an image XObject described by `image_params`, refusing images over
/// `max_pixels` pixels or whose decoded raster would take more than `limit` bytes (3 or 4 bytes
/// for each pixel, before the alpha channel is split off); `None` when it can't be decoded.
pub(crate) fn decode(data: &[u8], image_params: &ImageDecodeParams, limit: usize, max_pixels: u64) -> Option<FilterResult<'static>> {
    let image = JxlImage::builder().read(std::io::Cursor::new(data)).ok()?;
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
    let pixels = (width as usize).checked_mul(height as usize)?;
    let bytes = pixels.checked_mul(channels)?;
    if bytes > limit {
        return None;
    }
    let render = image.render_frame(0).ok()?;
    let mut stream = render.stream();
    if stream.channels() as usize != channels {
        return None;
    }
    let mut buf = vec![0u8; bytes];
    if stream.write_to_buffer::<u8>(&mut buf) != buf.len() {
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

    fn params(width: u32, height: u32) -> ImageDecodeParams {
        ImageDecodeParams { width, height, ..ImageDecodeParams::default() }
    }

    #[test]
    fn a_lossless_codestream_decodes_to_its_pixels() {
        let result = decode(LOSSLESS, &params(W as u32, H as u32), usize::MAX, 1 << 28).unwrap();
        let image = result.image_data.unwrap();
        assert!(matches!(image.color_space, Some(ImageColorSpace::Rgb)));
        assert_eq!((image.width, image.height, image.bits_per_component), (W as u32, H as u32, 8));
        assert!(image.alpha.is_none());
        let expected: Vec<u8> = (0..H).flat_map(|y| (0..W).flat_map(move |x| pixel(x, y))).collect();
        assert_eq!(result.data.as_ref(), expected.as_slice());
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
}
