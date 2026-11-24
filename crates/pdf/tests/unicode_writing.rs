//! Opt-in contracts with externally supplied, licensed fonts; see the crate README.
#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::panic)]

use backend::geom::Point;
use backend::page::PageSettings;
use backend::text::{GlyphId, KrillaGlyph};
use craft_pdf::write::{self, Metadata, Writer, backend};
use hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
    font::Glyph,
};
use kurbo::{Affine, BezPath, Rect};

#[cfg(craft_pdf_unicode_fixtures)]
const INTER: &[u8] = include_bytes!(concat!(env!("CRAFT_PDF_TEST_FONT_DIR"), "/Inter-Regular.ttf"));
#[cfg(craft_pdf_unicode_fixtures)]
const ARABIC: &[u8] = include_bytes!(concat!(env!("CRAFT_PDF_TEST_FONT_DIR"), "/NotoSansArabic.ttf"));
#[cfg(craft_pdf_unicode_fixtures)]
const THAI: &[u8] = include_bytes!(concat!(env!("CRAFT_PDF_TEST_FONT_DIR"), "/NotoSansThai[wdth,wght].ttf"));

#[derive(Default)]
struct TextDevice {
    text: String,
    visible_glyphs: usize,
    outlines: Vec<BezPath>,
}

impl Device<'_> for TextDevice {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {
        let Glyph::Outline(outline) = glyph else { panic!("expected an embedded outline font") };
        assert_ne!(outline.glyph_id().to_u32(), 0, "reopened page must not draw .notdef");
        assert!(outline.font_data().is_some(), "expected an embedded font");
        let text = match glyph.as_unicode().unwrap() {
            BfString::Char(c) => c.to_string(),
            BfString::String(s) => s,
        };
        if !text.chars().all(char::is_whitespace) {
            assert!(!outline.outline().elements().is_empty(), "non-space text must have a visible outline");
            self.visible_glyphs += 1;
            self.outlines.push(outline.outline());
        }
        self.text.push_str(&text);
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn mapped_document(font_bytes: &[u8], text: &str, variations: Option<&[([u8; 4], f32)]>) -> Vec<u8> {
    let mut face = ttf_parser::Face::parse(font_bytes, 0).unwrap();
    if let Some(coords) = variations {
        for (tag, value) in coords {
            face.set_variation(ttf_parser::Tag::from_bytes(tag), *value).unwrap();
        }
    }
    // Deliberately test explicit scalar-to-glyph mappings, not shaping or bidi layout.
    // Applications own those operations and supply the final glyph run to the writer.
    let glyphs: Vec<_> = text
        .char_indices()
        .map(|(start, c)| {
            let id = face.glyph_index(c).unwrap();
            assert_ne!(id.0, 0, "fixture font must cover {c}");
            let advance = f32::from(face.glyph_hor_advance(id).unwrap()) / f32::from(face.units_per_em());
            KrillaGlyph::new(GlyphId::new(u32::from(id.0)), advance, 0.0, 0.0, 0.0, start..start + c.len_utf8(), None)
        })
        .collect();
    let font = write::font(font_bytes, 0, variations).unwrap();
    let settings = backend::SerializeSettings { compress_content_streams: false, ..Default::default() };
    // Do not put the source string in metadata: only real page text may satisfy the assertion.
    let mut writer = Writer::new(settings, Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(600.0, 100.0).unwrap());
    let mut surface = page.surface();
    surface.draw_glyphs(Point::from_xy(20.0, 50.0), &glyphs, font, text, 18.0, false);
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

#[cfg_attr(not(craft_pdf_unicode_fixtures), ignore = "requires hash-verified external fonts and craft_pdf_unicode_fixtures cfg")]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn embedded_unicode_mappings_survive_serialization_and_reopen_as_page_text() {
    let [inter, arabic, thai] = fixture_fonts();
    for (font, text) in [(inter, "Café Ελληνικά Русский Tiếng Việt"), (arabic, "اللغة العربية"), (thai, "ภาษาไทย")]
    {
        let bytes = mapped_document(font, text, None);
        assert!(String::from_utf8_lossy(&bytes).contains("/ToUnicode"));
        let device = reopened_text(&bytes);
        assert_eq!(device.text, text, "saved page text must preserve every requested character");
        assert_eq!(device.visible_glyphs, text.chars().filter(|c| !c.is_whitespace()).count());
    }
}

fn reopened_text(bytes: &[u8]) -> TextDevice {
    let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(bytes.to_vec())).unwrap();
    let page = &pdf.pages()[0];
    let cache = InterpreterCache::new();
    let mut context =
        Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 600.0, 100.0), &cache, page.xref(), InterpreterSettings::default());
    let mut device = TextDevice::default();
    hayro_interpret::interpret_page(page, &mut context, &mut device);
    device
}

#[cfg_attr(not(craft_pdf_unicode_fixtures), ignore = "requires hash-verified external fonts and craft_pdf_unicode_fixtures cfg")]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn variable_coordinates_change_embedded_outlines_and_preserve_real_text() {
    let [_, _, thai] = fixture_fonts();
    let text = "ภาษาไทย";
    let default = reopened_text(&mapped_document(thai, text, None));
    let empty = reopened_text(&mapped_document(thai, text, Some(&[])));
    let light = reopened_text(&mapped_document(thai, text, Some(&[(*b"wght", 100.0)])));
    let bold = reopened_text(&mapped_document(thai, text, Some(&[(*b"wght", 900.0)])));
    for device in [&default, &empty, &light, &bold] {
        assert_eq!(device.text, text);
        assert_eq!(device.visible_glyphs, text.chars().count());
    }
    assert_eq!(default.outlines, empty.outlines, "empty coordinates preserve the default instance");
    assert_ne!(light.outlines, bold.outlines, "weight coordinates must affect the embedded glyph outlines");
}

fn fixture_fonts() -> [&'static [u8]; 3] {
    #[cfg(craft_pdf_unicode_fixtures)]
    {
        [INTER, ARABIC, THAI]
    }
    #[cfg(not(craft_pdf_unicode_fixtures))]
    {
        panic!("enable craft_pdf_unicode_fixtures and supply CRAFT_PDF_TEST_FONT_DIR to run this contract")
    }
}
