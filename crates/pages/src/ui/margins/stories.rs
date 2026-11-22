//! Stories of the margin handles: the edges of a page on a page canvas.

use super::{MarginHandles, PageEdges};
use crate::setup::Margins;
use crate::ui::fixtures::fitted_page;
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, Validity};
use craft_ui::runtime::env::{Message, UnitKind};
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Ui, Vec2};

/// How far the drag moves the left edge, in drag thresholds.
const DRAG_THRESHOLDS: f32 = 12.0;

fn drag_left_edge(fixture: &mut Fixture) {
    let target = fixture.by_label("Left Margin").expect("the handles name their left edge Left Margin");
    let travel = DRAG_THRESHOLDS * craft_ui::testing::input::slop(fixture.ctx());
    fixture.drag(&target, Vec2::new(travel, 0.0));
}

/// The handles over `edges` on a page canvas.
fn handles(ui: &mut Ui, edges: Bind<'_, PageEdges>) -> StoryOutput {
    let full = craft_ui::testing::stage(ui);
    let mut view = CanvasView::new(full, Point::ZERO, 1.0);
    let page = fitted_page(&mut view, ui.ctx().pixels_per_point());
    let mut edges = edges;
    Canvas::new("Pages")
        .view(view)
        .show_at(ui, full, |ui, body| {
            Some(MarginHandles::new("Margins", &page, edges.reborrowed()).header_footer().unit(UnitKind::Inches).show_at(ui, body.rect))
        })
        .inner
        .flatten()
        .map(StoryOutput::from)
        .unwrap_or_default()
}

#[story(MarginHandles<'static>, covers = [Default], drive = Drive::Custom(drag_left_edge))]
fn margin_handles_on_rulers(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (edges, edges_at) = data.value_at("state", PageEdges::new(Margins::uniform(72.0)).header_footer(36.0, 36.0));
    handles(ui, edges_at.bind(edges))
}

#[story(MarginHandles<'static>, covers = [Invalid], drive = Drive::Never)]
fn margin_handles_header_below_the_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (edges, edges_at) = data.value_at("reaching", PageEdges::new(Margins::uniform(72.0)).header_footer(96.0, 36.0));
    let refuse_overlap = |edges: &PageEdges| {
        if edges.header > edges.margins.top { Validity::Refuse(Message::new("The header reaches the text")) } else { Validity::Ok }
    };
    handles(ui, edges_at.bind(edges).check(refuse_overlap))
}
