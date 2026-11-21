use crate::CacheKey;
use crate::font::cid::Type0Font;
use crate::font::true_type::TrueTypeFont;
use crate::font::type1::Type1Font;
use hayro_cmap::BfString;
use kurbo::BezPath;
use skrifa::GlyphId;
use skrifa::outline::OutlinePen;
use std::rc::Rc;

/// Font data and metadata for downstream use.
#[derive(Clone)]
pub struct OutlineFontData {
    /// Raw font bytes (TrueType/OpenType/CFF data).
    pub data: crate::font::FontData,
    /// Cache key for font deduplication.
    pub cache_key: u128,
    /// Craft patch (flatten): the face index of `data` in a font collection.
    pub index: u32,
    /// Craft patch (flatten): true when `data` comes from the PDF (`FontFile*`); false when it
    /// is the font resolver's substitute for a font the PDF doesn't embed.
    pub embedded: bool,
    /// PostScript name (e.g., "TimesNewRomanPS-BoldMT").
    pub postscript_name: Option<String>,
    /// Font weight (100-900, 400=normal, 700=bold).
    pub weight: Option<u32>,
    /// Whether the font is italic/oblique.
    pub is_italic: bool,
    /// Whether the font is serif (vs sans-serif).
    pub is_serif: bool,
    /// Whether the font is monospace.
    pub is_monospace: bool,
}

pub(crate) struct OutlinePath(BezPath);

impl OutlinePath {
    pub(crate) fn new() -> Self {
        Self(BezPath::new())
    }

    pub(crate) fn take(self) -> BezPath {
        self.0
    }
}

impl OutlinePen for OutlinePath {
    #[inline]
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x, y));
    }

    #[inline]
    fn line_to(&mut self, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.line_to((x, y));
        }
    }

    #[inline]
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.quad_to((cx, cy), (x, y));
        }
    }

    #[inline]
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.curve_to((cx0, cy0), (cx1, cy1), (x, y));
        }
    }

    #[inline]
    fn close(&mut self) {
        if !self.0.elements().is_empty() {
            self.0.close_path();
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum OutlineFont {
    Type1(Rc<Type1Font>),
    TrueType(Rc<TrueTypeFont>),
    Type0(Rc<Type0Font>),
}

impl CacheKey for OutlineFont {
    fn cache_key(&self) -> u128 {
        match self {
            Self::Type1(f) => f.cache_key(),
            Self::TrueType(t) => t.cache_key(),
            Self::Type0(t0) => t0.cache_key(),
        }
    }
}

impl OutlineFont {
    pub(crate) fn outline_glyph(&self, glyph: GlyphId, code: u32) -> BezPath {
        match self {
            Self::Type1(t) => t.outline_glyph(glyph),
            Self::TrueType(t) => t.outline_glyph(glyph),
            Self::Type0(t) => t.outline_glyph(glyph, code),
        }
    }

    pub(crate) fn char_code_to_unicode(&self, char_code: u32) -> Option<BfString> {
        match self {
            Self::Type1(t) => t.char_code_to_unicode(char_code),
            Self::TrueType(t) => t.char_code_to_unicode(char_code),
            Self::Type0(t) => t.char_code_to_unicode(char_code),
        }
    }

    /// PdfCraft patch (107): the kind of font, as [`crate::font::FontFacts::kind`] gives it.
    pub(crate) fn font_kind(&self) -> crate::font::FontKind {
        match self {
            Self::Type1(t) => t.font_kind(),
            Self::TrueType(_) => crate::font::FontKind::TrueType,
            Self::Type0(t) => t.font_kind(),
        }
    }

    /// PdfCraft patch (107): `f` called with the font's facts and those of code `char_code`
    /// drawn as glyph `gid` ([`crate::font::Glyph::with_facts`]).
    pub(crate) fn with_facts<R>(
        &self,
        char_code: u32,
        gid: GlyphId,
        f: impl FnOnce(&crate::font::FontFacts<'_>, &crate::font::GlyphFacts<'_>) -> R,
    ) -> R {
        match self {
            Self::Type1(t) => t.with_facts(char_code, gid, f),
            Self::TrueType(t) => t.with_facts(char_code, gid, f),
            Self::Type0(t) => t.with_facts(char_code, gid, f),
        }
    }

    /// PdfCraft patch (107): the font's key ([`crate::font::Glyph::font_key`]).
    pub(crate) fn font_key(&self) -> u128 {
        let facts = match self {
            Self::Type1(t) => t.dict_facts(),
            Self::TrueType(t) => t.dict_facts(),
            Self::Type0(t) => t.dict_facts(),
        };
        facts
            .object_key
            .get()
            .copied()
            .unwrap_or_else(|| self.cache_key())
    }

    /// Get the advance width for a glyph by character code.
    pub(crate) fn glyph_advance_width(&self, char_code: u32) -> Option<f32> {
        match self {
            Self::Type1(t) => t.glyph_width(char_code as u8),
            Self::TrueType(t) => Some(t.glyph_width(char_code as u8)),
            Self::Type0(t) => Some(t.code_advance(char_code).x as f32),
        }
    }

    /// Get raw font bytes and metadata.
    ///
    /// Returns None for Type1 (`FontFile`), standard and non-embedded simple fonts.
    pub(crate) fn font_data(&self) -> Option<OutlineFontData> {
        match self {
            // Craft patch (flatten): Type1C (`FontFile3`) fonts hand out their bare CFF bytes.
            // The font dictionary's descriptive fields are not read.
            Self::Type1(t) => Some(OutlineFontData {
                data: t.font_data()?,
                cache_key: t.cache_key(),
                index: 0,
                embedded: true,
                postscript_name: None,
                weight: None,
                is_italic: false,
                is_serif: false,
                is_monospace: false,
            }),
            Self::TrueType(t) => Some(OutlineFontData {
                data: t.font_data()?,
                cache_key: t.cache_key(),
                // Craft patch (flatten): only an embedded `FontFile2`, read as face 0, has bytes.
                index: 0,
                embedded: true,
                postscript_name: t.postscript_name().map(|s| s.to_string()),
                weight: t.weight(),
                is_italic: t.is_italic(),
                is_serif: t.is_serif(),
                is_monospace: t.is_monospace(),
            }),
            Self::Type0(t) => Some(OutlineFontData {
                data: t.font_data()?,
                cache_key: t.cache_key(),
                // Craft patch (flatten): a substitute from the resolver is not embedded.
                index: t.font_index(),
                embedded: t.is_embedded(),
                postscript_name: t.postscript_name().map(|s| s.to_string()),
                weight: t.weight(),
                is_italic: t.is_italic(),
                is_serif: t.is_serif(),
                is_monospace: t.is_monospace(),
            }),
        }
    }
}
