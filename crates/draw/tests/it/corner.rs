//! The corner widgets' drag reports its edit on the pass that commits it.

use super::presses::Presses;
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_draw::ui::corner::{CornerPoint, CornerTarget};
use craft_draw::ui::transform::TransformReach;
use craft_draw::ui::{CornerEdit, CornerKind, CornerWidgets};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{EditKind, Fixture, ToolFixture, trial};
use egui::vec2;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const START_RADIUS: f64 = 4.0;

struct Host {
    targets: Vec<CornerTarget>,
    /// Per pass: whether it carried a `Commit`, and the `change` it reported.
    passes: Vec<(bool, Option<CornerEdit>)>,
}

fn square() -> CornerTarget {
    let vertices = [Point::new(130.0, 150.0), Point::new(290.0, 150.0), Point::new(290.0, 250.0), Point::new(130.0, 250.0)];
    let corners =
        (0..4).map(|i| CornerPoint::new(vertices[i], vertices[(i + 3) % 4], vertices[(i + 1) % 4], START_RADIUS, CornerKind::Round)).collect();
    CornerTarget::new(1, corners, true)
}

#[trial]
fn a_drag_reports_its_change_on_the_commit_pass() {
    let host = Rc::new(RefCell::new(Host { targets: vec![square()], passes: Vec::new() }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let st = &mut *held;
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(210.0, 200.0), 1.0);
        let canvas = Canvas::new("Document")
            .view(view)
            .show(ui, |ui, body| CornerWidgets::new("Corner radius", Bind::bound(&mut st.targets)).show_at(ui, body.rect));
        let Some(out) = canvas.inner else { return };
        let committed = out.edit.events.iter().any(|event| matches!(EditKind::of(event), Some(EditKind::Commit)));
        st.passes.push((committed, out.change));
    });
    fixture.settle();
    let widget = fixture.by_label("Corner 1 of 4").expect("the first corner has a widget");
    // Toward the inside of the corner: the radius grows.
    fixture.drag(&widget, vec2(30.0, 30.0));

    let held = host.borrow();
    let commits: Vec<&Option<CornerEdit>> = held.passes.iter().filter(|(committed, _)| *committed).map(|(_, change)| change).collect();
    assert!(!commits.is_empty(), "the drag committed");
    for change in &commits {
        assert!(change.is_some(), "the commit pass reports the edit");
    }
    let change = commits.last().and_then(|change| change.as_ref()).expect("a commit with a change");
    let reported = change.targets.first().and_then(|target| target.radii.first()).copied().expect("the change names the first corner");
    let taken = held.targets[0].corners[0].radius;
    assert!(taken > START_RADIUS, "the drag rounded the corner");
    assert_eq!(reported, taken, "the reported change is the last move the targets took");
}

/// A triangle, apex first, its corners at `radius`.
fn triangle(radius: f64) -> CornerTarget {
    let vertices = [Point::new(210.0, 150.0), Point::new(290.0, 250.0), Point::new(130.0, 250.0)];
    let corners = (0..3).map(|i| CornerPoint::new(vertices[i], vertices[(i + 2) % 3], vertices[(i + 1) % 3], radius, CornerKind::Round)).collect();
    CornerTarget::new(1, corners, true)
}

/// The radius the apex's widget gives it when dragged `down` px toward the triangle's inside.
fn apex_radius_after_drag(down: f32) -> f64 {
    let host = Rc::new(RefCell::new(vec![triangle(START_RADIUS)]));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut targets = shown.borrow_mut();
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(210.0, 200.0), 1.0);
        Canvas::new("Document")
            .view(view)
            .show(ui, |ui, body| CornerWidgets::new("Corner radius", Bind::bound(&mut *targets)).show_at(ui, body.rect));
    });
    fixture.settle();
    let widget = fixture.by_label("Corner 1 of 3").expect("the apex has a widget");
    fixture.drag(&widget, vec2(0.0, down));
    host.borrow()[0].corners[0].radius
}

/// A press on a widget of a shown set reaches what lies under the widgets, the Selection tool: the widgets declare no
/// part and show no node, and no radius changes.
#[trial]
fn shown_corner_widgets_leave_a_press_on_a_widget_to_the_canvas_background() {
    let targets = Rc::new(RefCell::new(vec![square()]));
    let reach = Rc::new(Cell::new(TransformReach::Box));
    let (shown, mode) = (Rc::clone(&targets), Rc::clone(&reach));
    let view = |room: egui::Rect| CanvasView::new(room, Point::new(210.0, 200.0), 1.0);
    let mut fixture = ToolFixture::with_view("Document", Presses::default(), view, move |ui, body| {
        let mut targets = shown.borrow_mut();
        let _ = CornerWidgets::new("Corner radius", Bind::bound(&mut *targets)).reach(mode.get()).show_at(ui, body.rect);
    });
    // Where the widget is while it takes presses.
    let at = fixture.fixture().by_label("Corner 1 of 4").expect("the first corner has a widget").center();
    reach.set(TransformReach::Shown);
    fixture.fixture().settle();
    assert!(fixture.fixture().by_label("Corner 1 of 4").is_none(), "a shown widget has no node");
    fixture.drag_from(at, vec2(30.0, 30.0));
    assert_eq!(fixture.tool().count, 1, "the press reached the tool");
    assert_eq!(targets.borrow()[0].corners[0].radius, START_RADIUS, "no radius changed");
}

/// The apex of a triangle is a corner like any other: pulling its widget inward rounds it, and the further the
/// pointer travels the larger the radius.
#[trial]
fn a_triangle_apex_rounds_with_the_pointer_travel() {
    let small = apex_radius_after_drag(10.0);
    let large = apex_radius_after_drag(30.0);
    assert!(small > START_RADIUS, "a short pull rounds the apex: {small}");
    assert!(large > small, "a longer pull rounds it more: {small} then {large}");
}
