//! PostScript written from print PDFs and run again by craft-ps's interpreter: the composite
//! separates like the source, and each plate file prints what the source's plate holds.
//!
//! Pages are 100 pt square and separated at 36 dpi; sample points are in points, y down.
//! The text cases need Inter from `CRAFT_PDF_TEST_FONT_DIR` or `~/.cache/craft-pdf-test-fonts`,
//! and are skipped when it is missing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use craft_pdf::write::backend::SerializeSettings;
use craft_pdf::write::backend::geom::Point as KPoint;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::Fill;
use craft_pdf::write::backend::text::{GlyphId, KrillaGlyph};
use craft_pdf::write::print::{self, PrintColor};
use craft_pdf::write::{Metadata, Writer};
use kurbo::{Affine, Point, Rect, Shape};

use super::*;
use crate::flatten::write::page_pdf;
use crate::flatten::{FillItem, GradientGeometry, ImageSpace, Paint};
use crate::ink::{Alternate, InkColor, InkId, InkSet, Spot};
use crate::{CmsConversion, Options, Plates, plates};

const C: InkId = InkId::process(Process::Cyan);
const M: InkId = InkId::process(Process::Magenta);
const K: InkId = InkId::process(Process::Black);

fn settings() -> SerializeSettings {
    // Reading the profile takes a quarter second in a debug build.
    static SETTINGS: OnceLock<SerializeSettings> = OnceLock::new();
    SETTINGS.get_or_init(|| print::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)).unwrap()).clone()
}

fn fill(r: Rect, color: InkColor, overprint: Overprint) -> Item {
    Item::Fill(FillItem { path: r.to_path(0.1), rule: FillRule::NonZero, paint: Paint::Solid(color), overprint })
}

fn tints(t: &[(InkId, f32)]) -> InkColor {
    InkColor::Tints(t.to_vec())
}

fn source(inks: InkSet, items: Vec<Item>) -> Vec<u8> {
    let list = DisplayList { page: Rect::new(0.0, 0.0, 100.0, 100.0), inks, items };
    page_pdf(&list, settings(), &CmsConversion::fogra39()).unwrap()
}

fn gold() -> Spot {
    Spot { name: "Gold".into(), alternate: Alternate::Cmyk([0.0, 0.2, 0.8, 0.1]), solidity: None }
}

/// Cyan under everything; magenta overprinting it on the top left; Gold knocking it out on the
/// bottom right; black overprinting with a zero cyan component under OPM 1 on the top right; a
/// registration mark on the bottom left.
fn scene() -> Vec<u8> {
    let mut inks = InkSet::new();
    let gold = inks.add(Ink::Spot(gold())).unwrap();
    let over = Overprint { fill: true, ..Overprint::default() };
    source(
        inks,
        vec![
            fill(Rect::new(0.0, 0.0, 100.0, 100.0), tints(&[(C, 1.0)]), Overprint::default()),
            fill(Rect::new(0.0, 0.0, 50.0, 50.0), tints(&[(M, 0.6)]), over),
            fill(Rect::new(50.0, 50.0, 100.0, 100.0), tints(&[(gold, 0.8)]), Overprint::default()),
            fill(
                Rect::new(60.0, 10.0, 90.0, 40.0),
                tints(&[(C, 0.0), (M, 0.0), (InkId::process(Process::Yellow), 0.0), (K, 1.0)]),
                Overprint { nonzero: true, ..over },
            ),
            fill(Rect::new(10.0, 80.0, 20.0, 90.0), InkColor::All(1.0), Overprint::default()),
        ],
    )
}

const POINTS: [(u32, u32); 5] = [(25, 25), (75, 75), (75, 25), (25, 70), (15, 85)];

fn separate(pdf: &[u8]) -> Plates {
    plates(pdf, 0, 36.0, &Options::default()).unwrap()
}

fn tint(p: &Plates, name: &str, (x, y): (u32, u32)) -> u8 {
    p.inks.lookup(name).and_then(|id| p.tint(id, x / 2, y / 2)).unwrap_or(0)
}

/// `ps` run by craft-ps's interpreter, as a one-page PDF. A plate or composite file of one page
/// is read as an EPS.
fn run(ps: &[u8]) -> Vec<u8> {
    let eps = match ps.strip_prefix(b"%!PS-Adobe-3.0\n") {
        Some(rest) => [b"%!PS-Adobe-3.0 EPSF-3.0\n".as_slice(), rest].concat(),
        None => ps.to_vec(),
    };
    let converted = craft_ps::read::eps_to_pdf(&eps, craft_ps::read::Limits::default()).unwrap();
    assert!(converted.warnings.is_empty(), "{:?}", converted.warnings);
    converted.pdf
}

