//! The corner widgets' drag reports its edit on the pass that commits it.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_draw::ui::corner::{CornerPoint, CornerTarget};
use craft_draw::ui::{CornerEdit, CornerKind, CornerWidgets};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{EditKind, Fixture, trial};
use egui::vec2;
use std::cell::RefCell;
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
