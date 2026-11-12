//! The stroke button's popover: it closes when the value changes from outside, and stays open for its own edits and a
//! change of width.

use craft_draw::ui::stroke::{LineStyle, StrokeButton, StrokeStyle};
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

struct Host {
    style: StrokeStyle,
    open: bool,
}

/// A button over the host's style and open flag, its popover open.
fn opened() -> (Fixture, Rc<RefCell<Host>>) {
    let host = Rc::new(RefCell::new(Host { style: StrokeStyle::default(), open: false }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let Host { style, open } = &mut *host;
        let _ = StrokeButton::new("Stroke Options", Bind::bound(style), Bind::bound(open)).show(ui);
    });
    fixture.settle();
    host.borrow_mut().open = true;
    fixture.idle(3);
    assert!(host.borrow().open, "the bound flag opens the popover");
    assert!(fixture.by_label("Dashed").is_some(), "the editor shows");
    (fixture, host)
}

#[trial]
fn popover_closes_on_a_change_from_outside_only() {
    let (mut fixture, host) = opened();

    let dashed = fixture.by_label("Dashed").expect("the editor shows the presets");
    fixture.click(&dashed);
    fixture.idle(3);
    assert!(matches!(host.borrow().style.line_style(), LineStyle::Dashed), "the pick reaches the host");
    assert!(host.borrow().open, "the editor's own edit leaves the popover open");

    host.borrow_mut().style.width += 2.0;
    fixture.idle(3);
    assert!(host.borrow().open, "a width change from outside leaves it open");

    {
        let mut host = host.borrow_mut();
        host.style = host.style.with_line_style(LineStyle::Dotted);
    }
    fixture.idle(3);
    assert!(!host.borrow().open, "another line style from outside (Undo, another control) closes it");
}