fn opts(level: PsLevel, data_format: PsDataFormat, color: PsColor) -> PsOptions {
    PsOptions { level, data_format, color, flatten: FlattenOptions { raster_dpi: 72.0, ..FlattenOptions::default() }, ..PsOptions::default() }
}

fn assert_like(got: &Plates, want: &Plates, names: &[&str], points: &[(u32, u32)], what: &str) {
    for name in names {
        for &p in points {
            let (g, w) = (tint(got, name, p), tint(want, name, p));
            assert!(g.abs_diff(w) <= 4, "{what}: {name} at {p:?}: {g}, the source has {w}");
        }
    }
}

/// The points of [`scene`] where neither overprint nor the spot ink decides the plates.
const PLAIN_POINTS: [(u32, u32); 2] = [(25, 70), (15, 85)];

/// craft-ps's interpreter writes device colors only and keeps `setoverprint` out of the PDF it
/// writes: a spot comes back as its CMYK alternate, and everything knocks out. The composite's
/// spot and overprint are read from the program; its plates are compared where they don't
/// depend on them, and where a knockout of the same inks looks the same.
#[test]
fn a_composite_eps_separates_like_its_source() {
    let pdf = scene();
    let want = separate(&pdf);
    for format in [PsDataFormat::Ascii, PsDataFormat::Binary] {
        let (eps, report) = eps_document(&pdf, &opts(PsLevel::Level3, format, PsColor::Composite)).unwrap();
        assert!(eps.bytes.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0\n"));
        assert_eq!(report.approximated_overprints, 0);
        let ps = String::from_utf8_lossy(&eps.bytes);
        assert!(ps.contains("%%DocumentCustomColors: (Gold)"));
        assert!(ps.contains("[/Separation /Gold /DeviceCMYK {dup 0 mul exch dup 0.2 mul exch dup 0.8 mul exch 0.1"));
        // The magenta and the OPM 1 black overprint, each naming only its own ink.
        assert_eq!(ps.matches("q true setoverprint\n").count(), 2);
        assert!(ps.contains("q true setoverprint\n[/Separation /Magenta /DeviceCMYK"));
        assert!(ps.contains("q true setoverprint\n[/Separation /Black /DeviceCMYK"));
        let got = separate(&run(&eps.bytes));
        let what = format!("{format:?}");
        assert_like(&got, &want, &["Cyan", "Magenta", "Yellow", "Black"], &PLAIN_POINTS, &what);
        assert_like(&got, &want, &["Magenta", "Black"], &[(25, 25), (75, 25)], &what);
        assert_like(&got, &want, &["Cyan"], &[(75, 75)], &what);
    }
}

#[test]
fn level_2_writes_overprint_exactly_for_one_ink_and_says_when_it_cannot() {
    let pdf = scene();
    let want = separate(&pdf);
    let (eps, report) = eps_document(&pdf, &opts(PsLevel::Level2, PsDataFormat::Ascii, PsColor::Composite)).unwrap();
    assert_eq!(report.approximated_overprints, 0, "one named ink is a Separation color");
    let ps = String::from_utf8_lossy(&eps.bytes);
    assert!(!ps.contains("DeviceN") && !ps.contains("shfill") && !ps.contains("FlateDecode"));
    assert!(ps.contains("%%LanguageLevel: 2") && ps.contains("[/Separation /Gold /DeviceCMYK"));
    let got = separate(&run(&eps.bytes));
    assert_like(&got, &want, &["Cyan", "Magenta", "Yellow", "Black"], &PLAIN_POINTS, "Level 2");

    // Magenta and yellow overprinting cyan need a DeviceN color.
    let over = Overprint { fill: true, ..Overprint::default() };
    let two = source(
        InkSet::new(),
        vec![
            fill(Rect::new(0.0, 0.0, 100.0, 100.0), tints(&[(C, 1.0)]), Overprint::default()),
            fill(Rect::new(0.0, 0.0, 50.0, 50.0), tints(&[(M, 0.5), (InkId::process(Process::Yellow), 0.5)]), over),
        ],
    );
    let (eps, report) = eps_document(&two, &opts(PsLevel::Level2, PsDataFormat::Ascii, PsColor::Composite)).unwrap();
    assert_eq!(report.approximated_overprints, 1);
    // 50% as the source PDF stores it, 128/255.
    assert!(String::from_utf8_lossy(&eps.bytes).contains("q true setoverprint\n0 0.502 0.502 0 k\n"));
    let (eps, report) = eps_document(&two, &opts(PsLevel::Level3, PsDataFormat::Ascii, PsColor::Composite)).unwrap();
    assert_eq!(report.approximated_overprints, 0);
    assert!(String::from_utf8_lossy(&eps.bytes).contains("[/DeviceN [/Magenta /Yellow] /DeviceCMYK"));
}

#[test]
fn each_plate_file_prints_the_plate_the_separations_preview_shows() {
    let pdf = scene();
    let want = separate(&pdf);
    for level in [PsLevel::Level2, PsLevel::Level3] {
        let (outputs, report) = ps_document(&pdf, &opts(level, PsDataFormat::Ascii, PsColor::Separations)).unwrap();
        let names: Vec<_> = outputs.iter().filter_map(|o| o.plate.as_deref()).collect();
        assert_eq!(names, ["Cyan", "Magenta", "Yellow", "Black", "Gold"]);
        assert_eq!(report.totals.plates, 5);
        for out in &outputs {
            let plate = out.plate.as_deref().unwrap();
            // A plate is gray film: it comes back on the black plate.
            let got = separate(&run(&out.bytes));
            for &p in &POINTS {
                let (g, w) = (tint(&got, "Black", p), tint(&want, plate, p));
                assert!(g.abs_diff(w) <= 4, "{level:?} {plate} at {p:?}: {g}, the source's plate has {w}");
            }
        }
    }
}

/// Cyan rising from left to right, and a CMYK image on the right half.
fn gradient_and_image() -> Vec<u8> {
    let gradient = Paint::Gradient {
        stops: vec![(0.0, tints(&[(C, 0.0)])), (1.0, tints(&[(C, 1.0)]))],
        geometry: GradientGeometry::Axial {
            start: Point::new(0.0, 0.0),
            end: Point::new(100.0, 0.0),
            extend: [true, true],
            transform: Affine::IDENTITY,
        },
    };
    let samples: Vec<u8> = [[0, 255, 0, 0], [0, 0, 255, 0], [0, 0, 0, 255], [0, 128, 128, 0]].concat();
    let image = ImageItem {
        space: ImageSpace::Cmyk,
        size: (2, 2),
        samples: Arc::from(samples),
        transform: Affine::new([40.0, 0.0, 0.0, 40.0, 55.0, 55.0]),
        smask: None,
        overprint: Overprint::default(),
        jpeg: None,
    };
    source(
        InkSet::new(),
        vec![
            Item::Fill(FillItem {
                path: Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.1),
                rule: FillRule::NonZero,
                paint: gradient,
                overprint: Overprint::default(),
            }),
            Item::Image(image),
        ],
    )
}

