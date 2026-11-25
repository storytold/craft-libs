//! JBIG2 images drawn through the interpreter, on synthetic streams (ISO 14492; ISO 32000-2 §7.4.7):
//! what is drawn, and what the page says when an image is left out.
#![cfg(feature = "raster")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use craft_pdf::{
    interpret::InterpreterSettings,
    raster::{self, Options, RenderCache},
};
use std::sync::{Arc, Mutex};

fn assemble(objects: &[Vec<u8>]) -> Arc<Vec<u8>> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n", offsets.len()).as_bytes());
    Arc::new(bytes)
}

/// One embedded JBIG2 segment (ISO 14492 §7.2) of page 1, referring to `refs`.
fn segment(out: &mut Vec<u8>, number: u32, kind: u8, refs: &[u8], data: &[u8]) {
    out.extend_from_slice(&number.to_be_bytes());
    out.push(kind);
    out.push(u8::try_from(refs.len()).unwrap() << 5);
    out.extend_from_slice(refs);
    out.push(1);
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    out.extend_from_slice(data);
}

/// The page information of a `width` × `height` JBIG2 page whose default pixel is black, so the
/// image draws black if it decodes, whatever its other segments hold.
fn black_page(width: u32, height: u32) -> Vec<u8> {
    let mut page = [width.to_be_bytes(), height.to_be_bytes()].concat();
    page.extend_from_slice(&[0; 8]); // resolution
    page.extend_from_slice(&[4, 0, 0]); // flags (default pixel black), striping
    let mut stream = Vec::new();
    segment(&mut stream, 0, 48, &[], &page);
    stream
}

/// The header of a Huffman-coded symbol dictionary that says it exports `exported` symbols and has
/// `new` new ones, and nothing after it.
fn dictionary_counting(exported: u32, new: u32) -> Vec<u8> {
    let mut dictionary = vec![0x00, 0x01]; // Huffman, tables B.4, B.2, B.1, B.1
    dictionary.extend_from_slice(&exported.to_be_bytes());
    dictionary.extend_from_slice(&new.to_be_bytes());
    dictionary
}

/// A 40 × 40 page drawing `stream` as an image of `size` with the entries `kind`, over 5..25 both
/// ways: whether the image's middle came out black, and the warnings the interpreter reported, in
/// order.
fn draw(stream: &[u8], (width, height): (u32, u32), kind: &str) -> (bool, Vec<String>) {
    let content = "q 20 0 0 20 5 5 cm /Im Do Q";
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} {kind} /Filter /JBIG2Decode /Length {} >>\nstream\n",
        stream.len()
    )
    .into_bytes();
    image.extend_from_slice(stream);
    image.extend_from_slice(b"\nendstream");
    let bytes = assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources << /XObject << /Im 5 0 R >> >> >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
        image,
    ]);
    let doc = craft_pdf::open(bytes, None).unwrap();
    let reported = Arc::new(Mutex::new(Vec::new()));
    let settings = InterpreterSettings {
        warning_sink: {
            let reported = reported.clone();
            Arc::new(move |warning| reported.lock().unwrap().push(format!("{warning:?}")))
        },
        ..Default::default()
    };
    let page = raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &Options::default()).unwrap();
    let at = ((25 * page.width + 15) * 4) as usize;
    let drawn = page.bytes().get(at..at + 4) == Some(&[0, 0, 0, 255][..]);
    let warnings = reported.lock().unwrap().clone();
    (drawn, warnings)
}

const RASTER: &str = "/BitsPerComponent 1 /ColorSpace /DeviceGray";
const STENCIL: &str = "/ImageMask true";

