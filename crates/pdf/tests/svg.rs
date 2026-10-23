//! OpenType SVG glyphs, drawn from a minimal synthetic font built in the test.
#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::panic)]

use backend::geom::Point;
use backend::page::PageSettings;
use backend::text::{GlyphId, KrillaGlyph};
use craft_pdf::write::{self, Metadata, WriteError, Writer, backend};
use kurbo::{Rect, Shape};
use write_fonts::FontBuilder;
use write_fonts::tables::glyf::{GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::head::{Flags, Head, MacStyle};
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::loca::LocaFormat;
use write_fonts::tables::maxp::Maxp;
use write_fonts::types::{FWord, Fixed, LongDateTime, Tag, UfWord};

/// The glyph the font's `SVG ` table has a document for.
const SVG: u16 = 1;
/// A glyph with an outline only.
const PLAIN: u16 = 2;

/// A font of two squares whose `SVG ` table holds a document for [`SVG`] alone.
fn svg_font() -> Vec<u8> {
    let square = Glyph::Simple(SimpleGlyph::from_bezpath(&Rect::new(0.0, 0.0, 500.0, 700.0).to_path(0.1)).unwrap());
    let glyphs = [Glyph::Empty, square.clone(), square];
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

    // The `SVG ` table: its header (version, the document list's offset, reserved), then the
    // document list with one record (first and last glyph, the document's offset from the list
    // and length) and the document.
    let doc = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect id="glyph{SVG}" width="500" height="700"/></svg>"#);
    let mut svg = [0u16.to_be_bytes().as_slice(), &10u32.to_be_bytes(), &0u32.to_be_bytes()].concat();
    svg.extend(1u16.to_be_bytes());
    svg.extend([SVG.to_be_bytes(), SVG.to_be_bytes()].concat());
    svg.extend([14u32.to_be_bytes(), (doc.len() as u32).to_be_bytes()].concat());
    svg.extend(doc.as_bytes());

    let mut builder = FontBuilder::new();
    builder.add_table(&head).unwrap();
    builder.add_table(&hhea).unwrap();
    builder.add_table(&Maxp::new(num_glyphs)).unwrap();
    builder.add_table(&hmtx).unwrap();
    builder.add_table(&glyf).unwrap();
    builder.add_table(&loca).unwrap();
    builder.add_raw(Tag::new(b"SVG "), svg);
    builder.build()
}

/// Export one glyph of the font, uncompressed, behind the writer's unwind guard.
fn export(glyph: u16) -> Result<String, WriteError> {
    write::guard(|| {
        let font = write::font(&svg_font(), 0, None)?;
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
fn only_glyphs_with_an_svg_document_go_into_a_type3_font() {
    let pdf = export(SVG).unwrap();
    assert!(pdf.contains("/Type3"), "the SVG glyph is a Type 3 glyph");
    let pdf = export(PLAIN).unwrap();
    assert!(!pdf.contains("/Type3"), "a glyph without a document stays in the font");
    assert!(pdf.contains("/CIDFontType2"), "as a TrueType glyph");
}
