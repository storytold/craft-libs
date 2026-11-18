use crate::bit_reader::BitWriter;
use crate::filter::Filter;
use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::iter;

/// Decode JBIG2 data from a PDF stream.
///
/// The `params` dictionary may contain a `JBIG2Globals` entry pointing to
/// a stream with shared symbol dictionaries.
///
/// PdfCraft patch: `limit` is the most bytes the page may decode to (see `filter::decode_limit`),
/// checked against the page header's dimensions before any output is allocated.
///
/// PdfCraft patch: `max_pixels` is the most pixels the page may decode to (the document's
/// [`DecodeLimits::max_image_pixels`](crate::DecodeLimits::max_image_pixels), 2^28 by default),
/// checked against the page header's own dimensions before any output is allocated: the image
/// dictionary's smaller ones say nothing about them. `hayro_jbig2` decodes the page within the
/// same pixel limit, and its other bounds follow it (`hayro_jbig2::Limits`).
pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
    limit: usize,
    max_pixels: u64,
) -> Option<FilterResult<'static>> {
    // PdfCraft patch: globals are plain segment data. A globals stream filtered as an image
    // could name further globals of its own, recursing once per stream in the chain.
    let globals = params
        .get::<Stream<'_>>(JBIG2_GLOBALS)
        .filter(|g| {
            !g.filters().iter().any(|f| {
                matches!(
                    f,
                    Filter::Jbig2Decode | Filter::DctDecode | Filter::JpxDecode | Filter::CcittFaxDecode
                )
            })
        })
        .and_then(|g| g.decoded().ok());

    let limits = hayro_jbig2::Limits::with_max_pixels(max_pixels);
    let image = hayro_jbig2::Image::new_embedded_with_limits(data, globals.as_deref(), limits).ok()?;

    if u64::from(image.width()) * u64::from(image.height()) > max_pixels {
        warn!("embedded JBIG2 page is too large");

        return None;
    }

    // PdfCraft patch: the page's bit-packed size (also what `hayro_jbig2` holds for the page
    // bitmap) and the size of the output built below. An image of known size has a limit in its
    // dictionary's 1-bit pixels, so its packed size is checked, and the 8-bit output it is widened
    // to stays within `max_pixels`. A stream decoded without image dimensions, such as an output
    // intent's ICC profile, gets back every byte built, so the output itself must fit.
    let packed_len = (image.width() as usize).div_ceil(8).checked_mul(image.height() as usize)?;
    let luma_len = (image.width() as usize).checked_mul(image.height() as usize)?;
    let sized_image = image_params.bpc.is_some()
        && image_params.num_components.is_some()
        && image_params.width != 0
        && image_params.height != 0;
    let output_len = if image_params.is_indexed { packed_len } else { luma_len };
    if (if sized_image { packed_len } else { output_len }) > limit {
        warn!("embedded JBIG2 page decodes past the stream's limit");

        return None;
    }

    // Whenever possible (if we don't have an indexed color space), we convert
    // the data as 8-bit instead of 1-bit, so that it can be easier converted
    // into an RGBA8 image.

    // We need to invert the color because JBIG2 uses black = 1 and
    // white = 0, but PDF uses the opposite.
    let (decoded, bpc) = if image_params.is_indexed {
        let row_bytes = (image.width() as usize).div_ceil(8);
        // PdfCraft patch: checked and fallible, so a failed allocation is a decode error.
        let len = row_bytes.checked_mul(image.height() as usize)?;
        let mut packed = Vec::new();
        packed.try_reserve_exact(len).ok()?;
        packed.resize(len, 0_u8);

        struct BitWriterDecoder<'a> {
            writer: BitWriter<'a>,
        }

        impl hayro_jbig2::Decoder for BitWriterDecoder<'_> {
            fn push_pixel(&mut self, black: bool) {
                let _ = self.writer.write(u32::from(!black));
            }

            fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
                let byte_value = if black { 0x00 } else { 0xFF };
                let _ = self.writer.fill_bytes(byte_value, chunk_count as usize);
            }

            fn next_line(&mut self) {
                // Images need to be padded to the byte boundary after each row.
                self.writer.align();
            }
        }

        let writer = BitWriter::new(&mut packed, 1)?;
        let mut decoder = BitWriterDecoder { writer };
        image.decode(&mut decoder).ok()?;

        (packed, 1)
    } else {
        struct Luma8Decoder {
            output: Vec<u8>,
        }

        impl hayro_jbig2::Decoder for Luma8Decoder {
            fn push_pixel(&mut self, black: bool) {
                self.output.push(if black { 0x00 } else { 0xFF });
            }

            fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
                let byte = if black { 0x00 } else { 0xFF };
                self.output
                    .extend(iter::repeat_n(byte, chunk_count as usize * 8));
            }

            fn next_line(&mut self) {}
        }

        let mut output = Vec::new();
        output
            .try_reserve_exact((image.width() as usize).checked_mul(image.height() as usize)?)
            .ok()?;
        let mut decoder = Luma8Decoder { output };
        image.decode(&mut decoder).ok()?;

        (decoder.output, 8)
    };

    Some(FilterResult {
        data: Cow::Owned(decoded),
        image_data: Some(ImageData {
            alpha: None,
            color_space: Some(ImageColorSpace::Gray),
            bits_per_component: bpc,
            width: image.width(),
            height: image.height(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page information segment declaring `width` x `height`, and nothing else.
    fn page_information(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u32.to_be_bytes()); // segment number
        out.push(48); // page information
        out.push(0); // no referred-to segments
        out.push(1); // page 1
        out.extend_from_slice(&19u32.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[0; 8]); // resolution
        out.push(0); // flags
        out.extend_from_slice(&[0; 2]); // striping
        out
    }

    #[test]
    fn an_oversized_embedded_page_is_refused_before_allocating() {
        let data = page_information(65_535, 65_535);
        for is_indexed in [false, true] {
            let params = ImageDecodeParams { is_indexed, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 1, height: 1 };
            assert!(decode(&data, &Dict::empty(), &params, usize::MAX, DEFAULT_PIXELS).is_none());
        }
    }

    const DEFAULT_PIXELS: u64 = crate::filter::MAX_IMAGE_PIXELS;

    /// The page's 4,096 pixels are decoded with a pixel limit of 4,096 and refused with one of
    /// 4,095, and a page past the default opens with the limit raised.
    #[test]
    fn the_pixel_limit_given_is_the_one_applied() {
        let data = page_information(64, 64);
        let sized = ImageDecodeParams { is_indexed: true, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 64, height: 64 };
        let decoded = |max_pixels| decode(&data, &Dict::empty(), &sized, 1 << 20, max_pixels).map(|r| r.data.len());
        assert_eq!(decoded(4096), Some(64 / 8 * 64));
        assert_eq!(decoded(4095), None);
        let large = page_information(32_768, 16_384);
        assert!(hayro_jbig2::Image::new_embedded_with_limits(&large, None, hayro_jbig2::Limits::with_max_pixels(DEFAULT_PIXELS)).is_err());
        assert!(hayro_jbig2::Image::new_embedded_with_limits(&large, None, hayro_jbig2::Limits::with_max_pixels(1 << 29)).is_ok());
    }

    /// PdfCraft patch: a page within the pixel cap but past the caller's limit is refused before
    /// its output is allocated. Without image dimensions (an ICC profile decoded with a 16 MiB
    /// limit) the 8-bit output must fit; an image of known size is measured in 1-bit pixels.
    #[test]
    fn a_page_past_the_limit_is_refused() {
        let data = page_information(16_384, 16_384);
        let untyped = ImageDecodeParams::default();
        assert!(decode(&data, &Dict::empty(), &untyped, 16 << 20, DEFAULT_PIXELS).is_none());
        let sized = ImageDecodeParams { is_indexed: false, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 16_384, height: 16_384 };
        assert!(decode(&data, &Dict::empty(), &sized, (16_384 / 8 * 16_384) - 1, DEFAULT_PIXELS).is_none());
    }

    #[test]
    fn a_page_within_the_limit_decodes() {
        let data = page_information(64, 64);
        let untyped = ImageDecodeParams::default();
        let out = decode(&data, &Dict::empty(), &untyped, 64 * 64, DEFAULT_PIXELS).expect("64 x 64 page");
        assert_eq!(out.data.len(), 64 * 64);
        assert!(decode(&data, &Dict::empty(), &untyped, 64 * 64 - 1, DEFAULT_PIXELS).is_none());
        let sized = ImageDecodeParams { is_indexed: false, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 64, height: 64 };
        assert_eq!(decode(&data, &Dict::empty(), &sized, 2 * 64 / 8 * 64, DEFAULT_PIXELS).expect("sized page").data.len(), 64 * 64);
    }
}