/// PdfCraft patch 73 (storytold/pdfcraft#7316): a JBIG2 image drawn on its own, a stencil mask
/// included, that one of the decoder's limits refuses is reported as content left out, as a
/// picture's mask so refused is. Reported as a failed image, which a renderer doesn't show, the
/// page drew white where it is with nothing said (PDFBOX-6263). A damaged one is still a failed
/// image. (One past the bound on an image's pixels is content left out as well, storytold/pdfcraft#7440.)
#[test]
fn a_jbig2_image_drawn_on_its_own_that_a_decoder_limit_refuses_is_content_left_out() {
    let left_out = (false, vec!["ContentTruncated".to_string()]);
    let failed = (false, vec!["ImageDecodeFailure".to_string()]);
    for kind in [RASTER, STENCIL] {
        assert_eq!(draw(&black_page(65536, 2), (65536, 2), kind), left_out, "{kind}: past the decoder's bound on a side");
        let mut counted = black_page(16, 2);
        segment(&mut counted, 1, 0, &[], &dictionary_counting(0, u32::MAX));
        assert_eq!(draw(&counted, (16, 2), kind), left_out, "{kind}: a dictionary of more symbols than a page holds");
        assert_eq!(draw(&black_page(65535, 65535), (65535, 65535), kind), left_out, "{kind}: past the bound on an image's pixels");
        let mut damaged = black_page(16, 2);
        segment(&mut damaged, 1, 0, &[], &dictionary_counting(2, 1));
        assert_eq!(draw(&damaged, (16, 2), kind), failed, "{kind}: two exported of one is damage");
    }
    assert_eq!(draw(&black_page(16, 2), (16, 2), RASTER), (true, vec![]), "a page that decodes is drawn");
}

/// A 100 × 100 pt page drawing image XObject 5 (`width` × `height`, `entries`, stream `data`) as
/// `/Im`, as `content` says, with objects from 6 up in `more`.
fn image_page(width: u32, height: u32, entries: &str, data: &[u8], content: &str, more: &[Vec<u8>]) -> Arc<Vec<u8>> {
    let mut image =
        format!("<< /Type /XObject /Subtype /Image /Width {width} /Height {height} {entries} /Length {} >>\nstream\n", data.len()).into_bytes();
    image.extend_from_slice(data);
    image.extend_from_slice(b"\nendstream");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /Im 5 0 R >> >> >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
        image,
    ];
    objects.extend_from_slice(more);
    assemble(&objects)
}

/// A stream object of `entries` holding `data`.
fn stream_object(entries: &str, data: &[u8]) -> Vec<u8> {
    let mut object = format!("<< {entries} /Length {} >>\nstream\n", data.len()).into_bytes();
    object.extend_from_slice(data);
    object.extend_from_slice(b"\nendstream");
    object
}

/// The first page of `bytes` on white, and the warnings the interpreter reported, in order.
fn render(bytes: Arc<Vec<u8>>) -> (raster::Raster, Vec<String>) {
    let doc = craft_pdf::open(bytes, None).unwrap();
    let reported = Arc::new(Mutex::new(Vec::new()));
    let settings = InterpreterSettings {
        warning_sink: {
            let reported = reported.clone();
            Arc::new(move |warning| reported.lock().unwrap().push(format!("{warning:?}")))
        },
        ..Default::default()
    };
    let options = Options { background: [255; 4], ..Default::default() };
    let page = raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &options).unwrap();
    let warnings = reported.lock().unwrap().clone();
    (page, warnings)
}

/// The page's pixel at `(x, y)`, top-down.
fn pixel(page: &raster::Raster, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * page.width + x) * 4) as usize;
    page.bytes()[at..at + 4].try_into().unwrap()
}

