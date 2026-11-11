//! Stories of the split views.

use super::{DIVIDER, PANE, SplitAt, SplitLayout, SplitViews};
use crate::canvas::Canvas;
use crate::canvas::RasterSource as _;
use crate::canvas::testing::canvas_story::{StoryDocument, show_document};
use crate::contract::{Bind, ShowContainer as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, Mixed};
use egui::{Ui, Vec2};

/// The widest the left views of the invalid story may be, as a fraction of the area.
const MOST_LEFT: f32 = 0.8;

/// The names of the panes' canvases, by pane.
const NAMES: [&str; 4] = ["Main view", "View", "View", "View"];

/// Shows a view of the document, fitted to the pane, in each pane of `layout`, the first pane active.
fn split(ui: &mut Ui, layout: SplitLayout, at: Bind<'_, SplitAt>, shown: SplitAt) -> StoryOutput {
    let document = &StoryDocument::STANDARD;
    let size = document.size();
    let ppp = ui.pixels_per_point();
    let views = SplitViews::new("Views", layout, at).show(ui, |ui, body| {
        for (pane, rect) in layout.placed(shown, body.rect, body.env.direction().is_rtl()).into_iter().enumerate() {
            let mut view = CanvasView::new(rect, document.centre(), 1.0);
            zoom::fit(&mut view, Size::new(f64::from(size.width), f64::from(size.height)), ppp, 0.0);
            Canvas::new(NAMES[pane]).view(view).show_at(ui, rect, |ui, body| show_document(ui, body.rect, document));
        }
    });
    views.output.into()
}

/// Drags the divider between side by side views by twice the drag threshold.
fn drag_divider(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(grip) = fixture.part(id, DIVIDER.at(0)) else { return };
    let travel = 2.0 * crate::testing::input::max_click_dist(fixture.ctx());
    fixture.drag(&grip, Vec2::new(travel, 0.0));
}

/// Clicks the second pane, which is not active.
fn click_inactive_pane(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(pane) = fixture.part(id, PANE.at(1)) else { return };
    fixture.click(&pane);
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(drag_divider))]
fn two_views(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = data.value("split", SplitAt::default());
    let shown = *at;
    split(ui, SplitLayout::Two, Bind::bound(at), shown)
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(drag_divider))]
fn four_views(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = data.value("split", SplitAt::default());
    let shown = *at;
    split(ui, SplitLayout::Four, Bind::bound(at), shown)
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(click_inactive_pane))]
fn another_view_clicked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = data.value("split", SplitAt::default());
    let shown = *at;
    split(ui, SplitLayout::Two, Bind::bound(at), shown)
}

#[story(SplitViews<'static>, covers = [Mixed], drive = Drive::Custom(drag_divider))]
fn splits_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = data.value("split", Mixed::<SplitAt>::Mixed);
    split(ui, SplitLayout::Two, Bind::bound_mixed(at), SplitAt::default())
}

#[story(SplitViews<'static>, covers = [Invalid], drive = Drive::Custom(drag_divider))]
fn views_too_far_left(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = data.value("split", SplitAt::new(MOST_LEFT + 0.1, 0.5));
    let shown = *at;
    let too_wide = |at: &SplitAt| {
        if at.x > MOST_LEFT { Validity::Refuse(Message::new("The left view is too wide")) } else { Validity::Ok }
    };
    split(ui, SplitLayout::Two, Bind::bound(at).check(too_wide), shown)
}
