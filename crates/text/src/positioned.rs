//! A placed glyph with source mapping, geometry and optional application-owned artwork.

use kurbo::{Affine, BezPath, Point};

/// Application-owned payloads and conversion to document artwork.
pub trait Artwork {
    /// Bitmap glyph data.
    type Image: Clone + std::fmt::Debug;
    /// Colour glyph data.
    type Colour: Clone + std::fmt::Debug;
    /// Curved-baseline metadata.
    type Bend: Clone + std::fmt::Debug;
    /// Application document node.
    type Node;
    /// Transform from image pixels to font space.
    fn image_transform(image: &Self::Image) -> Affine;
    /// Place a bitmap in document space.
    fn image_node(image: &Self::Image, transform: Affine) -> Self::Node;
    /// Place colour artwork in document space.
    fn colour_node(colour: &Self::Colour, transform: Affine) -> Self::Node;
}

/// A glyph after shaping and composition. Coordinates are in the composer's text space;
/// `xf`, `pivot` and `unturned` refer to font space. Image, colour artwork and bend metadata
/// stay generic so the model does not own an application's document or rendering types.
#[derive(Clone, Debug)]
pub struct PositionedGlyph<A: Artwork> {
    /// Outline in text space.
    pub outline: BezPath,
    /// Source style run index.
    pub run: usize,
    /// Source cluster's byte offset.
    pub byte: usize,
    /// Pen position on the baseline.
    pub origin: Point,
    /// Advance including composition spacing.
    pub advance: f64,
    /// Number of source bytes in the cluster.
    pub len: usize,
    /// Baseline angle in radians.
    pub angle: f64,
    /// Composed line index.
    pub line: usize,
    /// Identity of the resolved face.
    pub font_id: u32,
    /// Glyph identifier in that face.
    pub gid: u32,
    /// Transform from glyph font space to text space.
    pub xf: Affine,
    /// Whether the logical leading edge is on the right.
    pub rtl: bool,
    /// Paragraph glyph scaling factor.
    pub scale: f64,
    /// Optional bitmap artwork.
    pub image: Option<A::Image>,
    /// Optional colour artwork.
    pub colour: Option<A::Colour>,
    /// Optional curved-baseline geometry.
    pub bend: Option<A::Bend>,
    /// Transform restoring the glyph before character rotation.
    pub unturned: Affine,
    /// Character rotation pivot in font space.
    pub pivot: Point,
    /// Source-byte caret stops and their fractions of the glyph width.
    pub carets: Vec<(usize, f64)>,
    /// Glyph width excluding composition spacing.
    pub width: f64,
    /// Offset from the left end of the advance to the glyph width.
    pub width_start: f64,
    /// Source byte identifying a warichu block, when present.
    pub warichu: Option<usize>,
    /// Warichu block baseline offset.
    pub warichu_line: f64,
    /// Glyph lies before an open path and is retained for caret mapping only.
    pub off_path: bool,
}

