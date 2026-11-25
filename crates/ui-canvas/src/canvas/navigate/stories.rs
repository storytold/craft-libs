//! Stories of the compass, the navigate bar and the canvas's latched middle scroll.

use crate::bars::{Bar, BarKind};
use crate::canvas::rotate::RIM;
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_with, show_document};
use crate::canvas::{Canvas, NavigateBar, NavigationOptions, RotateView, RotateViewOutput, ViewTarget};
use crate::contract::{Bind, Show as _, ShowContainer as _, Validity};
use crate::runtime::access::Role;
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, pointer, story};
use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, MiddleButton, Mixed};
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
    let (angle, angle_at) = data.value_at("angle", TURNED);
    in_corner(ui, *angle, |ui, at| RotateView::new("Rotation", angle_at.bind(&mut *angle), false).show_at(ui, at))
}

#[story(RotateView<'static>, covers = [Mixed], drive = Drive::Custom(drag_rim))]
fn views_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (angle, angle_at) = data.value_at("angle", Mixed::<f64>::Mixed);
    in_corner(ui, 0.0, |ui, at| RotateView::new("Rotation", angle_at.bind_mixed(&mut *angle), false).show_at(ui, at))
}

#[story(RotateView<'static>, covers = [Invalid], drive = Drive::Custom(drag_rim))]
fn too_far(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (angle, angle_at) = data.value_at("angle", MOST + TURNED);
    in_corner(ui, *angle, |ui, at| {
        let near =
            |angle: &f64| if angle.abs() > MOST { Validity::Refuse(Message::new("The view turns at most a quarter turn")) } else { Validity::Ok };
        RotateView::new("Rotation", angle_at.bind(&mut *angle).check(near), false).show_at(ui, at)
    })
}

/// Shows a navigation tool's options bar from `bar` over the view `view` and the story document's target above a canvas
/// that shows the same view of the same target, and returns the bar's output. The canvas fits the view on its first
/// pass and is the instance the bar's view commands reach.
fn bar_over_canvas(ui: &mut Ui, view: &mut CanvasView, mut bar: impl FnMut(&mut Ui, Bind<'_, CanvasView>, ViewTarget) -> StoryOutput) -> StoryOutput {
    let target = ViewTarget::new(StoryDocument::STANDARD.rect().size());
    let shown = Bar::new("Options", BarKind::Options).show(ui, |ui, _| bar(ui, Bind::bound(&mut *view), target));
    let room = crate::testing::stage(ui);
    on_canvas_with(ui, room, Canvas::new("Document").view(Bind::bound(view)).document(target), Some(&StoryDocument::STANDARD), |_, _| {
        StoryOutput::default()
    });
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
    bar_over_canvas(ui, view, |ui, bound, target| NavigateBar::zoom(bound, target, &mut *options, Bind::bound(&mut *out)).show(ui).into())
}

#[story(NavigateBar<'static>, covers = [], drive = Drive::Never)]
fn rotate_view_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let Navigation { view, .. } = data.value("navigation", Navigation::default());
    bar_over_canvas(ui, view, |ui, bound, _| NavigateBar::rotate_view(bound).show(ui).into())
}

#[story(NavigateBar<'static>, covers = [], drive = Drive::Never)]
fn hand_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let Navigation { view, options, .. } = data.value("navigation", Navigation::default());
    bar_over_canvas(ui, view, |ui, _, _| NavigateBar::hand(&mut *options).show(ui).into())
}

/// The latched scroll story's zoom, in percent: the document is taller than the canvas, so the latch has room to
/// scroll.
const LATCHED: f64 = 200.0;

/// Latches the middle button's scroll with a middle click in the canvas's middle, then rests the pointer below the
/// anchor, where the view scrolls. The testing kit has no middle-button helper, so the drive sends the raw events.
fn latch_middle(fixture: &mut Fixture) {
    let Some(canvas) = fixture.by_role(Role::CANVAS).first().map(|target| target.rect) else { return };
    let anchor = canvas.center();
    for pressed in [true, false] {
        let button = egui::Event::PointerButton { pos: anchor, button: egui::PointerButton::Middle, pressed, modifiers: egui::Modifiers::NONE };
        fixture.frame(vec![pointer::moved(anchor), button]);
    }
    fixture.frame(vec![pointer::moved(anchor + Vec2::new(0.0, canvas.height() / 4.0))]);
    fixture.idle(3);
}

/// A canvas whose middle button latches a scroll ([`MiddleButton::AutoScroll`]): the mark shows at the anchor while
/// the latch runs.
#[story(Canvas, covers = [], drive = Drive::Custom(latch_middle))]
fn latched_middle_scroll(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = crate::testing::stage(ui);
    let view = data.value("view", StoryDocument::STANDARD.view(room, ui.pixels_per_point(), LATCHED));
    let navigation = NavigationOptions { middle_button: MiddleButton::AutoScroll, ..NavigationOptions::default() };
    let target = ViewTarget::new(StoryDocument::STANDARD.rect().size());
    let canvas = Canvas::new("Document").view(Bind::bound(view)).document(target).navigation(navigation);
    canvas.show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD)).output.into()
}
