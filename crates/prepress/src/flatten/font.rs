//! Source fonts prepared for embedding again. The PDF-in adapter notes every outline glyph it
//! reads; after the page, [`FontCache::finish`] gives one [`TextFont`] per source font, or
//! `None` where the font's runs are drawn from their outlines.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use craft_pdf::interpret::font::OutlineGlyph;
use craft_pdf::write::backend::geom::Point;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::text::{Font, GlyphId, KrillaGlyph};
use craft_pdf::write::backend::{Data, SerializeSettings};
use craft_pdf::write::{Metadata, WriteError, Writer, guard};

use super::{FontBytes, TextFont};

/// The fonts of one page, collected glyph by glyph.
#[derive(Default)]
pub struct FontCache {
    fonts: HashMap<u128, Noted>,
}

struct Noted {
    /// The font's own bytes and face index; `None` when the PDF doesn't embed the font or
    /// hayro has no bytes for it.
    source: Option<(FontBytes, u32)>,
    /// Glyph id to advance in glyph space (1000 units per em), from the PDF's widths. Ordered,
    /// so the trial embedding is deterministic.
    advances: BTreeMap<u32, f32>,
}

impl FontCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a glyph seen on the page (font key, glyph id, PDF width) and returns the key.
    pub fn note(&mut self, glyph: &OutlineGlyph) -> u128 {
        let key = glyph.font_cache_key();
        let noted = self.fonts.entry(key).or_insert_with(|| Noted {
            // A substitute from the font resolver is not the file's font.
            source: glyph.font_data().filter(|d| d.embedded).map(|d| (d.data, d.index)),
            advances: BTreeMap::new(),
        });
        if noted.source.is_some() {
            noted.advances.insert(glyph.glyph_id().to_u32(), glyph.advance_width().unwrap_or(0.0));
        }
        key
    }

    /// After the page is read: sniff, wrap bare CFF, trial-embed. `None` means: draw from
    /// outlines. Every noted key is present.
    pub fn finish(self) -> HashMap<u128, Option<Arc<TextFont>>> {
        self.fonts.into_iter().map(|(key, noted)| (key, noted.prepare(key).map(Arc::new))).collect()
    }
}

impl Noted {
    fn prepare(self, key: u128) -> Option<TextFont> {
        let (data, index) = self.source?;
        let magic: [u8; 4] = (*data).as_ref().get(..4)?.try_into().ok()?;
        let font = match &magic {
            [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf" => TextFont { data, index, key },
            // The CFF wrap needs the widths of all glyphs used, hence once per page.
            [1, _, _, _] => TextFont { data: Arc::new(wrap_cff((*data).as_ref(), &self.advances)?), index: 0, key },
            _ => return None,
        };
        embeds(&font, self.advances.keys().copied()).then_some(font)
    }
}

/// Whether krilla embeds `glyphs` of `font`. `Font::new` accepts fonts whose subsetting fails
/// later, in `Writer::finish`, which is too late to fall back for one font; a throwaway
/// one-page document finds those failures here.
fn embeds(font: &TextFont, glyphs: impl Iterator<Item = u32>) -> bool {
    let glyphs: Vec<KrillaGlyph> = glyphs.map(|id| KrillaGlyph::new(GlyphId::new(id), 0.0, 0.0, 0.0, 0.0, 0..0, None)).collect();
    let trial = || {
        let font = Font::new(Data::from(font.data.clone()), font.index).ok_or(WriteError::InvalidFont)?;
        let mut writer = Writer::new(SerializeSettings::default(), Metadata::default());
        let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(10.0, 10.0).ok_or(WriteError::InvalidFont)?);
        let mut surface = page.surface();
        surface.draw_glyphs(Point::from_xy(0.0, 0.0), &glyphs, font, "", 10.0, false);
        surface.finish();
        page.finish();
        writer.finish()
    };
    guard(trial).is_ok()
}

