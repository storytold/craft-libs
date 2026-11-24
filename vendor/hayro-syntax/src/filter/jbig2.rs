use crate::bit_reader::BitWriter;
use crate::filter::Filter;
use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec::Vec;

const MAX_PIXELS: u64 = 1 << 28;
use core::iter;

/// PdfCraft patch (31): the steps of decoding a page's JBIG2 images may take between them, as many
/// as one may. Each has its own limit, which doesn't bound a page of many made to reach it. That
/// is about 6 s to 17 s, inside a render's watchdog; an image of the largest size costs a quarter of
/// it to decode and a quarter for its page, and ordinary ones are far below that.
pub const MAX_PAGE_JBIG2_WORK: u64 = 1 << 30;

#[cfg(feature = "std")]
std::thread_local! {
    // Unbounded until a page says otherwise: what isn't drawing a page has no page to run out on.
    static PAGE_WORK_LEFT: core::cell::Cell<u64> = const { core::cell::Cell::new(u64::MAX) };
    // PdfCraft patch (31): what refused the image decoded last on this thread, if one of the
    // decoder's limits did: `STREAM_LIMIT` or `WORK_LIMIT`, 0 otherwise.
    static LIMITED: core::cell::Cell<u8> = const { core::cell::Cell::new(0) };
}

/// PdfCraft patch (31): a limit the stream reaches by itself, whatever ran before it (a bitmap's
/// size, a dictionary's symbols, the bitmaps retained and the like), so decoding it again is
/// refused again.
#[cfg(feature = "std")]
const STREAM_LIMIT: u8 = 1;
/// PdfCraft patch (31): the decoding work, which may be what the page or run had left.
#[cfg(feature = "std")]
const WORK_LIMIT: u8 = 2;

/// PdfCraft patch (31): whether the JBIG2 image decoded last on this thread was refused by one of
/// the decoder's limits rather than failing on its data. A soft mask a limit refused hides what
/// its image would show without it: hayro-interpret leaves such an image out, where a mask the
/// file damaged leaves its image whole (#6722). A size or count the file damaged can read as past
/// a limit, and is taken for one: it can't be told from a hostile one, and leaving the image out
/// is the safe side.
#[cfg(feature = "std")]
pub fn jbig2_limited() -> bool {
    LIMITED.with(core::cell::Cell::get) != 0
}

#[cfg(not(feature = "std"))]
pub fn jbig2_limited() -> bool {
    false
}

/// PdfCraft patch (31): whether the refusal [`jbig2_limited`] reports came from a limit the stream
/// reaches by itself, not from the decoding work, so the same stream is refused again whenever it
/// is decoded.
#[cfg(feature = "std")]
pub fn jbig2_limited_by_stream() -> bool {
    LIMITED.with(core::cell::Cell::get) == STREAM_LIMIT
}

#[cfg(not(feature = "std"))]
pub fn jbig2_limited_by_stream() -> bool {
    false
}

/// PdfCraft patch (31): forget what refused the image decoded last, before decoding another
/// through filters that may fail before JBIG2 is reached.
#[cfg(feature = "std")]
pub fn clear_jbig2_limited() {
    LIMITED.with(|limited| limited.set(0));
}

#[cfg(not(feature = "std"))]
pub fn clear_jbig2_limited() {}

#[cfg(feature = "std")]
fn note_limited(error: Option<&hayro_jbig2::DecodeError>) {
    let refused = match error {
        Some(hayro_jbig2::DecodeError::Overflow(hayro_jbig2::OverflowError::DecodingWork)) => {
            WORK_LIMIT
        }
        Some(e) if e.is_limit() => STREAM_LIMIT,
        _ => 0,
    };
    LIMITED.with(|limited| limited.set(refused));
}

#[cfg(not(feature = "std"))]
fn note_limited(_: Option<&hayro_jbig2::DecodeError>) {}

/// PdfCraft patch (31): what the JBIG2 images decoded next on this thread may take, for a new page.
#[cfg(feature = "std")]
pub fn set_page_jbig2_work(work: u64) {
    PAGE_WORK_LEFT.with(|left| left.set(work));
    clear_jbig2_limited();
}

/// Pays for reading `bytes` of a stream and its globals out of what the page has left, or `None`.
/// An image that doesn't decode isn't always kept (not past the page's allowance, nor drawn under a
/// transfer function: hayro-interpret patch 91), so a page drawing it again and again can read it
/// again.
#[cfg(feature = "std")]
fn charge_page_reading(bytes: usize) -> Option<()> {
    let mut left = PAGE_WORK_LEFT.with(core::cell::Cell::get);
    let charged = hayro_jbig2::charge_reading(&mut left, bytes);
    PAGE_WORK_LEFT.with(|cell| cell.set(left));
    charged.ok()
}

