//! A CID that is its character's vertical form draws a substitute's vertical glyph (vendored
//! hayro-interpret, PdfCraft patch 107, PdfCraft #7167). Adobe-Japan1 gives 「 (U+300C) CID 686
//! and its vertical form CID 7911; a font the PDF doesn't embed draws both through their
//! character, so a vertical column showed the horizontal 「, and 」、。ー…（） and the small kana
//! the same way.
//!
//! The substitute is the fonts-build fixture `Tiny-glyf.ttf`, whose own `vert` feature replaces
//! 「 with `uni300C.vert`. Other tests replace its GSUB with one written here, whose lookup
//! replaces 「 with the fixture's "A", so which glyph is drawn tells the forms apart.

use craft_pdf::interpret::font::{FontData, Glyph};
use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
};
use kurbo::{Affine, BezPath, Rect};
use std::sync::Arc;

const TINY_GLYF: &[u8] = include_bytes!("../../../fonts-build/tests/fixture/subset/Tiny-glyf.ttf");

/// The glyph indexes drawn, in order.
#[derive(Default)]
struct Glyphs(Vec<u32>);

impl Device<'_> for Glyphs {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {
        if let Glyph::Outline(glyph) = glyph {
            self.0.push(glyph.glyph_id().to_u32());
        }
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The fixture's glyph for `c`.
fn glyph(c: char) -> u16 {
    let face = ttf_parser::Face::parse(TINY_GLYF, 0).unwrap();
    face.glyph_index(c).unwrap_or_else(|| panic!("the fixture covers {c:?}")).0
}

/// The fixture's own vertical form of 「.
fn vertical_bracket() -> u16 {
    let face = ttf_parser::Face::parse(TINY_GLYF, 0).unwrap();
    face.glyph_index_by_name("uni300C.vert").expect("the fixture has a vertical 「").0
}

fn u16s(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// How the `vert` lookup's single substitution is written.
#[derive(Clone, Copy, Debug)]
enum Subtable {
    /// Format 2, a list of substitutes.
    List,
    /// Format 1, a delta, reached through an extension lookup (type 7), as large CJK faces have it.
    DeltaInExtension,
}

/// A GSUB table whose one feature, tagged `feature`, is in the `DFLT` script's default language
/// system and replaces glyph `from` with glyph `to`.
fn gsub(feature: &[u8; 4], from: u16, to: u16, subtable: Subtable) -> Vec<u8> {
    // No required feature (0xFFFF); one optional feature, feature 0.
    gsub_listing(from, &[(feature, to)], subtable, &[(b"DFLT", &[0xFFFF, 1, 0])])
}

/// A GSUB table with `features`, each one's tag and the glyph its lookup replaces `from` with, and
/// `scripts`, each one's tag and its default language system's `requiredFeatureIndex`,
/// `featureIndexCount` and `featureIndices`.
fn gsub_listing(from: u16, features: &[(&[u8; 4], u16)], subtable: Subtable, scripts: &[(&[u8; 4], &[u16])]) -> Vec<u8> {
    // The default language system at offset 4; no other language systems; no lookup order.
    let script_list = list(scripts.iter().map(|(tag, language)| (tag.as_slice(), [u16s(&[4, 0, 0]), u16s(language)].concat())));
    // Feature n has one lookup, lookup n.
    let feature_list = list(features.iter().zip(0..).map(|((tag, _), n)| (tag.as_slice(), u16s(&[0, 1, n]))));
    let lookup_list = list(features.iter().map(|(_, to)| (&[][..], lookup(from, *to, subtable))));
    let script_offset = 10;
    let feature_offset = script_offset + u16::try_from(script_list.len()).unwrap();
    let lookup_offset = feature_offset + u16::try_from(feature_list.len()).unwrap();
    [u16s(&[1, 0, script_offset, feature_offset, lookup_offset]), script_list, feature_list, lookup_list].concat()
}

/// An OpenType list of `tables`: their count, each one's tag (a lookup has none) and offset, then
/// the tables.
fn list<'a>(tables: impl Iterator<Item = (&'a [u8], Vec<u8>)>) -> Vec<u8> {
    let tables: Vec<_> = tables.collect();
    let data = 2 + tables.iter().map(|(tag, _)| tag.len() + 2).sum::<usize>();
    let mut records = u16s(&[u16::try_from(tables.len()).unwrap()]);
    let mut bodies = Vec::new();
    for (tag, table) in tables {
        records.extend(tag);
        records.extend(u16s(&[u16::try_from(data + bodies.len()).unwrap()]));
        bodies.extend(table);
    }
    [records, bodies].concat()
}

