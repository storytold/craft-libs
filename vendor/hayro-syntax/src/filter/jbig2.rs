use crate::bit_reader::BitWriter;
use crate::filter::Filter;
use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::iter;

/// PdfCraft patch: most pixels an embedded JBIG2 page may decode to, matching the renderer's
/// `MAX_IMAGE_PIXELS` and `hayro_jbig2`'s region limit. The page header's own dimensions are
/// checked here, before any output is allocated: the image dictionary's smaller ones say nothing
/// about them.
const MAX_PIXELS: u64 = 1 << 28;

/// Decode JBIG2 data from a PDF stream.
///
/// The `params` dictionary may contain a `JBIG2Globals` entry pointing to
/// a stream with shared symbol dictionaries.
pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
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

    let image = hayro_jbig2::Image::new_embedded(data, globals.as_deref()).ok()?;

    if u64::from(image.width()) * u64::from(image.height()) > MAX_PIXELS {
        warn!("embedded JBIG2 page is too large");

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
            assert!(decode(&data, &Dict::empty(), &params).is_none());
        }
    }
}
