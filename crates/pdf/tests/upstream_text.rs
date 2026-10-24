//! Text contracts adapted from PdfCraft a2ec27b4775f53c40cad3456936d9406611e3a8d (MIT OR Apache-2.0):
//! Annex D.2 alias codes, glyph names outside the Adobe Glyph List, and `/ToUnicode` identity names.
#![cfg(feature = "interpret")]
#![allow(clippy::unwrap_used, clippy::panic)]

use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
    font::Glyph,
};
use hayro_cmap::BfString;
use kurbo::{Affine, BezPath, Rect};

/// The text of each glyph drawn, `None` where the font gives none.
#[derive(Default)]
struct TextDevice {
    glyphs: Vec<Option<String>>,
}

impl Device<'_> for TextDevice {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {
        self.glyphs.push(glyph.as_unicode().map(|text| match text {
            BfString::Char(c) => c.to_string(),
            BfString::String(s) => s,
        }));
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The glyph texts of a one-page PDF whose font `/F1` is object 5; `extra` holds further objects.
fn glyph_texts(font: &str, extra: &str, content: &[u8]) -> Vec<Option<String>> {
    let mut pdf = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n\
         4 0 obj << /Length {} >> stream\n",
        content.len()
    )
    .into_bytes();
    pdf.extend_from_slice(content);
    pdf.extend_from_slice(format!("\nendstream endobj\n5 0 obj {font} endobj\n{extra}trailer << /Root 1 0 R >>\n%%EOF\n").as_bytes());
    let pdf = craft_pdf::open_slice(&pdf, None).unwrap();
    let page = &pdf.pages()[0];
    let cache = InterpreterCache::new();
    let mut context =
        Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 300.0, 300.0), &cache, page.xref(), InterpreterSettings::default());
    let mut device = TextDevice::default();
    craft_pdf::interpret_page(page, &mut context, &mut device).unwrap();
    device.glyphs
}

fn helvetica_texts(encoding: &str, content: &[u8]) -> Vec<String> {
    let font = format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding {encoding} >>");
    glyph_texts(&font, "", content).into_iter().map(|text| text.unwrap_or_default()).collect()
}

/// PdfCraft ledger hayro-interpret (30): WinAnsiEncoding gives 0xAD and 0xA0 the glyph names
/// `hyphen` and `space`, and MacRomanEncoding gives 0xCA `space` (Annex D.2), so they read as the
/// ordinary hyphen and space rather than U+00AD and U+00A0. A `/Differences` entry naming
/// `sfthyphen` or `nbspace` keeps its glyph list value.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_annex_d2_alias_codes_read_as_an_ordinary_hyphen_and_space() {
    assert_eq!(
        helvetica_texts("/WinAnsiEncoding", b"BT /F1 20 Tf 20 150 Td (a\xadb\xa0c) Tj ET"),
        ["a", "-", "b", " ", "c"],
        "WinAnsi 0xAD and 0xA0"
    );
    assert_eq!(helvetica_texts("/MacRomanEncoding", b"BT /F1 20 Tf 20 150 Td (a\xcab) Tj ET"), ["a", " ", "b"], "MacRoman 0xCA");
    assert_eq!(
        helvetica_texts(
            "<< /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [88 /sfthyphen 89 /nbspace] >>",
            b"BT /F1 20 Tf 20 150 Td (aXbYc) Tj ET"
        ),
        ["a", "\u{ad}", "b", "\u{a0}", "c"],
        "a named glyph keeps its glyph list value"
    );
}

/// PdfCraft ledger hayro-interpret P6 (#3169, #3170): ligature (`f_i`) and suffixed
/// (`hyphen.case`) names read by the glyph list specification's full mapping. A name that only
/// numbers the glyph (`C39`, `c251`), a ZapfDingbats name (`a12`) and a name with no meaning
/// left (`foo`) read as nothing.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn glyph_names_outside_the_glyph_list_still_read_as_text() {
    let encoding = "<< /Type /Encoding /Differences [30 /f_i /f_l 2 /hyphen.case 39 /C39 97 /c251 174 /C174 \
                    88 /a12 /foo /uni00660069 61 /C39.sc 233 /C1] >>";
    let texts = helvetica_texts(encoding, b"BT /F1 20 Tf 20 150 Td (\x1e\x1f\x02'a\xaeXYZ=\xe9) Tj ET");
    assert_eq!(texts, ["fi", "fl", "-", "", "", "", "", "", "fi", "", ""]);
}

/// PdfCraft ledger hayro-interpret P6: the `uni` and `u` forms take uppercase hex digits only.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn glyph_names_with_lowercase_hex_read_as_nothing() {
    let encoding = "<< /Type /Encoding /Differences [65 /uni20ac0308 /uni20AC0308 /u1040c.alt /u1040C.alt /f_uni20ac /f_uni20AC] >>";
    let texts = helvetica_texts(encoding, b"BT /F1 20 Tf 20 150 Td (ABCDEF) Tj ET");
    assert_eq!(texts, ["", "\u{20ac}\u{308}", "", "\u{1040c}", "f", "f\u{20ac}"]);
}

/// PdfCraft ledger hayro-interpret (44): `/ToUnicode /Identity-H` (a name, not a CMap stream)
/// reads two-byte codes as their Unicode values. Controls (a DEL and a C1 control) are not text.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn identity_to_unicode_names_give_text() {
    for name in ["Identity-H", "Identity-V"] {
        let font = format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /RobotoMono-Regular /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode /{name} >>"
        );
        let cid = "6 0 obj << /Type /Font /Subtype /CIDFontType2 /BaseFont /RobotoMono-Regular \
                   /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 600 /CIDToGIDMap /Identity >> endobj\n";
        let texts = glyph_texts(&font, cid, b"BT /F1 12 Tf 20 200 Td <005A00FC0072006900630068002020AC007F0085> Tj ET");
        let texts: Vec<String> = texts.into_iter().map(|text| text.unwrap_or_default()).collect();
        assert_eq!(texts, ["Z", "\u{fc}", "r", "i", "c", "h", " ", "\u{20ac}", "", ""], "{name}");
    }
}