#[test]
fn gradients_and_images_come_back_at_both_levels_and_in_both_data_formats() {
    let pdf = gradient_and_image();
    let want = separate(&pdf);
    let image_points = [(65, 65), (85, 65), (65, 85), (85, 85)];
    for level in [PsLevel::Level2, PsLevel::Level3] {
        for format in [PsDataFormat::Ascii, PsDataFormat::Binary] {
            let (eps, report) = eps_document(&pdf, &opts(level, format, PsColor::Composite)).unwrap();
            assert_eq!(report.rasterized_gradients > 0, level == PsLevel::Level2);
            let got = separate(&run(&eps.bytes));
            let what = format!("{level:?} {format:?}");
            assert_like(&got, &want, &["Cyan", "Magenta", "Yellow", "Black"], &image_points, &what);
            let cyan: Vec<u8> = [10, 30, 50, 70, 90].iter().map(|x| tint(&got, "Cyan", (*x, 25))).collect();
            assert!(cyan.windows(2).all(|w| w[0] < w[1]), "{what}: cyan rises left to right: {cyan:?}");
            for (x, c) in [10, 30, 50, 70, 90].iter().zip(&cyan) {
                assert!(c.abs_diff(tint(&want, "Cyan", (*x, 25))) <= 12, "{what}: cyan at {x}: {c}");
            }
        }
    }
}

