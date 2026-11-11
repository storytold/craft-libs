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
    use craft_ui::canvas::{Canvas, ToolHost};
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
        let canvas = Canvas::new("Document")
            .view(view)
            .show(ui, |ui, body| SymmetryHandles::new("Symmetry", Bind::bound(&mut preview), page).session().show_at(ui, body.rect));
        if let Some(out) = canvas.inner {
            out.edit.events.iter().for_each(|event| shown.borrow_mut().apply(event));
            sn.borrow_mut().discarded |= out.discard;
        }
        let held = shown.borrow();
        let reply = ToolHost::new().run(ui, &canvas.output, &mut SymmetryTool::new(&held, true));
        sn.borrow_mut().ignored_presses += usize::from(reply.ignored_press.is_some());
    });
    fixture.settle().key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    assert!(seen.borrow().discarded, "Escape with nothing focused discards the session");
    fixture.click_at(egui::pos2(40.0, 40.0)).settle();
    assert_eq!(seen.borrow().ignored_presses, 0, "the press away from the handles is the session tool's");
}
