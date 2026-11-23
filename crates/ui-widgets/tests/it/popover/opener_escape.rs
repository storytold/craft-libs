//! A popover that leaves the focus on its opener (`Initial::Keep`, no focus trap) closes on an Escape at that opener
//! only when its dismissal asks for it (`Dismiss::opener_escape`). `Close` keeps the opener focused; `CloseAndBlur`
//! also gives up the opener's focus. Without the rule the Escape at the opener leaves the popover as it was.

use craft_ui::buttons::Button;
use craft_ui::contract::overlay::OpenerEscape;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, clicked};
use craft_ui::popover::Popover;
use craft_ui::runtime::focus::{self, FocusTarget, Initial};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// A popover of group choices hanging from a "Group" button, which keeps the focus while the popover is open.
struct Rig {
    fixture: Fixture,
    open: Rc<Cell<bool>>,
    opener: Rc<Cell<Option<Id>>>,
}

fn rig(rule: OpenerEscape) -> Rig {
    let open = Rc::new(Cell::new(false));
    let opener = Rc::new(Cell::new(None));
    let (shown_open, shown_opener) = (open.clone(), opener.clone());
    let mut fixture = Fixture::new(move |ui| {
        Button::new("Before").show(ui);
        let picker = Button::new("Group").show(ui);
        shown_opener.set(Some(picker.id));
        let mut is_open = shown_open.get();
        if clicked(&picker) {
            is_open = !is_open;
        }
        Popover::new("Groups", Bind::bound(&mut is_open))
            .opened_by(&picker)
            .initial_focus(Initial::Keep)
            .overlay(|mut overlay| {
                overlay.focus = overlay.focus.trap(false);
                overlay.dismiss = overlay.dismiss.focus_left(false).opener_escape(rule);
                overlay
            })
            .show(ui, |ui, _| {
                Button::new("Portraits").show(ui);
                Button::new("Landscapes").show(ui);
            });
        shown_open.set(is_open);
    });
    fixture.settle();
    let id = opener.get().expect("the opener ran");
    focus::request(fixture.ctx(), FocusTarget::Id(id));
    fixture.settle();
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(open.get(), "Enter on the focused opener opens the popover");
    assert!(fixture.by_label("Portraits").is_some(), "the popover shows its choices");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), Some(id), "the opener keeps the focus (Initial::Keep)");
    Rig { fixture, open, opener }
}

impl Rig {
    fn escape(&mut self) {
        self.fixture.key(Key::Escape, Modifiers::NONE);
        self.fixture.settle();
    }

    fn opener_focused(&self) -> bool {
        self.fixture.focused_stop().map(|(id, _)| id) == self.opener.get()
    }
}

#[trial]
fn escape_at_the_opener_leaves_the_popover_open_by_default() {
    let mut rig = rig(OpenerEscape::Default);
    rig.escape();
    assert!(rig.open.get() && rig.fixture.by_label("Portraits").is_some(), "without the rule the popover stays open");
}

#[trial]
fn opener_escape_close_closes_the_popover_and_keeps_the_opener_focused() {
    let mut rig = rig(OpenerEscape::Close);
    rig.escape();
    assert!(!rig.open.get(), "the Escape at the opener closes the popover");
    assert!(rig.fixture.by_label("Portraits").is_none(), "the choices are gone");
    assert!(rig.opener_focused(), "the opener keeps the focus");
}

#[trial]
fn opener_escape_close_and_blur_closes_the_popover_and_leaves_the_opener() {
    let mut rig = rig(OpenerEscape::CloseAndBlur);
    rig.escape();
    assert!(!rig.open.get(), "the Escape at the opener closes the popover");
    assert!(!rig.opener_focused(), "the opener gives up the focus");
}
