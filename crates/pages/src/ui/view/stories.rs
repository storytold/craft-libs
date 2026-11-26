//! Stories of the page view and its leaves.

use super::pictures::{PageMetrics, PagePictures};
use super::{PageSnap, PageSource as _, PageView, PageViewState};
use crate::layout::{LayoutOptions, PageDisplay};
use crate::ui::fixtures::{DemoPages, PAPER, demo_placed};
use craft_behavior::geometry::{SnapKind, SnapOptions, SnapTarget};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{Axis, CancelReason, CanvasView};
use craft_ui::canvas::{Canvas, CanvasTool, ToolAction, ToolContext, ToolHost, ToolPath, ToolPointer};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::primitives::Draw;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Pos2, Ui};

/// Pages in the story document: enough to scroll.
const PAGES: usize = 6;

fn view(ui: &mut Ui, data: &mut StoryData, rulers: bool, change: impl FnOnce(&mut PageViewState)) -> StoryOutput {
    view_of(ui, data, &DemoPages::new(PAGES), rulers, change)
}

fn view_of(ui: &mut Ui, data: &mut StoryData, pages: &DemoPages, rulers: bool, change: impl FnOnce(&mut PageViewState)) -> StoryOutput {
    let state = data.value("view", {
        let mut state = PageViewState::default();
        change(&mut state);
        state
    });
    let out = PageView::new("Pages", pages, state, PAPER).options(LayoutOptions::ACROBAT).rulers(rulers).show(ui, |_, _| ());
    out.canvas.response.into()
}

/// The pages while their pictures render: each page is captioned in place of its picture.
#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn captioned_loading(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view_of(ui, data, &DemoPages::rendering(PAGES), false, |state| state.continuous = false)
}

/// The pages when their pictures fail to render: each page is captioned with the failure.
#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn captioned_failed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view_of(ui, data, &DemoPages::failing(PAGES), false, |state| state.continuous = false)
}

#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn continuous(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view(ui, data, false, |_| ())
}

#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn with_rulers(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view(ui, data, true, |_| ())
}

#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn spreads(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view(ui, data, false, |state| state.display = PageDisplay::TwoUp)
}

/// A long document whose view keeps only the pictures of the pages drawn and the row either side
/// (`picture_reach(0.0)`): a page scrolled away is rendered again when it comes back.
#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn pictures_near_the_view_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("view", PageViewState::default());
    let pages = DemoPages::new(8 * PAGES);
    let out = PageView::new("Pages", &pages, state, PAPER).options(LayoutOptions::ACROBAT).picture_reach(0.0).show(ui, |_, _| ());
    out.canvas.response.into()
}

#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn one_page_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    view(ui, data, false, |state| state.continuous = false)
}

/// A page margin's inset from each edge, in page points.
const MARGIN: f64 = 72.0;

/// A tool whose drag snaps the point under the pointer as a point-sized box, so the host draws the smart guide it
/// snapped to.
struct MarginSnapper;

impl CanvasTool for MarginSnapper {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(space) = cx.space() {
            let at = space.view.to_doc(pointer.pos);
            let _ = cx.snap_rect(DocRect::from_points(at, at));
        }
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Snap to Margins command")
    }
}

/// Two-up spreads whose page margins are snap targets handed over in page points ([`PageSnap`]): a drag snaps to the
/// margins of the page it is over, where that page shows, and the smart guide shows where it snapped.
#[story(PageView<'static>, covers = [], drive = Drive::Never)]
fn snapped_margins(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("view", PageViewState { display: PageDisplay::TwoUp, ..PageViewState::default() });
    let pages = DemoPages::new(PAGES);
    let snap = (0..PAGES).fold(PageSnap::new(SnapOptions::default()), |snap, page| {
        let size = pages.page_size(page);
        let margins = vec![
            SnapTarget::new(Axis::Y, MARGIN, SnapKind::LayerEdge),
            SnapTarget::new(Axis::Y, size.width - MARGIN, SnapKind::LayerEdge),
            SnapTarget::new(Axis::X, MARGIN, SnapKind::LayerEdge),
            SnapTarget::new(Axis::X, size.height - MARGIN, SnapKind::LayerEdge),
        ];
        snap.page(page, margins)
    });
    let out = PageView::new("Pages", &pages, state, PAPER).options(LayoutOptions::ACROBAT).snap(Some(snap)).show_with(
        ui,
        &mut MarginSnapper,
        |ui, body, tool| body.pass.and_then(|pass| ToolHost::new().run(ui, pass, tool).view),
        |_, _, _| (),
    );
    out.canvas.response.into()
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
    let owner = ui.scope_id().with("page_pictures");
    let shown =
        Canvas::new("Document").view(view).show_at(ui, room, |ui, body| PagePictures::new(&pages, &placed, owner, PAPER).show_at(ui, body.rect));
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}
