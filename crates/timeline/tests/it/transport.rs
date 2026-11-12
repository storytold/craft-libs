//! The transport bar: its "+" editor sits at the end edge, and unchecking a button there hides it from the row and
//! reports the settings changed.

use std::cell::RefCell;
use std::rc::Rc;

use craft_timeline::ui::{TransportBar, TransportBarSettings, TransportButton};
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::{Fixture, trial};
use egui::vec2;

#[derive(Default)]
struct Seen {
    settings: TransportBarSettings,
    changed: bool,
}

fn bar() -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shown = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let seen = &mut *shown.borrow_mut();
        let out = TransportBar::new("Program transport", &TransportButton::PROGRAM).settings(Bind::bound(&mut seen.settings)).show(ui);
        seen.changed |= out.settings_changed;
    });
    fixture.room(vec2(900.0, 80.0));
    fixture.settle();
    (fixture, seen)
}

#[trial]
fn unchecking_a_button_in_the_end_edge_editor_hides_it_from_the_row() {
    let (mut fixture, seen) = bar();
    let row = fixture.by_role(Role::TOOLBAR).first().expect("the bar").rect;
    let opener = fixture.by_label("Button Editor").expect("the editor's opener");
    assert!(row.max.x - opener.rect.max.x < 16.0, "the opener sits at the end edge: {:?} in {:?}", opener.rect, row);
    let in_row = |fixture: &Fixture| fixture.by_role(Role::BUTTON).iter().filter(|button| row.contains(button.center())).count();
    let buttons = in_row(&fixture);

    fixture.click(&opener);
    fixture.settle();
    let first = fixture.by_role(Role::CHECK_BOX).first().cloned().expect("a checkbox per button");
    fixture.click(&first);
    fixture.settle();
    let seen = seen.borrow();
    assert_eq!(seen.settings.hidden, [TransportButton::AddMarker], "the unchecked button is hidden");
    assert!(seen.changed, "the bar reports the settings changed");
    drop(seen);
    assert_eq!(in_row(&fixture), buttons - 1, "the row left the hidden button out");
}
