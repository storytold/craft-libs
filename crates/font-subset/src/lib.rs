#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The font subsetter, on HarfBuzz (`harfbuzz-subset-sys`). Native only: HarfBuzz's C++ builds with the target's
//! C++ compiler.
//!
//! - [`subset`]: one face, a set of codepoints and an instance in, a renamed WOFF2 out (font previews).
//!   [`subset_renamed`] and [`encode_woff2`] are its two halves.
//! - [`cut_face`]: one face of a font or collection cut to a set of codepoints, every layout feature kept, not renamed
//!   (build-time interface faces).
//! - [`embed_permission`]: what a font's OS/2 `fsType` allows an exported document (PDF) to embed.
//!
//! Synchronous and free of I/O.
#![warn(missing_docs)]

mod cff;
pub mod closure;
pub mod codepoints;
mod embed;
mod instance;
mod rename;
#[cfg(test)]
mod tests;
mod woff2;

use std::collections::BTreeSet;

use harfbuzz_subset_sys::safe as hb;
use skrifa::FontRef;

pub use cff::CffError;
pub use closure::closure;
pub use embed::{EmbedPermission, Embedding, embed_permission};
pub use instance::Instance;
pub use rename::{Licence, Naming};
pub use woff2::{Quality, Woff2Error};

/// Why a subset failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The font, or a table renaming needs, does not parse.
    #[error("cannot read font: {0}")]
    Read(#[from] skrifa::raw::ReadError),
    /// HarfBuzz refused the face, an axis location or the subset.
    #[error("HarfBuzz could not subset the font")]
    Subset,
    /// The renamed font could not be written.
    #[error("cannot write font: {0}")]
    Build(#[from] write_fonts::BuilderError),
    /// The CFF table could not be renamed.
    #[error(transparent)]
    Cff(#[from] CffError),
    /// A generated name contains a Reserved Font Name.
    #[error("generated name {0:?} contains a Reserved Font Name")]
    ReservedName(String),
    /// A CFF string still names the upstream font after renaming.
    #[error("CFF string {0:?} still names the upstream font")]
    CffOldName(String),
    /// The WOFF2 container could not be written.
    #[error("WOFF2 encoding failed: {0}")]
    Woff2(#[from] woff2::Woff2Error),
}

/// Subsets `font` to `codepoints` (already through [`closure`]) at
/// `instance`, renames it for its licence and encodes it as WOFF2.
pub fn subset(font: &[u8], codepoints: &BTreeSet<u32>, instance: &Instance, naming: &Naming, quality: Quality) -> Result<Vec<u8>, Error> {
    encode_woff2(&subset_renamed(font, codepoints, instance, naming)?, quality)
}

/// [`subset`] before WOFF2 encoding: the renamed sfnt, for checks that
/// need to read the subset.
pub fn subset_renamed(font: &[u8], codepoints: &BTreeSet<u32>, instance: &Instance, naming: &Naming) -> Result<Vec<u8>, Error> {
    let sfnt = subset_sfnt(font, codepoints, instance)?;
    rename::rename(&sfnt, naming)
}

/// Encodes a [`subset_renamed`] result as WOFF2.
pub fn encode_woff2(sfnt: &[u8], quality: Quality) -> Result<Vec<u8>, Error> {
    Ok(woff2::encode(sfnt, quality)?)
}

fn subset_sfnt(font: &[u8], codepoints: &BTreeSet<u32>, instance: &Instance) -> Result<Vec<u8>, Error> {
    let font_ref = FontRef::new(font)?;
    let pins = instance::pins(&font_ref, instance);
    let face = hb::Face::new(font, 0).ok_or(Error::Subset)?;
    let mut input = hb::Input::new().ok_or(Error::Subset)?;
    input.add_flags(harfbuzz_subset_sys::HB_SUBSET_FLAGS_NO_HINTING);
    {
        let mut unicodes = input.unicodes();
        for &cp in codepoints.iter().filter(|&&cp| char::from_u32(cp).is_some()) {
            unicodes.add(cp);
        }
    }
    if keeps_legacy_kern(&font_ref) {
        keep_kern(&mut input);
    }
    {
        let mut name_ids = input.name_ids();
        for id in rename::KEPT_IDS {
            name_ids.add(id);
        }
        if pins.is_none() {
            // fvar and STAT name their axes and instances with IDs from 256.
            name_ids.add_range(256, 32767);
        }
    }
    if let Some(pins) = pins {
        let pinned = input.pin_all_axes_to_default(&face)
            && pins.iter().all(|(tag, value)| input.pin_axis_location(&face, u32::from_be_bytes(tag.to_be_bytes()), *value));
        if !pinned {
            return Err(Error::Subset);
        }
    }
    face.subset(&input).ok_or(Error::Subset)
}

/// Face `index` of `font` (0 for a single font) with the glyphs `codepoints` map to and the glyphs its layout
/// tables reach from them (vertical forms, ligatures, marks). Every layout feature and script is kept, and so is the
/// hinting; the name table keeps IDs 0–6 in US English. Variations stay; nothing is renamed. The same input gives the
/// same bytes. The result is an sfnt.
pub fn cut_face(font: &[u8], index: u32, codepoints: &BTreeSet<u32>) -> Result<Vec<u8>, Error> {
    let face = hb::Face::new(font, index).ok_or(Error::Subset)?;
    let mut input = hb::Input::new().ok_or(Error::Subset)?;
    {
        let mut unicodes = input.unicodes();
        for &cp in codepoints.iter().filter(|&&cp| char::from_u32(cp).is_some()) {
            unicodes.add(cp);
        }
    }
    input.layout_features().fill();
    if keeps_legacy_kern(&FontRef::from_index(font, index)?) {
        keep_kern(&mut input);
    }
    face.subset(&input).ok_or(Error::Subset)
}

/// Keeps the `kern` table whole. HarfBuzz has no subsetter for `kern` and drops it by default; the table indexes by
/// glyph ID, so the GIDs must stay.
fn keep_kern(input: &mut hb::Input) {
    let kern = u32::from_be_bytes(*b"kern");
    input.drop_tables().remove(kern);
    input.no_subset_tables().add(kern);
    input.add_flags(harfbuzz_subset_sys::HB_SUBSET_FLAGS_RETAIN_GIDS);
}

/// True when the font has a `kern` table and no `GPOS` `kern` feature, so HarfBuzz shapes with
/// `kern`. A GPOS that doesn't parse counts as
/// having one, so `kern` is dropped as for any other font.
fn keeps_legacy_kern(f: &FontRef) -> bool {
    use skrifa::raw::TableProvider;
    f.table_data(skrifa::Tag::new(b"kern")).is_some()
        && (f.table_data(skrifa::Tag::new(b"GPOS")).is_none()
            || f.gpos().is_ok_and(|gpos| {
                gpos.feature_list().is_ok_and(|list| list.feature_records().iter().all(|r| r.feature_tag() != skrifa::Tag::new(b"kern")))
            }))
}
