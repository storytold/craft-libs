//! Cutting an interface face down to the characters the translation catalogs use, with skera.

use std::collections::BTreeSet;

use skera::{Plan, SubsetFlags, subset_font};
use skrifa::raw::collections::IntSet;
use skrifa::raw::types::NameId;
use skrifa::{FontRef, GlyphId, Tag};

/// Why a face could not be cut.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The face has CFF outlines: skera's CFF output draws the wrong glyphs.
    NotTrueType,
    Other(String),
}

/// Face `index` of `font` with the glyphs of those `chars` it maps and the glyphs its layout tables
/// reach from them (vertical forms, ligatures). Every layout feature and script is kept; the name
/// table keeps IDs 0–6 in US English, as fonttools' subsetter does. The same input gives the same
/// bytes.
pub(crate) fn subset(font: &[u8], index: u32, chars: &BTreeSet<char>) -> Result<Vec<u8>, Failure> {
    let font = FontRef::from_index(font, index).map_err(|e| Failure::Other(e.to_string()))?;
    if font.table_data(Tag::new(b"glyf")).is_none() {
        return Err(Failure::NotTrueType);
    }
    let mut unicodes = IntSet::<u32>::empty();
    unicodes.extend(chars.iter().map(|&c| u32::from(c)));
    let mut name_ids = IntSet::<NameId>::empty();
    name_ids.insert_range(NameId::new(0)..=NameId::new(6));
    let mut languages = IntSet::<u16>::empty();
    languages.insert(0x0409);
    let plan = Plan::new(
        &IntSet::<GlyphId>::empty(),
        &unicodes,
        &font,
        SubsetFlags::default(),
        &IntSet::<Tag>::empty(),
        &IntSet::<Tag>::all(),
        &IntSet::<Tag>::all(),
        &name_ids,
        &languages,
    );
    subset_font(&font, &plan).map_err(|e| Failure::Other(e.to_string()))
}
