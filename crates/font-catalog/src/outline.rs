//! Unhinted glyph outlines and metrics for catalog faces.

use kurbo::BezPath;
use skrifa::instance::{Location, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};

use crate::{FontData, LoadedFace};

/// A parsed face and its variation location. Coordinates use font design units, with y up.
/// The bytes can come from a catalog or any other source, including a collection face.
pub struct FontOutline<'a> {
    font: FontRef<'a>,
    location: Location,
}

/// Receives unhinted glyph commands in design units (y up). Implementations own their
/// storage and resource policy; drawing writes directly to the sink without an intermediate path.
pub trait GlyphPen {
    /// Start a contour.
    fn move_to(&mut self, x: f32, y: f32);
    /// Add a straight segment.
    fn line_to(&mut self, x: f32, y: f32);
    /// Add a quadratic segment.
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32);
    /// Add a cubic segment.
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32);
    /// Close the current contour.
    fn close(&mut self);
}

struct Sink<'a>(&'a mut dyn GlyphPen);
impl OutlinePen for Sink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.curve_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// Why a font or glyph outline could not be read.
#[derive(Debug, thiserror::Error)]
pub enum OutlineError {
    /// The face index or font data is invalid.
    #[error("invalid font face")]
    InvalidFace,
    /// The glyph has no outline in this face.
    #[error("glyph has no outline")]
    MissingGlyph,
    /// The glyph's outline is malformed.
    #[error("cannot draw glyph outline: {0}")]
    Draw(#[from] skrifa::outline::DrawError),
    /// The glyph emitted more path elements than the safety limit.
    #[error("glyph outline exceeds the element limit")]
    TooComplex,
    /// The outline starts without a move command.
    #[error("glyph outline has no initial move")]
    InvalidPath,
}

impl<'a> FontOutline<'a> {
    /// Parse the given face and select axis values in user units. Unknown axes are ignored and
    /// values are clamped to their axis ranges by the font parser.
    pub fn new(data: &'a FontData, variations: &[([u8; 4], f32)]) -> Result<Self, OutlineError> {
        Self::from_bytes(&data.bytes, data.index, variations)
    }

    /// Parse borrowed bytes and a collection face index without copying the font.
    pub fn from_bytes(bytes: &'a [u8], index: u32, variations: &[([u8; 4], f32)]) -> Result<Self, OutlineError> {
        let font = FontRef::from_index(bytes, index).map_err(|_| OutlineError::InvalidFace)?;
        let location = font.axes().location(variations.iter().map(|(tag, value)| (Tag::new(tag), *value)));
        Ok(Self { font, location })
    }

    /// Parse a catalog face, retaining the axis settings of a named instance.
    pub fn from_loaded(face: &'a LoadedFace) -> Result<Self, OutlineError> {
        Self::new(&face.data, &face.info.variations)
    }

    /// Font-wide metrics in unscaled design units at this variation location.
    pub fn metrics(&self) -> skrifa::metrics::Metrics {
        self.font.metrics(Size::unscaled(), &self.location)
    }

    /// Resolve a Unicode scalar to a glyph identifier.
    pub fn glyph_for(&self, character: char) -> Option<u32> {
        self.font.charmap().map(character).map(|id| id.to_u32())
    }

    /// Stream an unhinted glyph into the caller's sink. On an error, commands already sent
    /// remain in the sink. The sink owns any allocation and enforces its own resource limit.
    pub fn draw_to(&self, glyph: u32, pen: &mut dyn GlyphPen) -> Result<(), OutlineError> {
        let outline = self.font.outline_glyphs().get(GlyphId::new(glyph)).ok_or(OutlineError::MissingGlyph)?;
        outline.draw(DrawSettings::unhinted(Size::unscaled(), &self.location), &mut Sink(pen))?;
        Ok(())
    }

    /// Draw a glyph in design units. Empty glyphs such as spaces can produce an empty path.
    /// A malformed or excessively complex glyph returns an error instead of partial geometry.
    pub fn outline(&self, glyph: u32) -> Result<BezPath, OutlineError> {
        let (path, status) = self.outline_recovering(glyph)?;
        status.map(|()| path)
    }

