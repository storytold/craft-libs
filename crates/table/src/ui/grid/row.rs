//! One row of the grid: its cells as parts, the gridlines, the selection tint and the app's cell paint.

use craft_behavior::{ColumnKey, Metrics, PartKey, Sense};
use craft_ui::contract::{Access, Component, Cx, MeasureCx, Name, PaintUi, PartsCx, Role, Styled as _};
use craft_ui::primitives::{PaintShape, RuleKind};
use craft_ui::style::{ComponentStyle, Flags, Stateful, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

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

/// One row of the sheet: a cell part per shown column. The app paints each cell through [`GridModel::paint_cell`]; the
/// row draws the gridlines (none inside a merged cell) and the selection tint.
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

    /// The merged cell holding `cell`, if any.
    fn merge_of(&self, cell: CellAt<u32>) -> Option<(u32, u32, u32, u32)> {
        self.model
            .merges()
            .iter()
            .map(edges)
            .find(|&(top, bottom, left, right)| (top..=bottom).contains(&cell.row) && (left..=right).contains(&cell.column))
    }

    /// The rect of the merge anchored at `cell` whose first cell has `rect`, extended along the reading direction.
    fn merged_rect(&self, rect: Rect, (top, bottom, left, right): (u32, u32, u32, u32), rtl: bool) -> Rect {
        let (rows, columns) = (self.model.rows(), self.model.columns());
        let height = ((rows.start(bottom + 1) - rows.start(top)) * f64::from(self.zoom)) as f32;
        let width = ((columns.start(right + 1) - columns.start(left)) * f64::from(self.zoom)) as f32;
        let min_x = if rtl { rect.right() - width } else { rect.left() };
        Rect::from_min_size(Pos2::new(min_x, rect.top()), Vec2::new(width, height))
    }
}

impl Component for GridRow<'_> {
    const ROLE: Role = Role::ROW;
    // The grid is the one stop: its keys move the active cell, so a row and its cells take no focus.
    const STOPS: bool = false;
    type Style = GridRowStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: GridRowStyleKey) -> GridRowStyle {
        let clear = cx.color(craft_theme::Rgba::TRANSPARENT);
        GridRowStyle {
            metrics: Metrics::new(),
            background: cx.color(cx.tokens.color.surface.background),
            tint: Stateful::new(clear).when(Flags::LEAD, clear).when(Flags::SELECTED, cx.color(cx.tokens.color.state.text_selection)),
        }
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
        let rtl = cx.env.direction().is_rtl();
        let gridlines = self.model.show_gridlines();
        let columns = self.cells.clone();
        for (key, rect) in &columns {
            let Some(column) = key.position() else { continue };
            let cell = CellAt::new(self.row, column);
            let merge = self.merge_of(cell);
            let anchored = merge.is_none_or(|(top, _, left, _)| top == self.row && left == column);
            if !anchored {
                continue;
            }
            let shape = merge.map_or(*rect, |merge| self.merged_rect(*rect, merge, rtl));
            {
                let draw = ui.draw();
                draw.fill(&PaintShape::Rect(shape, CornerRadius::ZERO), cx.style.background);
                draw.fill(&PaintShape::Rect(shape, CornerRadius::ZERO), cx.resolve_part(PartKey::CELL.at(u64::from(column)), &cx.style.tint));
                if gridlines {
                    let (top, bottom) = if rtl { (shape.left_top(), shape.left_bottom()) } else { (shape.right_top(), shape.right_bottom()) };
                    draw.rule(RuleKind::Grid, shape.left_bottom(), shape.right_bottom());
                    draw.rule(RuleKind::Grid, top, bottom);
                }
            }
            self.model.paint_cell(ui, shape, cell, self.zoom);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