#[test]
fn pages_keep_their_size_and_boxes_and_every_page_is_written() {
    let mut writer = Writer::new(settings(), Metadata { title: Some("Two pages".into()), ..Metadata::default() });
    for (w, h, trim) in [(200.0, 100.0, true), (100.0, 150.0, false)] {
        let mut settings = PageSettings::from_wh(w, h).unwrap();
        if trim {
            settings = settings.with_trim_box(craft_pdf::write::backend::geom::Rect::from_ltrb(10.0, 10.0, w - 10.0, h - 10.0));
        }
        let mut page = writer.document_mut().start_page_with(settings);
        page.surface().finish();
        page.finish();
    }
    let pdf = writer.finish().unwrap();
    let (outputs, report) = ps_document(&pdf, &PsOptions::default()).unwrap();
    assert_eq!(report.totals.pages, 2);
    let ps = String::from_utf8(outputs[0].bytes.clone()).unwrap();
    assert!(ps.starts_with("%!PS-Adobe-3.0\n") && ps.trim_end().ends_with("%%EOF"));
    assert!(ps.contains("%%Pages: 2") && ps.contains("%%Title: Two pages"));
    assert_eq!(ps.matches("%%Page: ").count(), 2);
    assert_eq!(ps.matches("showpage").count(), 2);
    assert!(ps.contains("/PageSize [200 100]") && ps.contains("/PageSize [100 150]"));
    assert!(ps.contains("/TrimBox [10 10 190 90] /PAGE pdfmark"));
    assert_eq!(ps.matches("/PAGE pdfmark").count(), 1, "only page 1 has boxes");
}

