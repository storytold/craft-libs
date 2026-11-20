use crate::object::Dict;
use crate::object::dict::keys::COLOR_TRANSFORM;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use core::num::NonZeroU32;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::colorspace::ColorSpace::CMYK;
use zune_jpeg::zune_core::options::DecoderOptions;

pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
) -> Option<FilterResult<'static>> {
    if image_params.width > u16::MAX as u32 || image_params.height > u16::MAX as u32 {
        return None;
    }

    // PdfCraft patch (56): the data is read from its start-of-image marker.
    let data = from_start_of_image(data);

    // Some PDFs have weird JPEGs where the JPEG metadata is completely wrong
    // (for example indicating that one of the dimensions is u16::MAX), but the
    // metadata in the PDF image dictionary is correct. Therefore, we first
    // validate the JPEG metadata and patch the data if any of the dimensions
    // are too large (if they are too small, they will just be padded later on).
    let data = maybe_patch_jpeg_dimensions(data, image_params)?;

    let options = DecoderOptions::default()
        .set_max_width(u16::MAX as usize)
        .set_max_height(u16::MAX as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(&*data), options);
    decoder.decode_headers().ok()?;

    let color_transform = params.get::<u8>(COLOR_TRANSFORM);
    let input_color_space = decoder.input_colorspace().unwrap();

    let mut out_colorspace = if let Some(num_components) = image_params.num_components
        && !matches!(num_components, 1 | 3 | 4)
    {
        ColorSpace::MultiBand(NonZeroU32::new(num_components as u32)?)
    } else {
        match input_color_space {
            ColorSpace::YCbCr => {
                if color_transform.is_none_or(|c| c == 1) {
                    ColorSpace::RGB
                } else {
                    ColorSpace::YCbCr
                }
            }
            ColorSpace::RGB | ColorSpace::RGBA => ColorSpace::RGB,
            ColorSpace::Luma | ColorSpace::LumaA => ColorSpace::Luma,
            // TODO: Find test case with color transform on cmyk
            CMYK => CMYK,
            ColorSpace::YCCK => ColorSpace::YCCK,
            _ => ColorSpace::RGB,
        }
    };

    // In case image had APP14 marker, we might have to override the colorspace.
    if input_color_space == CMYK && decoder.info().unwrap().components == 3 {
        out_colorspace = ColorSpace::RGB;
    }

    decoder.set_options(DecoderOptions::default().jpeg_set_out_colorspace(out_colorspace));
    let mut decoded = decoder.decode().ok()?;

    if out_colorspace == ColorSpace::YCCK {
        // See <https://github.com/mozilla/pdf.js/blob/69595a29192b7704733404a42a2ebb537601117b/src/core/jpg.js#L1331>
        for c in decoded.chunks_mut(4) {
            let y = c[0] as f32;
            let cb = c[1] as f32;
            let cr = c[2] as f32;
            c[0] = (434.456 - y - 1.402 * cr) as u8;
            c[1] = (119.541 - y + 0.344 * cb + 0.714 * cr) as u8;
            c[2] = (481.816 - y - 1.772 * cb) as u8;
        }
    }

    let width = decoder.dimensions().unwrap().0 as u32;
    let height = decoder.dimensions().unwrap().1 as u32;

    let image_data = ImageData {
        alpha: None,
        color_space: match out_colorspace {
            ColorSpace::RGB | ColorSpace::YCbCr => Some(ImageColorSpace::Rgb),
            ColorSpace::Luma => Some(ImageColorSpace::Gray),
            ColorSpace::YCCK | CMYK => Some(ImageColorSpace::Cmyk),
            ColorSpace::MultiBand(_) => None,
            _ => None,
        },
        bits_per_component: 8,
        width,
        height,
    };

    Some(FilterResult {
        data: Cow::Owned(decoded),
        image_data: Some(image_data),
    })
}

