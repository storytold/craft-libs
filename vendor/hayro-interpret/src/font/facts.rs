//! PdfCraft patch (107): what a font says about the characters its glyphs draw, beyond the
//! strict text.
//!
//! [`crate::font::Glyph::as_unicode`] reads a glyph's text the way ISO 32000-2 §9.10.2 lays
//! out: `/ToUnicode`, then a known encoding or glyph name. Some fonts still say which character
//! a glyph draws in ways the specification doesn't define: glyph names that only number the
//! character, glyph ids in a standard order, a CID-keyed program's charset. A reader that wants
//! to recover such text (PdfCraft's Copy with formatting) reads it from these facts
//! ([`crate::font::Glyph::with_facts`]); nothing here changes `as_unicode`, and the interpreter
//! doesn't read them itself.
//!
//! The facts are borrowed from the font, built for one call, and cheap: nothing is parsed or
//! decoded to build them. A reader that keeps decisions per font keys them by
//! [`crate::font::Glyph::font_key`], with [`crate::InterpreterSettings::font_per_object`] on
//! so that two font objects with the same dictionary are two fonts.

use crate::CacheKey;
use crate::font::FontFlags;
use crate::util::hash128;
use hayro_cmap::BfString;
use hayro_syntax::object::dict::keys::{
    BASE_FONT, ENCODING, FLAGS, FONT_DESC, FONT_WEIGHT, ITALIC_ANGLE,
};
use hayro_syntax::object::{Dict, Name};
use skrifa::{FontRef, GlyphId};
use std::sync::{Arc, OnceLock};

/// The kind of font a glyph comes from, as a reader of its [`FontFacts`] tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontKind {
    /// A simple Type 1 font, embedded (`/FontFile`) or standard.
    Type1,
    /// A simple font with a CFF program (`/FontFile3`).
    Cff,
    /// A simple TrueType font.
    TrueType,
    /// A Type 0 font whose descendant has a TrueType (or OpenType) program.
    CidTrueType,
    /// A Type 0 font whose descendant is an embedded CID-keyed CFF program (`CIDFontType0`),
    /// whose glyphs are found by CID through the program's charset (ISO 32000-2 §9.7.4.2:
    /// `/CIDToGIDMap` applies to `CIDFontType2` only; a stream there is a producer's error, and
    /// such a font is [`Self::CidOther`]).
    CidCff,
    /// Any other Type 0 font, a substitute for a missing program included.
    CidOther,
    /// A Type 3 font.
    Type3,
}

/// What a font says about itself ([`crate::font::Glyph::with_facts`]). Each field is borrowed
/// from the font; the closures look things up when asked and parse nothing.
pub struct FontFacts<'a> {
    /// The kind of font.
    pub kind: FontKind,
    /// `/BaseFont` as written, subset tag included (readers strip it themselves).
    pub base_font: Option<&'a str>,
    /// The font descriptor's `/Flags` (a Type 0 font's descendant's), any number read and a
    /// real one truncated; bits ISO 32000-2 leaves undefined are dropped and the defined ones
    /// kept.
    pub flags: Option<&'a FontFlags>,
    /// The font descriptor's `/FontWeight`.
    pub font_weight: Option<f32>,
    /// The font descriptor's `/ItalicAngle`.
    pub italic_angle: Option<f32>,
    /// Whether the font has a `/ToUnicode` map (or reads Unicode in a way of its own, such as
    /// a UCS2 CMap or `/ToUnicode /Identity-H`).
    pub has_to_unicode: bool,
    /// Simple fonts: whether `/Encoding` is present (a name or a dictionary); always `false`
    /// for a Type 0 font, whose `/Encoding` is its CMap.
    pub has_encoding: bool,
    /// Simple fonts: the glyph name the encoding (with `/Differences`) gives a code. A
    /// symbolic TrueType font without an encoding, whose code reads as the `post` table name of
    /// the glyph it selects, gives that name (`Asmall`, where the standard encoding's name for
    /// the code is `A`).
    pub names: Option<&'a dyn Fn(u8) -> Option<&'a str>>,
    /// The embedded TrueType or OpenType program, when there is one (not a substitute's).
    pub program: Option<&'a FontRef<'a>>,
    /// Simple TrueType fonts: the glyph a code draws and the code's strict text.
    pub code_glyph: Option<&'a dyn Fn(u8) -> (GlyphId, Option<BfString>)>,
    /// Type 0 fonts: whether `has_to_unicode` comes from a `/ToUnicode` map alone (not the
    /// `/Identity-H` name or an Adobe character collection), with codes that are CIDs and CIDs
    /// that are glyph ids, so its answers can be checked against [`Self::id_text`].
    pub to_unicode_checkable: bool,
    /// The numbers [`GlyphFacts::id`] takes in this font: the CIDs of a CID-keyed CFF program's
    /// charset, in glyph order (at most 65,536), and the codes of a Type 3 font that have a
    /// glyph procedure.
    pub ids: Option<&'a dyn Fn() -> Vec<u32>>,
    /// The width the font gives an id (`/W` by CID, `/Widths` by code), when it gives one.
    pub id_width: Option<&'a dyn Fn(u32) -> Option<f32>>,
    /// For a Type 0 font whose codes are CIDs and CIDs glyph ids (an `Identity-H` or
    /// `Identity-V` CMap and an identity `/CIDToGIDMap`): the strict text of the code that
    /// draws glyph id `id`.
    pub id_text: Option<&'a dyn Fn(u32) -> Option<BfString>>,
    /// Type 0 fonts: whether the encoding CMap is `Identity-H` or `Identity-V`, whatever the
    /// `/CIDToGIDMap` (ISO 32000-2 §9.7.5.2: two-byte codes that are CIDs).
    pub identity_encoding: bool,
    /// How many glyphs the embedded program has: a TrueType or OpenType program's `maxp`
    /// count, a CFF program's number of charstrings. `None` without an embedded program.
    pub program_glyphs: Option<u32>,
    /// A Type 0 font that draws with a substitute for its missing program: its glyph ids, and
    /// its program's weight and style, are the substitute's.
    pub substitute: bool,
}