const DRAW_IN_THE_MIDDLE: &str = "q 50 0 0 50 25 25 cm /Im Do Q";
const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// PdfCraft patch 113 (storytold/pdfcraft#7440): a JBIG2 page just past 2^28 pixels, and one of
/// 2^29, all black, is drawn black over the area it is drawn at, as an image and as a stencil mask,
/// with nothing reported. Before, the image was skipped and the page came out white.
#[test]
fn jbig2_images_past_the_full_size_bound_are_drawn_reduced() {
    for (width, height) in [(16385, 16384), (32768, 16384)] {
        let stream = black_page(width, height);
        for entries in [RASTER, STENCIL] {
            let (page, warnings) = render(image_page(width, height, &format!("{entries} /Filter /JBIG2Decode"), &stream, DRAW_IN_THE_MIDDLE, &[]));
            let what = format!("{width} × {height}, {entries}");
            assert!(warnings.is_empty(), "{what}: {warnings:?}");
            assert_eq!(pixel(&page, 50, 50), BLACK, "{what}: drawn black in the middle");
            assert_eq!(pixel(&page, 30, 30), BLACK, "{what}: drawn black near its corner");
            assert_eq!(pixel(&page, 10, 10), WHITE, "{what}: white outside it");
            assert_eq!(pixel(&page, 90, 50), WHITE, "{what}: white beside it");
        }
    }
}

/// PdfCraft patch 113: a JBIG2 soft mask past 2^28 pixels is decoded reduced too, so the picture it
/// masks is drawn through it. Before, the mask was refused, and the picture was left out with it.
#[test]
fn a_jbig2_soft_mask_past_the_full_size_bound_is_drawn_reduced() {
    let (width, height) = (16385, 16384);
    // A 1 × 1 red picture under a mask whose black page, decoded the other way, is opaque.
    let mask = stream_object(
        &format!("/Type /XObject /Subtype /Image /Width {width} /Height {height} {RASTER} /Decode [1 0] /Filter /JBIG2Decode"),
        &black_page(width, height),
    );
    let pdf = image_page(1, 1, "/BitsPerComponent 8 /ColorSpace /DeviceRGB /SMask 6 0 R", &[255, 0, 0], DRAW_IN_THE_MIDDLE, &[mask]);
    let (page, warnings) = render(pdf);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(pixel(&page, 50, 50), [255, 0, 0, 255], "the picture is drawn through its mask");
    assert_eq!(pixel(&page, 10, 10), WHITE, "white outside it");
}

/// A 16,385 × 16,384 JBIG2 page with an intermediate generic region as large as itself: past 2^28
/// pixels, which only the page bitmap of an image read reduced may be, so the decoder refuses it.
fn page_with_a_region_past_the_bitmap_bound() -> Vec<u8> {
    let mut stream = black_page(16385, 16384);
    let mut region = [16385_u32, 16384, 0, 0].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    region.push(0); // combination operator
    region.push(0x08); // arithmetic coding, template 0, typical prediction
    region.extend_from_slice(&[3, 0xff, 0xfd, 0xff, 2, 0xfe, 0xfe, 0xfe]); // AT pixels
    // Every row typical, the first white: a white 16,384 square's coding.
    region.extend_from_slice(&[0xb6, 0xc2, 0x1f, 0xff, 0x7f, 0xff, 0xac]);
    segment(&mut stream, 1, 36, &[], &region);
    stream
}

