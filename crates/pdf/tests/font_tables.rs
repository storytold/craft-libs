//! The font programs the writer embeds keep the source font's `OS/2` table (#1781 in DesignCraft):
//! its `fsType` is where the font states its embedding rights, and the subsetter writes none.
#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::panic)]

use backend::geom::Point;
use backend::page::PageSettings;
use backend::text::{GlyphId, KrillaGlyph};
use craft_pdf::write::{self, Metadata, Writer, backend};
use hayro_syntax::object::{Object, Stream};

/// A two-glyph TrueType font with an `OS/2` table whose `fsType` is 4 (Preview & Print).
const TINY_GLYF: &[u8] = include_bytes!("../../fonts-build/tests/fixture/subset/Tiny-glyf.ttf");

/// `text` set in `font_bytes`, one glyph per character, as a one-page PDF.
fn document(font_bytes: &[u8], text: &str) -> Vec<u8> {
    let face = ttf_parser::Face::parse(font_bytes, 0).unwrap();
    let glyphs: Vec<_> = text
        .char_indices()
        .map(|(start, c)| {
            let id = face.glyph_index(c).unwrap_or_else(|| panic!("the fixture covers {c:?}"));
            let advance = f32::from(face.glyph_hor_advance(id).unwrap()) / f32::from(face.units_per_em());
            KrillaGlyph::new(GlyphId::new(u32::from(id.0)), advance, 0.0, 0.0, 0.0, start..start + c.len_utf8(), None)
        })
        .collect();
    let font = write::font(font_bytes, 0, None).unwrap();
    let mut writer = Writer::new(backend::SerializeSettings::default(), Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(200.0, 100.0).unwrap());
    let mut surface = page.surface();
    surface.draw_glyphs(Point::from_xy(20.0, 50.0), &glyphs, font, text, 18.0, false);
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

/// The decoded `FontFile2` programs of `pdf`, through the font descriptors that name them.
fn font_files(pdf: &[u8]) -> Vec<Vec<u8>> {
    let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(pdf.to_vec())).unwrap();
    pdf.objects()
        .into_iter()
        .filter_map(|o| match o {
            Object::Dict(d) => d.get::<Stream<'_>>(b"FontFile2"),
            _ => None,
        })
        .map(|s| s.decoded().unwrap().to_vec())
        .collect()
}

/// `(tag, offset, length, checksum)` of each table in the sfnt `font`.
fn tables(font: &[u8]) -> Vec<([u8; 4], usize, usize, u32)> {
    let be32 = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().unwrap());
    let n = usize::from(u16::from_be_bytes(font[4..6].try_into().unwrap()));
    (0..n)
        .map(|k| (font[12 + 16 * k..16 + 16 * k].try_into().unwrap(), be32(20 + 16 * k) as usize, be32(24 + 16 * k) as usize, be32(16 + 16 * k)))
        .collect()
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut word = [0u8; 4];
        word[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    tables(font).into_iter().find(|t| &t.0 == tag).map(|(_, off, len, _)| &font[off..off + len])
}

#[test]
fn an_embedded_truetype_subset_keeps_the_os2_table_and_its_fstype() {
    let face = ttf_parser::Face::parse(TINY_GLYF, 0).unwrap();
    let text: String = ('!'..='~').filter(|c| face.glyph_index(*c).is_some()).collect();
    assert!(!text.is_empty(), "the fixture maps some ASCII");
    let files = font_files(&document(TINY_GLYF, &text));
    assert_eq!(files.len(), 1, "one embedded TrueType program");
    let sub = &files[0];
    let os2 = table(sub, b"OS/2").expect("the subset carries an OS/2 table");
    assert_eq!(os2, table(TINY_GLYF, b"OS/2").unwrap(), "the source's OS/2, unchanged");
    assert_eq!(u16::from_be_bytes(os2[8..10].try_into().unwrap()), 4, "fsType: Preview & Print");
    // A valid sfnt: tags in order, each table's checksum right, the whole font summing to the
    // magic number through head.checkSumAdjustment.
    let t = tables(sub);
    assert!(
        t.windows(2).all(|w| w[0].0 < w[1].0),
        "tables in tag order: {:?}",
        t.iter().map(|t| String::from_utf8_lossy(&t.0).to_string()).collect::<Vec<_>>()
    );
    for (tag, off, len, sum) in &t {
        let mut data = sub[*off..off + len].to_vec();
        if tag == b"head" {
            data[8..12].fill(0);
        }
        assert_eq!(checksum(&data), *sum, "{} checksum", String::from_utf8_lossy(tag));
        assert_eq!(off % 4, 0, "{} is 4-byte aligned", String::from_utf8_lossy(tag));
    }
    assert_eq!(checksum(sub), 0xB1B0_AFBA, "whole-font checksum");
    // And it still reads as a font with the glyphs it was given.
    let reread = ttf_parser::Face::parse(sub, 0).unwrap();
    // A subset, not the source font embedded whole: of the fixture's 11 glyphs, .notdef and the
    // ones the text uses, and only their outlines.
    assert!(
        2 <= reread.number_of_glyphs() && reread.number_of_glyphs() < face.number_of_glyphs(),
        "{} of {} glyphs",
        reread.number_of_glyphs(),
        face.number_of_glyphs()
    );
    assert!(table(sub, b"glyf").unwrap().len() < table(TINY_GLYF, b"glyf").unwrap().len(), "only the used glyphs' outlines");
    assert_eq!(reread.permissions(), face.permissions());
}
