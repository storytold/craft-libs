//! Text read from a PDF through hayro-interpret and written again with krilla: as live text
//! when the font can be embedded again, as outlines with `/ActualText` otherwise. Covers what
//! the PDF-in adapter relies on: glyph ids, positions, Unicode and plates survive the trip.
//!
//! Fonts not in the repository are read from paths and their cases are skipped when missing:
//! Inter from `CRAFT_PDF_TEST_FONT_DIR`, a CFF-flavored OpenType font from
//! `CRAFT_PREPRESS_CFF_FONT` (default: Debian's `fonts-urw-base35` Nimbus Sans). The Type1C
//! case embeds hayro's Foxit Sans, which is bare CFF.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use craft_pdf::interpret::font::{Glyph, OutlineFontData, StandardFont};
use craft_pdf::interpret::hayro_cmap::BfString;
use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
    interpret_page,
};
use craft_pdf::write::backend::color::cmyk;
use craft_pdf::write::backend::geom::{PathBuilder, Point as KPoint, Transform};
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::Fill;
use craft_pdf::write::backend::surface::Surface;
use craft_pdf::write::backend::tagging::{ContentTag, SpanTag};
use craft_pdf::write::backend::text::{Font, GlyphId, KrillaGlyph};
use craft_pdf::write::backend::{self, Data};
use craft_pdf::write::{Metadata, Writer};
use craft_prepress::flatten::font::FontCache;
use craft_prepress::flatten::kurbo::{Affine, BezPath, PathEl, Rect, Shape};
use craft_prepress::flatten::{PlacedGlyph, TextFont, TextItem};
use craft_prepress::{InkColor, InkId, Options, Overprint, Process, plates};

const W: f64 = 300.0;
const H: f64 = 120.0;
const SAMPLE: &str = "Hamburgefonstiv 0123";

// ---- fonts ---------------------------------------------------------------------------------

fn read_font(path: Option<PathBuf>, what: &str) -> Option<Vec<u8>> {
    let data = path.as_ref().and_then(|p| std::fs::read(p).ok());
    if data.is_none() {
        println!("note: {what} not found at {path:?}; case skipped");
    }
    data
}

fn inter() -> Option<Vec<u8>> {
    read_font(std::env::var_os("CRAFT_PDF_TEST_FONT_DIR").map(|d| PathBuf::from(d).join("Inter-Regular.ttf")), "Inter (CRAFT_PDF_TEST_FONT_DIR)")
}

fn cff_otf() -> Option<Vec<u8>> {
    let path = std::env::var_os("CRAFT_PREPRESS_CFF_FONT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/share/fonts/opentype/urw-base35/NimbusSans-Regular.otf"));
    read_font(Some(path), "a CFF OpenType font (CRAFT_PREPRESS_CFF_FONT)")
}

// ---- writing -------------------------------------------------------------------------------

fn black() -> Fill {
    Fill { paint: cmyk::Color::new(0, 0, 0, 255).into(), ..Default::default() }
}