/// A lookup that replaces glyph `from` with glyph `to`.
fn lookup(from: u16, to: u16, subtable: Subtable) -> Vec<u8> {
    // Coverage of `from` only.
    let coverage = u16s(&[1, 1, from]);
    let single = match subtable {
        Subtable::List => [u16s(&[2, 8, 1, to]), coverage].concat(),
        Subtable::DeltaInExtension => [u16s(&[1, 6, to.wrapping_sub(from)]), coverage].concat(),
    };
    let (lookup_type, body) = match subtable {
        Subtable::List => (1, single),
        // Extension format 1: the type it holds, then a 32-bit offset to it.
        Subtable::DeltaInExtension => (7, [u16s(&[1, 1, 0, 8]), single].concat()),
    };
    [u16s(&[lookup_type, 0, 1, 8]), body].concat()
}

/// A `cmap` table whose one subtable (Windows Unicode BMP, format 4) maps `character` to `glyph`.
fn cmap(character: char, glyph: u16) -> Vec<u8> {
    let code = u16::try_from(u32::from(character)).unwrap();
    // Two segments: the character, and the closing 0xFFFF one.
    let subtable = u16s(&[4, 32, 0, 4, 4, 1, 0, code, 0xFFFF, 0, code, 0xFFFF, glyph.wrapping_sub(code), 1, 0, 0]);
    [u16s(&[0, 1, 3, 1, 0, 12]), subtable].concat()
}

/// The font file `font` with its table `tag` replaced by `table` (or added, or, for `None`, removed).
fn with_table(font: &[u8], tag: &[u8; 4], table: Option<&[u8]>) -> Vec<u8> {
    let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
    let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..count)
        .map(|i| {
            let record = &font[12 + 16 * i..28 + 16 * i];
            let offset = u32::from_be_bytes(record[8..12].try_into().unwrap()) as usize;
            let length = u32::from_be_bytes(record[12..16].try_into().unwrap()) as usize;
            (record[..4].try_into().unwrap(), font[offset..offset + length].to_vec())
        })
        .filter(|(t, _)| t != tag)
        .collect();
    tables.extend(table.map(|table| (*tag, table.to_vec())));
    tables.sort_by_key(|(t, _)| *t);
    let count = tables.len();
    let power = 1usize << count.ilog2();
    let mut out = font[..4].to_vec();
    out.extend(u16s(&[
        u16::try_from(count).unwrap(),
        u16::try_from(power * 16).unwrap(),
        u16::try_from(power.ilog2()).unwrap(),
        u16::try_from((count - power) * 16).unwrap(),
    ]));
    let mut data = Vec::new();
    let start = 12 + 16 * count;
    for (t, bytes) in &tables {
        out.extend(t);
        out.extend([0; 4]);
        out.extend(u32::try_from(start + data.len()).unwrap().to_be_bytes());
        out.extend(u32::try_from(bytes.len()).unwrap().to_be_bytes());
        data.extend(bytes);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    out.extend(data);
    out
}

/// A page showing `code` in a Ryumin-Light of Adobe-Japan1 that the PDF doesn't embed.
fn pdf(encoding: &str, code: &str) -> Vec<u8> {
    let content = format!("BT /F1 40 Tf 20 60 Td <{code}> Tj ET");
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
5 0 obj << /Type /Font /Subtype /Type0 /BaseFont /Ryumin-Light /Encoding /{encoding} /DescendantFonts [6 0 R] >> endobj
6 0 obj << /Type /Font /Subtype /CIDFontType0 /BaseFont /Ryumin-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 2 >>
  /FontDescriptor 7 0 R /DW 1000 >> endobj
7 0 obj << /Type /FontDescriptor /FontName /Ryumin-Light /Flags 6 /FontBBox [-170 -331 1024 903] /ItalicAngle 0 /Ascent 752 /Descent -271 /CapHeight 742 /StemV 54 >> endobj
trailer << /Root 1 0 R >>
%%EOF",
        content.len()
    )
    .into_bytes()
}

/// The glyphs drawn for `pdf`'s page with `face` as the substitute for every font.
fn glyphs(pdf: &[u8], face: &[u8]) -> Vec<u32> {
    glyphs_with(pdf, face, InterpreterSettings::default())
}