    /// Draw a glyph while retaining the bounded prefix produced before a draw error. This is
    /// useful for consumers preserving existing rendering of damaged fonts. The inner result
    /// reports whether the returned path is complete; the outer error means no glyph was found.
    /// Paths stop growing at 100,000 elements, even when the font is untrusted.
    pub fn outline_recovering(&self, glyph: u32) -> Result<(BezPath, Result<(), OutlineError>), OutlineError> {
        let outline = self.font.outline_glyphs().get(GlyphId::new(glyph)).ok_or(OutlineError::MissingGlyph)?;
        let mut pen = PathPen::new();
        let result = outline.draw(DrawSettings::unhinted(Size::unscaled(), &self.location), &mut pen);
        let status = if pen.invalid {
            Err(OutlineError::InvalidPath)
        } else if pen.truncated {
            Err(OutlineError::TooComplex)
        } else {
            result.map(|_| ()).map_err(OutlineError::Draw)
        };
        Ok((pen.path, status))
    }
}

const MAX_OUTLINE_ELEMENTS: usize = 100_000;
struct PathPen {
    path: BezPath,
    truncated: bool,
    invalid: bool,
}
impl PathPen {
    fn new() -> Self {
        Self { path: BezPath::new(), truncated: false, invalid: false }
    }
    fn has_start(&mut self) -> bool {
        if self.path.elements().is_empty() {
            self.invalid = true;
            false
        } else {
            true
        }
    }
    fn room(&mut self) -> bool {
        if self.path.elements().len() >= MAX_OUTLINE_ELEMENTS {
            self.truncated = true;
            false
        } else {
            true
        }
    }
}
impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.room() {
            self.path.move_to((f64::from(x), f64::from(y)));
        }
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if self.has_start() && self.room() {
            self.path.line_to((f64::from(x), f64::from(y)));
        }
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        if self.has_start() && self.room() {
            self.path.quad_to((f64::from(x1), f64::from(y1)), (f64::from(x), f64::from(y)));
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        if self.has_start() && self.room() {
            self.path.curve_to((f64::from(x1), f64::from(y1)), (f64::from(x2), f64::from(y2)), (f64::from(x), f64::from(y)));
        }
    }
    fn close(&mut self) {
        if self.has_start() && self.room() {
            self.path.close_path();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::PathEl;
    use std::sync::Arc;

    impl GlyphPen for PathPen {
        fn move_to(&mut self, x: f32, y: f32) {
            OutlinePen::move_to(self, x, y);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            OutlinePen::line_to(self, x, y);
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            OutlinePen::quad_to(self, x1, y1, x, y);
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            OutlinePen::curve_to(self, x1, y1, x2, y2, x, y);
        }
        fn close(&mut self) {
            OutlinePen::close(self);
        }
    }

    #[test]
    fn rejects_invalid_face_and_collection_index() {
        let data = FontData { bytes: Arc::from(&b"not a font"[..]), index: 0 };
        assert!(matches!(FontOutline::new(&data, &[]), Err(OutlineError::InvalidFace)));
    }

    #[test]
    fn pen_preserves_geometry_and_upward_y() {
        let mut pen = PathPen::new();
        OutlinePen::move_to(&mut pen, 1.0, 2.0);
        OutlinePen::line_to(&mut pen, 3.0, 4.0);
        OutlinePen::quad_to(&mut pen, 5.0, 6.0, 7.0, 8.0);
        OutlinePen::curve_to(&mut pen, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0);
        OutlinePen::close(&mut pen);
        assert_eq!(
            pen.path.elements(),
            &[
                PathEl::MoveTo((1.0, 2.0).into()),
                PathEl::LineTo((3.0, 4.0).into()),
                PathEl::QuadTo((5.0, 6.0).into(), (7.0, 8.0).into()),
                PathEl::CurveTo((9.0, 10.0).into(), (11.0, 12.0).into(), (13.0, 14.0).into()),
                PathEl::ClosePath,
            ]
        );
    }

    #[test]
    fn streaming_adapter_forwards_each_command_and_rejects_an_invalid_prefix() {
        let mut pen = PathPen::new();
        {
            let mut sink = Sink(&mut pen);
            OutlinePen::move_to(&mut sink, 1.0, 2.0);
            OutlinePen::line_to(&mut sink, 3.0, 4.0);
            OutlinePen::quad_to(&mut sink, 5.0, 6.0, 7.0, 8.0);
            OutlinePen::curve_to(&mut sink, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0);
            OutlinePen::close(&mut sink);
        }
        assert_eq!(pen.path.elements().len(), 5);
        assert_eq!(pen.path.elements()[2], PathEl::QuadTo((5.0, 6.0).into(), (7.0, 8.0).into()));
        let mut invalid = PathPen::new();
        OutlinePen::line_to(&mut invalid, 1.0, 2.0);
        assert!(invalid.invalid);
        assert!(invalid.path.elements().is_empty());
    }

    #[test]
    fn pen_caps_path_growth() {
        let mut pen = PathPen::new();
        OutlinePen::move_to(&mut pen, 0.0, 0.0);
        for _ in 0..MAX_OUTLINE_ELEMENTS {
            OutlinePen::line_to(&mut pen, 1.0, 2.0);
        }
        assert_eq!(pen.path.elements().len(), MAX_OUTLINE_ELEMENTS);
        assert!(pen.truncated);
    }

    #[test]
    fn reads_craft_font_when_available() {
        let Ok(dir) = std::env::var("CRAFT_FONTS_DIR") else { return };
        let bytes = std::fs::read(std::path::Path::new(&dir).join("fonts/noto-sans-hebrew/NotoSansHebrew-Regular.ttf")).unwrap();
        let data = FontData { bytes: bytes.into(), index: 0 };
        let face = FontOutline::new(&data, &[]).unwrap();
        assert!(face.metrics().units_per_em > 0);
        let glyph = face.glyph_for('א').unwrap();
        assert!(!face.outline(glyph).unwrap().elements().is_empty());
        assert!(matches!(face.outline(u32::MAX), Err(OutlineError::MissingGlyph)));
        let invalid_index = FontData { bytes: data.bytes.clone(), index: u32::MAX };
        assert!(matches!(FontOutline::new(&invalid_index, &[]), Err(OutlineError::InvalidFace)));
    }

    #[test]
    fn variation_changes_glyph_geometry() {
        let Ok(dir) = std::env::var("CRAFT_FONTS_DIR") else { return };
        let bytes = std::fs::read(std::path::Path::new(&dir).join("fonts/noto-sans-arabic/NotoSansArabic.ttf")).unwrap();
        let data = FontData { bytes: bytes.into(), index: 0 };
        let light = FontOutline::new(&data, &[(*b"wght", 100.0)]).unwrap();
        let bold = FontOutline::new(&data, &[(*b"wght", 900.0)]).unwrap();
        let glyph = light.glyph_for('ب').unwrap();
        let light_path = light.outline(glyph).unwrap();
        let bold_path = bold.outline(glyph).unwrap();
        assert!(!light_path.elements().is_empty());
        assert_ne!(light_path, bold_path);
    }

    // Construct a minimal CFF font so CI can exercise the public API without installed fonts.
    fn cff_font(program: &[u8]) -> Vec<u8> {
        let mut cff = vec![1, 0, 4, 4, 0, 1, 1, 1, 2, b'X', 0, 1, 1, 1, 7, 29, 0, 0, 0, 25, 17, 0, 0, 0, 0];
        cff.extend_from_slice(&[0, 1, 4]);
        cff.extend_from_slice(&1u32.to_be_bytes());
        cff.extend_from_slice(&((program.len() as u32) + 1).to_be_bytes());
        cff.extend_from_slice(program);
        let mut builder = write_fonts::FontBuilder::new();
        let mut head = vec![0; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        let mut hhea = vec![0; 36];
        hhea[34..36].copy_from_slice(&1u16.to_be_bytes());
        builder.add_raw(write_fonts::types::Tag::new(b"head"), head);
        builder.add_raw(write_fonts::types::Tag::new(b"hhea"), hhea);
        builder.add_raw(write_fonts::types::Tag::new(b"hmtx"), vec![0, 100, 0, 0]);
        builder.add_raw(write_fonts::types::Tag::new(b"maxp"), vec![0, 0, 0x50, 0, 0, 1]);
        builder.add_raw(write_fonts::types::Tag::new(b"CFF "), cff);
        builder.build()
    }

    #[test]
    fn malformed_cff_recovers_the_same_prefix_as_direct_drawing() {
        // Move and line, then callsubr without an operand: a prefix is emitted before failure.
        let bytes = cff_font(&[139, 139, 21, 239, 139, 5, 10, 14]);
        let gid = 0;
        let f = FontRef::new(&bytes).unwrap();
        let mut old = PathPen::new();
        let result =
            f.outline_glyphs().get(GlyphId::new(gid)).unwrap().draw(DrawSettings::unhinted(Size::unscaled(), &Location::default()), &mut old);
        assert!(result.is_err());
        assert!(!old.path.elements().is_empty());
        let shared = FontOutline::from_bytes(&bytes, 0, &[]).unwrap();
        assert_eq!(shared.metrics().units_per_em, 1000);
        assert_eq!(shared.glyph_for('A'), None);
        let (prefix, status) = shared.outline_recovering(gid).unwrap();
        assert!(matches!(status, Err(OutlineError::Draw(_))));
        assert_eq!(prefix, old.path);
        let mut streamed = PathPen::new();
        assert!(matches!(shared.draw_to(gid, &mut streamed), Err(OutlineError::Draw(_))));
        assert_eq!(streamed.path, old.path);
        assert!(matches!(shared.outline(gid), Err(OutlineError::Draw(_))));
    }

    #[test]
    fn streaming_preserves_a_large_valid_cff_outline() {
        // Minimal CFF with more than 100,000 line segments.
        let mut program = vec![139, 139, 21];
        for _ in 0..100_001 {
            program.extend_from_slice(&[140, 6]);
        }
        program.push(14);
        let bytes = cff_font(&program);
        let shared = FontOutline::from_bytes(&bytes, 0, &[]).unwrap();
        struct Count(usize);
        impl GlyphPen for Count {
            fn move_to(&mut self, _: f32, _: f32) {
                self.0 += 1;
            }
            fn line_to(&mut self, _: f32, _: f32) {
                self.0 += 1;
            }
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
                self.0 += 1;
            }
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
                self.0 += 1;
            }
            fn close(&mut self) {
                self.0 += 1;
            }
        }
        impl OutlinePen for Count {
            fn move_to(&mut self, x: f32, y: f32) {
                GlyphPen::move_to(self, x, y);
            }
            fn line_to(&mut self, x: f32, y: f32) {
                GlyphPen::line_to(self, x, y);
            }
            fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
                GlyphPen::quad_to(self, x1, y1, x, y);
            }
            fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
                GlyphPen::curve_to(self, x1, y1, x2, y2, x, y);
            }
            fn close(&mut self) {
                GlyphPen::close(self);
            }
        }
        let mut old = Count(0);
        FontRef::new(&bytes)
            .unwrap()
            .outline_glyphs()
            .get(GlyphId::new(0))
            .unwrap()
            .draw(DrawSettings::unhinted(Size::unscaled(), &Location::default()), &mut old)
            .unwrap();
        let mut count = Count(0);
        shared.draw_to(0, &mut count).unwrap();
        assert_eq!(count.0, old.0);
        assert!(count.0 > MAX_OUTLINE_ELEMENTS);
        assert!(matches!(shared.outline(0), Err(OutlineError::TooComplex)));
    }
}
