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

    /// Draw a glyph in design units. Empty glyphs such as spaces can produce an empty path.
    pub fn outline(&self, glyph: u32) -> Result<BezPath, OutlineError> {
        let outline = self.font.outline_glyphs().get(GlyphId::new(glyph)).ok_or(OutlineError::MissingGlyph)?;
        let mut pen = PathPen(BezPath::new());
        outline.draw(DrawSettings::unhinted(Size::unscaled(), &self.location), &mut pen)?;
        Ok(pen.0)
    }
}

struct PathPen(BezPath);
impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(x1), f64::from(y1)), (f64::from(x), f64::from(y)));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(x1), f64::from(y1)), (f64::from(x2), f64::from(y2)), (f64::from(x), f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::PathEl;
    use std::sync::Arc;

    #[test]
    fn rejects_invalid_face_and_collection_index() {
        let data = FontData { bytes: Arc::from(&b"not a font"[..]), index: 0 };
        assert!(matches!(FontOutline::new(&data, &[]), Err(OutlineError::InvalidFace)));
    }

    #[test]
    fn pen_preserves_geometry_and_upward_y() {
        let mut pen = PathPen(BezPath::new());
        pen.move_to(1.0, 2.0);
        pen.line_to(3.0, 4.0);
        pen.quad_to(5.0, 6.0, 7.0, 8.0);
        pen.curve_to(9.0, 10.0, 11.0, 12.0, 13.0, 14.0);
        pen.close();
        assert_eq!(
            pen.0.elements(),
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
}