/// PdfCraft patch (56): `data` from its first start-of-image marker (`FF D8` and the next
/// marker's `FF`), or all of it when it has none.
///
/// DCTDecode data is a JPEG interchange-format stream (ISO 32000-2 §7.4.8), which begins with
/// that marker. Bytes before it were refused, so the image was not drawn: an inline image with
/// one end-of-line too many after `ID` (§8.9.7 allows a single white-space character there), as
/// in the files of PDFBOX-5992, or an image XObject whose stream data starts with a newline.
/// PDFium, MuPDF and Poppler draw both, and also skip other bytes before the marker, in image
/// XObjects as in inline images. The search is one pass over data already in memory.
fn from_start_of_image(data: &[u8]) -> &[u8] {
    if data.starts_with(&[0xFF, 0xD8]) {
        return data;
    }
    data.windows(3)
        .position(|w| w == [0xFF, 0xD8, 0xFF])
        .and_then(|start| data.get(start..))
        .unwrap_or(data)
}

fn maybe_patch_jpeg_dimensions<'a>(
    data: &'a [u8],
    image_params: &ImageDecodeParams,
) -> Option<Cow<'a, [u8]>> {
    let sof_offset = find_sof_marker(data)?;

    let height_offset = sof_offset.checked_add(5)?;
    let width_offset = sof_offset.checked_add(7)?;

    let jpeg_height = u16::from_be_bytes([
        *data.get(height_offset)?,
        *data.get(height_offset.checked_add(1)?)?,
    ]);
    let jpeg_width = u16::from_be_bytes([
        *data.get(width_offset)?,
        *data.get(width_offset.checked_add(1)?)?,
    ]);

    let jpeg_area = (jpeg_width as usize).checked_mul(jpeg_height as usize)?;
    let image_area = (image_params.width as usize).checked_mul(image_params.height as usize)?;
    let need_patch = jpeg_area > image_area;

    if !need_patch {
        return Some(Cow::Borrowed(data));
    }

    let target_w = (image_params.width as u16).to_be_bytes();
    let target_h = (image_params.height as u16).to_be_bytes();

    let mut patched = data.to_vec();
    patched[height_offset..height_offset.checked_add(2)?].copy_from_slice(&target_h);
    patched[width_offset..width_offset.checked_add(2)?].copy_from_slice(&target_w);

    Some(Cow::Owned(patched))
}

fn find_sof_marker(data: &[u8]) -> Option<usize> {
    let mut i = 0_usize;

    while i.checked_add(1).is_some_and(|next| next < data.len()) {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }

        let marker = data[i + 1];

        // Note: Not sure if 100% correct/robust, is AI-generated.
        match marker {
            // All SOF markers carry dimensions: SOF0–SOF15, excluding
            // 0xC4 (DHT), 0xC8 (JPG), 0xCC (DAC) which are not frame markers.
            0xC0..=0xCF if marker != 0xC4 && marker != 0xC8 && marker != 0xCC => {
                return Some(i);
            }
            // Skip padding bytes (0xFF followed by 0xFF).
            0xFF => {
                i += 1;

                continue;
            }
            // SOI (0xD8), EOI (0xD9), TEM (0x01) and stuffed byte (0x00)
            // are standalone markers with no payload.
            0xD8 | 0xD9 | 0x01 | 0x00 => {
                i += 2;

                continue;
            }
            // All other markers have a 2-byte length field — skip over them.
            _ => {
                let len_start = i.checked_add(2)?;
                let len_end = i.checked_add(3)?;
                let seg_len =
                    u16::from_be_bytes([*data.get(len_start)?, *data.get(len_end)?]) as usize;

                i = i.checked_add(2)?.checked_add(seg_len)?;
            }
        }
    }

    None
}

#[cfg(test)]
mod pdfcraft_tests {
    use super::*;
    use crate::content::{ContentTokenizer, UntypedIter};
    use crate::object::{Object, Stream};
    use alloc::vec::Vec;

