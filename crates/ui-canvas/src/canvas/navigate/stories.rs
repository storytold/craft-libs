//! Stories of the compass and the navigate bar.

use crate::canvas::rotate::RIM;
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_with};
use crate::bars::{Bar, BarKind};
use crate::canvas::{Canvas, NavigateBar, NavigationOptions, RotateView, RotateViewOutput};
use crate::contract::{Bind, Show as _, ShowContainer as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, Mixed};
use egui::{Rect, Ui, Vec2};

/// The angle the stories start at, in degrees.
const TURNED: f64 = 30.0;

/// The largest angle the invalid story accepts, in degrees.
const MOST: f64 = 90.0;

/// Shows `compass` in the corner of a canvas over the story document turned by `rotation` degrees, and returns its
/// output.
fn in_corner(ui: &mut Ui, rotation: f64, mut compass: impl FnMut(&mut Ui, egui::Rect) -> RotateViewOutput) -> StoryOutput {
    let room = crate::testing::stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0).rotation(rotation);
    on_canvas_with(ui, room, Canvas::new("Document").view(view), Some(&StoryDocument::STANDARD), |ui, rect| {
        let at = RotateView::compass_rect(ui, rect);
        compass(ui, at)
    })
}

/// Drags the compass rim to the right by twice the drag threshold. The compass is a member of its canvas's arena, so
/// the automatic drive finds no drag part.
fn drag_rim(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(rim) = fixture.part(id, RIM) else { return };
    let travel = 2.0 * crate::testing::input::slop(fixture.ctx());
    fixture.drag(&rim, Vec2::new(travel, 0.0));
}

#[story(RotateView<'static>, covers = [Default], drive = Drive::Custom(drag_rim))]
fn compass(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let angle = data.value("angle", TURNED);
    in_corner(ui, *angle, |ui, at| RotateView::new("Rotation", Bind::bound(&mut *angle), false).show_at(ui, at))
}

#[story(RotateView<'static>, covers = [Mixed], drive = Drive::Custom(drag_rim))]
fn views_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let angle = data.value("angle", Mixed::<f64>::Mixed);
    in_corner(ui, 0.0, |ui, at| RotateView::new("Rotation", Bind::bound_mixed(&mut *angle), false).show_at(ui, at))
}

#[story(RotateView<'static>, covers = [Invalid], drive = Drive::Custom(drag_rim))]
fn too_far(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let angle = data.value("angle", MOST + TURNED);
    in_corner(ui, *angle, |ui, at| {
        let near =
            |angle: &f64| if angle.abs() > MOST { Validity::Refuse(Message::new("The view turns at most a quarter turn")) } else { Validity::Ok };
        RotateView::new("Rotation", Bind::bound(&mut *angle).check(near), false).show_at(ui, at)
    })
}

/// Shows a navigation tool's options bar from `bar` over the view `view` above a canvas on the story document that
/// shows the same view, and returns the bar's output. The canvas fits the view on its first pass and is the instance
/// the bar's view commands reach.
fn bar_over_canvas(ui: &mut Ui, view: &mut CanvasView, mut bar: impl FnMut(&mut Ui, Bind<'_, CanvasView>) -> StoryOutput) -> StoryOutput {
    let shown = Bar::new("Options", BarKind::Options).show(ui, |ui, _| bar(ui, Bind::bound(&mut *view)));
    let room = crate::testing::stage(ui);
    let document = StoryDocument::STANDARD.rect().size();
    on_canvas_with(ui, room, Canvas::new("Document").view(Bind::bound(view)).document(document), Some(&StoryDocument::STANDARD), |_, _| StoryOutput::default());
    shown.inner.unwrap_or_default()
}

/// What a navigate bar story keeps between passes.
#[derive(Clone, Copy, PartialEq)]
struct Navigation {
    view: CanvasView,
    options: NavigationOptions,
    out: bool,
}

impl Default for Navigation {
    /// A view no canvas has shown yet (the canvas fits it), the default options and Zoom In.
    fn default() -> Self {
        Navigation { view: CanvasView::new(Rect::ZERO, Point::ORIGIN, 1.0), options: NavigationOptions::default(), out: false }
    }
}

#[story(NavigateBar<'static>, covers = [], drive = Drive::Never)]
fn zoom_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let Navigation { view, options, out } = data.value("navigation", Navigation::default());
    let document: Size = StoryDocument::STANDARD.rect().size();
    bar_over_canvas(ui, view, |ui, bound| NavigateBar::zoom(bound, document, &mut *options, Bind::bound(&mut *out)).show(ui).into())
}

#[story(NavigateBar<'static>, covers = [], drive = Drive::Never)]
fn rotate_view_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let Navigation { view, .. } = data.value("navigation", Navigation::default());
    bar_over_canvas(ui, view, |ui, bound| NavigateBar::rotate_view(bound).show(ui).into())
}

#[story(NavigateBar<'static>, covers = [], drive = Drive::Never)]
fn hand_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let Navigation { view, options, .. } = data.value("navigation", Navigation::default());
    bar_over_canvas(ui, view, |ui, _| NavigateBar::hand(&mut *options).show(ui).into())
}
