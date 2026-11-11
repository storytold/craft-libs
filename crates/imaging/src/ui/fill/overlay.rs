//! [`FillOverlay`]: the sampling area as a tint source for the canvas, and the fill area's outline.
//!
//! The planes are the app's. The tint is a [`RasterSource`] the full-window canvas draws over the picture with
//! `RasterOverlay::tint` (the colour and opacity are the user's settings, data, not theme tokens); the fill area's
//! ants are a `Marquee` the workspace shows from [`fill_area_edges`].

use egui::{Color32, ColorImage};

use craft_behavior::PixelSize;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;

use crate::trace::{CoverageGrid, GRID_EDGE_LIMIT, edges_of_grid};

/// A plane of the app's coverage: `bits` row-major over `size` cells of `texel` document pixels from `origin`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct SamplingPlane {
    /// The document position of the first cell's corner.
    pub origin: [i32; 2],
    /// The columns and rows.
    pub size: [u32; 2],
    /// Document pixels per cell side.
    pub texel: u32,
    /// Whether each cell is covered.
    pub bits: Vec<bool>,
}

impl SamplingPlane {
    /// A plane of `size` columns and rows of `texel` document pixels from `origin`, with `bits` row-major.
    pub fn new(origin: [i32; 2], size: [u32; 2], texel: u32, bits: Vec<bool>) -> Self {
        Self { origin, size, texel, bits }
    }

    /// Whether the document point `(x, y)` lies in the plane's extent, and if so whether its cell is covered.
    fn cell_at(&self, x: f64, y: f64) -> Option<bool> {
        let texel = f64::from(self.texel.max(1));
        let cell = |at: f64, origin: i32, size: u32| {
            let c = ((at - f64::from(origin)) / texel).floor();
            (c >= 0.0 && c < f64::from(size)).then_some(c as usize)
        };
        let (column, row) = (cell(x, self.origin[0], self.size[0])?, cell(y, self.origin[1], self.size[1])?);
        Some(self.bits.get(row * self.size[0] as usize + column).copied().unwrap_or(false))
    }
}

/// The sampling tint as a raster source over the whole document: opaque white where the tint goes and transparent
/// elsewhere, so the overlay's tint colour (with the user's opacity) gives the colour.
///
/// - **Sampling Area** (`excluded` off): the tint marks the cells of `sampling` the fill area does not cover.
/// - **Excluded Area** (`excluded` on): it marks the cells of `sampling`'s extent that are not sampled and not in
///   the fill area.
///
/// Nothing outside `sampling`'s extent is tinted.
pub struct FillOverlay<'a> {
    sampling: &'a SamplingPlane,
    fill_area: Option<&'a SamplingPlane>,
    excluded: bool,
    document: PixelSize,
}

impl<'a> FillOverlay<'a> {
    /// The tint of `sampling` less `fill_area`, over a document of `document` pixels.
    pub fn new(sampling: &'a SamplingPlane, fill_area: Option<&'a SamplingPlane>, document: PixelSize) -> Self {
        Self { sampling, fill_area, excluded: false, document }
    }

    /// Tints the excluded area instead of the sampling area.
    #[must_use]
    pub fn excluded(mut self, excluded: bool) -> Self {
        self.excluded = excluded;
        self
    }

    /// Whether the document point `(x, y)` is tinted.
    fn tinted(&self, x: f64, y: f64) -> bool {
        let Some(sampled) = self.sampling.cell_at(x, y) else { return false };
        let filled = self.fill_area.and_then(|plane| plane.cell_at(x, y)).unwrap_or(false);
        !filled && sampled != self.excluded
    }
}

impl RasterSource for FillOverlay<'_> {
    fn size(&self) -> PixelSize {
        self.document
    }

    fn revision(&self) -> Revision {
        Revision::of(&(self.sampling, self.fill_area, self.excluded, self.document.width, self.document.height))
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [columns, rows] = out.size;
        let factor = f64::from(factor.max(1));
        for row in 0..rows {
            let y = f64::from(region.y0) + (row as f64 + 0.5) * factor;
            for column in 0..columns {
                let x = f64::from(region.x0) + (column as f64 + 0.5) * factor;
                out.pixels[row * columns + column] = if self.tinted(x, y) { Color32::from_white_alpha(u8::MAX) } else { Color32::TRANSPARENT };
            }
        }
    }
}

/// The outline of the covered cells of `plane` in document pixels, for the ants: the cell edges, or the bounding box
/// of the covered cells when the plane has more edges than [`GRID_EDGE_LIMIT`].
pub fn fill_area_edges(plane: &SamplingPlane) -> Vec<(Point, Point)> {
    let texel = i64::from(plane.texel.max(1));
    let to_doc = |cell: [i32; 2]| {
        Point::new((i64::from(plane.origin[0]) + i64::from(cell[0]) * texel) as f64, (i64::from(plane.origin[1]) + i64::from(cell[1]) * texel) as f64)
    };
    let grid = CoverageGrid::new([0, 0], plane.size, plane.bits.clone());
    if let Some(edges) = edges_of_grid(&grid, GRID_EDGE_LIMIT) {
        return edges.into_iter().map(|(a, b)| (to_doc(a), to_doc(b))).collect();
    }
    let [width, height] = plane.size;
    let mut bounds: Option<[i32; 4]> = None;
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            if grid.inside(x, y) {
                let b = bounds.get_or_insert([x, y, x + 1, y + 1]);
                *b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
    }
    let [x0, y0, x1, y1] = bounds.unwrap_or([0; 4]);
    let (a, b) = (to_doc([x0, y0]), to_doc([x1, y1]));
    let corners = [a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)];
    (0..corners.len()).map(|i| (corners[i], corners[(i + 1) % corners.len()])).collect()
}
