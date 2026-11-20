use crate::bit_reader::BitWriter;
use crate::filter::Filter;
use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec::Vec;

use core::iter;

/// PdfCraft patch (31): the steps of decoding a page's JBIG2 images may take between them, as many
/// as one may. Each has its own limit, which doesn't bound a page of many made to reach it. That
/// is about 6 s to 17 s, inside a render's watchdog; an image of the largest size costs a quarter of
/// it to decode and a quarter for its page, and ordinary ones are far below that.
pub const MAX_PAGE_JBIG2_WORK: u64 = 1 << 30;

/// PdfCraft patch (53): the most pixels a JBIG2 image may have when its caller takes it at a
/// reduced size (`ImageDecodeParams::reduce_past_full_size`), 2^29: hayro-jbig2 then holds the
/// page bitmap to this (`hayro_jbig2::Limits::with_max_page_pixels`), and every other bitmap to the
/// document's pixel limit. A 1,200 dpi scan of a newspaper page, 19,467 × 25,517, has 497 million.
pub const MAX_REDUCED_JBIG2_PIXELS: u64 = 1 << 29;

/// PdfCraft patch (53): the most pixels a JBIG2 image past the document's pixel limit is reduced
/// to, 2^25: 32 MiB at its byte a pixel. That keeps the 497-million-pixel page above at a quarter
/// of its 1,200 dpi, 300 dpi, sharp at a screen's sizes, and its decode to one kept copy, whatever
/// size it is drawn at.
const MAX_REDUCED_OUTPUT_PIXELS: u64 = 1 << 25;

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

    // PdfCraft patch (53): a caller that takes the image reduced may open a page past
    // `max_pixels`, up to `MAX_REDUCED_JBIG2_PIXELS`; every other bitmap keeps `max_pixels`. The
    // palette of an indexed image needs its pixels' own values, which an average isn't, so such an
    // image is held to the full size.
    let reduce = image_params.reduce_past_full_size && !image_params.is_indexed;
    let limits = hayro_jbig2::Limits::with_max_pixels(max_pixels);
    let limits = if reduce { limits.with_max_page_pixels(MAX_REDUCED_JBIG2_PIXELS) } else { limits };
    let image = hayro_jbig2::Image::new_embedded_with_limits(data, globals.as_deref(), limits)
        .inspect_err(|e| note_limited(Some(e)))
        .ok()?;
    if reduce && u64::from(image.width()) * u64::from(image.height()) > max_pixels {
        return decode_reduced(&image, limit);
    }

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

