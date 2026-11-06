//! Whole documents flattened: pages, boxes, metadata, output intent and shared fonts carry over,
//! and each page's plates match the source.
//!
//! The text cases need Inter from `CRAFT_PDF_TEST_FONT_DIR` or `~/.cache/craft-pdf-test-fonts`,
//! and are skipped when it is missing.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Instant;

use craft_pdf::syntax::object::dict::keys::{BLEED_BOX, DEST_OUTPUT_PROFILE, OUTPUT_CONDITION_IDENTIFIER, OUTPUT_INTENTS, TRIM_BOX};
use craft_pdf::syntax::object::{Array, Dict, Rect as PdfRect, Stream, String as PdfString};
use craft_pdf::write::backend;
use craft_pdf::write::backend::color::cmyk;
use craft_pdf::write::backend::color::device_n::{self, Colorant, DeviceNSpace, Process};
use craft_pdf::write::backend::configure::PdfVersion;
use craft_pdf::write::backend::geom::{PathBuilder, Point, Rect as KRect};
use craft_pdf::write::backend::metadata::DateTime;
use craft_pdf::write::backend::num::NormalizedF32;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::Fill;
use craft_pdf::write::backend::surface::Surface;
use craft_pdf::write::backend::text::{Font, GlyphId, KrillaGlyph};
use craft_pdf::write::backend::{GroupColorSpace, SerializeSettings};
use craft_pdf::write::print::{self, PrintColor};
use craft_pdf::write::{Metadata, Writer};
use craft_plate_test::reference;
use craft_plate_test::structure;
use craft_prepress::flatten::{
    FlattenDocumentOptions, FlattenOptions, FlattenPageOptions, flatten_document, flatten_document_with_progress, flatten_page,
};
use craft_prepress::{Options, Plates, plates};

const DPI: f32 = 36.0;
const SIZES: [(f32, f32); 3] = [(200.0, 160.0), (150.0, 200.0), (220.0, 120.0)];

