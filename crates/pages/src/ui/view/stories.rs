//! Stories of the page view and its leaves.

use super::pictures::{PageMetrics, PagePictures};
use super::{PageView, PageViewState};
use crate::layout::{LayoutOptions, PageDisplay};
use crate::ui::fixtures::{DemoPages, PAPER, demo_placed};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Pos2, Ui, Vec2};

/// Pages in the story document: enough to scroll.
const PAGES: usize = 6;

fn view(ui: &mut Ui, data: &mut StoryData, change: impl FnOnce(&mut PageViewState)) -> StoryOutput {
    let state = data.value("view", {
        let mut state = PageViewState::default();
        change(&mut state);
        state
    });
    let pages = DemoPages::new(PAGES);
    let out = PageView::new("Pages", &pages, state, PAPER).options(LayoutOptions::ACROBAT).show(ui, |_, _| ());
    out.canvas.response.into()
}

#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn continuous(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view(ui, data, |_| ())
}

#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn spreads(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view(ui, data, |state| state.display = PageDisplay::TwoUp)
}

#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn one_page_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view(ui, data, |state| state.continuous = false)
}

#[story(PageMetrics, covers = [Default], drive = Drive::Never)]
fn page_metrics(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = craft_ui::testing::stage(ui);
    let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| PageMetrics::new().show_at(ui, body.rect));
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}

#[story(PagePictures<'static>, covers = [Default], drive = Drive::Never)]
fn page_pictures(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = craft_ui::testing::stage(ui);
    let pages = DemoPages::new(1);
    let placed = demo_placed(1, Pos2::new(room.min.x + 20.0, room.min.y + 10.0));
    let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
    let near: [(usize, Vec2); 0] = [];
    let shown =
        Canvas::new("Document").view(view).show_at(ui, room, |ui, body| PagePictures::new(&pages, &placed, &near, PAPER).show_at(ui, body.rect));
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}
