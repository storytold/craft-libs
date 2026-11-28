//! A popover that leaves the focus on its opener (`Initial::Keep`, no focus trap) and hangs from an opener inside
//! another layer (a dialog's or a popover's button) closes on an Escape at that opener only when its dismissal asks for
//! it (`Dismiss::opener_escape`): with the focus in the outer layer, the inner popover is the top layer but holds no
//! focus, so without the rule the Escape closes nothing. `Close` keeps the opener focused; `CloseAndBlur` also gives
//! up the opener's focus. An opener in the window (no layer) needs no rule: a popup already takes an Escape while the
//! focus is in no layer.

use craft_ui::buttons::Button;
use craft_ui::contract::overlay::OpenerEscape;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, clicked};
use craft_ui::popover::Popover;
use craft_ui::runtime::focus::{self, FocusTarget, Initial};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Key, Modifiers, Ui};
use std::cell::Cell;
use std::rc::Rc;

/// The group choices popover hanging from a "Group" button, which keeps the focus while the popover is open.
fn choices(ui: &mut Ui, open: &Cell<bool>, opener: &Cell<Option<Id>>, rule: OpenerEscape) {
    let picker = Button::new("Group").show(ui);
    opener.set(Some(picker.id));
    let mut is_open = open.get();
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
    open.set(is_open);
}

struct Rig {
    fixture: Fixture,
    open: Rc<Cell<bool>>,
    /// The outer popover's open state, kept outside the fixture: a settle pass cannot reopen it and hide a wrong close.
    outer: Rc<Cell<bool>>,
    opener: Rc<Cell<Option<Id>>>,
}

/// The choices, with their opener inside an outer popover ("Save effect") when `nested`, else in the window.
fn rig(rule: OpenerEscape, nested: bool) -> Rig {
    let open = Rc::new(Cell::new(false));
    let opener = Rc::new(Cell::new(None));
    let outer_open = Rc::new(Cell::new(true));
    let (shown_open, shown_opener, shown_outer) = (open.clone(), opener.clone(), outer_open.clone());
    let mut fixture = Fixture::new(move |ui| {
        let save = Button::new("Save effect").show(ui);
        if nested {
            let mut outer = shown_outer.get();
            Popover::new("Save effect", Bind::bound(&mut outer)).opened_by(&save).show(ui, |ui, _| {
                Button::new("Name").show(ui);
                choices(ui, &shown_open, &shown_opener, rule);
            });
            shown_outer.set(outer);
        } else {
            choices(ui, &shown_open, &shown_opener, rule);
        }
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
    Rig { fixture, open, outer: outer_open, opener }
}

impl Rig {
    fn escape(&mut self) {
        self.fixture.key(Key::Escape, Modifiers::NONE);
        self.fixture.settle();
    }

    fn opener_focused(&self) -> bool {
        self.fixture.focused_stop().map(|(id, _)| id) == self.opener.get()
    }

    fn outer_open(&self) -> bool {
        self.outer.get() && self.fixture.by_label("Name").is_some()
    }
}

#[trial]
fn escape_at_an_opener_in_another_layer_leaves_the_popover_open_by_default() {
    let mut rig = rig(OpenerEscape::Default, true);
    rig.escape();
    assert!(rig.open.get() && rig.fixture.by_label("Portraits").is_some(), "without the rule the popover stays open");
    assert!(rig.outer_open(), "and so does the layer around the opener");
}

#[trial]
fn opener_escape_close_closes_the_popover_and_keeps_the_opener_focused() {
    let mut rig = rig(OpenerEscape::Close, true);
    rig.escape();
    assert!(!rig.open.get(), "the Escape at the opener closes the popover");
    assert!(rig.fixture.by_label("Portraits").is_none(), "the choices are gone");
    assert!(rig.outer_open(), "the layer around the opener stays open");
    assert!(rig.opener_focused(), "the opener keeps the focus");
}

#[trial]
fn opener_escape_close_and_blur_closes_the_popover_and_leaves_the_opener() {
    let mut rig = rig(OpenerEscape::CloseAndBlur, true);
    rig.escape();
    assert!(!rig.open.get(), "the Escape at the opener closes the popover");
    assert!(rig.outer_open(), "the layer around the opener stays open");
    assert!(!rig.opener_focused(), "the opener gives up the focus");
}

#[trial]
fn escape_at_an_opener_in_the_window_closes_the_popover_without_the_rule() {
    let mut rig = rig(OpenerEscape::Default, false);
    rig.escape();
    assert!(!rig.open.get(), "a popup takes the Escape while the focus is in no layer");
    assert!(rig.opener_focused(), "and the opener keeps the focus");
}