fn inter() -> Option<Vec<u8>> {
    let dir = std::env::var_os(reference::FONT_DIR_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/craft-pdf-test-fonts")))?;
    let font = std::fs::read(dir.join(reference::FONT_FILE)).ok();
    if font.is_none() {
        eprintln!("skipped: no {} in {}", reference::FONT_FILE, dir.display());
    }
    font
}

fn fill(c: u8, m: u8, y: u8, k: u8) -> Fill {
    Fill { paint: cmyk::Color::new(c, m, y, k).into(), ..Default::default() }
}

fn rect(s: &mut Surface<'_>, x0: f32, y0: f32, x1: f32, y1: f32, f: Fill) {
    let mut b = PathBuilder::new();
    b.push_rect(tiny_rect(x0, y0, x1, y1));
    s.set_fill(Some(f));
    s.draw_path(&b.finish().unwrap());
}

fn tiny_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> craft_pdf::write::backend::geom::Rect {
    KRect::from_ltrb(x0, y0, x1, y1).unwrap()
}

fn text(s: &mut Surface<'_>, font: &Font, face: &ttf_parser::Face<'_>, at: (f32, f32), size: f32, string: &str) {
    let upem = f32::from(face.units_per_em());
    let glyphs: Vec<KrillaGlyph> = string
        .char_indices()
        .map(|(i, c)| {
            let id = face.glyph_index(c).unwrap();
            let advance = f32::from(face.glyph_hor_advance(id).unwrap()) / upem;
            KrillaGlyph::new(GlyphId::new(u32::from(id.0)), advance, 0.0, 0.0, 0.0, i..i + c.len_utf8(), None)
        })
        .collect();
    s.set_fill(Some(fill(0, 0, 0, 255)));
    s.draw_glyphs(Point::from_xy(at.0, at.1), &glyphs, font.clone(), string, size, false);
}

/// Three pages of different sizes: transparency beside text, no transparency, and transparency
/// over text in page 1's font. Page 1 has a TrimBox and a BleedBox.
fn three_pages(font_bytes: &[u8]) -> Vec<u8> {
    let face = ttf_parser::Face::parse(font_bytes, 0).unwrap();
    let font = craft_pdf::write::font(font_bytes, 0, None).unwrap();
    let metadata = Metadata {
        title: Some("Flatten test".into()),
        authors: vec!["A. Writer".into()],
        created: Some(DateTime::new(2026).month(10).day(20).hour(9).minute(30).second(0).utc_offset_hour(2)),
        modified: Some(DateTime::new(2026).month(10).day(21).hour(16).minute(5).second(0).utc_offset_hour(2)),
        ..Metadata::default()
    };
    // DeviceCMYK blending and no output intent: parsing a profile takes half a second in a debug build.
    let settings = SerializeSettings { no_device_cs: false, group_color_space: GroupColorSpace::Cmyk, ..SerializeSettings::default() };
    let mut writer = Writer::new(settings, metadata);
    let mut page = |settings: PageSettings, draw: &dyn Fn(&mut Surface<'_>)| {
        let mut page = writer.document_mut().start_page_with(settings);
        let mut s = page.surface();
        draw(&mut s);
        s.finish();
        page.finish();
    };
    let size = |i: usize| PageSettings::from_wh(SIZES[i].0, SIZES[i].1).unwrap();
    page(size(0).with_trim_box(Some(tiny_rect(10.0, 10.0, 190.0, 150.0))).with_bleed_box(Some(tiny_rect(5.0, 5.0, 195.0, 155.0))), &|s| {
        rect(s, 20.0, 20.0, 120.0, 100.0, fill(255, 0, 0, 0));
        s.push_opacity(NormalizedF32::new(0.5).unwrap());
        rect(s, 70.0, 60.0, 170.0, 130.0, fill(0, 255, 0, 0));
        s.pop();
        text(s, &font, &face, (10.0, 150.0), 10.0, "Hello");
    });
    page(size(1), &|s| {
        rect(s, 10.0, 10.0, 140.0, 90.0, fill(0, 0, 255, 0));
        text(s, &font, &face, (10.0, 150.0), 14.0, "Opaque");
    });
    page(size(2), &|s| {
        text(s, &font, &face, (20.0, 60.0), 30.0, "Hello");
        s.push_opacity(NormalizedF32::new(0.4).unwrap());
        rect(s, 40.0, 25.0, 120.0, 80.0, fill(255, 0, 0, 0));
        s.pop();
        text(s, &font, &face, (20.0, 110.0), 12.0, "World");
    });
    drop(page);
    writer.finish().unwrap()
}

fn opts() -> FlattenDocumentOptions {
    FlattenDocumentOptions { flatten: FlattenOptions { raster_dpi: 72.0, ..FlattenOptions::default() }, ..FlattenDocumentOptions::default() }
}

fn separate(pdf: &[u8], page: usize) -> Plates {
    plates(pdf, page, DPI, &Options::default()).unwrap()
}

fn tint(p: &Plates, name: &str, x: f32, y: f32) -> u8 {
    let s = p.dpi / 72.0;
    p.inks.lookup(name).and_then(|id| p.tint(id, (x * s) as u32, (y * s) as u32)).unwrap_or(0)
}

/// No transparency groups, soft masks in graphics states, blend modes or constant alpha.
fn assert_opaque(pdf: &[u8]) {
    let s = structure::summarize(pdf).unwrap();
    assert!(s.groups.iter().all(|g| g.s != "Transparency"), "transparency groups: {:?}", s.groups);
    for gs in &s.ext_g_states {
        assert!(gs.smask.as_deref().is_none_or(|m| m == "None" || m.is_empty()), "soft mask: {gs:?}");
        assert!(gs.bm.as_deref().is_none_or(|b| b == "Normal" || b == "Compatible"), "blend mode: {gs:?}");
        assert!(gs.alpha_fill.is_none_or(|a| a >= 1.0) && gs.alpha_stroke.is_none_or(|a| a >= 1.0), "alpha: {gs:?}");
    }
}

fn has_text(pdf: &craft_pdf::syntax::Pdf, page: usize) -> bool {
    let pages = pdf.pages();
    let mut ops = pages.get(page).unwrap().operations();
    let mut found = false;
    while let Some(op) = ops.next() {
        found |= matches!(&**op.operator, b"Tj" | b"TJ");
    }
    found
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack.windows(needle.len()).filter(|w| *w == needle).count()
}

fn page_box(pdf: &craft_pdf::syntax::Pdf, page: usize, key: &[u8]) -> Option<[f64; 4]> {
    pdf.pages().get(page)?.raw().get::<PdfRect>(key).map(|r| [r.x0, r.y0, r.x1, r.y1])
}

#[test]
fn a_document_flattens_page_by_page_and_keeps_boxes_metadata_and_shared_fonts() {
    let Some(font) = inter() else { return };
    let source = three_pages(&font);
    let start = Instant::now();
    let (out, report) = flatten_document(&source, &opts()).unwrap();
    eprintln!("flatten_document: {:?}, totals {:?}", start.elapsed(), report.totals);

    let doc = craft_pdf::open_slice(&out, None).unwrap();
    let src = craft_pdf::open_slice(&source, None).unwrap();
    assert_eq!(doc.pages().len(), 3);
    for (i, (w, h)) in SIZES.iter().enumerate() {
        assert_eq!(doc.pages()[i].render_dimensions(), (*w, *h), "page {i}");
    }
    assert_eq!(report.pages.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [0, 1, 2]);
    assert!(report.pages[0].1.flatten.flattened > 0 && report.pages[2].1.flatten.flattened > 0);
    assert_eq!(report.pages[1].1.flatten.flattened, 0);
    assert!(report.unflattened.is_empty() && report.other_intents.is_empty());

    assert_opaque(&out);
    for page in 0..3 {
        assert!(has_text(&doc, page), "page {page} has live text");
    }

    // One embedded program for the font all three pages use.
    assert_eq!(count(&source, b"/FontFile2"), 1);
    assert_eq!(count(&out, b"/FontFile2"), 1, "the shared font is embedded once");

    for key in [TRIM_BOX, BLEED_BOX] {
        assert_eq!(page_box(&doc, 0, key), page_box(&src, 0, key));
        assert!(page_box(&doc, 0, key).is_some());
        assert_eq!(page_box(&doc, 1, key), None);
    }

    let info = doc.metadata();
    assert_eq!(info.title.as_deref(), Some(b"Flatten test".as_slice()));
    assert_eq!(info.author.as_deref(), Some(b"A. Writer".as_slice()));
    let created = info.creation_date.unwrap();
    assert_eq!((created.year, created.month, created.day, created.hour, created.minute), (2026, 10, 20, 9, 30));
    let modified = info.modification_date.unwrap();
    assert_eq!((modified.year, modified.month, modified.day, modified.hour, modified.minute), (2026, 10, 21, 16, 5));

    let points: [&[(f32, f32)]; 3] = [
        &[(40.0, 40.0), (100.0, 80.0), (150.0, 120.0), (190.0, 15.0)],
        &[(50.0, 50.0), (100.0, 180.0)],
        &[(110.0, 30.0), (115.0, 75.0), (180.0, 50.0)],
    ];
    for (page, points) in points.iter().enumerate() {
        let (a, b) = (separate(&source, page), separate(&out, page));
        for &(x, y) in *points {
            for ink in ["Cyan", "Magenta", "Yellow", "Black"] {
                let (want, got) = (tint(&a, ink, x, y), tint(&b, ink, x, y));
                assert!(want.abs_diff(got) <= 3, "page {page} {ink} at ({x}, {y}): {got}, the source has {want}");
            }
        }
    }

    let single: usize = (0..3)
        .map(|p| {
            let opts = FlattenPageOptions { flatten: opts().flatten, ..FlattenPageOptions::default() };
            flatten_page(&source, p, &opts).unwrap().0.len()
        })
        .sum();
    eprintln!("source {} bytes; flattened document {} bytes; three flatten_page outputs {single} bytes", source.len(), out.len());
}

#[test]
fn a_page_range_flattens_only_those_pages() {
    let Some(font) = inter() else { return };
    let t = Instant::now();
    let source = three_pages(&font);
    eprintln!("three_pages {:?}", t.elapsed());
    let mut calls = Vec::new();
    let opts = FlattenDocumentOptions { pages: Some(1..3), ..opts() };
    let t = Instant::now();
    let (out, report) = flatten_document_with_progress(&source, &opts, &mut |done, total| calls.push((done, total))).unwrap();
    eprintln!("flatten {:?}", t.elapsed());
    let doc = craft_pdf::open_slice(&out, None).unwrap();
    assert_eq!(doc.pages().len(), 2);
    assert_eq!(doc.pages()[0].render_dimensions(), SIZES[1]);
    assert_eq!(doc.pages()[1].render_dimensions(), SIZES[2]);
    assert_eq!(report.pages.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(calls, [(1, 2), (2, 2)]);

    let opts = FlattenDocumentOptions { pages: Some(2..4), ..FlattenDocumentOptions::default() };
    assert!(flatten_document(&source, &opts).is_err());
}

#[test]
fn the_source_output_intent_is_written_once() {
    let color = PrintColor {
        profile: craft_cms::builtin::COATED_CMYK_ICC,
        condition_identifier: "Test coated",
        condition: Some("A coated test condition"),
        registry_name: None,
        info: Some("A coated test condition"),
    };
    let mut writer = Writer::new(print::settings(color).unwrap(), Metadata::default());
    for _ in 0..2 {
        let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
        let mut s = page.surface();
        rect(&mut s, 10.0, 10.0, 60.0, 60.0, fill(255, 0, 0, 0));
        s.push_opacity(NormalizedF32::new(0.5).unwrap());
        rect(&mut s, 30.0, 30.0, 90.0, 90.0, fill(0, 0, 0, 255));
        s.pop();
        s.finish();
        page.finish();
    }
    let source = writer.finish().unwrap();
    let (out, report) = flatten_document(&source, &opts()).unwrap();
    assert!(report.other_intents.is_empty());
    assert_opaque(&out);

    let doc = craft_pdf::open_slice(&out, None).unwrap();
    let xref = doc.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id()).unwrap();
    let intents: Vec<Dict<'_>> = catalog.get::<Array<'_>>(OUTPUT_INTENTS).unwrap().iter::<Dict<'_>>().collect();
    assert_eq!(intents.len(), 1);
    let id = intents[0].get::<PdfString<'_>>(OUTPUT_CONDITION_IDENTIFIER).unwrap();
    assert_eq!(id.as_bytes(), b"Test coated");
    let icc = intents[0].get::<Stream<'_>>(DEST_OUTPUT_PROFILE).unwrap().decoded().unwrap();
    assert_eq!(&*icc, craft_cms::builtin::COATED_CMYK_ICC);
    assert_eq!(count(&out, b"/DestOutputProfile"), 1);
}

/// PDF/X-1a output: the source's DeviceN fill (cyan and a spot, NChannel at 1.7) is written at
/// 1.4 without the 1.6 NChannel attributes, and the PDF/X version goes to Info and XMP.
#[test]
fn pdf_x1a_output_is_pdf_14_without_nchannel() {
    let gold = cmyk::Color::new(0, 51, 255, 26);
    let space = DeviceNSpace::new(vec![Colorant::Process(Process::Cyan), Colorant::Spot { name: "Gold".to_owned(), alternate: gold }]).unwrap();
    let color = device_n::Color::new(vec![255, 128], space).unwrap();
    let mut writer = Writer::new(print::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)).unwrap(), Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
    let mut s = page.surface();
    rect(&mut s, 10.0, 10.0, 60.0, 60.0, Fill { paint: backend::color::Color::from(color).into(), ..Default::default() });
    s.push_opacity(NormalizedF32::new(0.5).unwrap());
    rect(&mut s, 30.0, 30.0, 90.0, 90.0, fill(0, 0, 0, 255));
    s.pop();
    s.finish();
    page.finish();
    let source = writer.finish().unwrap();
    assert!(count(&source, b"/NChannel") > 0);

    let opts = FlattenDocumentOptions {
        version: PdfVersion::Pdf14,
        pdfx_version: Some("PDF/X-1:2003".to_owned()),
        pdfx_conformance: Some("PDF/X-1a:2003".to_owned()),
        ..opts()
    };
    let (out, _) = flatten_document(&source, &opts).unwrap();
    assert_opaque(&out);
    assert!(out.starts_with(b"%PDF-1.4"));
    assert_eq!(count(&out, b"NChannel"), 0);
    assert_eq!(count(&out, b"/GTS_PDFXVersion(PDF/X-1:2003)"), 1);
    assert_eq!(count(&out, b"/GTS_PDFXConformance(PDF/X-1a:2003)"), 1);
    assert_eq!(count(&out, b"<pdfxid:GTS_PDFXVersion>PDF/X-1:2003</pdfxid:GTS_PDFXVersion>"), 1);
    assert_eq!(count(&out, b"<pdf:PDFVersion>1.4</pdf:PDFVersion>"), 1);
    let gold_plate = separate(&out, 0);
    assert!(tint(&gold_plate, "Gold", 20.0, 20.0) > 100);
}

