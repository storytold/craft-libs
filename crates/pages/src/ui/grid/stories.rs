//! Stories of the page grids.

use super::{PageGrid, PageGridColors, PageGrids};
use crate::ui::view::PlacedPage;
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect as PageRect};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui, Vec2, pos2};

/// How much of a Letter page the story's room shows.
const SCALE: f64 = 0.3;

#[story(PageGrids<'static>, covers = [Default], drive = Drive::Never)]
fn margins_and_columns_with_a_baseline_grid(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = craft_ui::testing::stage(ui);
    let page = PageRect::new(0.0, 0.0, 612.0, 792.0);
    let placed = [PlacedPage {
        page: 0,
        rect: Rect::from_min_size(pos2(room.min.x + 20.0, room.min.y + 10.0), Vec2::new(page.width() as f32, page.height() as f32) * SCALE as f32),
        scale: SCALE,
        band: (0.0, page.height() as f32 * SCALE as f32),
    }];
    let grids = [PageGrid {
        index: 0,
        page,
        margins: PageRect::new(36.0, 36.0, 576.0, 756.0),
        columns: vec![PageRect::new(36.0, 36.0, 300.0, 756.0), PageRect::new(312.0, 36.0, 576.0, 756.0)],
        baseline: Some((36.0, 12.0)),
    }];
    let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
    let shown = Canvas::new("Document")
        .view(view)
        .show_at(ui, room, |ui, body| PageGrids::new(&placed, &grids, 1.0, PageGridColors::default()).show_at(ui, body.rect));
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}