/// PdfCraft patch 113: an image too large to decode is content left out at one of the renderer's
/// limits, and the page says so: an image past 2^28 pixels in any filter but JBIG2 (a 45,009 ×
/// 33,555 CCITT floor plan among them), a JBIG2 image past 2^29, an indexed JBIG2 image past 2^28,
/// whose palette needs its pixels' own values, a colour-keyed one, whose key ranges are in those
/// values too, one with a side past the JBIG2 decoder's 65,535, which it refuses at any size, and
/// one with a region past 2^28, which the decoder refuses for that region. Before, each was skipped
/// as a failed image, and a page that was only that image came out white with nothing said.
#[test]
fn images_too_large_to_decode_are_reported_as_content_left_out() {
    let fax = "/BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 45009 >>";
    let flate = "/BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /FlateDecode";
    let jbig2 = "/BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /JBIG2Decode";
    let indexed = "/BitsPerComponent 1 /ColorSpace [/Indexed /DeviceGray 1 <00FF>] /Filter /JBIG2Decode";
    let colour_keyed = "/BitsPerComponent 1 /ColorSpace /DeviceGray /Mask [0 0] /Filter /JBIG2Decode";
    let cases = [
        ("a CCITT floor plan", (45009, 33555), fax, vec![0; 16]),
        ("a Flate image past 2^28", (16385, 16384), flate, vec![0; 16]),
        ("a JBIG2 image past 2^29", (32768, 16385), jbig2, black_page(32768, 16385)),
        ("an indexed JBIG2 image past 2^28", (16385, 16384), indexed, black_page(16385, 16384)),
        ("a colour-keyed JBIG2 image past 2^28", (16385, 16384), colour_keyed, black_page(16385, 16384)),
        ("a JBIG2 image past 2^28 with a side past 65,535", (65536, 4097), jbig2, black_page(65536, 4097)),
        ("a JBIG2 image past 2^28 with a region the decoder refuses", (16385, 16384), jbig2, page_with_a_region_past_the_bitmap_bound()),
    ];
    for (what, (width, height), entries, data) in cases {
        let content = format!("{DRAW_IN_THE_MIDDLE} 1 0 0 rg 0 0 10 10 re f");
        let (page, warnings) = render(image_page(width, height, entries, &data, &content, &[]));
        assert_eq!(warnings, ["ContentTruncated"], "{what}");
        assert_eq!(pixel(&page, 50, 50), WHITE, "{what}: not drawn");
        assert_eq!(pixel(&page, 5, 95), [255, 0, 0, 255], "{what}: the rest of the page is drawn");
    }
}

mod decodes {
    use super::*;
    use craft_pdf::interpret::font::Glyph;
    use craft_pdf::interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, Paint, PathDrawMode, SoftMask, TransformExt, interpret_page,
    };
    use kurbo::{Affine, BezPath, Rect};

    /// Counts the images a page hands its device, each asked for its pixels at the size it is
    /// drawn at, as a renderer does.
    #[derive(Default)]
    struct Draws(usize);

    impl<'a> Device<'a> for Draws {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'a>, _: Affine, _: Affine, _: &Paint<'a>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
            let [a, b, c, d, ..] = transform.as_coeffs();
            let target = ((a.hypot(b) * f64::from(image.width())).ceil() as u32, (c.hypot(d) * f64::from(image.height())).ceil() as u32);
            match image {
                Image::Raster(r) => r.with_rgba(|_, _| self.0 += 1, Some(target)),
                Image::Stencil(s) => s.with_stencil(|_, _| self.0 += 1, Some(target)),
            }
        }
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    /// The images drawn and the decodes the interpreter's cache made, interpreting the first page
    /// of `bytes`.
    fn draws_and_decodes(bytes: Arc<Vec<u8>>) -> (usize, usize) {
        let pdf = craft_pdf::syntax::Pdf::new(bytes).expect("parses");
        let page = &pdf.pages()[0];
        let cache = InterpreterCache::new();
        let mut ctx = Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            &cache,
            page.xref(),
            InterpreterSettings::default(),
        );
        let mut device = Draws::default();
        interpret_page(page, &mut ctx, &mut device);
        (device.0, cache.decoded_image_decodes())
    }

    const THREE_SIZES: &str = "q 50 0 0 50 25 25 cm /Im Do Q q 20 0 0 20 0 0 cm /Im Do Q q 90 0 0 90 5 5 cm /Im Do Q";

    /// PdfCraft patch 113: the reduced size depends on the image alone, not on the size it is
    /// drawn at, so a page drawing the image at three sizes decodes it once.
    #[test]
    fn a_reduced_jbig2_image_is_decoded_once_for_every_size_it_is_drawn_at() {
        let (width, height) = (16385, 16384);
        let pdf = image_page(width, height, "/ImageMask true /Filter /JBIG2Decode", &black_page(width, height), THREE_SIZES, &[]);
        assert_eq!(draws_and_decodes(pdf), (3, 1), "drawn three times, decoded once");
    }

    /// PdfCraft patch 113: a reduced JBIG2 image one of its own stream's limits refuses (a region
    /// past 2^28 pixels) is refused again whenever it is decoded, so its failure is kept: drawn
    /// three times, it is decoded once.
    #[test]
    fn a_reduced_jbig2_image_its_stream_refuses_is_decoded_once() {
        let entries = "/BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /JBIG2Decode";
        let pdf = image_page(16385, 16384, entries, &page_with_a_region_past_the_bitmap_bound(), THREE_SIZES, &[]);
        assert_eq!(draws_and_decodes(pdf), (0, 1), "not drawn, decoded once");
    }
}

