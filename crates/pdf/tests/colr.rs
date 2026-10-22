//! COLR colour glyphs, drawn from a minimal synthetic font built in the test.
#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::panic)]

use backend::geom::Point;
use backend::page::PageSettings;
use backend::text::{GlyphId, KrillaGlyph};
use craft_pdf::write::{self, Metadata, WriteError, Writer, backend};
use kurbo::{Rect, Shape};
use write_fonts::FontBuilder;
use write_fonts::tables::colr::{BaseGlyph, BaseGlyphList, BaseGlyphPaint, Colr, Layer, Paint};
use write_fonts::tables::cpal::{ColorRecord, Cpal};
use write_fonts::tables::glyf::{GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::head::{Flags, Head, MacStyle};
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::loca::LocaFormat;
use write_fonts::tables::maxp::Maxp;
use write_fonts::types::{F2Dot14, FWord, Fixed, GlyphId16, LongDateTime, UfWord};

/// COLRv0: layers with palette entries 0 and 1.
const IN_RANGE: u16 = 1;
/// COLRv0: a second layer with a palette entry the font doesn't have, and an outline.
const OUT_OF_RANGE: u16 = 2;
/// COLRv1: a solid paint that no glyph or clip box bounds, and an outline.
const UNBOUNDED: u16 = 3;
/// COLRv1: palette entry 0 with an alpha above 1.
const OVER_OPAQUE: u16 = 4;
const LEFT: u16 = 5;
const RIGHT: u16 = 6;

const BLUE: &str = "0 0 1 rg";
const GREEN: &str = "0 1 0 rg";
const RED: &str = "1 0 0 rg";
/// The outline of the invalid colour glyphs, a square 123 units wide:
const OUTLINE: &str = "0 0 m 123 0 l 123 700 l";

/// A font whose only palette starts at colour record 2: records 0 and 1 are red and
/// must not be used, palette entries 0 and 1 are blue and green.
fn colr_font() -> Vec<u8> {
    let square = |x0, x1| Glyph::Simple(SimpleGlyph::from_bezpath(&Rect::new(x0, 0.0, x1, 700.0).to_path(0.1)).unwrap());
    let glyphs = [Glyph::Empty, Glyph::Empty, square(0.0, 123.0), square(0.0, 123.0), Glyph::Empty, square(0.0, 250.0), square(250.0, 500.0)];
    let mut glyf_loca = GlyfLocaBuilder::new();
    for glyph in &glyphs {
        glyf_loca.add_glyph(glyph).unwrap();
    }
    let (glyf, loca, loca_format) = glyf_loca.build();
    let index_to_loc_format = if loca_format == LocaFormat::Long { 1 } else { 0 };
    let date = LongDateTime::new(0);
    let head = Head::new(Fixed::ONE, 0, Flags::empty(), 1000, date, date, 0, 0, 500, 700, MacStyle::empty(), 8, index_to_loc_format);
    let num_glyphs = glyphs.len() as u16;
    let hhea = Hhea {
        ascender: FWord::new(800),
        descender: FWord::new(-200),
        advance_width_max: UfWord::new(600),
        number_of_h_metrics: num_glyphs,
        ..Default::default()
    };
    let hmtx = Hmtx::new(glyphs.iter().map(|_| LongMetric::new(600, 0)).collect(), vec![]);

    let id = GlyphId16::new;
    let layers = [(LEFT, 0), (RIGHT, 1), (LEFT, 0), (RIGHT, 7)];
    let mut colr = Colr::new(
        2,
        Some(vec![BaseGlyph::new(id(IN_RANGE), 0, 2), BaseGlyph::new(id(OUT_OF_RANGE), 2, 2)]),
        Some(layers.iter().map(|&(glyph, index)| Layer::new(id(glyph), index)).collect()),
        layers.len() as u16,
    );
    let paints = vec![
        BaseGlyphPaint::new(id(UNBOUNDED), Paint::solid(0, F2Dot14::ONE)),
        BaseGlyphPaint::new(id(OVER_OPAQUE), Paint::glyph(Paint::solid(0, F2Dot14::from_f32(1.5)), id(LEFT))),
    ];
    colr.base_glyph_list = BaseGlyphList::new(paints.len() as u32, paints).into();
    let red = ColorRecord::new(0, 0, 255, 255);
    let records = vec![red.clone(), red, ColorRecord::new(255, 0, 0, 255), ColorRecord::new(0, 255, 0, 255)];
    let cpal = Cpal::new(2, 1, records.len() as u16, Some(records), vec![2]);

    let mut builder = FontBuilder::new();
    builder.add_table(&head).unwrap();
    builder.add_table(&hhea).unwrap();
    builder.add_table(&Maxp::new(num_glyphs)).unwrap();
    builder.add_table(&hmtx).unwrap();
    builder.add_table(&glyf).unwrap();
    builder.add_table(&loca).unwrap();
    builder.add_table(&colr).unwrap();
    builder.add_table(&cpal).unwrap();
    builder.build()
}

/// Export one glyph of the font, uncompressed, behind the writer's unwind guard.
fn export(glyph: u16) -> Result<String, WriteError> {
    write::guard(|| {
        let font = write::font(&colr_font(), 0, None)?;
        let settings = backend::SerializeSettings { compress_content_streams: false, ..Default::default() };
        let mut writer = Writer::new(settings, Metadata::default());
        let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
        let mut surface = page.surface();
        let glyphs = [KrillaGlyph::new(GlyphId::new(u32::from(glyph)), 0.6, 0.0, 0.0, 0.0, 0..1, None)];
        surface.draw_glyphs(Point::from_xy(10.0, 80.0), &glyphs, font, "x", 50.0, false);
        surface.finish();
        page.finish();
        Ok(String::from_utf8_lossy(&writer.finish()?).into_owned())
    })
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn colr_layers_take_their_colours_from_the_palette_offset() {
    let pdf = export(IN_RANGE).unwrap();
    assert!(pdf.contains(BLUE), "palette entry 0 is blue");
    assert!(pdf.contains(GREEN), "palette entry 1 is green");
    assert!(!pdf.contains(RED), "the records before the palette are not used");
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn invalid_colr_glyphs_fall_back_to_their_outline() {
    for glyph in [OUT_OF_RANGE, UNBOUNDED] {
        let pdf = export(glyph).unwrap();
        assert!(pdf.contains(OUTLINE), "invalid colour glyph {glyph} draws its outline");
        for colour in [BLUE, GREEN, RED] {
            assert!(!pdf.contains(colour), "invalid colour glyph {glyph} draws no layer");
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn colr_alpha_above_one_is_opaque() {
    assert!(export(OVER_OPAQUE).unwrap().contains(BLUE));
}
