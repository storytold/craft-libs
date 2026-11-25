//! Text contracts adapted from PdfCraft a2ec27b4775f53c40cad3456936d9406611e3a8d (MIT OR Apache-2.0):
//! Annex D.2 alias codes, glyph names outside the Adobe Glyph List, and `/ToUnicode` identity names;
//! and from PdfCraft 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e, clipping-only text.
#![cfg(feature = "interpret")]
#![allow(clippy::unwrap_used, clippy::panic)]

use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
    font::Glyph,
};
use hayro_cmap::BfString;
use kurbo::{Affine, BezPath, Rect};

/// The text of each glyph drawn, `None` where the font gives none, and how it was drawn.
#[derive(Default)]
struct TextDevice {
    glyphs: Vec<Option<String>>,
    modes: Vec<char>,
}

impl Device<'_> for TextDevice {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
        self.modes.push(match mode {
            GlyphDrawMode::Fill => 'F',
            GlyphDrawMode::Stroke(_) => 'S',
            GlyphDrawMode::Clip => 'C',
            GlyphDrawMode::Invisible => 'I',
        });
        self.glyphs.push(glyph.as_unicode().map(|text| match text {
            BfString::Char(c) => c.to_string(),
            BfString::String(s) => s,
        }));
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The glyphs a one-page PDF draws with its font `/F1`, object 5; `extra` holds further objects.
fn drawn_glyphs(font: &str, extra: &str, content: &[u8]) -> TextDevice {
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
    device
}

fn glyph_texts(font: &str, extra: &str, content: &[u8]) -> Vec<Option<String>> {
    drawn_glyphs(font, extra, content).glyphs
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

/// PdfCraft #3109 / #3116: text in mode 7 (clip only) reaches the device as
/// `GlyphDrawMode::Clip`, apart from mode 3 (invisible OCR text), so extraction that leaves out
/// OCR text keeps a heading drawn through its clip.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn clipping_only_text_reaches_the_device_apart_from_invisible_text() {
    let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";
    for (mode, expected) in [(0, 'F'), (3, 'I'), (7, 'C')] {
        let content = format!("q BT /F1 24 Tf {mode} Tr 30 200 Td (Hi) Tj ET 0 0 1 rg 0 0 300 300 re f Q");
        let drawn = drawn_glyphs(font, "", content.as_bytes());
        assert_eq!(drawn.glyphs, [Some("H".into()), Some("i".into())], "{mode} Tr");
        assert_eq!(drawn.modes, [expected; 2], "{mode} Tr");
    }
}

/// PdfCraft patch 54 (storytold/pdfcraft#7465): an octal escape above `\377` in a shown string
/// keeps its low eight bits (ISO 32000-2 §7.3.4.2, Table 3: "high-order overflow shall be
/// ignored"). PDFBOX-142's form buttons write `(Abschlie\737en)`, `(L\766schen)` and
/// `(Pr\774fsumme)`; the escapes were dropped, so they read "Abschlieen", "Lschen" and
/// "Prfsumme". In WinAnsi 0o737 & 0xFF is ß, 0o766 & 0xFF ö and 0o774 & 0xFF ü.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_octal_escape_past_0o377_reads_its_low_eight_bits() {
    let text = |string: &str| helvetica_texts("/WinAnsiEncoding", format!("BT /F1 20 Tf 20 150 Td {string} Tj ET").as_bytes()).concat();
    assert_eq!(text(r"(Abschlie\737en)"), "Abschließen");
    assert_eq!(text(r"(L\766schen)"), "Löschen");
    assert_eq!(text(r"(Pr\774fsumme)"), "Prüfsumme");
    assert_eq!(text(r"(Abschlie\337en)"), "Abschließen", "control: the escape in range");
    assert_eq!(text(r"(\7377)"), "ß7", "three digits at most");
}

/// PdfCraft patch 116 (storytold/pdfcraft#7513): a Type 0 font whose `/Encoding` names a CMap
/// Adobe publishes for a supplement of its collections (`/Adobe-Japan1-2`), which ISO 32000-2
/// §9.7.5.2 doesn't list in Table 116, reads two-byte codes and its `/ToUnicode` (§9.10.2). The
/// font was dropped for the Helvetica substitute, which read the codes one byte at a time and
/// never looked at its `/ToUnicode`.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_adobe_collection_cid_cmap_name_reads_two_byte_codes() {
    let to_unicode = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def /CMapType 2 def \
                      1 begincodespacerange <0000> <FFFF> endcodespacerange \
                      3 beginbfchar <0022> <0041> <0465> <4E9C> <034B> <3042> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    let texts = |encoding: &str| {
        let font = format!("<< /Type /Font /Subtype /Type0 /BaseFont /TestJapan1 /Encoding /{encoding} /DescendantFonts [6 0 R] /ToUnicode 8 0 R >>");
        let extra = format!(
            "6 0 obj << /Type /Font /Subtype /CIDFontType2 /BaseFont /TestJapan1 /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 2 >> \
             /FontDescriptor 7 0 R /DW 1000 >> endobj\n\
             7 0 obj << /Type /FontDescriptor /FontName /TestJapan1 /Flags 4 /FontBBox [0 -120 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >> endobj\n\
             8 0 obj << /Length {} >> stream\n{to_unicode}\nendstream endobj\n",
            to_unicode.len() + 1
        );
        glyph_texts(&font, &extra, b"BT /F1 20 Tf 20 150 Td <00220465034B> Tj ET").into_iter().map(Option::unwrap_or_default).collect::<Vec<_>>()
    };
    assert_eq!(texts("Identity-H"), ["A", "亜", "あ"], "control: Identity-H");
    assert_eq!(texts("Adobe-Japan1-2"), ["A", "亜", "あ"]);
}