/// Bits, most significant first, as T.4 and T.6 codes are written.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    used: u32,
}

impl Bits {
    fn put(&mut self, code: &str) {
        for bit in code.bytes() {
            if self.used.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if bit == b'1' {
                *self.bytes.last_mut().unwrap() |= 0x80 >> (self.used % 8);
            }
            self.used += 1;
        }
    }
}

/// A JBIG2 page of 4,100 × 65,535 (268,693,500 pixels, just past 2^28), white on its left 2,048
/// columns and black on the rest: one immediate generic region over the page, MMR coded (ITU-T
/// T.88 6.2.6, which codes it as T.6 does). Its first row is a horizontal mode (`001`), a white run
/// of 2,048 (extended make-up code 2048, then terminating code 0) and a black run of 2,052 (make-up
/// 2048, terminating 4); every row after it repeats the one above, two vertical modes of 0 (`1`).
fn half_black_mmr_page() -> (u32, u32, Vec<u8>) {
    let (width, height) = (4100_u32, 65535_u32);
    let mut mmr = Bits::default();
    mmr.put("001");
    mmr.put("000000010011");
    mmr.put("00110101");
    mmr.put("000000010011");
    mmr.put("011");
    for _ in 1..height {
        mmr.put("11");
    }
    let mut page = [width.to_be_bytes(), height.to_be_bytes()].concat();
    page.extend_from_slice(&[0; 8]); // resolution
    page.extend_from_slice(&[0, 0, 0]); // flags (default pixel white), striping
    let mut region = [width, height, 0, 0].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    region.push(0); // combination operator
    region.push(1); // MMR
    region.extend_from_slice(&mmr.bytes);
    let mut stream = Vec::new();
    segment(&mut stream, 0, 48, &[], &page);
    segment(&mut stream, 1, 38, &[], &region);
    (width, height, stream)
}

/// PdfCraft patch 53: a JBIG2 image past 2^28 pixels is reduced by the smallest whole factor that
/// keeps it within 2^25 pixels (3 here, to 1,367 × 21,845), each sample the share of white among
/// the pixels of its block: white left of the edge between columns 2,047 and 2,048, black right of
/// it, two thirds white in the block across it, and black in the last column, which is two pixels
/// wide. A caller that doesn't take it reduced gets it refused, as before.
#[test]
fn a_reduced_jbig2_image_averages_each_block_of_pixels() {
    use craft_pdf::syntax::object::stream::ImageDecodeParams;
    let (width, height, stream) = half_black_mmr_page();
    let pdf = image_page(width, height, &format!("{RASTER} /Filter /JBIG2Decode"), &stream, DRAW_IN_THE_MIDDLE, &[]);
    let parsed = craft_pdf::syntax::Pdf::new(pdf.clone()).expect("parses");
    let image = parsed.objects().into_iter().filter_map(|object| object.into_stream()).find(|stream| !stream.filters().is_empty()).unwrap();
    let params = ImageDecodeParams { width, height, reduce_past_full_size: true, ..ImageDecodeParams::default() };
    let decoded = image.decoded_image(&params).expect("decodes reduced");
    let data = decoded.image_data.unwrap();
    assert_eq!((data.width, data.height, data.bits_per_component), (1367, 21845, 8));
    assert_eq!(decoded.data.len(), 1367 * 21845);
    for row in [0, 1, 10_000, 21_844] {
        let samples = &decoded.data[row * 1367..][..1367];
        assert!(samples[..682].iter().all(|&s| s == 255), "row {row}: white left of the edge");
        assert_eq!(samples[682], 170, "row {row}: the block across the edge, two of its three columns white");
        assert!(samples[683..].iter().all(|&s| s == 0), "row {row}: black right of it, the narrow last column included");
    }
    assert!(image.decoded_image(&ImageDecodeParams { reduce_past_full_size: false, ..params }).is_err(), "refused at full size");

    // Drawn, it is the same: white on the left of its area, black on the right.
    let (page, warnings) = render(pdf);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(pixel(&page, 30, 50), WHITE, "white on its left");
    assert_eq!(pixel(&page, 70, 50), BLACK, "black on its right");
}