/// Wraps bare CFF (`FontFile3`) in the smallest OpenType font krilla loads: `CFF `, `head`,
/// `hhea`, `hmtx`, `maxp` 0.5 and `post` 3. `hmtx` holds `advances` (glyph space, 1000 units
/// per em) and 0 for the glyphs not listed; `unitsPerEm` follows the CFF `FontMatrix`.
pub(crate) fn wrap_cff(cff: &[u8], advances: &BTreeMap<u32, f32>) -> Option<Vec<u8>> {
    let table = ttf_parser::cff::Table::parse(cff)?;
    let glyphs = table.number_of_glyphs();
    let upem = (1.0 / table.matrix().sx).round().clamp(16.0, 16384.0) as u16;
    let scale = f32::from(upem) / 1000.0;
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_be_bytes());

    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0x000B);
    be16(&mut head, upem);
    head.extend_from_slice(&[0; 16]);
    for v in [0, 0, upem, upem] {
        be16(&mut head, v);
    }
    for v in [0, 8, 2, 0, 0] {
        be16(&mut head, v);
    }

    let mut hmtx = Vec::new();
    let mut max_advance = 0;
    for gid in 0..u32::from(glyphs) {
        let advance = advances.get(&gid).map_or(0, |a| (a * scale).round().clamp(0.0, 65535.0) as u16);
        max_advance = max_advance.max(advance);
        be16(&mut hmtx, advance);
        be16(&mut hmtx, 0);
    }

    // krilla tells fonts apart by their metrics and `checksumAdjustment`, and two pages' wraps
    // of one CFF differ only in `hmtx`: a hash of it keeps them apart in one document.
    let hash = hmtx.iter().fold(0x811C_9DC5_u32, |h, b| (h ^ u32::from(*b)).wrapping_mul(0x0100_0193));
    head[8..12].copy_from_slice(&hash.to_be_bytes());

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    be16(&mut hhea, (u32::from(upem) * 4 / 5) as u16);
    be16(&mut hhea, (-(i32::from(upem) / 5)) as i16 as u16);
    be16(&mut hhea, 0);
    be16(&mut hhea, max_advance);
    hhea.extend_from_slice(&[0; 4]);
    be16(&mut hhea, max_advance);
    be16(&mut hhea, 1);
    hhea.extend_from_slice(&[0; 14]);
    be16(&mut hhea, glyphs);

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0000_5000);
    be16(&mut maxp, glyphs);

    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    post.extend_from_slice(&[0; 28]);

    // Tables in tag order, as the sfnt directory requires.
    let tables: [(&[u8; 4], &[u8]); 6] = [(b"CFF ", cff), (b"head", &head), (b"hhea", &hhea), (b"hmtx", &hmtx), (b"maxp", &maxp), (b"post", &post)];
    let mut out = Vec::new();
    out.extend_from_slice(b"OTTO");
    for v in [6, 64, 2, 32] {
        be16(&mut out, v);
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        be32(&mut out, 0);
        be32(&mut out, u32::try_from(offset).ok()?);
        be32(&mut out, u32::try_from(data.len()).ok()?);
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use craft_pdf::interpret::font::StandardFont;

    #[test]
    fn bare_cff_is_wrapped_in_an_opentype_font_with_the_pdf_widths() {
        // hayro's Foxit Sans substitute is bare CFF, as `FontFile3/Type1C` holds it.
        let (data, _) = StandardFont::Helvetica.get_font_data();
        let cff = (*data).as_ref();
        assert_eq!(cff[0], 1);
        assert!(Font::new(Data::from(cff.to_vec()), 0).is_none(), "krilla takes OpenType only");

        let advances = BTreeMap::from([(1, 278.0), (36, 667.4)]);
        let wrapped = wrap_cff(cff, &advances).unwrap();
        let face = ttf_parser::Face::parse(&wrapped, 0).unwrap();
        assert_eq!(face.units_per_em(), 1000);
        assert_eq!(face.number_of_glyphs(), ttf_parser::cff::Table::parse(cff).unwrap().number_of_glyphs());
        let advance = |id| face.glyph_hor_advance(ttf_parser::GlyphId(id));
        assert_eq!((advance(1), advance(36), advance(2)), (Some(278), Some(667), Some(0)));

        let font = TextFont { data: Arc::new(wrapped.clone()), index: 0, key: 1 };
        assert!(embeds(&font, advances.keys().copied()));

        // Another page's glyphs with the same width sum: krilla must not take it for the same font.
        let other = wrap_cff(cff, &BTreeMap::from([(1, 278.0), (37, 667.4)])).unwrap();
        assert_ne!(Font::new(Data::from(wrapped), 0), Font::new(Data::from(other), 0));
    }

    #[test]
    fn unreadable_bytes_draw_from_outlines() {
        let noted = Noted { source: Some((Arc::new(b"%!PS-AdobeFont-1.0".to_vec()), 0)), advances: BTreeMap::from([(1, 500.0)]) };
        assert!(noted.prepare(7).is_none());
        let noted = Noted { source: Some((Arc::new(vec![1, 0, 4, 1, 0, 0]), 0)), advances: BTreeMap::new() };
        assert!(noted.prepare(7).is_none(), "broken CFF");
    }
}
