//! The selected cells of a document table, drawn over the table as a tint with an accent outline, as InDesign and Word
//! show selected cells.
//!
//! The selection and its rules are craft-behavior's one cell model, the one craft-table's sheet uses too. This module
//! keeps what is a document table's own: where its rows and columns are on screen ([`TableGeometry`]) and the tint
//! ([`CellSelectionOverlay`]). The app's table tool runs the input over the model:
//!
//! - the table as a `CellSpace`: `CellSpace::new(&Positions::new(rows), columns).merges(&merges)`, each merge a
//!   `CellRange<u32>` (a merged cell counts as one; ranges grow over the merges they cross,
//!   `CellRange::covering_merges`);
//! - a press in a cell `CellSelection::select`; a drag or Shift-click `CellSelection::extend_to`; Shift-arrows
//!   `CellSelection::extend`;
//! - Tab and Shift-Tab: `CellSpace::next_in_reading_order` from the active cell, wrapping to the next row; `None` past
//!   the last cell, where the app adds a row (InDesign, Word);
//! - Esc toggles between the cells and the first cell's text (the app's text tool).

use craft_behavior::{CellAt, CellRange, CellSelection, Metrics, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Role};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2, pos2};

/// Where the table's rows and columns are on screen: the edges, top to bottom and left to right (n + 1 values for n
/// rows).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TableGeometry {
    /// Screen y of each row boundary.
    pub row_edges: Vec<f32>,
    /// Screen x of each column boundary.
    pub column_edges: Vec<f32>,
}

impl TableGeometry {
    /// A table whose rows and columns lie between these edges.
    pub fn new(row_edges: Vec<f32>, column_edges: Vec<f32>) -> Self {
        TableGeometry { row_edges, column_edges }
    }

    /// The cell under a screen point, if any (a merged cell answers with the cell the point is in; the model's
    /// `CellSpace::merge_at` maps it to the merge).
    pub fn cell_at(&self, p: Pos2) -> Option<CellAt<u32>> {
        let row = band(&self.row_edges, p.y)?;
        let column = band(&self.column_edges, p.x)?;
        Some(CellAt::new(row, column))
    }

    /// The screen rectangle of a range: its first row's and column's leading edges to its last row's and column's
    /// trailing edges, past the table's last edge clamped to it.
    pub fn rect(&self, range: &CellRange<u32>) -> Rect {
        let edge = |edges: &[f32], i: u32| edges.get(i as usize).or(edges.last()).copied();
        let (top_row, bottom_row) = (range.first.min(range.last), range.first.max(range.last));
        match (
            edge(&self.column_edges, range.columns.start),
            edge(&self.row_edges, top_row),
            edge(&self.column_edges, range.columns.end),
            edge(&self.row_edges, bottom_row + 1),
        ) {
            (Some(left), Some(top), Some(right), Some(bottom)) => Rect::from_min_max(pos2(left, top), pos2(right, bottom)),
            _ => Rect::NOTHING,
        }
    }
}

/// The index of the band of `edges` (n + 1 ascending values for n bands) holding `v`: a band owns its leading edge and
/// not its trailing one.
fn band(edges: &[f32], v: f32) -> Option<u32> {
    let after = edges.partition_point(|&e| e <= v);
    let last = edges.len().checked_sub(1)?;
    (1..=last).contains(&after).then(|| (after - 1) as u32)
}

/// The style of the selected cells' overlay.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CellSelectionStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The tint of every selected range.
    fill: StyleColor,
    /// The outline of every selected range.
    outline: StyleColor,
}

/// The overlay over selected cells, painted after the table.
///
/// - Every range of the selection tints with the selection fill, the active cell included (InDesign tints it too,
///   unlike Excel), and each range is outlined in the accent colour. A caret or text selection inside one cell draws as
///   text (InDesign; DesignCraft).
/// - Paint only: the app's table tool runs clicks, drags and keys over the shared model (module docs). Not in the
///   accessibility tree: the table's cells are the document's text.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct CellSelectionOverlay<'a> {
    #[common]
    common: Common,
    geometry: &'a TableGeometry,
    selection: &'a CellSelection<u32>,
}

impl<'a> CellSelectionOverlay<'a> {
    /// An overlay of `selection` over the table at `geometry`.
    pub fn new(geometry: &'a TableGeometry, selection: &'a CellSelection<u32>) -> Self {
        CellSelectionOverlay { common: Common::new(), geometry, selection }
    }
}

impl Component for CellSelectionOverlay<'_> {
    const ROLE: Role = Role::None;
    const OVERLAYS: bool = false;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    type Style = CellSelectionStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: CellSelectionOverlayStyleKey) -> CellSelectionStyle {
        let colors = &cx.tokens.color;
        CellSelectionStyle { metrics: Metrics::new(), fill: cx.color(colors.state.selected), outline: cx.color(colors.accent.solid) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        for range in self.selection.ranges() {
            let shape = PaintShape::Rect(self.geometry.rect(range), CornerRadius::ZERO);
            draw.fill(&shape, cx.style.fill);
            draw.stroke(&shape, StrokeWidth::Thin, cx.style.outline);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
