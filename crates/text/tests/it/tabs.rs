//! The Tabs dialog: its focus (it opens on the default interval, Set moves into the new stop's row), staged edits, Set and
//! Clear, and a held OK.

use craft_text::ui::ruler::{TabAlign, TabLeader, TabStop};
use craft_text::ui::tabs_dialog::{TabStopsDialog, TabStopsResult};
use craft_ui::dialog::{DialogResult, DialogStack, StackDialog as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

/// Which of `labels` holds the focus.
fn focused<'a>(fixture: &Fixture, labels: &[&'a str]) -> Option<&'a str> {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    labels.iter().copied().find(|label| fixture.by_label(label).is_some_and(|target| target.node == focused))
}

/// A fixture showing `dialog`, and where the result of a closing OK lands.
type Shown = (Fixture, Rc<RefCell<TabStopsDialog>>, Rc<RefCell<Option<TabStopsResult>>>);

fn shown(dialog: TabStopsDialog) -> Shown {
    let dialog = Rc::new(RefCell::new(dialog));
    let result = Rc::new(RefCell::new(None));
    let (shown, landed) = (dialog.clone(), result.clone());
    let mut fixture = Fixture::new(move |ui| {
        if let Some(DialogResult::Ok(answer)) = shown.borrow_mut().show(ui, &DialogStack::default()) {
            *landed.borrow_mut() = Some(answer);
        }
    });
    fixture.idle(30);
    (fixture, dialog, result)
}

fn press(fixture: &mut Fixture, label: &str) {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.click(&target).idle(4);
}

fn stop(position: f32, align: TabAlign, leader: TabLeader) -> TabStop {
    TabStop { position, align, leader }
}

/// Types `text` into the first row's position field and leaves the field, which ends the edit.
fn retype_first_position(fixture: &mut Fixture, text: &str) {
    press(fixture, "Tab stop position:");
    fixture.key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Tab, Modifiers::NONE).idle(4);
}

#[trial]
fn the_dialog_opens_on_the_default_interval_and_set_moves_into_the_new_stop() {
    let dialog = Rc::new(RefCell::new(TabStopsDialog::new(Vec::new(), 36.0)));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = shown.borrow_mut().show(ui, &DialogStack::default());
    });
    fixture.idle(30);
    let labels = ["Default tab stops:", "Tab stop position:", "Set"];
    assert_eq!(focused(&fixture, &labels), Some("Default tab stops:"), "the dialog opens on its first field");
    let set = fixture.by_label("Set").expect("Set is shown");
    fixture.click(&set).idle(4);
    assert_eq!(focused(&fixture, &labels), Some("Tab stop position:"), "Set moves the focus into the new stop's position");
    let set = fixture.by_label("Set").expect("Set is shown");
    fixture.click(&set).idle(4);
    let key = craft_ui::testing::access::focused_key(fixture.tree());
    assert_eq!(fixture.tree().name_of(key), Some("Tab stop position:"), "a second Set moves into a position field too");
    let value = fixture.tree().value_of(key).unwrap_or_default();
    assert!(value.contains("72"), "the focus is in the new row's field, at 72 pt: {value:?}");
}

#[trial]
fn editing_a_stop_position_stages_a_clear_and_a_set_and_the_selection_follows_it() {
    let stops = vec![stop(72.0, TabAlign::Right, TabLeader::Dot), stop(144.0, TabAlign::Left, TabLeader::None)];

    let (mut fixture, _, result) = shown(TabStopsDialog::new(stops.clone(), 36.0));
    retype_first_position(&mut fixture, "108");
    press(&mut fixture, "OK");
    let moved = result.borrow().clone().expect("OK answered");
    assert_eq!(
        moved.stops,
        vec![stop(108.0, TabAlign::Right, TabLeader::Dot), stop(144.0, TabAlign::Left, TabLeader::None)],
        "the moved stop keeps its alignment and leader"
    );
    assert_eq!(moved.cleared, vec![72.0], "the old position is cleared");

    let (mut fixture, _, result) = shown(TabStopsDialog::new(stops, 36.0));
    retype_first_position(&mut fixture, "144");
    press(&mut fixture, "OK");
    let merged = result.borrow().clone().expect("OK answered");
    assert_eq!(merged.stops, vec![stop(144.0, TabAlign::Right, TabLeader::Dot)], "two stops on one position merge, the edited one winning");
    assert_eq!(merged.cleared, vec![72.0]);
}

#[trial]
fn set_adds_the_next_free_multiple_and_clear_keeps_the_position_cleared() {
    let stops = vec![stop(72.0, TabAlign::Right, TabLeader::None), stop(144.0, TabAlign::Left, TabLeader::None)];
    let (mut fixture, _, result) = shown(TabStopsDialog::new(stops, 36.0));
    press(&mut fixture, "Set");
    press(&mut fixture, "Clear");
    press(&mut fixture, "Set");
    press(&mut fixture, "OK");
    let answer = result.borrow().clone().expect("OK answered");
    let positions: Vec<f32> = answer.stops.iter().map(|stop| stop.position).collect();
    assert_eq!(positions, vec![72.0, 144.0, 216.0], "Set steps past the last stop, and past a position that was cleared");
    assert_eq!(answer.cleared, vec![180.0], "Clear keeps the position to be cleared");
}

#[trial]
fn a_held_ok_stays_open_until_answered_and_a_refusal_shows_until_a_value_changes() {
    let (mut fixture, dialog, result) = shown(TabStopsDialog::new(Vec::new(), 36.0).held_ok());
    press(&mut fixture, "OK");
    assert!(result.borrow().is_none(), "a held OK does not close the dialog");
    let ok = fixture.by_label("OK").expect("OK is shown");
    assert!(fixture.tree().is_disabled_key(ok.node), "OK waits disabled for the answer");
    assert!(dialog.borrow_mut().take_ok().is_some(), "the filed result reaches the app once");
    assert!(dialog.borrow_mut().take_ok().is_none());

    dialog.borrow_mut().answer_ok(Err("Not allowed".into()));
    fixture.idle(4);
    assert!(fixture.by_label("Not allowed").is_some(), "the refusal is shown");
    let ok = fixture.by_label("OK").expect("OK is shown");
    assert!(!fixture.tree().is_disabled_key(ok.node), "a refusal enables OK again");

    press(&mut fixture, "Set");
    assert!(fixture.by_label("Not allowed").is_none(), "an edit ends the refusal");

    press(&mut fixture, "OK");
    assert!(dialog.borrow_mut().take_ok().is_some());
    dialog.borrow_mut().answer_ok(Ok(()));
    fixture.idle(4);
    let answer = result.borrow().clone().expect("the accepted answer closes the dialog");
    assert_eq!(answer.stops.len(), 1, "the dialog closes with the answer it filed");
}