/// [`glyphs`] with `settings`, whose font resolver is replaced.
fn glyphs_with(pdf: &[u8], face: &[u8], settings: InterpreterSettings) -> Vec<u32> {
    let data: FontData = Arc::new(face.to_vec());
    let settings = InterpreterSettings { font_resolver: Arc::new(move |_| Some((data.clone(), 0))), ..settings };
    let pdf = craft_pdf::open_slice(pdf, None).unwrap();
    let page = &pdf.pages()[0];
    let cache = InterpreterCache::new();
    let mut context = Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 100.0, 100.0), &cache, page.xref(), settings);
    let mut device = Glyphs::default();
    craft_pdf::interpret_page(page, &mut context, &mut device).unwrap();
    device.0
}

/// 「 by its vertical form's CID, through a `-V` CMap or as `Identity-H` or `Identity-V` CID 7911
/// (0x1EE7, an old Japanese document's column, PdfCraft #7167), draws the substitute's vertical
/// glyph; by its horizontal CID, 686 (0x02AE), the ordinary one. The substitute's lookup is read
/// as the fixture writes it, as a format 2 list, and as a format 1 delta in an extension lookup.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_vertical_cid_draws_the_substitutes_vertical_glyph() {
    let bracket = glyph('「');
    let a = glyph('A');
    let faces = [
        ("the fixture's own vert", TINY_GLYF.to_vec(), vertical_bracket()),
        ("a format 2 vert", with_table(TINY_GLYF, b"GSUB", Some(&gsub(b"vert", bracket, a, Subtable::List))), a),
        ("a format 1 vert in an extension", with_table(TINY_GLYF, b"GSUB", Some(&gsub(b"vert", bracket, a, Subtable::DeltaInExtension))), a),
    ];
    for (which, face, vertical) in faces {
        assert_ne!(vertical, bracket, "{which}: the vertical form is another glyph");
        for (encoding, code, expected, form) in [
            ("UniJIS-UCS2-V", "300C", vertical, "vertical"),
            ("Identity-H", "1EE7", vertical, "vertical"),
            ("Identity-V", "1EE7", vertical, "vertical"),
            ("UniJIS-UCS2-H", "300C", bracket, "horizontal"),
            ("Identity-H", "02AE", bracket, "horizontal"),
            ("Identity-V", "02AE", bracket, "horizontal"),
        ] {
            assert_eq!(glyphs(&pdf(encoding, code), &face), [u32::from(expected)], "{which}: {encoding} <{code}> is the {form} 「");
        }
    }
}

/// A face whose vertical forms are under `vrt2` alone substitutes them too; a face with no
/// vertical substitution, or one whose `vert` covers other glyphs, draws the character's
/// ordinary glyph, as before.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_face_without_a_vertical_glyph_draws_the_ordinary_one() {
    let (bracket, a, comma) = (glyph('「'), glyph('A'), glyph('、'));
    let vrt2 = with_table(TINY_GLYF, b"GSUB", Some(&gsub(b"vrt2", bracket, a, Subtable::List)));
    assert_eq!(glyphs(&pdf("UniJIS-UCS2-V", "300C"), &vrt2), [u32::from(a)], "vrt2 stands in for vert");
    let other = with_table(TINY_GLYF, b"GSUB", Some(&gsub(b"vert", comma, a, Subtable::List)));
    let other_feature = with_table(TINY_GLYF, b"GSUB", Some(&gsub(b"smcp", bracket, a, Subtable::List)));
    let none = with_table(TINY_GLYF, b"GSUB", None);
    for (face, which) in [(&other, "a vert lookup for other glyphs"), (&other_feature, "another feature"), (&none, "no GSUB")] {
        assert_eq!(glyphs(&pdf("UniJIS-UCS2-V", "300C"), face), [u32::from(bracket)], "{which}: the ordinary 「");
    }
}

/// A language system may list `vert` only as its required feature (`requiredFeatureIndex`, with
/// no optional features): its lookups apply too, and `kana`'s before `DFLT`'s.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_vert_feature_the_language_system_requires_applies() {
    let (bracket, a, comma) = (glyph('「'), glyph('A'), glyph('、'));
    // `kana` requires feature 1 and lists no optional features; `DFLT` lists feature 0.
    let gsub = gsub_listing(bracket, &[(b"vert", comma), (b"vert", a)], Subtable::List, &[(b"DFLT", &[0xFFFF, 1, 0]), (b"kana", &[1, 0])]);
    let required = with_table(TINY_GLYF, b"GSUB", Some(&gsub));
    assert_eq!(glyphs(&pdf("UniJIS-UCS2-V", "300C"), &required), [u32::from(a)], "the required vert feature applies");
    assert_eq!(glyphs(&pdf("UniJIS-UCS2-H", "300C"), &required), [u32::from(bracket)], "a horizontal CID keeps its glyph");
}