fn write_page(draw: impl FnOnce(&mut Surface<'_>)) -> Vec<u8> {
    let settings = backend::SerializeSettings { compress_content_streams: false, ..Default::default() };
    let mut writer = Writer::new(settings, Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(W as f32, H as f32).unwrap());
    let mut surface = page.surface();
    draw(&mut surface);
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

/// The source document: `SAMPLE` mapped through the font's cmap (no shaping), once on a
/// baseline and once rotated and scaled.
fn source_pdf(font_bytes: &[u8]) -> Vec<u8> {
    let face = ttf_parser::Face::parse(font_bytes, 0).unwrap();
    let upem = f32::from(face.units_per_em());
    let glyphs: Vec<_> = SAMPLE
        .char_indices()
        .map(|(i, c)| {
            let id = face.glyph_index(c).unwrap();
            let advance = f32::from(face.glyph_hor_advance(id).unwrap()) / upem;
            KrillaGlyph::new(GlyphId::new(u32::from(id.0)), advance, 0.0, 0.0, 0.0, i..i + c.len_utf8(), None)
        })
        .collect();
    let font = craft_pdf::write::font(font_bytes, 0, None).unwrap();
    write_page(|s| {
        s.set_fill(Some(black()));
        s.draw_glyphs(KPoint::from_xy(10.0, 40.0), &glyphs, font.clone(), SAMPLE, 24.0, false);
        s.push_transform(&Transform::from_row(0.96, -0.26, 0.26, 0.96, 20.0, 100.0));
        s.draw_glyphs(KPoint::from_xy(0.0, 0.0), &glyphs, font, SAMPLE, 13.0, false);
        s.pop();
    })
}

fn to_krilla(a: Affine) -> Transform {
    let [a, b, c, d, e, f] = a.as_coeffs().map(|v| v as f32);
    Transform::from_row(a, b, c, d, e, f)
}

fn to_krilla_path(path: &BezPath) -> Option<backend::geom::Path> {
    let mut b = PathBuilder::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(p1, p) => b.quad_to(p1.x as f32, p1.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(p1, p2, p) => b.cubic_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

/// Writes text runs as the flattener's output would: live text through krilla when the run
/// has a font, else its outlines with the Unicode text as `/ActualText`.
fn rewrite(items: &[TextItem]) -> Vec<u8> {
    let mut fonts: HashMap<u128, Font> = HashMap::new();
    write_page(|s| {
        s.set_fill(Some(black()));
        for item in items {
            let text = item.text.as_deref().unwrap_or("");
            match &item.font {
                Some(tf) => {
                    let font = fonts
                        .entry(tf.key)
                        .or_insert_with(|| Font::new(Data::from(tf.data.clone()), tf.index).expect("krilla reads the font"))
                        .clone();
                    let size = item.size as f64;
                    let glyphs: Vec<_> = item
                        .glyphs
                        .iter()
                        .enumerate()
                        .map(|(i, g)| {
                            let next_x = item.glyphs.get(i + 1).map_or(g.origin.x, |n| n.origin.x);
                            let x_advance = ((next_x - g.origin.x) / size) as f32;
                            KrillaGlyph::new(GlyphId::new(g.id), x_advance, 0.0, (g.origin.y / size) as f32, 0.0, g.text.clone(), None)
                        })
                        .collect();
                    // Text space is y up; krilla's glyph placement is y down.
                    s.push_transform(&to_krilla(item.transform * Affine::scale_non_uniform(1.0, -1.0)));
                    s.draw_glyphs(KPoint::from_xy(0.0, 0.0), &glyphs, font, text, item.size, false);
                    s.pop();
                }
                None => {
                    let span = SpanTag { actual_text: Some(text), ..SpanTag::empty() };
                    s.start_tagged(ContentTag::Span(span));
                    if let Some(path) = to_krilla_path(&item.outlines) {
                        s.draw_path(&path);
                    }
                    s.end_tagged();
                }
            }
        }
    })
}

/// A one-page PDF from raw content and resources, objects numbered from 5.
fn raw_pdf(content: &str, resources: &str, objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    out.extend_from_slice(b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    out.extend_from_slice(b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n");
    out.extend_from_slice(
        format!("3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 {W} {H}] /Contents 4 0 R /Resources << {resources} >> >> endobj\n").as_bytes(),
    );
    out.extend_from_slice(format!("4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n", content.len()).as_bytes());
    for (i, body) in objects.iter().enumerate() {
        out.extend_from_slice(format!("{} 0 obj {body} endobj\n", i + 5).as_bytes());
    }
    out.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF");
    out
}

fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >> stream\n{data}\nendstream", data.len())
}

// ---- reading -------------------------------------------------------------------------------

/// What the PDF-in adapter needs from one drawn glyph.
struct Drawn {
    /// The font's key in the [`FontCache`]; `None` for Type3 glyphs.
    key: Option<u128>,
    /// What hayro hands out for the font, for the checks.
    data: Option<OutlineFontData>,
    id: u32,
    /// Glyph space (1000 units per em, y up) to page space.
    matrix: Affine,
    unicode: Option<String>,
    vertical: bool,
    type3: bool,
    outline: BezPath,
}

#[derive(Default)]
struct Recorder {
    glyphs: Vec<Drawn>,
    actual_text: String,
    fonts: FontCache,
}

#[derive(Default)]
struct PathSink(BezPath);

impl Device<'_> for PathSink {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, path: &BezPath, transform: Affine, _: &Paint<'_>, _: &PathDrawMode) {
        self.0.extend((transform * path.clone()).iter());
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn unicode(s: BfString) -> String {
    match s {
        BfString::Char(c) => c.to_string(),
        BfString::String(s) => s,
    }
}

impl<'a> Device<'a> for Recorder {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, glyph_transform: Affine, paint: &Paint<'a>, mode: &GlyphDrawMode) {
        if !matches!(mode, GlyphDrawMode::Fill) {
            return;
        }
        let matrix = transform * glyph_transform;
        let drawn = match glyph {
            Glyph::Outline(o) => Drawn {
                key: Some(self.fonts.note(o)),
                data: o.font_data(),
                id: o.glyph_id().to_u32(),
                matrix,
                unicode: o.as_unicode().map(unicode),
                vertical: o.is_vertical(),
                type3: false,
                outline: matrix * o.outline(),
            },
            Glyph::Type3(t) => {
                let mut sink = PathSink::default();
                t.interpret(&mut sink, transform, glyph_transform, paint);
                Drawn { key: None, data: None, id: 0, matrix, unicode: t.as_unicode().map(unicode), vertical: false, type3: true, outline: sink.0 }
            }
        };
        self.glyphs.push(drawn);
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn actual_text(&mut self, text: &[u8]) {
        self.actual_text.push_str(&pdf_text_string(text));
    }
}

fn pdf_text_string(bytes: &[u8]) -> String {
    match bytes {
        [0xFE, 0xFF, rest @ ..] => {
            let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
            String::from_utf16_lossy(&units)
        }
        _ => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

fn read(pdf: &[u8]) -> Recorder {
    let doc = craft_pdf::syntax::Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let page = &doc.pages()[0];
    let cache = InterpreterCache::new();
    let mut context =
        Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, W, H), &cache, page.xref(), InterpreterSettings::default());
    let mut recorder = Recorder::default();
    interpret_page(page, &mut context, &mut recorder);
    recorder
}

// ---- runs, as the adapter builds them ------------------------------------------------------

fn same_linear(a: Affine, b: Affine) -> bool {
    let (a, b) = (a.as_coeffs(), b.as_coeffs());
    let scale = a[..4].iter().fold(0.0f64, |m, v| m.max(v.abs()));
    a[..4].iter().zip(&b[..4]).all(|(x, y)| (x - y).abs() <= scale * 1e-6)
}

/// Groups drawn glyphs into runs: one font and one text matrix (up to translation) each.
fn text_items(drawn: &[Drawn], fonts: &HashMap<u128, Option<Arc<TextFont>>>) -> Vec<TextItem> {
    let mut items: Vec<TextItem> = Vec::new();
    for d in drawn {
        let font = d.key.and_then(|key| fonts[&key].clone());
        let linear = d.matrix * Affine::scale(1000.0);
        let continues = items
            .last()
            .is_some_and(|last| last.font == font && last.font.is_some() && same_linear(last.transform * Affine::scale(last.size as f64), linear));
        if !continues {
            let size = linear.determinant().abs().sqrt();
            let origin = d.matrix.translation();
            let [a, b, c, dd, ..] = linear.as_coeffs();
            items.push(TextItem {
                font: font.clone(),
                size: size as f32,
                transform: Affine::new([a / size, b / size, c / size, dd / size, origin.x, origin.y]),
                glyphs: Vec::new(),
                text: Some(String::new()),
                outlines: BezPath::new(),
                paint: craft_prepress::flatten::Paint::Solid(InkColor::Tints(vec![(InkId::process(Process::Black), 1.0)])),
                overprint: Overprint::default(),
            });
        }
        let item = items.last_mut().unwrap();
        let text = item.text.as_mut().unwrap();
        let start = text.len();
        text.push_str(d.unicode.as_deref().unwrap_or(""));
        let origin = item.transform.inverse() * d.matrix.translation().to_point();
        item.glyphs.push(PlacedGlyph { id: d.id, origin, text: start..text.len() });
        item.outlines.extend(d.outline.iter());
    }
    items
}

// ---- checks --------------------------------------------------------------------------------

fn black_plate(pdf: &[u8]) -> Vec<u8> {
    let p = plates(pdf, 0, 72.0, &Options::default()).unwrap();
    p.plane(InkId::process(Process::Black)).unwrap().to_vec()
}

/// Mean and largest per-pixel difference of the black plates.
fn plate_difference(a: &[u8], b: &[u8]) -> (f64, u8) {
    assert_eq!(a.len(), b.len());
    let max = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
    let mean = a.iter().zip(b).map(|(x, y)| f64::from(x.abs_diff(*y))).sum::<f64>() / a.len() as f64;
    (mean, max)
}

fn text_of(r: &Recorder) -> String {
    r.glyphs.iter().filter_map(|g| g.unicode.as_deref()).collect()
}

/// Reads `source`, writes it again from the runs, and checks positions, outlines, text and
/// plates. Returns the runs, the source and the rewrite as read.
fn round_trip(source: &[u8]) -> (Vec<TextItem>, Recorder, Recorder) {
    let mut before = read(source);
    assert!(!before.glyphs.is_empty());
    let items = text_items(&before.glyphs, &std::mem::take(&mut before.fonts).finish());
    let pdf = rewrite(&items);
    let after = read(&pdf);

    let (mean, max) = plate_difference(&black_plate(source), &black_plate(&pdf));
    println!("black plates: mean difference {mean:.4}, largest {max}");
    assert!(mean < 0.01 && max <= 4, "plates differ: mean {mean}, max {max}");

    if items.iter().all(|i| i.font.is_some()) {
        assert_eq!(after.glyphs.len(), before.glyphs.len());
        for (b, a) in before.glyphs.iter().zip(&after.glyphs) {
            let (pa, pb) = (a.matrix.translation(), b.matrix.translation());
            assert!((pa - pb).hypot() < 1e-3, "glyph origin {pa:?} vs {pb:?}");
            assert!(same_linear(a.matrix, b.matrix));
            let (ba, bb) = (a.outline.bounding_box(), b.outline.bounding_box());
            assert!((ba.min_x() - bb.min_x()).abs() < 1e-3 && (ba.max_y() - bb.max_y()).abs() < 1e-3, "outline {ba:?} vs {bb:?}");
        }
        assert_eq!(text_of(&after), text_of(&before), "Unicode survives");
    } else {
        assert_eq!(after.actual_text, text_of(&before), "Unicode survives as ActualText");
    }
    (items, before, after)
}

/// krilla keeps glyph ids when the source is one of its own subsets: those are compact in
/// first-use order already.
fn assert_same_ids(before: &Recorder, after: &Recorder) {
    assert!(before.glyphs.iter().zip(&after.glyphs).all(|(b, a)| a.id == b.id), "glyph ids match");
}

/// Whether the rewrite has the text as live text in an embedded CFF font.
fn has_live_cff(after: &Recorder) -> bool {
    after.glyphs.iter().all(|g| g.data.as_ref().is_some_and(|d| d.embedded && (*d.data).as_ref().starts_with(&[1])))
}

#[test]
fn truetype_text_reembeds_as_live_text() {
    let Some(font) = inter() else { return };
    let source = source_pdf(&font);
    let (items, before, after) = round_trip(&source);
    assert_eq!(items.len(), 2, "two runs: one per text matrix");
    assert!(items.iter().all(|i| i.font.is_some()));
    assert_same_ids(&before, &after);
    assert_eq!(text_of(&after), SAMPLE.repeat(2));
}

#[test]
fn cff_text_reembeds_after_wrapping_the_bare_cff() {
    let Some(font) = cff_otf() else { return };
    let source = source_pdf(&font);
    assert!(String::from_utf8_lossy(&source).contains("/CIDFontType0C"), "krilla writes CFF as bare CIDFontType0C");
    let (items, before, after) = round_trip(&source);
    let data = before.glyphs[0].data.as_ref().unwrap().data.clone();
    assert_eq!((*data).as_ref()[0], 1, "hayro hands out the bare CFF");
    assert!(Font::new(Data::from(data), 0).is_none(), "krilla needs an OpenType wrapper");
    assert!(items.iter().all(|i| i.font.is_some()));
    assert!(has_live_cff(&after), "the rewrite embeds CFF again, not outlines");
    assert_same_ids(&before, &after);
    assert_eq!(text_of(&after), SAMPLE.repeat(2));
}

const TYPE3_CMAP: &str = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T3 def \
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def 1 begincodespacerange <00> <FF> endcodespacerange \
2 beginbfchar <41> <0041> <42> <0042> endbfchar endcmap CMapName currentdict /CMapResource defineresource pop end end";

#[test]
fn type3_text_falls_back_to_outlines_with_actual_text() {
    let font = "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] \
        /CharProcs << /a 6 0 R /b 7 0 R >> /Encoding << /Type /Encoding /Differences [65 /a /b] >> \
        /FirstChar 65 /LastChar 66 /Widths [1000 1000] /Resources << >> /ToUnicode 8 0 R >>";
    let objects = [
        font.to_owned(),
        stream("", "1000 0 0 0 800 800 d1 0 0 800 800 re f"),
        stream("", "1000 0 0 0 800 800 d1 0 0 m 800 0 l 400 800 l f"),
        stream("", TYPE3_CMAP),
    ];
    let source = raw_pdf("0 0 0 1 k BT /F1 30 Tf 10 30 Td (ABBA) Tj ET", "/Font << /F1 5 0 R >>", &objects);
    let (items, before, _) = round_trip(&source);
    assert!(before.glyphs.iter().all(|g| g.type3 && g.key.is_none()));
    assert!(items.iter().all(|i| i.font.is_none()));
}

#[test]
fn standard_14_text_has_no_font_bytes_and_falls_back_to_outlines() {
    let source = raw_pdf(
        "0 0 0 1 k BT /F1 24 Tf 10 40 Td (Hamburg) Tj ET",
        "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> >>",
        &[],
    );
    let (items, before, _) = round_trip(&source);
    assert!(before.glyphs.iter().all(|g| !g.type3 && g.data.is_none()), "hayro returns no bytes for standard fonts");
    assert!(items.iter().all(|i| i.font.is_none()));
    assert_eq!(text_of(&before), "Hamburg");
}

#[test]
fn vertical_text_reembeds_as_horizontal_glyphs_at_their_positions() {
    let Some(font) = cff_otf().or_else(inter) else { return };
    let mut source = source_pdf(&font);
    let at: Vec<usize> = source.windows(11).enumerate().filter(|(_, w)| *w == b"/Identity-H").map(|(i, _)| i).collect();
    assert_eq!(at.len(), 1, "one Type0 font");
    // Same length, so the cross-reference offsets stay valid.
    source[at[0] + 10] = b'V';
    let (items, before, after) = round_trip(&source);
    assert!(before.glyphs.iter().all(|g| g.vertical));
    assert!(items.iter().all(|i| i.font.is_some()));
    assert_same_ids(&before, &after);
    assert!(after.glyphs.iter().all(|g| !g.vertical), "krilla writes horizontal fonts only");
}

const IDENTITY_CMAP: &str = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /U def \
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def 1 begincodespacerange <0000> <FFFF> endcodespacerange \
2 beginbfchar <0041> <0041> <0042> <0042> endbfchar endcmap CMapName currentdict /CMapResource defineresource pop end end";

#[test]
fn a_non_embedded_cid_font_falls_back_to_outlines() {
    let objects = [
        "<< /Type /Font /Subtype /Type0 /BaseFont /NotEmbedded /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 8 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /NotEmbedded /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
            /FontDescriptor 7 0 R >>"
            .to_owned(),
        "<< /Type /FontDescriptor /FontName /NotEmbedded /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 \
            /CapHeight 700 /StemV 80 >>"
            .to_owned(),
        stream("", IDENTITY_CMAP),
    ];
    let source = raw_pdf("0 0 0 1 k BT /F1 24 Tf 10 40 Td <00410042> Tj ET", "/Font << /F1 5 0 R >>", &objects);
    let (items, before, _) = round_trip(&source);
    // hayro hands out the font resolver's substitute, marked as not embedded.
    assert!(before.glyphs.iter().all(|g| g.data.as_ref().is_some_and(|d| !d.embedded)));
    assert!(items.iter().all(|i| i.font.is_none()));
    assert_eq!(text_of(&before), "AB");
}

/// A simple font with `FontFile3/Type1C`: hayro's Foxit Sans substitute, which is bare CFF,
/// hex-encoded so the PDF stays text. The widths are made up; positions follow them.
fn type1c_pdf() -> Vec<u8> {
    let (data, _) = StandardFont::Helvetica.get_font_data();
    let hex: String = (*data).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let widths: Vec<String> = (32..=126).map(|c| (400 + c % 7 * 50).to_string()).collect();
    let objects = [
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /ChromSansOTF /FirstChar 32 /LastChar 126 /Widths [{}] \
            /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R >>",
            widths.join(" ")
        ),
        "<< /Type /FontDescriptor /FontName /ChromSansOTF /Flags 32 /FontBBox [-166 -225 1000 931] /ItalicAngle 0 /Ascent 718 \
            /Descent -207 /CapHeight 718 /StemV 88 /FontFile3 7 0 R >>"
            .to_owned(),
        stream("/Subtype /Type1C /Filter /ASCIIHexDecode", &format!("{hex}>")),
    ];
    let content = format!("0 0 0 1 k BT /F1 24 Tf 10 40 Td ({SAMPLE}) Tj ET BT /F1 13 Tf 0.96 0.26 -0.26 0.96 20 80 Tm ({SAMPLE}) Tj ET");
    raw_pdf(&content, "/Font << /F1 5 0 R >>", &objects)
}

#[test]
fn type1c_text_reembeds_after_wrapping_the_cff() {
    let source = type1c_pdf();
    let (items, before, after) = round_trip(&source);
    assert!(before.glyphs.iter().all(|g| g.data.as_ref().is_some_and(|d| d.embedded && (*d.data).as_ref()[0] == 1)), "bare CFF from the file");
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i.font.is_some()));
    assert!(has_live_cff(&after), "the rewrite embeds CFF again, not outlines");
    assert_eq!(text_of(&after), SAMPLE.repeat(2));
}