/// PdfCraft patch (53): `image`, past the document's pixel limit, decoded at a reduced size: each
/// block of [`reduction_factor`] × that many pixels averaged into one 8-bit gray sample as it is
/// read out, so the image never exists a byte a pixel. Only the decoder's page bitmap, a bit a
/// pixel, is held at the full size (64 MiB at most). Craft patch: the reduced samples are held to
/// `limit`, the most bytes the stream may decode to, as every other output of this filter is.
fn decode_reduced(image: &hayro_jbig2::Image<'_>, limit: usize) -> Option<FilterResult<'static>> {
    let factor = reduction_factor(image.width(), image.height())?;
    let mut reducer = Reducer::new(image.width(), image.height(), factor)?;
    if reducer.out_len > limit {
        warn!("embedded JBIG2 page decodes past the stream's limit, reduced");

        return None;
    }
    decode_for_page(image, &mut reducer)?;
    let (data, width, height) = reducer.finish()?;
    Some(FilterResult {
        data: Cow::Owned(data),
        image_data: Some(ImageData {
            alpha: None,
            color_space: Some(ImageColorSpace::Gray),
            bits_per_component: 8,
            width,
            height,
        }),
    })
}

/// PdfCraft patch (53): the size a JBIG2 image of `width` × `height` past the document's pixel
/// limit is decoded at when its caller takes it reduced (`ImageDecodeParams::reduce_past_full_size`):
/// each side divided by [`reduction_factor`], rounded up, as the decode reports it. `None` past any
/// factor.
pub fn reduced_jbig2_size(width: u32, height: u32) -> Option<(u32, u32)> {
    let factor = reduction_factor(width, height)?;
    Some((width.div_ceil(factor), height.div_ceil(factor)))
}

/// PdfCraft patch (53): the smallest whole factor that reduces an image of `width` × `height`
/// to [`MAX_REDUCED_OUTPUT_PIXELS`] or fewer, the same on both axes so the samples stay square.
/// An image of 2^29 pixels takes 4, or a little more when a side doesn't divide evenly; 12 holds
/// the largest page hayro-jbig2 allows at all, 65,535 a side, and 64 is past any.
fn reduction_factor(width: u32, height: u32) -> Option<u32> {
    (2..=64_u32).find(|&f| u64::from(width.div_ceil(f)) * u64::from(height.div_ceil(f)) <= MAX_REDUCED_OUTPUT_PIXELS)
}

/// PdfCraft patch (53): a [`hayro_jbig2::Decoder`] that averages the pixels it is given, a band of
/// `factor` rows at a time, into 8-bit gray samples (255 white, 0 black, as the full-size read
/// gives them). It holds one sum a reduced column and the reduced image.
struct Reducer {
    /// The image's width in pixels.
    width: u32,
    factor: u32,
    /// The black pixels of each reduced column in the current band.
    black: Vec<u32>,
    /// The column of the next pixel in its row.
    x: u32,
    /// The rows read into the current band.
    rows: u32,
    /// The reduced image, a byte a sample, `out_len` bytes once all rows are read.
    out: Vec<u8>,
    out_len: usize,
    out_width: u32,
    out_height: u32,
}

impl Reducer {
    fn new(width: u32, height: u32, factor: u32) -> Option<Self> {
        if width == 0 || height == 0 || factor == 0 {
            return None;
        }
        let out_width = width.div_ceil(factor);
        let out_height = height.div_ceil(factor);
        let out_len = usize::try_from(u64::from(out_width) * u64::from(out_height)).ok()?;
        let columns = usize::try_from(out_width).ok()?;
        let mut black = Vec::new();
        black.try_reserve_exact(columns).ok()?;
        black.resize(columns, 0);
        let mut out = Vec::new();
        out.try_reserve_exact(out_len).ok()?;
        Some(Self { width, factor, black, x: 0, rows: 0, out, out_len, out_width, out_height })
    }

    /// `count` pixels, all black or all white, from the current column on (what lies past the
    /// image's width is dropped).
    fn add(&mut self, black: bool, count: u32) {
        let end = self.x.saturating_add(count).min(self.width);
        if black {
            let mut x = self.x;
            while x < end {
                let column = x / self.factor;
                // `column < out_width`, so `(column + 1) · factor` is at most `width + factor`.
                let column_end = column.saturating_add(1).saturating_mul(self.factor).min(end);
                if let Some(sum) = self.black.get_mut(column as usize) {
                    *sum = sum.saturating_add(column_end - x);
                }
                x = column_end;
            }
        }
        self.x = self.x.saturating_add(count);
    }

    /// The band read so far, as a row of reduced samples: each the share of white in its block,
    /// a block at the right or bottom edge counting only the pixels it has.
    fn flush(&mut self) {
        if self.rows == 0 {
            return;
        }
        for (column, sum) in self.black.iter_mut().enumerate() {
            // `column < out_width`, so `column · factor < width`; `area <= 64 · 64`.
            let left = u32::try_from(column).unwrap_or(u32::MAX).saturating_mul(self.factor);
            let area = self.width.saturating_sub(left).min(self.factor).saturating_mul(self.rows);
            let white = area.saturating_sub(*sum);
            let sample = if area == 0 { 255 } else { white.saturating_mul(255).saturating_add(area / 2) / area };
            if self.out.len() < self.out_len {
                self.out.push(u8::try_from(sample).unwrap_or(u8::MAX));
            }
            *sum = 0;
        }
        self.rows = 0;
    }

    /// The reduced image and its size, once every row has been read.
    fn finish(mut self) -> Option<(Vec<u8>, u32, u32)> {
        self.flush();
        (self.out.len() == self.out_len).then_some((self.out, self.out_width, self.out_height))
    }
}

impl hayro_jbig2::Decoder for Reducer {
    fn push_pixel(&mut self, black: bool) {
        self.add(black, 1);
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        self.add(black, chunk_count.saturating_mul(8));
    }

    fn next_line(&mut self) {
        self.x = 0;
        self.rows = self.rows.saturating_add(1);
        if self.rows >= self.factor {
            self.flush();
        }
    }
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
            let params = ImageDecodeParams { is_indexed, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 1, height: 1, ..Default::default() };
            assert!(decode(&data, &Dict::empty(), &params, usize::MAX, DEFAULT_PIXELS).is_none());
        }
    }

    const DEFAULT_PIXELS: u64 = crate::filter::MAX_IMAGE_PIXELS;

    /// The page's 4,096 pixels are decoded with a pixel limit of 4,096 and refused with one of
    /// 4,095, and a page past the default opens with the limit raised.
    #[test]
    fn the_pixel_limit_given_is_the_one_applied() {
        let data = page_information(64, 64);
        let sized = ImageDecodeParams { is_indexed: true, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 64, height: 64, ..Default::default() };
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
        let sized = ImageDecodeParams { is_indexed: false, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 16_384, height: 16_384, ..Default::default() };
        assert!(decode(&data, &Dict::empty(), &sized, (16_384 / 8 * 16_384) - 1, DEFAULT_PIXELS).is_none());
    }

    #[test]
    fn a_page_within_the_limit_decodes() {
        let data = page_information(64, 64);
        let untyped = ImageDecodeParams::default();
        let out = decode(&data, &Dict::empty(), &untyped, 64 * 64, DEFAULT_PIXELS).expect("64 x 64 page");
        assert_eq!(out.data.len(), 64 * 64);
        assert!(decode(&data, &Dict::empty(), &untyped, 64 * 64 - 1, DEFAULT_PIXELS).is_none());
        let sized = ImageDecodeParams { is_indexed: false, bpc: Some(1), num_components: Some(1), target_dimension: None, width: 64, height: 64, ..Default::default() };
        assert_eq!(decode(&data, &Dict::empty(), &sized, 2 * 64 / 8 * 64, DEFAULT_PIXELS).expect("sized page").data.len(), 64 * 64);
    }

    /// PdfCraft patch (53): the smallest whole factor that brings an image within 2^25 pixels, the
    /// same on both sides, and the size it reports.
    #[test]
    fn a_reduced_size_is_the_smallest_whole_factor_within_2_to_the_25_pixels() {
        assert_eq!(reduction_factor(16_385, 16_384), Some(3));
        assert_eq!(reduced_jbig2_size(16_385, 16_384), Some((5462, 5462)));
        assert_eq!(reduced_jbig2_size(32_768, 16_384), Some((8192, 4096)), "2^29 pixels by 4, exactly 2^25");
        assert_eq!(reduced_jbig2_size(19_467, 25_517), Some((4867, 6380)), "a 1,200 dpi newspaper page, by 4");
        assert_eq!(reduced_jbig2_size(4100, 65_535), Some((1367, 21_845)));
        assert_eq!(reduction_factor(65_535, 65_535), Some(12), "the largest page there is");
        assert_eq!(reduction_factor(u32::MAX, u32::MAX), None);
    }

    /// PdfCraft patch (53): the reducer averages each block into the share of white in it, a block
    /// at the right or bottom edge counting only the pixels it has, and drops what a chunk holds
    /// past the width.
    #[test]
    fn the_reducer_averages_each_block_into_its_share_of_white() {
        use hayro_jbig2::Decoder;
        let rows: [&[bool]; 3] = [&[true, true, false, false, true], &[false, false, false, false, true], &[true, false, false, false, false]];
        let mut reducer = Reducer::new(5, 3, 2).unwrap();
        for row in rows {
            for &black in row {
                reducer.push_pixel(black);
            }
            reducer.next_line();
        }
        assert_eq!(reducer.finish(), Some((vec![128, 255, 0, 128, 255, 255], 3, 2)));
        let mut chunks = Reducer::new(5, 2, 2).unwrap();
        for _ in 0..2 {
            chunks.push_pixel_chunk(true, 1);
            chunks.next_line();
        }
        assert_eq!(chunks.finish(), Some((vec![0, 0, 0], 3, 1)), "a chunk of 8 on a row of 5");
        assert!(Reducer::new(0, 1, 2).is_none() && Reducer::new(1, 1, 0).is_none());
        assert_eq!(Reducer::new(4, 4, 2).unwrap().finish(), None, "rows that never came");
    }

    /// The page information of a `width` × `height` page whose default pixel is black.
    fn black_page_information(width: u32, height: u32) -> Vec<u8> {
        let mut out = page_information(width, height);
        let flags = out.len() - 3;
        out[flags] = 4; // default pixel black
        out
    }

    /// PdfCraft patch (53): a JBIG2 image past the document's pixel limit, up to 2^29 pixels, is
    /// decoded reduced when its caller takes it so, and reports the size it reduced to; refused as
    /// before otherwise, when indexed (its palette needs its pixels' own values) and past 2^29.
    #[test]
    fn a_jbig2_image_past_the_pixel_limit_is_decoded_reduced_when_asked() {
        let params = |width, height, reduce, is_indexed| ImageDecodeParams {
            is_indexed,
            bpc: Some(1),
            num_components: Some(1),
            width,
            height,
            reduce_past_full_size: reduce,
            ..Default::default()
        };
        let decode_page = |width, height, reduce, is_indexed| {
            decode(&black_page_information(width, height), &Dict::empty(), &params(width, height, reduce, is_indexed), usize::MAX, DEFAULT_PIXELS)
        };
        let reduced = decode_page(16_385, 16_384, true, false).expect("decodes reduced");
        let data = reduced.image_data.as_ref().unwrap();
        assert_eq!((data.width, data.height, data.bits_per_component), (5462, 5462, 8));
        assert_eq!(reduced.data.len(), 5462 * 5462);
        assert!(reduced.data.iter().all(|&s| s == 0), "black");
        assert!(decode_page(16_385, 16_384, false, false).is_none(), "refused at full size");
        assert!(decode_page(16_385, 16_384, true, true).is_none(), "an indexed image is held to the full size");
        assert!(decode_page(32_768, 16_385, true, false).is_none(), "past 2^29");
        assert_eq!(decode_page(64, 64, true, false).map(|r| r.data.len()), Some(64 * 64), "within the limit, at full size");
        // The reduced samples are held to the stream's limit too.
        let small = decode(&black_page_information(16_385, 16_384), &Dict::empty(), &params(16_385, 16_384, true, false), 5462 * 5462 - 1, DEFAULT_PIXELS);
        assert!(small.is_none());
    }
}
