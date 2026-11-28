//! Painting symmetry: the model's geometry, and the handles and tool of an open Transform Symmetry.

use craft_behavior::kurbo::{Point, Size};
use craft_draw::ui::symmetry::{Symmetry, SymmetryMode};
use craft_ui::testing::trial;

/// Every mode on 100 x 100 and 101 x 53 gives segments inside the page; a horizontal axis at y -5 gives none.
#[trial]
fn segments_clip_to_the_page_and_vanish_when_the_centre_line_misses_it() {
    for size in [Size::new(100.0, 100.0), Size::new(101.0, 53.0)] {
        for mode in SymmetryMode::ALL {
            let s = Symmetry::new(mode, Point::new(size.width / 2.0, size.height / 2.0), 0.0).expect("a valid symmetry");
            let segments = s.segments(size);
            assert_eq!(segments.len(), if mode == SymmetryMode::Dual { 2 } else { 1 });
            for line in segments {
                for p in [line.p0, line.p1] {
                    assert!(p.x >= -1e-9 && p.x <= size.width + 1e-9 && p.y >= -1e-9 && p.y <= size.height + 1e-9);
                }
            }
        }
    }
    let off = Symmetry::new(SymmetryMode::Horizontal, Point::new(50.0, -5.0), 0.0).expect("a valid symmetry");
    assert!(off.segments(Size::new(100.0, 100.0)).is_empty());
}

/// Transform Symmetry open on a canvas at 100 % with the page's centre in the middle: Escape with nothing focused
/// reaches the handles, and a press away from them is spent by the session's tool, so it never paints.
#[trial]
fn the_open_transform_symmetry_takes_escape_and_every_press_on_the_canvas() {
    use craft_draw::ui::symmetry::{SymmetryHandles, SymmetryTool, SymmetryTransform};
    use craft_ui::canvas::{Canvas, ToolHost, tool_pass};
    use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
    use craft_ui::testing::Fixture;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Seen {
        discarded: bool,
        ignored_presses: usize,
    }
    let page = Size::new(400.0, 300.0);
    let symmetry = Symmetry::new(SymmetryMode::Vertical, Point::new(200.0, 150.0), 0.0).expect("a valid symmetry");
    let session = Rc::new(RefCell::new(SymmetryTransform::begin(symmetry)));
    let seen = Rc::new(RefCell::new(Seen::default()));
    let (shown, sn) = (Rc::clone(&session), Rc::clone(&seen));
    let mut fixture = Fixture::new(move |ui| {
        let view = craft_behavior::CanvasView::new(ui.available_rect_before_wrap(), Point::new(200.0, 150.0), 1.0);
        let mut preview = shown.borrow().preview();
        let mut reply = None;
        let canvas = Canvas::new("Document").view(view).show(ui, |ui, body| {
            let out = SymmetryHandles::new("Symmetry", Bind::bound(&mut preview), page).session().show_at(ui, body.rect);
            if let Some(pass) = tool_pass(&body.env) {
                let held = shown.borrow();
                reply = Some(ToolHost::new().run(ui, &pass, &mut SymmetryTool::new(&held, true)));
            }
            out
        });
        if let Some(out) = canvas.inner {
            out.edit.events.iter().for_each(|event| shown.borrow_mut().apply(event));
            sn.borrow_mut().discarded |= out.discard;
        }
        sn.borrow_mut().ignored_presses += usize::from(reply.is_some_and(|reply| reply.ignored_press.is_some()));
    });
    fixture.settle().key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    assert!(seen.borrow().discarded, "Escape with nothing focused discards the session");
    fixture.click_at(egui::pos2(40.0, 40.0)).settle();
    assert_eq!(seen.borrow().ignored_presses, 0, "the press away from the handles is the session tool's");
}

/// The handles' gestures on a canvas at 100 %: a drag of the centre moves it by the drag in document units, and a drag
/// of the rotation handle turns the axes about the centre; with Shift the rotation is a multiple of
/// `TURN_STEP_DEGREES` counted from 0, not from the angle the drag started at.
#[trial]
fn the_handles_move_the_centre_and_turn_in_steps_with_shift() {
    use craft_draw::ui::symmetry::{SymmetryHandles, SymmetryTransform, TURN_STEP_DEGREES};
    use craft_ui::canvas::Canvas;
    use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _};
    use craft_ui::testing::Fixture;
    use std::cell::RefCell;
    use std::rc::Rc;

    let page = Size::new(400.0, 300.0);
    // A start rotation that is no multiple of the step, so a snap relative to the start differs from an absolute one.
    let start = Symmetry::new(SymmetryMode::Vertical, Point::new(200.0, 150.0), 10.0).expect("a valid symmetry");
    let session = Rc::new(RefCell::new(SymmetryTransform::begin(start)));
    let shown = Rc::clone(&session);
    let mut fixture = Fixture::new(move |ui| {
        let view = craft_behavior::CanvasView::new(ui.available_rect_before_wrap(), Point::new(200.0, 150.0), 1.0);
        let mut preview = shown.borrow().preview();
        let canvas = Canvas::new("Document")
            .view(view)
            .show(ui, |ui, body| SymmetryHandles::new("Symmetry", Bind::bound(&mut preview), page).session().show_at(ui, body.rect));
        if let Some(out) = canvas.inner {
            out.edit.events.iter().for_each(|event| shown.borrow_mut().apply(event));
        }
    });
    fixture.settle();

    // The centre follows the drag by the drag's length in document units; the rotation stays.
    let centre = fixture.by_role(Role::BUTTON)[0];
    fixture.drag_at(centre.center(), egui::vec2(30.0, -20.0));
    let moved = session.borrow().preview();
    assert!((moved.center.x - 230.0).abs() < 1e-3 && (moved.center.y - 130.0).abs() < 1e-3, "the centre moved by the drag: {:?}", moved.center);
    assert_eq!(moved.rotation, 10.0, "moving the centre keeps the rotation");

    // A handle drag that turns the axes by 30 degrees from 10 lands on 45 with Shift, not on 40.
    fixture.settle();
    let centre = fixture.by_role(Role::BUTTON)[0].center();
    let handle = fixture.by_role(Role::SLIDER)[0].center();
    let (arm, from) = (handle - centre, f64::from((handle - centre).y).atan2(f64::from((handle - centre).x)));
    let target = centre + arm.length() * egui::vec2((from + 30f64.to_radians()).cos() as f32, (from + 30f64.to_radians()).sin() as f32);
    fixture.hold(egui::Modifiers::SHIFT);
    fixture.drag_at(handle, target - handle);
    fixture.hold(egui::Modifiers::NONE);
    let turned = session.borrow().preview();
    assert!((turned.rotation - 45.0).abs() < 1e-6, "Shift rounds the rotation to a multiple of the step from 0: {}", turned.rotation);
    assert_eq!(turned.rotation % TURN_STEP_DEGREES, 0.0);
    assert_eq!(turned.center, moved.center, "turning keeps the centre");
}