impl<A: Artwork> PositionedGlyph<A> {
    /// Source caret stops as fractions of glyph width.
    pub fn width_stops(&self) -> impl Iterator<Item = (usize, f64)> + '_ {
        std::iter::once((self.byte, 0.0)).chain(self.carets.iter().copied()).chain(std::iter::once((self.byte + self.len, 1.0)))
    }

    /// Source caret stops measured from the logical leading edge.
    pub fn stops_along(&self, extent: f64) -> impl Iterator<Item = (usize, f64)> + '_ {
        let (width, start) = if self.width > 0.0 && self.width.is_finite() {
            (self.width, if self.width_start.is_finite() { self.width_start } else { 0.0 })
        } else {
            (extent, 0.0)
        };
        let ends = (self.byte, self.byte + self.len);
        self.width_stops().map(move |(b, f)| {
            if b == ends.0 {
                (b, 0.0)
            } else if b == ends.1 {
                (b, extent)
            } else if self.rtl {
                (b, extent - start - (1.0 - f) * width)
            } else {
                (b, start + f * width)
            }
        })
    }

    /// Distance from the logical leading edge to the source byte's caret.
    pub fn caret_offset(&self, byte: usize, extent: f64) -> f64 {
        if byte <= self.byte || self.len == 0 {
            return 0.0;
        }
        let mut from = (self.byte, 0.0);
        for (b, d) in self.stops_along(extent).skip(1) {
            if byte <= b {
                let span = b.saturating_sub(from.0).max(1) as f64;
                return from.1 + (d - from.1) * (byte - from.0) as f64 / span;
            }
            from = (b, d);
        }
        extent
    }

    /// Bitmap data and its transform into text space.
    pub fn placed_image(&self) -> Option<(&A::Image, Affine)> {
        self.image.as_ref().map(|image| (image, self.xf * A::image_transform(image)))
    }

    /// Bitmap artwork placed in document space.
    pub fn placed_image_node(&self, to_doc: Affine) -> Option<A::Node> {
        let (image, to_text) = self.placed_image()?;
        Some(A::image_node(image, to_doc * to_text))
    }

    /// Colour artwork placed in document space.
    pub fn placed_colour(&self, to_doc: Affine) -> Option<A::Node> {
        Some(A::colour_node(self.colour.as_ref()?, to_doc * self.xf))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;

    #[derive(Clone, Debug)]
    struct TestArt;
    impl Artwork for TestArt {
        type Image = Affine;
        type Colour = ();
        type Bend = ();
        type Node = Affine;
        fn image_transform(image: &Affine) -> Affine {
            *image
        }
        fn image_node(_: &Affine, transform: Affine) -> Affine {
            transform
        }
        fn colour_node(_: &(), transform: Affine) -> Affine {
            transform
        }
    }
    fn glyph() -> PositionedGlyph<TestArt> {
        PositionedGlyph {
            outline: BezPath::new(),
            run: 0,
            byte: 3,
            origin: Point::ZERO,
            advance: 12.0,
            len: 4,
            angle: 0.0,
            line: 0,
            font_id: 1,
            gid: 4,
            xf: Affine::translate((3.0, 5.0)),
            rtl: false,
            scale: 1.0,
            image: Some(Affine::scale(2.0)),
            colour: Some(()),
            bend: None,
            unturned: Affine::IDENTITY,
            pivot: Point::ZERO,
            carets: vec![(5, 0.5)],
            width: 10.0,
            width_start: 1.0,
            warichu: None,
            warichu_line: 0.0,
            off_path: false,
        }
    }
    #[trial]
    fn feature_config_caret_mapping_tracks_ligatures_rtl_and_kerning() {
        let mut g = glyph();
        assert_eq!((g.caret_offset(3, 12.0), g.caret_offset(5, 12.0), g.caret_offset(7, 12.0)), (0.0, 6.0, 12.0));
        assert_eq!(g.caret_offset(4, 12.0), 3.0);
        g.rtl = true;
        assert_eq!(g.caret_offset(5, 12.0), 6.0);
        assert_eq!(g.caret_offset(5, 8.0), 2.0);
        g.width = f64::NAN;
        assert_eq!(g.caret_offset(5, 8.0), 4.0);
    }
    #[trial]
    fn feature_config_artwork_uses_the_same_composition_order() {
        let g = glyph();
        let doc = Affine::translate((10.0, 20.0));
        let (_, image) = g.placed_image().unwrap();
        assert_eq!(image, g.xf * Affine::scale(2.0));
        assert_eq!(g.placed_image_node(doc), Some(doc * image));
        assert_eq!(g.placed_colour(doc), Some(doc * g.xf));
        let mut empty = g;
        empty.image = None;
        empty.colour = None;
        assert!(empty.placed_image_node(doc).is_none());
        assert!(empty.placed_colour(doc).is_none());
    }
}
