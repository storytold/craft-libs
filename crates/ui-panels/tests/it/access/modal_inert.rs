//! What sits behind a modal is inert in the access tree: assistive technology and the test kit's lookups cannot find it.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

fn named(fixture: &Fixture, label: &str) -> usize {
    let tree = fixture.tree();
    tree.keys().into_iter().filter(|key| tree.name_of(*key) == Some(label)).count()
}

#[trial]
fn a_modal_dialog_hides_the_window_behind_it_until_it_closes() {
    let open = Rc::new(Cell::new(false));
    let state = open.clone();
    let mut fixture = Fixture::new(move |ui| {
        Button::new("OK").show(ui);
        let mut now = state.get();
        Dialog::new("Options").opened(Bind::bound(&mut now)).buttons([FooterButton::new("OK", DialogRole::Accept)]).show(ui, |_, _| {});
        state.set(now);
    });
    fixture.idle(10);
    assert_eq!(named(&fixture, "OK"), 1, "the window's OK is found while no modal is open");
    let window_ok = fixture.by_label("OK").expect("the window's OK").rect;
    open.set(true);
    fixture.idle(30);
    assert_eq!(named(&fixture, "OK"), 1, "only the modal's OK is found");
    assert_ne!(fixture.by_label("OK").expect("the modal's OK").rect, window_ok);
    open.set(false);
    fixture.idle(30);
    assert_eq!(named(&fixture, "OK"), 1);
    assert_eq!(fixture.by_label("OK").expect("the window's OK again").rect, window_ok);
}