#[cfg(not(feature = "std"))]
fn charge_page_reading(_: usize) -> Option<()> {
    Some(())
}

/// `image` decoded through `decoder`, spending what the page has left, if there is anything.
#[cfg(feature = "std")]
fn decode_for_page<D: hayro_jbig2::Decoder>(image: &hayro_jbig2::Image<'_>, decoder: &mut D) -> Option<()> {
    let mut left = PAGE_WORK_LEFT.with(core::cell::Cell::get);
    let decoded = image.decode_within(decoder, &mut left);
    PAGE_WORK_LEFT.with(|cell| cell.set(left));
    note_limited(decoded.as_ref().err());
    decoded.ok()
}

#[cfg(not(feature = "std"))]
fn decode_for_page<D: hayro_jbig2::Decoder>(image: &hayro_jbig2::Image<'_>, decoder: &mut D) -> Option<()> {
    image.decode(decoder).ok()
}

/// PdfCraft patch (31): whether the page has nothing left for another image, which then isn't read
/// at all.
#[cfg(feature = "std")]
pub fn page_jbig2_work_spent() -> bool {
    PAGE_WORK_LEFT.with(core::cell::Cell::get) == 0
}

#[cfg(not(feature = "std"))]
pub fn page_jbig2_work_spent() -> bool {
    false
}
/// Decode JBIG2 data from a PDF stream.
///
/// The `params` dictionary may contain a `JBIG2Globals` entry pointing to
/// a stream with shared symbol dictionaries.
///
/// PdfCraft patch: `limit` is the most bytes the page may decode to (see `filter::decode_limit`),
/// checked against the page header's dimensions before any output is allocated.
pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
    limit: usize,
) -> Option<FilterResult<'static>> {
    if page_jbig2_work_spent() {
        return None;
    }

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

    charge_page_reading(data.len().saturating_add(globals.as_deref().map_or(0, <[u8]>::len)))?;

    let image = hayro_jbig2::Image::new_embedded(data, globals.as_deref())
        .inspect_err(|e| note_limited(Some(e)))
        .ok()?;

    if u64::from(image.width()) * u64::from(image.height()) > MAX_PIXELS {
        warn!("embedded JBIG2 page is too large");

        return None;
    }

    // PdfCraft patch: the page's bit-packed size (also what `hayro_jbig2` holds for the page
    // bitmap) and the size of the output built below. An image of known size has a limit in its
    // dictionary's 1-bit pixels, so its packed size is checked, and the 8-bit output it is widened
    // to stays within `MAX_PIXELS`. A stream decoded without image dimensions, such as an output
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
        decode_for_page(&image, &mut decoder)?;

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
        decode_for_page(&image, &mut decoder)?;

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
            assert!(decode(&data, &Dict::empty(), &params, usize::MAX).is_none());
        }
    }

    /// PdfCraft patch: a page within the pixel cap but past the caller's limit is refused before
    /// its output is allocated. Without image dimensions (an ICC profile decoded with a 16 MiB
    /// limit) the 8-bit output must fit; an image of known size is measured in 1-bit pixels.
    #[test]
    fn a_page_past_the_limit_is_refused() {
        let data = page_information(16_384, 16_384);
        let untyped = ImageDecodeParams::default();
        assert!(decode(&data, &Dict::empty(), &untyped, 16 << 20).is_none());
        let sized = ImageDecodeParams { is_indexed: false, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 16_384, height: 16_384 };
        assert!(decode(&data, &Dict::empty(), &sized, (16_384 / 8 * 16_384) - 1).is_none());
    }

    #[test]
    fn a_page_within_the_limit_decodes() {
        let data = page_information(64, 64);
        let untyped = ImageDecodeParams::default();
        let out = decode(&data, &Dict::empty(), &untyped, 64 * 64).expect("64 x 64 page");
        assert_eq!(out.data.len(), 64 * 64);
        assert!(decode(&data, &Dict::empty(), &untyped, 64 * 64 - 1).is_none());
        let sized = ImageDecodeParams { is_indexed: false, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 64, height: 64 };
        assert_eq!(decode(&data, &Dict::empty(), &sized, 2 * 64 / 8 * 64).expect("sized page").data.len(), 64 * 64);
    }
}
