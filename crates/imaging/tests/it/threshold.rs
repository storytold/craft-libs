//! `ThresholdEditor` under a disabled scope: an enclosing container's `enabled(false)` disables the editor, as it
//! does any component.

use std::cell::RefCell;
use std::rc::Rc;

use craft_imaging::ui::adjust::ThresholdEditor;
use craft_ui::contract::{Bind, Props as _, Role, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Fixture, trial};
use egui::vec2;

/// The level after a drag along the editor's slider, and how many edits the editor reported, inside a Stack that is
/// `enabled` or not.
fn drag_level(enabled: bool) -> (f64, usize) {
    let seen = Rc::new(RefCell::new((128.0, 0)));
    let shown = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut seen = shown.borrow_mut();
        let (level, edits) = &mut *seen;
        Stack::new().enabled(enabled).show(ui, |ui, _| {
            let out = ThresholdEditor::new("Threshold", Bind::bound(level)).show(ui);
            *edits += out.events.into_iter().count();
        });
    });
    fixture.settle();
    let slider = fixture.by_role(Role::SLIDER).first().cloned().expect("the Threshold Level slider");
    fixture.drag(&slider, vec2(slider.rect.width() / 4.0, 0.0));
    fixture.settle();
    *seen.borrow()
}

#[trial]
fn a_disabled_scope_takes_no_edit_from_the_slider() {
    let (level, edits) = drag_level(true);
    assert!(edits > 0 && level != 128.0, "an enabled editor takes the drag ({edits} edits, level {level})");
    assert_eq!(drag_level(false), (128.0, 0), "a disabled scope takes no edit");
}
