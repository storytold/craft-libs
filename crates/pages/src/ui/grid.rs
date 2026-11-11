//! `PageGrids`: InDesign's guides on a page, a paint-only canvas control. The margins (magenta), the column
//! boundaries (violet) and the baseline grid (light blue, from its zoom threshold) are hairlines drawn over the
//! pages, under the selection.

#[cfg(feature = "screen-testing")]
mod stories;

use super::view::PlacedPage;
use craft_behavior::{Input, Metrics, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Role};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::style::{ComponentStyle, DataColor, StyleCx};
use egui::{Color32, Pos2, Response, Vec2};
use kurbo::{Point, Rect};

/// The default colour of the margin rectangle.
pub const DEFAULT_MARGINS: DataColor = DataColor::new(Color32::from_rgb(0xFF, 0x38, 0xFF));
/// The default colour of the column rectangles.
pub const DEFAULT_COLUMNS: DataColor = DataColor::new(Color32::from_rgb(0xA6, 0x28, 0xFF));
/// The default colour of the baseline grid: light blue at 80 % opacity (DesignCraft draws it with
/// `gamma_multiply(0.8)`).
pub const DEFAULT_BASELINE: DataColor = DataColor::new(Color32::from_rgba_premultiplied(0x70, 0xA4, 0xB8, 0xCC));

/// The most baseline grid lines drawn on a page: one per point of the tallest page (216 in),
/// DesignCraft's `MAX_BASELINES`.
pub const MAX_BASELINES: usize = 216 * 72;

/// The zoom from which the baseline grid shows by default: InDesign's View Threshold, 75 %.
const BASELINE_THRESHOLD: f64 = 0.75;

/// Baseline grid lines inside a page `height` tall, measured from its top: `start + n x increment` for n >= 0
/// below the top (the bottom edge is not a line); lines above the page are skipped without walking them,
/// however far up `start` is; an increment under 1 pt counts as 1.
// designcraft: crates/ui-egui/src/canvas/page_decor.rs:19 `baseline_grid_lines`.
pub fn baseline_lines(start: f64, increment: f64, height: f64) -> Vec<f64> {
    let step = increment.max(1.0);
    let first = if start < 0.0 { start.rem_euclid(step) } else { start };
    (0..MAX_BASELINES).map(|n| first + n as f64 * step).take_while(|y| *y < height).collect()
}

/// One page's grids, in page points.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageGrid {
    /// The index of the page the grids belong to (`PlacedPage::page`); a page that is not on screen draws nothing.
    pub index: usize,
    /// The page rectangle.
    pub page: Rect,
    /// The margin rectangle.
    pub margins: Rect,
    /// The column rectangles inside the margins; one column draws no column lines.
    pub columns: Vec<Rect>,
    /// The baseline grid: start (from the page top) and increment, if shown.
    pub baseline: Option<(f64, f64)>,
}

/// The colours, InDesign's defaults (Preferences, Guides and Pasteboard; Document Setup's Margins and Columns
/// colours). They are document data.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageGridColors {
    /// Colour of the margin rectangle.
    pub margins: DataColor,
    /// Colour of the column rectangles.
    pub columns: DataColor,
    /// Colour of the baseline grid, drawn as given.
    pub baseline: DataColor,
}

impl Default for PageGridColors {
    fn default() -> Self {
        PageGridColors { margins: DEFAULT_MARGINS, columns: DEFAULT_COLUMNS, baseline: DEFAULT_BASELINE }
    }
}

/// The style of the page grids: they paint hairlines in the colours the document gives.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PageGridsStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The grids of the pages on screen, painted only. Margins are a hairline rectangle; columns are hairlines at each
/// inner column edge; baseline lines run across the page when the page view's zoom (1.0 = 100 %) is at least the
/// [`baseline_threshold`](Self::baseline_threshold). Not in the accessibility tree.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct PageGrids<'a> {
    #[common]
    common: Common,
    placed: &'a [PlacedPage],
    grids: &'a [PageGrid],
    colors: PageGridColors,
    zoom: f64,
    /// The zoom below which the baseline grid is not drawn.
    #[prop(quantity = "zoom factor")]
    baseline_threshold: f64,
}

impl<'a> PageGrids<'a> {
    /// The `grids` of the pages `placed` on screen, at the page view's `zoom`, in `colors`.
    pub fn new(placed: &'a [PlacedPage], grids: &'a [PageGrid], zoom: f64, colors: PageGridColors) -> Self {
        PageGrids { common: Common::new(), placed, grids, colors, zoom, baseline_threshold: BASELINE_THRESHOLD }
    }
}

impl Component for PageGrids<'_> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = PageGridsStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PageGridsStyleKey) -> PageGridsStyle {
        PageGridsStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        let draw = ui.draw();
        let line = |a: Pos2, b: Pos2, color: DataColor| draw.stroke(&PaintShape::Path(vec![a, b]), StrokeWidth::Hairline, color);
        for grid in self.grids {
            let Some(placed) = self.placed.iter().find(|placed| placed.page == grid.index) else { continue };
            let at = |x: f64, y: f64| placed.to_screen(Point::new(x, y));
            if let Some((start, increment)) = grid.baseline
                && self.zoom >= self.baseline_threshold
            {
                for y in baseline_lines(start, increment, grid.page.height()) {
                    let y = grid.page.y0 + y;
                    line(at(grid.page.x0, y), at(grid.page.x1, y), self.colors.baseline);
                }
            }
            let margins = grid.margins;
            let corners = vec![at(margins.x0, margins.y0), at(margins.x1, margins.y0), at(margins.x1, margins.y1), at(margins.x0, margins.y1)];
            draw.stroke(&PaintShape::Polygon(corners), StrokeWidth::Hairline, self.colors.margins);
            if grid.columns.len() > 1 {
                let last = grid.columns.len() - 1;
                for (i, column) in grid.columns.iter().enumerate() {
                    if i > 0 {
                        line(at(column.x0, column.y0), at(column.x0, column.y1), self.colors.columns);
                    }
                    if i < last {
                        line(at(column.x1, column.y0), at(column.x1, column.y1), self.colors.columns);
                    }
                }
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
