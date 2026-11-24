//! Text contracts adapted from PdfCraft #1477, with explicit-encoding precedence checks.
#![cfg(feature = "interpret")]
#![allow(clippy::unwrap_used, clippy::panic)]

use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
    font::Glyph,
};
use hayro_cmap::BfString;
use kurbo::{Affine, BezPath, Rect};

#[cfg(craft_pdf_unicode_fixtures)]
const INTER: &[u8] = include_bytes!(concat!(env!("CRAFT_PDF_TEST_FONT_DIR"), "/Inter-Regular.ttf"));

#[derive(Default)]
struct TextDevice {
    text: String,
    embedded_glyphs: usize,
    glyph_ids: Vec<u32>,
}

impl Device<'_> for TextDevice {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {
        let Glyph::Outline(outline) = glyph else { panic!("expected an embedded TrueType font") };
        assert!(outline.font_data().unwrap().data.as_ref().as_ref() == fixture_font(), "a fallback font must not satisfy this contract");
        self.embedded_glyphs += 1;
        self.glyph_ids.push(outline.glyph_id().to_u32());
        match glyph.as_unicode() {
            Some(BfString::Char(c)) => self.text.push(c),
            Some(BfString::String(s)) => self.text.push_str(&s),
            None => self.text.push('\u{fffd}'),
        }
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn fixture_font() -> &'static [u8] {
    #[cfg(craft_pdf_unicode_fixtures)]
    {
        INTER
    }
    #[cfg(not(craft_pdf_unicode_fixtures))]
    {
        panic!("enable craft_pdf_unicode_fixtures with the hash-verified fonts.json fixtures")
    }
}

fn interpreted_text(flags: u8, encoding: &str, unicode_map: Option<&str>, codes: &[u8]) -> TextDevice {
    let font = fixture_font();
    let mut content = b"BT /F1 14 Tf 1 0 0 1 20 200 Tm (".to_vec();
    content.extend_from_slice(codes);
    content.extend_from_slice(b") Tj ET");
    let mut pdf = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 600 400] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n\
         4 0 obj << /Length {} >> stream\n",
        content.len()
    )
    .into_bytes();
    pdf.extend_from_slice(&content);
    pdf.extend_from_slice(
        format!(
            "\nendstream endobj\n5 0 obj << /Type /Font /Subtype /TrueType /BaseFont /Inter-Regular /FontDescriptor 6 0 R {encoding} {} >> endobj\n\
             6 0 obj << /Type /FontDescriptor /FontName /Inter-Regular /Flags {flags} /FontFile2 7 0 R >> endobj\n\
             7 0 obj << /Length {} >> stream\n",
            if unicode_map.is_some() { "/ToUnicode 8 0 R" } else { "" },
            font.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(font);
    pdf.extend_from_slice(b"\nendstream endobj\n");
    if let Some(map) = unicode_map {
        let cmap = format!(
            "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Test def /CMapType 2 def \
             1 begincodespacerange <00> <FF> endcodespacerange {map} endcmap CMapName currentdict /CMap defineresource pop end end"
        );
        pdf.extend_from_slice(format!("8 0 obj << /Length {} >> stream\n{cmap}\nendstream endobj\n", cmap.len()).as_bytes());
    }
    pdf.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF\n");
    let pdf = craft_pdf::open_slice(&pdf, None).unwrap();
    let page = &pdf.pages()[0];
    let cache = InterpreterCache::new();
    let mut context =
        Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 600.0, 400.0), &cache, page.xref(), InterpreterSettings::default());
    let mut device = TextDevice::default();
    craft_pdf::interpret_page(page, &mut context, &mut device).unwrap();
    assert_eq!(device.embedded_glyphs, codes.len());
    device
}

fn encoded_text(flags: u8, encoding: &str, unicode_map: Option<&str>, codes: &[u8]) -> String {
    interpreted_text(flags, encoding, unicode_map, codes).text
}

#[cfg_attr(not(craft_pdf_unicode_fixtures), ignore = "requires hash-verified external fonts and craft_pdf_unicode_fixtures cfg")]
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn truetype_without_encoding_reads_win_ansi_even_with_partial_to_unicode() {
    let codes = b"R\xfcckfragen f\xfcr \xdcberhitzungsschutz: k\xf6nnen, wei\xdf, Caf\xe9";
    let expected = "Rückfragen für Überhitzungsschutz: können, weiß, Café";
    let ascii_map = "1 beginbfrange <20> <7E> <0020> endbfrange";
    assert_eq!(encoded_text(4, "", None, codes), expected, "symbolic, no ToUnicode");
    assert_eq!(encoded_text(32, "", Some(ascii_map), codes), expected, "nonsymbolic, ASCII-only ToUnicode");
}

#[cfg_attr(not(craft_pdf_unicode_fixtures), ignore = "requires hash-verified external fonts and craft_pdf_unicode_fixtures cfg")]
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn explicit_encoding_differences_and_to_unicode_override_the_fallback() {
    for flags in [4, 32] {
        assert_eq!(encoded_text(flags, "/Encoding /StandardEncoding", None, b"\xe9"), "Ø");
        assert_eq!(encoded_text(flags, "/Encoding /WinAnsiEncoding", None, b"\xe9"), "é");
        let differences = "/Encoding << /BaseEncoding /StandardEncoding /Differences [233 /C] >>";
        assert_eq!(encoded_text(flags, differences, None, b"\xe9"), "C");
        let map = "1 beginbfchar <E9> <03A9> endbfchar";
        assert_eq!(encoded_text(flags, differences, Some(map), b"\xe9"), "Ω");
    }
}

#[cfg_attr(not(craft_pdf_unicode_fixtures), ignore = "requires hash-verified external fonts and craft_pdf_unicode_fixtures cfg")]
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn unicode_fallback_preserves_the_existing_nonsymbolic_glyph_selection() {
    let fallback = interpreted_text(32, "", None, b"\xe9\xfc");
    let standard = interpreted_text(32, "/Encoding /StandardEncoding", None, b"\xe9\xfc");
    let ansi = interpreted_text(32, "/Encoding /WinAnsiEncoding", None, b"\xe9\xfc");
    assert_eq!(fallback.text, "éü");
    // The upstream port repairs extraction only. With no encoding, drawing still uses
    // StandardEncoding: Oslash for 0xE9 and .notdef for 0xFC. Do not claim a rendering fix.
    assert_eq!(fallback.glyph_ids, standard.glyph_ids);
    assert_ne!(fallback.glyph_ids[0], 0);
    assert_eq!(fallback.glyph_ids[1], 0);
    assert_eq!(ansi.text, "éü");
    assert!(ansi.glyph_ids.iter().all(|id| *id != 0));
    assert_ne!(fallback.glyph_ids[0], ansi.glyph_ids[0]);
}

#[cfg_attr(not(craft_pdf_unicode_fixtures), ignore = "requires hash-verified external fonts and craft_pdf_unicode_fixtures cfg")]
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn unicode_fallback_uses_the_upstream_win_ansi_bullet_mapping() {
    // The upstream fallback uses the complete WinAnsi mapping, including its unused-code
    // bullet rule. This is a fallback policy, not proof of the original author's text.
    for flags in [4, 32] {
        assert_eq!(encoded_text(flags, "", None, b"\x81"), "•");
        assert_eq!(encoded_text(flags, "/Encoding /WinAnsiEncoding", None, b"\x81"), "•");
    }
}