/// The text a Type 3 font whose `/Differences` gives `names` (code, glyph name), every glyph a
/// filled box, reads from `codes`.
fn type3_text(names: &[(u8, String)], codes: &[u8]) -> String {
    let differences: String = names.iter().map(|(code, name)| format!("{code} /{name} ")).collect();
    let procs: String = names.iter().map(|(_, name)| format!("/{name} 6 0 R ")).collect();
    let font = format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /FirstChar 0 /LastChar 255 \
         /Widths [{}] /Encoding << /Type /Encoding /Differences [{differences}] >> /CharProcs << {procs}>> >>",
        vec!["600"; 256].join(" ")
    );
    let glyph = "600 0 0 0 500 700 d1 0 0 500 700 re f";
    let extra = format!("6 0 obj << /Length {} >> stream\n{glyph}\nendstream endobj\n", glyph.len());
    let mut content = b"BT /F1 20 Tf 20 150 Td <".to_vec();
    content.extend(codes.iter().flat_map(|b| format!("{b:02X}").into_bytes()));
    content.extend_from_slice(b"> Tj ET");
    glyph_texts(&font, &extra, &content).into_iter().map(Option::unwrap_or_default).collect()
}

/// PdfCraft storytold/pdfcraft#7710: a Type 3 font naming all 256 codes in hexadecimal (`/x0` …
/// `/xff`, TeX's dvips fonts, PDFBOX-728) has numbered names at control codes, but names every
/// one of them, 0 through 31, in hexadecimal, which no decimal counter or counter from code 1
/// does: "für Weierstraß" reads in full, codes from 128 up included. A numbered name at its own
/// code reads as the code's Latin-1 character (§9.10.2 leaves such a font without a mapping).
/// A decimal counter `/C1` … `/C31` beside `/C174` (`/C10` at 10 reads 16 in hexadecimal), the
/// same names from `/x1` (code 0 left out) and a hexadecimal subset keeping a few glyphs at their
/// own control codes (`/G0A` at 10, `/G0D` at 13, `/GD9` at 0xD9) still read nothing from 128 up.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_type3_font_naming_its_codes_x0_to_xff_reads_them_from_128_up() {
    let names = |pairs: &[(u8, &str)]| pairs.iter().map(|&(code, n)| (code, n.to_string())).collect::<Vec<_>>();
    let hex_names: Vec<_> = (0..=255u8).map(|code| (code, format!("x{code:x}"))).collect();
    assert_eq!(type3_text(&hex_names, b"f\xfcr Weierstra\xdf"), "für Weierstraß", "/x0 … /xff");
    let mut counter_to_31: Vec<_> = (1..=31u8).map(|code| (code, format!("C{code}"))).collect();
    counter_to_31.push((174, "C174".to_string()));
    assert_eq!(type3_text(&counter_to_31, b"\xae"), "", "a counter /C1 … /C31 beside /C174 at 174");
    assert_eq!(type3_text(&hex_names[1..], b"f\xfcr Weierstra\xdf"), "fr Weierstra", "/x1 … /xff, code 0 left out");
    let subset = names(&[(10, "G0A"), (13, "G0D"), (174, "GAE"), (217, "GD9")]);
    assert_eq!(type3_text(&subset, b"\xae\xd9"), "", "/G0A at 10 and /G0D at 13 beside /GD9 at 217");
}

/// PdfCraft storytold/pdfcraft#7710 (review): the dvips hexadecimal block is a Type 3 rule. A
/// Type 1 font without `/ToUnicode` whose `/Differences` counts its glyphs in order of use in
/// hexadecimal from code 0 (`/zz0` … `/zzae`) fills the control block the same way, but its
/// `/zzae` at 0xAE reads as nothing, as it did before the Type 3 rule; the same names in a Type 3
/// font read `®`.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_type1_font_counting_its_glyphs_zz0_to_zzae_still_reads_nothing_from_them() {
    let counter: Vec<_> = (0..=0xaeu8).map(|code| (code, format!("zz{code:x}"))).collect();
    let differences: String = counter.iter().map(|(code, name)| format!("{code} /{name} ")).collect();
    let font = format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding << /Type /Encoding /Differences [{differences}] >> >>");
    let content = b"BT /F1 20 Tf 20 150 Td <AE> Tj ET";
    assert_eq!(glyph_texts(&font, "", content), [None], "Type 1 /zz0 … /zzae, code 0xAE");
    assert_eq!(type3_text(&counter, b"\xae"), "\u{ae}", "Type 3 /zz0 … /zzae, code 0xAE");
}