/// A script that exists but lists no vertical feature does not end the search: when `kana`, the
/// collection's script, lists a horizontal feature only, `DFLT`'s `vert` applies; when `DFLT`
/// lists none either, the `vert` of any script does.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_script_without_a_vertical_feature_leaves_it_to_the_next() {
    let (bracket, a, comma) = (glyph('「'), glyph('A'), glyph('、'));
    let (hkna, vert): (&[u16], &[u16]) = (&[0xFFFF, 1, 0], &[0xFFFF, 1, 1]);
    let only_dflt: &[(&[u8; 4], &[u16])] = &[(b"DFLT", vert), (b"kana", hkna)];
    let only_another: &[(&[u8; 4], &[u16])] = &[(b"DFLT", hkna), (b"kana", hkna), (b"latn", vert)];
    for (scripts, which) in [(only_dflt, "only DFLT"), (only_another, "only a script other than kana and DFLT")] {
        let gsub = gsub_listing(bracket, &[(b"hkna", comma), (b"vert", a)], Subtable::List, scripts);
        let face = with_table(TINY_GLYF, b"GSUB", Some(&gsub));
        assert_eq!(glyphs(&pdf("UniJIS-UCS2-V", "300C"), &face), [u32::from(a)], "{which} lists vert: it applies");
        assert_eq!(glyphs(&pdf("UniJIS-UCS2-H", "300C"), &face), [u32::from(bracket)], "{which} lists vert: a horizontal CID keeps its glyph");
    }
}

/// Adobe-Japan1 gives “ (U+201C) CID 108 and its vertical form CID 8279 (0x2057), which only the
/// older UCS-2 CMaps name: `UniJIS-UCS2-V` picks 8279 for U+201C, while `UniJIS-UTF16-V` keeps
/// 108. 8279, through that CMap or as an `Identity-H` CID, is a vertical form; 108 is not. The
/// fixture's `cmap` is replaced by one that draws “ with the glyph its own `vert` substitutes.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_vertical_cid_only_the_ucs2_cmaps_name_draws_the_vertical_glyph() {
    let (quote, vertical) = (glyph('「'), vertical_bracket());
    let face = with_table(TINY_GLYF, b"cmap", Some(&cmap('\u{201C}', quote)));
    for (encoding, code, expected, form) in [
        ("UniJIS-UCS2-V", "201C", vertical, "vertical"),
        ("Identity-H", "2057", vertical, "vertical"),
        ("UniJIS-UCS2-H", "201C", quote, "horizontal"),
        ("Identity-H", "006C", quote, "horizontal"),
    ] {
        assert_eq!(glyphs(&pdf(encoding, code), &face), [u32::from(expected)], "{encoding} <{code}> is the {form} “");
    }
}

/// A caller's CMap resolver may have the UCS-2 CMaps and not the UTF-16 ones: the UCS-2 pair
/// alone then tells the vertical forms from the horizontal ones.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn the_ucs2_cmaps_alone_tell_the_vertical_forms() {
    use hayro_cmap::CMapName;
    let (bracket, vertical) = (glyph('「'), vertical_bracket());
    let resolver = InterpreterSettings::default().cmap_resolver;
    let settings = InterpreterSettings {
        cmap_resolver: Arc::new(move |name| match name {
            CMapName::UniJisUtf16H | CMapName::UniJisUtf16V => None,
            name => resolver(name),
        }),
        ..Default::default()
    };
    for (encoding, code, expected, form) in [
        ("UniJIS-UCS2-V", "300C", vertical, "vertical"),
        ("Identity-H", "1EE7", vertical, "vertical"),
        ("UniJIS-UCS2-H", "300C", bracket, "horizontal"),
        ("Identity-H", "02AE", bracket, "horizontal"),
    ] {
        assert_eq!(glyphs_with(&pdf(encoding, code), TINY_GLYF, settings.clone()), [u32::from(expected)], "{encoding} <{code}> is the {form} 「");
    }
}
