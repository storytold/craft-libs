//! A simple font the PDF names but doesn't embed is asked of the font resolver by name (vendored
//! hayro-interpret, `Craft patch (named fallback)`). A font named `CenturyGothic` whose
//! descriptor sets the serif flag was drawn with the Times stand-in on a machine that has Century
//! Gothic installed: the resolver was only ever asked for the stand-in, so an app that knows the
//! installed fonts had no way to answer with the font the document names.
//!
//! The named font here is the fonts-build fixture `Tiny-glyf.ttf`, whose glyph for "A" has
//! another index than the bundled stand-in's, so which glyph is drawn tells the two apart.

use craft_pdf::interpret::font::{FontData, FontQuery, Glyph};
use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
};
use kurbo::{Affine, BezPath, Rect};
use std::sync::{Arc, Mutex};

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

/// A page that shows "A" in a simple font of `subtype` named `name`, with no font program. The
/// descriptor sets the serif flag, so the stand-in chosen from the flags is Times.
fn pdf(subtype: &str, name: &str) -> Vec<u8> {
    let content = "BT /F1 12 Tf 10 10 Td (A) Tj ET";
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
5 0 obj << /Type /Font /Subtype /{subtype} /BaseFont /{name} /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R >> endobj
6 0 obj << /Type /FontDescriptor /FontName /{name} /Flags 34 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 >> endobj
trailer << /Root 1 0 R >>
%%EOF",
        content.len()
    )
    .into_bytes()
}

/// What a resolver does with a query that names a font.
#[derive(Clone, Copy)]
enum Named {
    /// The default resolver answers every query, as an app without installed fonts does.
    Default,
    /// The fixture stands for the installed font.
    Fixture,
    /// The fixture stands for the installed font, and the resolver has no data for the standard
    /// fonts.
    FixtureOnly,
    /// The resolver has no such font.
    Missing,
    /// The answer is not a font program.
    Broken,
}

/// The glyphs drawn for `pdf`'s page, and the PostScript names the resolver was asked for.
fn draw(pdf: &[u8], named: Named) -> (Vec<u32>, Vec<String>) {
    let standard = InterpreterSettings::default().font_resolver;
    let asked = Arc::new(Mutex::new(Vec::new()));
    let names = Arc::clone(&asked);
    let fixture: FontData = Arc::new(TINY_GLYF.to_vec());
    let broken: FontData = Arc::new(b"not a font program".to_vec());
    let settings = InterpreterSettings {
        font_resolver: Arc::new(move |query| {
            let FontQuery::Fallback(fallback) = query else {
                return if matches!(named, Named::FixtureOnly) { None } else { standard(query) };
            };
            assert!(fallback.character_collection.is_none(), "a simple font has no character collection");
            names.lock().unwrap().push(fallback.post_script_name.clone().unwrap_or_default());
            match named {
                Named::Default => standard(query),
                Named::Fixture | Named::FixtureOnly => Some((fixture.clone(), 0)),
                Named::Missing => None,
                Named::Broken => Some((broken.clone(), 0)),
            }
        }),
        ..InterpreterSettings::default()
    };
    let pdf = craft_pdf::open_slice(pdf, None).unwrap();
    let page = &pdf.pages()[0];
    let cache = InterpreterCache::new();
    let mut context = Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 100.0, 100.0), &cache, page.xref(), settings);
    let mut device = Glyphs::default();
    craft_pdf::interpret_page(page, &mut context, &mut device).unwrap();
    let asked = asked.lock().unwrap().clone();
    (device.0, asked)
}

/// The fixture's glyph for "A".
fn fixture_a() -> u32 {
    let face = ttf_parser::Face::parse(TINY_GLYF, 0).unwrap();
    u32::from(face.glyph_index('A').expect("the fixture covers A").0)
}

/// A TrueType or Type 1 font the PDF names but doesn't embed is asked of the resolver by its
/// name, and the font the resolver answers with draws the text instead of the stand-in.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_resolver_that_has_the_named_font_draws_with_it() {
    for subtype in ["TrueType", "Type1"] {
        let page = pdf(subtype, "CenturyGothic");
        let (stand_in, _) = draw(&page, Named::Default);
        let (drawn, asked) = draw(&page, Named::Fixture);
        assert_eq!(asked, ["CenturyGothic"], "{subtype}: the resolver is asked for the font by name");
        assert_eq!(drawn, [fixture_a()], "{subtype}: the named font draws A");
        assert_ne!(drawn, stand_in, "{subtype}: the stand-in's A is another glyph");
    }
}

/// A resolver that has no font of that name, or answers with bytes that are no font program,
/// leaves the text to the stand-in, drawn as the default resolver draws it.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_resolver_without_the_named_font_draws_the_stand_in() {
    for subtype in ["TrueType", "Type1"] {
        let page = pdf(subtype, "CenturyGothic");
        let (stand_in, _) = draw(&page, Named::Default);
        assert_eq!(stand_in.len(), 1, "{subtype}: the stand-in draws A");
        for (named, which) in [(Named::Missing, "no such font"), (Named::Broken, "not a font program")] {
            assert_eq!(draw(&page, named).0, stand_in, "{subtype}, {which}: the stand-in draws A");
        }
    }
}

/// One of the standard 14 by name is the stand-in itself: the resolver is not asked for it by
/// name, so an installed font never replaces Helvetica or Times-Roman.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_standard_font_is_not_asked_for_by_name() {
    for subtype in ["TrueType", "Type1"] {
        for name in ["Helvetica", "Times-Roman"] {
            let page = pdf(subtype, name);
            let (stand_in, _) = draw(&page, Named::Default);
            let (drawn, asked) = draw(&page, Named::Fixture);
            assert!(asked.is_empty(), "{subtype} {name}: asked for {asked:?}");
            assert_eq!(drawn, stand_in, "{subtype} {name}: the standard font draws A");
        }
    }
}

/// A resolver with no data for the standard fonts is still not asked for one of them by name, so
/// its installed font doesn't draw a font named Helvetica. A font of another name draws with the
/// font it answers with.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_standard_font_the_resolver_lacks_is_not_asked_for_by_name() {
    for subtype in ["TrueType", "Type1"] {
        for name in ["Helvetica", "Times-Roman"] {
            let (drawn, asked) = draw(&pdf(subtype, name), Named::FixtureOnly);
            assert!(asked.is_empty(), "{subtype} {name}: asked for {asked:?}");
            assert_ne!(drawn, [fixture_a()], "{subtype} {name}: the installed font doesn't draw a standard font");
        }
        let (drawn, asked) = draw(&pdf(subtype, "CenturyGothic"), Named::FixtureOnly);
        assert_eq!(asked, ["CenturyGothic"], "{subtype}: the resolver is asked for the font by name");
        assert_eq!(drawn, [fixture_a()], "{subtype}: the named font draws A");
    }
}