/// The red channel along row 85 of a page drawing the image of `stream` (`width` × `height` in its
/// dictionary) under a `mask_width` × 2 soft mask of 32 bands, opaque and transparent in turn,
/// 90 × 10 pt along the bottom (rows 80 to 90): alone, and after a 1 pt draw near the top.
fn banded_mask_rows(width: u32, height: u32, stream: &[u8], mask_width: usize) -> (Vec<u8>, Vec<u8>) {
    let band = mask_width / 8 / 32;
    let row: Vec<u8> = (0..32).flat_map(|i| vec![if i % 2 == 0 { 0xff } else { 0x00 }; band]).collect();
    let mask = stream_object(&format!("/Type /XObject /Subtype /Image /Width {mask_width} /Height 2 {RASTER}"), &[row.clone(), row].concat());
    let entries = format!("{RASTER} /SMask 6 0 R /Filter /JBIG2Decode");
    let large = "q 90 0 0 10 5 10 cm /Im Do Q";
    let (alone, _) = render(image_page(width, height, &entries, stream, large, std::slice::from_ref(&mask)));
    let (after_small, warnings) = render(image_page(width, height, &entries, stream, &format!("q 1 0 0 1 5 90 cm /Im Do Q {large}"), &[mask]));
    assert!(warnings.is_empty(), "{warnings:?}");
    let row = |page: &raster::Raster| (5..95).map(|x| pixel(page, x, 85)[0]).collect::<Vec<u8>>();
    let alone = row(&alone);
    let bands = alone.windows(2).filter(|w| w[0].abs_diff(w[1]) > 64).count();
    assert!(bands > 10, "drawn alone, the mask's bands show: {alone:?}");
    (alone, row(&after_small))
}

/// PdfCraft patch 113: a reduced JBIG2 image whose soft mask is larger than the image as decoded
/// (5,462 × 5,462 here) but smaller than its full size has the mask averaged down to the size it is
/// drawn at, so each size it is drawn at gets its own alpha. Compared with the full size, the alpha
/// averaged for a small draw was reused for a large one.
#[test]
fn a_reduced_jbig2_image_keeps_a_soft_mask_for_each_size_it_is_drawn_at() {
    let (alone, after_small) = banded_mask_rows(16385, 16384, &black_page(16385, 16384), 8192);
    assert_eq!(after_small, alone, "drawn after a small draw, it keeps its own mask");
}

/// PdfCraft patch 113: the same when the JBIG2 page is another size than the dictionary says:
/// 8,193 × 32,768 under a dictionary of 16,385 × 16,384, so it decodes to 2,731 × 10,923, not the
/// 5,462 × 5,462 the dictionary predicts. Its 4,096-pixel-wide mask is larger than the first and
/// smaller than the second: it is kept whole, not averaged for one draw and reused for the next.
#[test]
fn a_reduced_jbig2_image_of_another_size_than_its_dictionary_keeps_its_soft_mask() {
    let (alone, after_small) = banded_mask_rows(16385, 16384, &black_page(8193, 32768), 4096);
    assert_eq!(after_small, alone, "drawn after a small draw, it keeps its own mask");
}