impl FontFacts<'_> {
    /// The facts of a font of `kind` that says nothing more about itself.
    pub fn of_kind(kind: FontKind) -> Self {
        Self {
            kind,
            base_font: None,
            flags: None,
            font_weight: None,
            italic_angle: None,
            has_to_unicode: false,
            has_encoding: false,
            names: None,
            program: None,
            code_glyph: None,
            to_unicode_checkable: false,
            ids: None,
            id_width: None,
            id_text: None,
            identity_encoding: false,
            program_glyphs: None,
            substitute: false,
        }
    }
}

/// What one glyph says ([`crate::font::Glyph::with_facts`]).
#[derive(Clone, Copy, Debug)]
pub struct GlyphFacts<'a> {
    /// The glyph id the code draws ([`GlyphId::NOTDEF`] for a Type 3 glyph).
    pub gid: GlyphId,
    /// Simple fonts: the code's glyph name.
    pub name: Option<&'a str>,
    /// The number [`FontFacts::ids`] holds for the glyph: a Type 0 glyph's CID, a Type 3
    /// glyph's code. `None` for other fonts, or a code the CMap gives no CID.
    pub id: Option<u32>,
}

/// The parts of a font's dictionary its [`FontFacts`] give that the font doesn't keep
/// otherwise, read once when the font is built.
#[derive(Debug, Default)]
pub(crate) struct DictFacts {
    /// `/BaseFont` as written, subset tag included.
    pub(crate) base_font: Option<Arc<str>>,
    /// The font descriptor's `/Flags` (a Type 0 font's descendant's).
    pub(crate) flags: Option<FontFlags>,
    /// The descriptor's `/FontWeight`.
    pub(crate) font_weight: Option<f32>,
    /// The descriptor's `/ItalicAngle`.
    pub(crate) italic_angle: Option<f32>,
    /// Simple fonts: whether `/Encoding` is present (`false` for a Type 0 font).
    pub(crate) has_encoding: bool,
    /// The key of the font object the font was built for ([`font_object_key`]), set when the
    /// interpreter keeps a font per object ([`crate::InterpreterSettings::font_per_object`]).
    pub(crate) object_key: OnceLock<u128>,
}

impl DictFacts {
    /// From a simple font's dictionary, or a Type 0 font's with its `descendant`.
    pub(crate) fn new(dict: &Dict<'_>, descendant: Option<&Dict<'_>>) -> Self {
        let base_font = dict
            .get::<Name<'_>>(BASE_FONT)
            .map(|n| Arc::from(n.as_str()));
        let descriptor = descendant.unwrap_or(dict).get::<Dict<'_>>(FONT_DESC);
        // Any number, a real one truncated (hayro-syntax's integer read takes `262144.0` as
        // 262144); bits ISO 32000-2 leaves undefined are dropped and the defined ones kept, so a
        // stray bit doesn't cost the font its ForceBold.
        let flags = descriptor
            .as_ref()
            .and_then(|d| d.get::<u32>(FLAGS))
            .map(FontFlags::from_bits_truncate);
        let font_weight = descriptor.as_ref().and_then(|d| d.get::<f32>(FONT_WEIGHT));
        let italic_angle = descriptor.as_ref().and_then(|d| d.get::<f32>(ITALIC_ANGLE));
        Self {
            base_font,
            flags,
            font_weight,
            italic_angle,
            has_encoding: descendant.is_none() && dict.contains_key(ENCODING),
            object_key: OnceLock::new(),
        }
    }

    /// The facts of a font of `kind`, with what its dictionary says.
    pub(crate) fn facts(&self, kind: FontKind) -> FontFacts<'_> {
        FontFacts {
            base_font: self.base_font.as_deref(),
            flags: self.flags.as_ref(),
            font_weight: self.font_weight,
            italic_angle: self.italic_angle,
            has_encoding: self.has_encoding,
            ..FontFacts::of_kind(kind)
        }
    }
}

/// The key of the font object `dict`, as [`crate::font::Glyph::font_key`] gives it to the
/// glyphs of a page read with [`crate::InterpreterSettings::font_per_object`]: its dictionary's
/// [`CacheKey`] and its object. Two objects with the same dictionary are the same font to draw,
/// but each shows its own glyphs, and the interpreter's font cache then keeps them apart by this
/// key. For a reader that finds fonts in a document's resources without drawing them.
pub fn font_object_key(dict: &Dict<'_>) -> u128 {
    let object = dict.obj_id().map(|id| (id.obj_number, id.gen_number));
    hash128(&(dict.cache_key(), object))
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