#[test]
fn an_eps_preview_sits_behind_a_binary_header() {
    let pdf = scene();
    let opts = PsOptions { preview: Some(PsPreview { dpi: 36.0 }), ..PsOptions::default() };
    let (eps, _) = eps_document(&pdf, &opts).unwrap();
    let sections = craft_eps::sections(&eps.bytes).unwrap();
    assert!(sections.postscript.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"));
    let tiff = sections.tiff.unwrap();
    assert!(tiff.starts_with(b"II*\0"));
    // 50 × 50 pixels of RGB after the header.
    assert!(tiff.len() > 50 * 50 * 3);
    assert!(craft_ps::read::eps_to_pdf(&eps.bytes, craft_ps::read::Limits::default()).is_ok());
}

#[test]
fn options_that_cannot_be_written_are_refused() {
    let pdf = scene();
    let cid2 = PsOptions { fonts: PsFonts::Cid, level: PsLevel::Level2, ..PsOptions::default() };
    assert!(matches!(ps_document(&pdf, &cid2), Err(PsError::Options(_))));
    let separated = PsOptions { color: PsColor::Separations, ..PsOptions::default() };
    assert!(matches!(eps_document(&pdf, &separated), Err(PsError::Options(_))));
    let two = PsOptions { pages: Some(0..2), ..PsOptions::default() };
    assert!(matches!(eps_document(&pdf, &two), Err(PsError::EpsPages(2))));
    assert!(matches!(ps_document(&pdf, &two), Err(PsError::PageRange { .. })));
}

// ---- text ----------------------------------------------------------------------------------

fn inter() -> Option<Vec<u8>> {
    let dir = std::env::var_os(craft_plate_test::reference::FONT_DIR_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/craft-pdf-test-fonts")))?;
    let font = std::fs::read(dir.join(craft_plate_test::reference::FONT_FILE)).ok();
    if font.is_none() {
        eprintln!("skipped: no {} in {}", craft_plate_test::reference::FONT_FILE, dir.display());
    }
    font
}

/// "Hamburg" in black at 30 pt, with `font`.
fn text_page(font_bytes: &[u8]) -> Vec<u8> {
    let face = ttf_parser::Face::parse(font_bytes, 0).unwrap();
    let font = craft_pdf::write::font(font_bytes, 0, None).unwrap();
    let mut writer = Writer::new(settings(), Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
    let mut s = page.surface();
    let upem = f32::from(face.units_per_em());
    let text = "Hamburg";
    let glyphs: Vec<KrillaGlyph> = text
        .char_indices()
        .map(|(i, c)| {
            let id = face.glyph_index(c).unwrap();
            let advance = f32::from(face.glyph_hor_advance(id).unwrap()) / upem;
            KrillaGlyph::new(GlyphId::new(u32::from(id.0)), advance, 0.0, 0.0, 0.0, i..i + c.len_utf8(), None)
        })
        .collect();
    s.set_fill(Some(Fill { paint: craft_pdf::write::backend::color::cmyk::Color::new(0, 0, 0, 255).into(), ..Default::default() }));
    s.draw_glyphs(KPoint::from_xy(5.0, 60.0), &glyphs, font, text, 22.0, false);
    s.finish();
    page.finish();
    writer.finish().unwrap()
}

fn inked(p: &Plates, name: &str) -> usize {
    p.inks.lookup(name).and_then(|id| p.plane(id)).map_or(0, |t| t.iter().filter(|v| **v > 128).count())
}

#[test]
fn truetype_text_stays_text_as_type_42_or_cid_fonts_and_prints_like_the_source() {
    let Some(font) = inter() else { return };
    let pdf = text_page(&font);
    let want = inked(&separate(&pdf), "Black");
    assert!(want > 20);
    for (level, fonts, marker) in [(PsLevel::Level2, PsFonts::TrueType42, "/FontType 42"), (PsLevel::Level3, PsFonts::Cid, "composefont")] {
        let opts = PsOptions { level, fonts, ..PsOptions::default() };
        let (eps, report) = eps_document(&pdf, &opts).unwrap();
        assert_eq!((report.fonts, report.outlined_runs), (1, 0), "{fonts:?}");
        let ps = String::from_utf8_lossy(&eps.bytes);
        assert!(ps.contains(marker) && ps.contains(" S\n"), "{fonts:?} shows glyphs");
        assert!(eps.bytes.len() < font.len(), "{fonts:?}: the font is subset");
        let got = inked(&separate(&run(&eps.bytes)), "Black");
        assert!(got.abs_diff(want) * 5 < want, "{fonts:?}: {got} inked pixels, the source has {want}");
    }
}

// ---- JPEG images ---------------------------------------------------------------------------

/// A 16 × 16 Adobe CMYK JPEG of C 100%, and a 16 × 16 gray JPEG of 25% gray (ImageMagick); the
/// fixtures of the flattener's tests.
const CYAN_JPEG: &[u8] = include_bytes!("../../tests/fixtures/cyan-cmyk.jpg");
const GRAY_JPEG: &[u8] = include_bytes!("../../tests/fixtures/gray-25.jpg");

/// The two JPEGs side by side on a 130 × 70 page, as a PDF that declares the CMYK one inverted.
fn jpeg_page() -> Vec<u8> {
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
    let content = "q 50 0 0 50 10 10 cm /Im1 Do Q q 50 0 0 50 70 10 cm /Im2 Do Q";
    let objects = [image("DeviceCMYK", "/Decode [1 0 1 0 1 0 1 0]", CYAN_JPEG), image("DeviceGray", "", GRAY_JPEG)];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    pdf.extend_from_slice(b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    pdf.extend_from_slice(b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n");
    pdf.extend_from_slice(
        b"3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 130 70] /Contents 4 0 R /Resources << /XObject << /Im1 5 0 R /Im2 6 0 R >> >> >> endobj\n",
    );
    pdf.extend_from_slice(format!("4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n", content.len()).as_bytes());
    for (i, body) in objects.iter().enumerate() {
        pdf.extend_from_slice(format!("{} 0 obj ", i + 5).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b" endobj\n");
    }
    pdf.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF");
    pdf
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack.windows(needle.len()).filter(|w| *w == needle).count()
}

#[test]
fn jpeg_images_keep_their_dct_data_in_a_composite_and_separate_like_the_source() {
    let pdf = jpeg_page();
    let want = separate(&pdf);
    let points = [(35, 35), (95, 35)];
    let inks = ["Cyan", "Magenta", "Yellow", "Black"];
    assert!(tint(&want, "Cyan", (35, 35)) > 240 && tint(&want, "Black", (95, 35)) > 150);
    for level in [PsLevel::Level2, PsLevel::Level3] {
        let (eps, _) = eps_document(&pdf, &opts(level, PsDataFormat::Binary, PsColor::Composite)).unwrap();
        assert_eq!((count(&eps.bytes, CYAN_JPEG), count(&eps.bytes, GRAY_JPEG)), (1, 1), "{level:?}: written as they are");
        assert!(count(&eps.bytes, b"/Decode [1 0 1 0 1 0 1 0]") == 1, "{level:?}: the Adobe CMYK data is inverted");
        assert_like(&separate(&run(&eps.bytes)), &want, &inks, &points, &format!("{level:?}"));
    }
    let (plates, _) = ps_document(&pdf, &opts(PsLevel::Level3, PsDataFormat::Ascii, PsColor::Separations)).unwrap();
    for out in &plates {
        let plate = out.plate.as_deref().unwrap();
        let got = separate(&run(&out.bytes));
        for &p in &points {
            let (g, w) = (tint(&got, "Black", p), tint(&want, plate, p));
            assert!(g.abs_diff(w) <= 4, "{plate} at {p:?}: {g}, the source's plate has {w}");
        }
    }
}
