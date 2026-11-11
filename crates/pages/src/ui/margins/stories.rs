//! Stories of the margin handles: the edges of a page on a page canvas.

use super::{MarginHandles, PageEdges};
use crate::setup::Margins;
use crate::ui::fixtures::demo_placed;
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, Validity};
use craft_ui::runtime::env::{Message, UnitKind};
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Ui, Vec2};

/// The document point the view is centred on.
const CENTRE: Point = Point::new(300.0, 400.0);

/// How far the drag moves the left edge, in drag thresholds.
const DRAG_THRESHOLDS: f32 = 12.0;

fn drag_left_edge(fixture: &mut Fixture) {
    let Some(target) = fixture.by_label("Left Margin") else { return };
    let travel = DRAG_THRESHOLDS * craft_ui::testing::input::max_click_dist(fixture.ctx());
    fixture.drag(&target, Vec2::new(travel, 0.0));
}

/// The handles over `edges` on a page canvas.
fn handles(ui: &mut Ui, edges: Bind<'_, PageEdges>) -> StoryOutput {
    let full = craft_ui::testing::stage(ui);
    let view = CanvasView::new(full, CENTRE, 1.0);
    let mut edges = edges;
    Canvas::new("Pages")
        .view(view)
        .show_at(ui, full, |ui, body| {
            let page = demo_placed(1, CanvasView::new(body.rect, CENTRE, 1.0).to_screen(Point::ZERO)).remove(0);
            Some(MarginHandles::new("Margins", &page, edges.reborrowed()).header_footer().unit(UnitKind::Inches).show_at(ui, body.rect))
        })
        .inner
        .flatten()
        .map(StoryOutput::from)
        .unwrap_or_default()
}

#[story(MarginHandles<'static>, covers = [Default], drive = Drive::Custom(drag_left_edge))]
fn margin_handles_on_rulers(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let edges = data.value("state", PageEdges::new(Margins::uniform(72.0)).header_footer(36.0, 36.0));
    handles(ui, Bind::bound(edges))
}

#[story(MarginHandles<'static>, covers = [Invalid], drive = Drive::Never)]
fn margin_handles_header_below_the_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let edges = data.value("reaching", PageEdges::new(Margins::uniform(72.0)).header_footer(96.0, 36.0));
    let refuse_overlap = |edges: &PageEdges| {
        if edges.header > edges.margins.top { Validity::Refuse(Message::new("The header reaches the text")) } else { Validity::Ok }
    };
    handles(ui, Bind::bound(edges).check(refuse_overlap))
}