    /// An 8 × 8 black grayscale baseline JPEG (contributor-original, written for these tests).
    const BLACK_JPEG: &[u8] = &[
        0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43,
        0x00, 0x10, 0x0b, 0x0c, 0x0e, 0x0c, 0x0a, 0x10, 0x0e, 0x0d, 0x0e, 0x12, 0x11, 0x10, 0x13, 0x18, 0x28, 0x1a, 0x18, 0x16, 0x16, 0x18, 0x31, 0x23,
        0x25, 0x1d, 0x28, 0x3a, 0x33, 0x3d, 0x3c, 0x39, 0x33, 0x38, 0x37, 0x40, 0x48, 0x5c, 0x4e, 0x40, 0x44, 0x57, 0x45, 0x37, 0x38, 0x50, 0x6d, 0x51,
        0x57, 0x5f, 0x62, 0x67, 0x68, 0x67, 0x3e, 0x4d, 0x71, 0x79, 0x70, 0x64, 0x78, 0x5c, 0x65, 0x67, 0x63, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x08,
        0x00, 0x08, 0x01, 0x01, 0x11, 0x00, 0xff, 0xc4, 0x00, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x07, 0xff, 0xc4, 0x00, 0x14, 0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, 0x3f, 0x7f, 0xff, 0xd9,
    ];

    fn params() -> ImageDecodeParams {
        ImageDecodeParams { num_components: Some(1), bpc: Some(8), width: 8, height: 8, ..Default::default() }
    }

    /// Whether `data` decodes to the 8 × 8 black image: every sample dark.
    fn decodes_black(data: &[u8]) -> bool {
        decode(data, &Dict::empty(), &params()).is_some_and(|r| r.data.len() == 64 && r.data.iter().all(|&s| s < 16))
    }

    fn with_prefix(prefix: &[u8]) -> Vec<u8> {
        [prefix, BLACK_JPEG].concat()
    }

    /// PdfCraft patch (56): data that begins with its start-of-image marker decodes as before, and
    /// bytes before the marker are skipped: white space too many after an inline image's `ID`
    /// (PDFBOX-5992's `ID` CR LF LF), a newline before an XObject's data, or other bytes.
    #[test]
    fn bytes_before_the_start_of_image_marker_are_skipped() {
        assert!(decodes_black(BLACK_JPEG), "as written");
        for prefix in [&b"\n"[..], b"\r\n", b"\r\n\n", b"\n\t\x0c\x00", b"xx"] {
            assert!(decodes_black(&with_prefix(prefix)), "after {prefix:?}");
        }
    }

    /// PdfCraft patch (56): data with no start-of-image marker, or one cut off at the end, still
    /// doesn't decode.
    #[test]
    fn data_without_a_start_of_image_marker_does_not_decode() {
        let no_marker = BLACK_JPEG.get(2..).unwrap_or_default();
        assert!(!decodes_black(no_marker));
        assert!(decode(no_marker, &Dict::empty(), &params()).is_none());
        assert!(decode(b"\n\xff\xd8", &Dict::empty(), &params()).is_none(), "a cut marker");
        assert!(decode(b"", &Dict::empty(), &params()).is_none());
    }

    /// PdfCraft patch (56): an inline image with one end-of-line more after `ID` than ISO 32000-2
    /// §8.9.7 allows keeps that byte in its data, and decodes from its marker.
    #[test]
    fn an_inline_jpeg_after_more_white_space_than_one_decodes() {
        for after_id in [&b" "[..], b"\n", b"\r\n", b"\r\n\n", b"\n\n", b"    "] {
            let mut content = b"BI /W 8 /H 8 /CS /G /BPC 8 /F /DCT ID".to_vec();
            content.extend_from_slice(after_id);
            content.extend_from_slice(BLACK_JPEG);
            content.extend_from_slice(b"\nEI Q");
            let mut iter = UntypedIter::new(&content);
            let instruction = iter.next_instruction().expect("the inline image");
            assert_eq!(instruction.operator.as_ref(), b"BI");
            let stream: Stream<'_> = match instruction.operands().next() {
                Some(Object::Stream(stream)) => stream.clone(),
                other => panic!("no inline image stream: {other:?}"),
            };
            let decoded = stream.decoded_image(&params()).expect("decodes");
            assert!(decoded.data.len() == 64 && decoded.data.iter().all(|&s| s < 16), "after {after_id:?}");
        }
    }
}
