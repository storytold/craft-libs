//! One row of the grid: its cells as parts, the gridlines, the selection tint and the app's cell paint.

use craft_behavior::{ColumnKey, Metrics, PartKey, Sense};
use craft_ui::contract::{Access, Component, Cx, MeasureCx, Name, PaintUi, PartsCx, Role};
use craft_ui::primitives::{PaintShape, RectSide, RuleKind};
use craft_ui::style::{ComponentStyle, Flags, StateSet, Stateful, StyleColor, StyleCx};
use egui::{CornerRadius, Rect, Response, Vec2};

use super::{CellAt, GridModel, edges};

/// The style of a grid row: the cell background and the tint of the selected cells, clear on the lead cell.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GridRowStyle {
    #[style(metrics)]
    metrics: Metrics,
    background: StyleColor,
    tint: Stateful<StyleColor>,
}

/// The tint of a cell by its flags: the selection's tint on a selected cell, clear on the lead cell. A row's cells and
/// the grid's merged cells resolve this one look.
pub(super) fn cell_tint(cx: &StyleCx<'_>) -> Stateful<StyleColor> {
    let clear = cx.color(craft_theme::Rgba::TRANSPARENT);
    Stateful::new(clear).when(Flags::LEAD, clear).when(Flags::SELECTED, cx.color(cx.tokens.color.state.text_selection))
}

/// How a cell paints, the same for a row's cells and the grid's merged cells.
pub(super) struct CellPaint<'p> {
    pub(super) model: &'p dyn GridModel,
    pub(super) background: StyleColor,
    pub(super) tint: &'p Stateful<StyleColor>,
    pub(super) zoom: f32,
    pub(super) rtl: bool,
}

impl CellPaint<'_> {
    /// Paints `cell` over `rect` in `state`: the sheet's background, the tint of `state`, the gridlines along its
    /// bottom and end edges when the model shows them, then the app's paint of the cell.
    pub(super) fn paint(&self, ui: &mut PaintUi<'_>, cell: CellAt<u32>, rect: Rect, state: &StateSet) {
        {
            let draw = ui.draw();
            draw.fill(&PaintShape::Rect(rect, CornerRadius::ZERO), self.background);
            draw.fill(&PaintShape::Rect(rect, CornerRadius::ZERO), state.resolve(self.tint));
            if self.model.show_gridlines() {
                draw.edge_rule(RuleKind::Divider, rect, RectSide::Bottom);
                draw.edge_rule(RuleKind::Divider, rect, RectSide::end(self.rtl));
            }
        }
        self.model.paint_cell(ui, rect, cell, self.zoom);
    }
}

/// One row of the sheet: a cell part per shown column. The app paints each cell through [`GridModel::paint_cell`]; the
/// row draws the gridlines and the selection tint. A merged cell spans rows, so the grid paints it, not its rows.
#[derive(craft_ui::contract::Component)]
pub struct GridRow<'r> {
    #[common]
    common: craft_ui::contract::Common,
    model: &'r dyn GridModel,
    row: u32,
    zoom: f32,
    cells: Vec<(ColumnKey, Rect)>,
}

impl<'r> GridRow<'r> {
    /// Row `row` of `model` at `zoom`, with the rects the item step lists for its cells.
    pub fn new(model: &'r dyn GridModel, row: u32, zoom: f32, cells: Vec<(ColumnKey, Rect)>) -> Self {
        GridRow { common: craft_ui::contract::Common::new(), model, row, zoom, cells }
    }

    /// Whether `cell` lies in a merged cell.
    fn merged(&self, cell: CellAt<u32>) -> bool {
        self.model
            .merges()
            .iter()
            .map(edges)
            .any(|(top, bottom, left, right)| (top..=bottom).contains(&cell.row) && (left..=right).contains(&cell.column))
    }
}

impl Component for GridRow<'_> {
    const ROLE: Role = Role::ROW;
    // The grid is the one stop: its keys move the active cell, so a row and its cells take no focus.
    const STOPS: bool = false;
    type Style = GridRowStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: GridRowStyleKey) -> GridRowStyle {
        GridRowStyle { metrics: Metrics::new(), background: cx.color(cx.tokens.color.surface.background), tint: cell_tint(cx) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// The row as the item step laid its cells out.
    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        self.cells.iter().fold(Rect::NOTHING, |all, (_, rect)| all.union(*rect)).size().max(Vec2::ZERO)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        for (key, rect) in &self.cells {
            if let Some(column) = key.position() {
                cx.part(PartKey::CELL.at(u64::from(column)), *rect);
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        for (key, _) in &self.cells {
            let Some(column) = key.position() else { continue };
            let node = access.node(PartKey::CELL.at(u64::from(column)), Role::GRID_CELL);
            node.name = Name::Text(self.model.cell_text(CellAt::new(self.row, column)));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let cells =
            CellPaint { model: self.model, background: cx.style.background, tint: &cx.style.tint, zoom: self.zoom, rtl: cx.env.direction().is_rtl() };
        for (key, rect) in &self.cells {
            let Some(column) = key.position() else { continue };
            let cell = CellAt::new(self.row, column);
            if !self.merged(cell) {
                cells.paint(ui, cell, *rect, &cx.part_state(PartKey::CELL.at(u64::from(column))));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
