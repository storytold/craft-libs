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
    let mut image =
        format!("<< /Type /XObject /Subtype /Image /Width {width} /Height {height} {kind} /Filter /JBIG2Decode /Length {} >>\nstream\n", stream.len())
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
/// page drew white where it is with nothing said (PDFBOX-6263). One past the interpreter's bound
/// on an image's pixels, which holds for any image, is still a failed image; so is a damaged one.
#[test]
fn a_jbig2_image_drawn_on_its_own_that_a_decoder_limit_refuses_is_content_left_out() {
    let left_out = (false, vec!["ContentTruncated".to_string()]);
    let failed = (false, vec!["ImageDecodeFailure".to_string()]);
    for kind in [RASTER, STENCIL] {
        assert_eq!(draw(&black_page(65536, 2), (65536, 2), kind), left_out, "{kind}: past the decoder's bound on a side");
        let mut counted = black_page(16, 2);
        segment(&mut counted, 1, 0, &[], &dictionary_counting(0, u32::MAX));
        assert_eq!(draw(&counted, (16, 2), kind), left_out, "{kind}: a dictionary of more symbols than a page holds");
        assert_eq!(draw(&black_page(65535, 65535), (65535, 65535), kind), failed, "{kind}: past the bound on an image's pixels");
        let mut damaged = black_page(16, 2);
        segment(&mut damaged, 1, 0, &[], &dictionary_counting(2, 1));
        assert_eq!(draw(&damaged, (16, 2), kind), failed, "{kind}: two exported of one is damage");
    }
    assert_eq!(draw(&black_page(16, 2), (16, 2), RASTER), (true, vec![]), "a page that decodes is drawn");
}
