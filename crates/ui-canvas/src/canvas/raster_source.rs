//! `RasterSource`, the document composite a raster display reads, and `PixelRect`, the whole-pixel rectangle it
//! reads by.

use crate::primitives::Revision;
use craft_behavior::PixelSize;
use egui::ColorImage;

/// A rectangle of whole document pixels, `x0..x1` by `y0..y1` (exclusive ends).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PixelRect {
    /// First column, inclusive.
    pub x0: i32,
    /// First row, inclusive.
    pub y0: i32,
    /// End column, exclusive.
    pub x1: i32,
    /// End row, exclusive.
    pub y1: i32,
}

impl PixelRect {
    /// A rectangle `x0..x1` by `y0..y1`.
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// The whole of a document of `size` pixels.
    pub fn of_size(size: PixelSize) -> Self {
        Self::new(0, 0, i32::try_from(size.width).unwrap_or(i32::MAX), i32::try_from(size.height).unwrap_or(i32::MAX))
    }

    /// Width in pixels, 0 when inverted.
    pub fn width(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    /// Height in pixels, 0 when inverted.
    pub fn height(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }

    /// Whether the rectangle covers no pixel.
    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    /// The part inside both; empty when they do not overlap.
    #[must_use]
    pub fn intersect(&self, other: &PixelRect) -> PixelRect {
        let r = PixelRect::new(self.x0.max(other.x0), self.y0.max(other.y0), self.x1.min(other.x1), self.y1.min(other.y1));
        if r.is_empty() { PixelRect::default() } else { r }
    }

    /// The smallest rect holding both (an empty one adds nothing).
    #[must_use]
    pub fn union(&self, other: &PixelRect) -> PixelRect {
        match (self.is_empty(), other.is_empty()) {
            (true, true) => PixelRect::default(),
            (true, false) => *other,
            (false, true) => *self,
            (false, false) => PixelRect::new(self.x0.min(other.x0), self.y0.min(other.y0), self.x1.max(other.x1), self.y1.max(other.y1)),
        }
    }
}

/// The document's composite, as a raster display reads it. The app implements it over its document (PhotoCraft's
/// compositor, a PDF page renderer, a video frame).
pub trait RasterSource {
    /// The document's size in pixels.
    fn size(&self) -> PixelSize;

    /// Changes whenever the pixels or the size change; the `Revision` the texture stores key their pictures by.
    fn revision(&self) -> Revision;

    /// The region that changed since the source had revision `since`, as one rectangle: empty when nothing changed.
    /// `None` when the source cannot tell (it no longer remembers `since`, or the whole document or its size
    /// changed), and a display then reads everything. A display asks with the revision its picture shows, so the
    /// answer covers every change since then, including those of passes the display did not run in.
    fn damage(&self, since: Revision) -> Option<PixelRect>;

    /// Writes the composite of `region` into `out` (sRGBA, unmultiplied), one texel per `factor` pixels each way
    /// (`factor` is a power of two for a detail texture and `texture_factor` of `primitives::textures` for the base
    /// one; 1 is full resolution). `out` is already `region / factor` texels, rounded up.
    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage);
}
