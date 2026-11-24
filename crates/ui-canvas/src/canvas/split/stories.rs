//! Stories of the split views.

use super::{DIVIDER, PANE, PanePress, SplitAt, SplitLayout, SplitPaneAt, SplitViews};
use crate::canvas::Canvas;
use crate::canvas::RasterSource as _;
use crate::canvas::testing::canvas_story::{StoryDocument, show_document};
use crate::contract::{Bind, ShowContainer as _, Validity};
use crate::runtime::env::Message;
use crate::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, Mixed};
use egui::{Pos2, Ui, Vec2};

/// The widest the left views of the invalid story may be, as a fraction of the area.
const MOST_LEFT: f32 = 0.8;

/// The pane of two side by side views that shows the main view: the right one, where the views have always kept it.
const MAIN_OF_TWO: usize = 1;

/// The pane of four views that shows the main view: the bottom right one.
const MAIN_OF_FOUR: usize = 3;

/// Shows a view of the document, fitted to the pane, in the pane `pane`. The active pane's canvas is named "Main view".
fn show_pane(ui: &mut Ui, pane: SplitPaneAt) {
    let document = &StoryDocument::STANDARD;
    let size = document.size();
    let mut view = CanvasView::new(pane.rect, document.centre(), 1.0);
    zoom::fit(
        &mut view,
        Size::new(f64::from(size.width), f64::from(size.height)),
        ui.pixels_per_point(),
        0.0,
        &craft_behavior::ZoomScheme::PHOTOSHOP,
    );
    let name = if pane.active { "Main view" } else { "View" };
    Canvas::new(name).view(view).show_at(ui, pane.rect, |ui, body| show_document(ui, body.rect, document));
}

/// Shows a view of the document, fitted to the pane, in each pane of `layout` with `count` documents, each pane in its
/// own body. Pane `main` is the active one and its canvas is named "Main view".
fn split(ui: &mut Ui, layout: SplitLayout, count: usize, main: usize, at: Bind<'_, SplitAt>) -> StoryOutput {
    let views = SplitViews::new("Views", layout, at).count(count).active(main).show_panes(ui, |ui, _, pane| show_pane(ui, pane));
    views.output.into()
}

/// Drags divider `index` by twice the drag threshold along `travel`.
fn drag_divider_at(fixture: &mut Fixture, index: u64, travel: Vec2) {
    let Some(id) = fixture.root_id() else { return };
    let Some(grip) = fixture.part(id, DIVIDER.at(index)) else { return };
    let slop = 2.0 * crate::testing::input::slop(fixture.ctx());
    fixture.drag(&grip, travel * slop);
}

/// Drags the first column divider (between side by side views) to the right.
fn drag_divider(fixture: &mut Fixture) {
    drag_divider_at(fixture, 0, Vec2::new(1.0, 0.0));
}

/// Drags the first row divider (between stacked views) down: the first divider whose line runs along x. The column
/// lines come first, so in 3-up Stacked the row line is divider 1.
fn drag_row_divider(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let row =
        (0..).map_while(|index| fixture.part(id, DIVIDER.at(index)).map(|part| (index, part.rect))).find(|(_, rect)| rect.width() > rect.height());
    if let Some((index, _)) = row {
        drag_divider_at(fixture, index, Vec2::new(0.0, 1.0));
    }
}

/// Drags the first document tab onto the right half of the unsplit views: the reading-end half, where a drop splits
/// them, in left-to-right.
fn drag_tab_onto_views(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(views) = fixture.part(id, PANE.at(0)) else { return };
    let Some(tab) = fixture.by_label("Untitled-1") else { return };
    let end = Pos2::new(views.rect.left() + views.rect.width() * 0.75, views.rect.center().y);
    fixture.drag_at(tab.center(), end - tab.center());
}

/// Clicks the first pane, which is not active: the main view of two views is pane 1.
fn click_inactive_pane(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(pane) = fixture.part(id, PANE.at(0)) else { return };
    fixture.click(&pane);
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(drag_divider))]
fn two_views(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::default());
    split(ui, SplitLayout::TwoUpVertical, 2, MAIN_OF_TWO, at_at.bind(at))
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(drag_divider))]
fn four_views(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::default());
    split(ui, SplitLayout::FourUp, 4, MAIN_OF_FOUR, at_at.bind(at))
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(drag_divider))]
fn grid_views(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::default());
    split(ui, SplitLayout::Tile, 5, 0, at_at.bind(at))
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(drag_row_divider))]
fn stacked_views(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::default());
    split(ui, SplitLayout::ThreeUpStacked, 3, 0, at_at.bind(at))
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(click_inactive_pane))]
fn another_view_clicked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::default());
    split(ui, SplitLayout::TwoUpVertical, 2, MAIN_OF_TWO, at_at.bind(at))
}

#[story(SplitViews<'static>, covers = [Default], drive = Drive::Custom(click_inactive_pane))]
fn interactive_panes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::default());
    SplitViews::new("Views", SplitLayout::TwoUpVertical, at_at.bind(at))
        .active(MAIN_OF_TWO)
        .pane_press(PanePress::Interact)
        .show_panes(ui, |ui, _, pane| show_pane(ui, pane))
        .output
        .into()
}

#[story(SplitViews<'static>, covers = [Mixed], drive = Drive::Custom(drag_divider))]
fn splits_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (at, at_at) = data.value_at("split", Mixed::<SplitAt>::Mixed);
    split(ui, SplitLayout::TwoUpVertical, 2, MAIN_OF_TWO, at_at.bind_mixed(at))
}

/// The documents the tab story opens with.
const DOCUMENTS: [(usize, &str); 2] = [(0, "Untitled-1"), (1, "Photo.psd")];

#[story(SplitViews<'static, ()>, covers = [Default], drive = Drive::Custom(drag_tab_onto_views))]
fn drop_a_tab_to_split(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    // The story hosts the strip as an app does: a selected or picked tab shows, a closed one goes.
    let open = data.value("open", DOCUMENTS.map(|(id, _)| id).to_vec()).clone();
    let tabs: Vec<DocumentTab<usize>> = DOCUMENTS.iter().filter(|(id, _)| open.contains(id)).map(|&(id, name)| DocumentTab::new(id, name)).collect();
    let selected = *data.value("selected", 0_usize);
    let strip = DocumentTabs::new("Documents", &tabs).selected(selected).reorderable(true).tear_off(true).show(ui);
    for event in strip.events {
        match event {
            DocumentTabsEvent::Selected(id) | DocumentTabsEvent::Picked(id) => *data.value("selected", 0_usize) = id,
            DocumentTabsEvent::CloseRequested(id) => data.value("open", Vec::<usize>::new()).retain(|open| *open != id),
            _ => {}
        }
    }
    let (at, at_at) = data.value_at("split", SplitAt::default());
    let views = SplitViews::new("Views", SplitLayout::Tile, at_at.bind(at))
        .count(1)
        .takes_tabs(|_: &[usize], _| true)
        .show_panes(ui, |ui, _, pane| show_pane(ui, pane));
    views.output.into()
}

#[story(SplitViews<'static>, covers = [Invalid], drive = Drive::Custom(drag_divider))]
fn views_too_far_left(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::new(MOST_LEFT + 0.1, 0.5));
    let too_wide = |at: &SplitAt| {
        if at.columns.first().is_some_and(|&x| x > MOST_LEFT) { Validity::Refuse(Message::new("The left view is too wide")) } else { Validity::Ok }
    };
    split(ui, SplitLayout::TwoUpVertical, 2, MAIN_OF_TWO, at_at.bind(at).check(too_wide))
}