/// A 16 × 16 Adobe CMYK JPEG of C 100%, and a 16 × 16 gray JPEG of 25% gray (ImageMagick).
const CYAN_JPEG: &[u8] = include_bytes!("fixtures/cyan-cmyk.jpg");
const GRAY_JPEG: &[u8] = include_bytes!("fixtures/gray-25.jpg");

#[test]
fn untouched_jpeg_images_are_written_as_they_are() {
    let image = |cs: &str, decode: &str, data: &[u8]| {
        let mut obj = format!(
            "<< /Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /{cs} {decode} /BitsPerComponent 8 \
             /Filter /DCTDecode /Length {} >> stream\n",
            data.len()
        )
        .into_bytes();
        obj.extend_from_slice(data);
        obj.extend_from_slice(b"\nendstream");
        obj
    };
    // The images, and a half-transparent black square away from them.
    let content = "q 50 0 0 50 10 10 cm /Im1 Do Q q 50 0 0 50 70 10 cm /Im2 Do Q \
        q /GS1 gs 0 0 0 1 k 140 10 50 50 re f Q";
    let objects =
        [image("DeviceCMYK", "/Decode [1 0 1 0 1 0 1 0]", CYAN_JPEG), image("DeviceGray", "", GRAY_JPEG), b"<< /Type /ExtGState /ca 0.5 >>".to_vec()];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    pdf.extend_from_slice(b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    pdf.extend_from_slice(b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n");
    pdf.extend_from_slice(
        b"3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 70] /Contents 4 0 R \
          /Resources << /XObject << /Im1 5 0 R /Im2 6 0 R >> /ExtGState << /GS1 7 0 R >> >> >> endobj\n",
    );
    pdf.extend_from_slice(format!("4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n", content.len()).as_bytes());
    for (i, body) in objects.iter().enumerate() {
        pdf.extend_from_slice(format!("{} 0 obj ", i + 5).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b" endobj\n");
    }
    pdf.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF");

    let (out, report) = flatten_document(&pdf, &opts()).unwrap();
    assert_eq!(report.totals.images, 2);
    assert!(report.totals.flattened > 0);
    assert_opaque(&out);
    assert_eq!(count(&out, CYAN_JPEG), 1, "the CMYK JPEG is written as it is");
    assert_eq!(count(&out, GRAY_JPEG), 1, "the gray JPEG is written as it is");
    assert_eq!(count(&out, b"/DCTDecode"), 2);
    let (a, b) = (separate(&pdf, 0), separate(&out, 0));
    for (x, y) in [(35.0, 35.0), (95.0, 35.0), (165.0, 35.0)] {
        for ink in ["Cyan", "Magenta", "Yellow", "Black"] {
            let (want, got) = (tint(&a, ink, x, y), tint(&b, ink, x, y));
            assert!(want.abs_diff(got) <= 3, "{ink} at ({x}, {y}): {got}, the source has {want}");
        }
    }
    assert!(tint(&b, "Cyan", 35.0, 35.0) > 240 && tint(&b, "Black", 95.0, 35.0) > 150);
}
