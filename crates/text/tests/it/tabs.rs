//! The Tabs dialog: its focus (it opens on the default interval, Set moves into the new stop's row), staged edits, Set and
//! Clear, and a held OK.

use craft_text::ui::ruler::{TabAlign, TabLeader, TabStop};
use craft_text::ui::tabs_dialog::{TabStopsDialog, TabStopsResult};
use craft_ui::dialog::{DialogResult, DialogStack, StackDialog as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

/// The node named `label`: for "Position", the first stop row's position field, not the column header of that name.
fn labelled(fixture: &Fixture, label: &str) -> Option<craft_ui::testing::Target> {
    if label != "Position" {
        return fixture.by_label(label);
    }
    let field = fixture.by_label("Default tab stops:")?;
    let role = fixture.tree().role_of(field.node)?;
    fixture.by_role(role).into_iter().find(|target| fixture.tree().name_of(target.node) == Some(label))
}

/// Which of `labels` holds the focus.
fn focused<'a>(fixture: &Fixture, labels: &[&'a str]) -> Option<&'a str> {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    labels.iter().copied().find(|label| labelled(fixture, label).is_some_and(|target| target.node == focused))
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
    let target = labelled(fixture, label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.click(&target).idle(4);
}

fn stop(position: f32, align: TabAlign, leader: TabLeader) -> TabStop {
    TabStop { position, align, leader }
}

/// Types `text` into the first row's position field and leaves the field, which ends the edit.
fn retype_first_position(fixture: &mut Fixture, text: &str) {
    press(fixture, "Position");
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
    let labels = ["Default tab stops:", "Position", "Set"];
    assert_eq!(focused(&fixture, &labels), Some("Default tab stops:"), "the dialog opens on its first field");
    let set = fixture.by_label("Set").expect("Set is shown");
    fixture.click(&set).idle(4);
    assert_eq!(focused(&fixture, &labels), Some("Position"), "Set moves the focus into the new stop's position");
    let set = fixture.by_label("Set").expect("Set is shown");
    fixture.click(&set).idle(4);
    let key = craft_ui::testing::access::focused_key(fixture.tree());
    assert_eq!(fixture.tree().name_of(key), Some("Position"), "a second Set moves into a position field too");
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

/// Drawn bare, for a window of its own that has the title and close button, the dialog shows its stops and its Cancel
/// and OK without its own close box, and OK still answers.
#[trial]
fn a_bare_tabs_dialog_has_no_close_box_of_its_own_and_ok_answers() {
    let (framed, _, _) = shown(TabStopsDialog::new(vec![stop(72.0, TabAlign::Left, TabLeader::None)], 36.0));
    assert!(framed.by_label("Close").is_some(), "the dialog over a window has its own close box");

    let (mut fixture, _, result) = shown(TabStopsDialog::new(vec![stop(72.0, TabAlign::Left, TabLeader::None)], 36.0).bare(true));
    assert!(fixture.by_label("Close").is_none(), "the bare dialog leaves the close box to its window");
    assert!(fixture.by_label("Cancel").is_some(), "Cancel is shown");
    press(&mut fixture, "OK");
    assert!(result.borrow().is_some(), "OK answered");
}

/// In the bare dialog the Enter or Escape that ends a stop's position edit ends only the edit: the dialog stays open, an
/// Enter keeps the typed position and an Escape the one the edit began from.
#[trial]
fn a_key_that_ends_a_position_edit_in_the_bare_dialog_does_not_settle_it() {
    let stops = vec![stop(72.0, TabAlign::Left, TabLeader::None)];
    let (mut fixture, _, result) = shown(TabStopsDialog::new(stops.clone(), 36.0).bare(true));
    press(&mut fixture, "Position");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("108").key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(result.borrow().is_none(), "the Enter that ends the edit does not answer");
    press(&mut fixture, "OK");
    let answer = result.borrow().clone().expect("OK answered");
    assert_eq!(answer.stops, vec![stop(108.0, TabAlign::Left, TabLeader::None)], "the edit kept the typed position");

    let cancelled = Rc::new(RefCell::new(false));
    let dialog = Rc::new(RefCell::new(TabStopsDialog::new(stops, 36.0).bare(true)));
    let (shown, seen) = (dialog.clone(), cancelled.clone());
    let mut fixture = Fixture::new(move |ui| {
        if let Some(DialogResult::Cancel) = shown.borrow_mut().show(ui, &DialogStack::default()) {
            *seen.borrow_mut() = true;
        }
    });
    fixture.idle(30);
    press(&mut fixture, "Position");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("108").key(Key::Escape, Modifiers::NONE).idle(4);
    assert!(!*cancelled.borrow(), "the Escape that ends the edit does not cancel the dialog");
    assert_eq!(dialog.borrow_mut().result().stops, vec![stop(72.0, TabAlign::Left, TabLeader::None)], "Escape restored the position");
}

/// A bare dialog's refused answer shows its reason, as the framed dialog does, until a value changes.
#[trial]
fn a_bare_dialog_shows_a_refusal_until_a_value_changes() {
    let (mut fixture, dialog, _) = shown(TabStopsDialog::new(Vec::new(), 36.0).held_ok().bare(true));
    press(&mut fixture, "OK");
    assert!(dialog.borrow_mut().take_ok().is_some(), "OK filed its result");
    dialog.borrow_mut().answer_ok(Err("Not allowed".into()));
    fixture.idle(4);
    assert!(fixture.by_label("Not allowed").is_some(), "the refusal is shown");
    press(&mut fixture, "Set");
    assert!(fixture.by_label("Not allowed").is_none(), "an edit ends the refusal");
}

/// In the bare dialog, the Enter that ends typing in the default interval's field takes the value and only that: the
/// dialog stays open, and the next Enter is OK again.
#[trial]
fn enter_in_the_default_interval_takes_the_value_and_the_next_enter_answers() {
    let (mut fixture, _, result) = shown(TabStopsDialog::new(Vec::new(), 36.0).bare(true));
    press(&mut fixture, "Default tab stops:");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("50").key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(result.borrow().is_none(), "the Enter that ends the edit does not answer");
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    let answer = result.borrow().clone().expect("the next Enter is OK");
    assert!((answer.default_interval - 36.0).abs() > 1.0, "the typed interval was taken: {}", answer.default_interval);
}

/// The Tab stop position box, as in Word: in the bare dialog, Enter there sets a stop at the typed position and leaves
/// the dialog open, and the next Enter is OK; Set does the same with the typed position.
#[trial]
fn enter_in_the_tab_stop_position_box_sets_a_stop_there_and_the_next_enter_answers() {
    let (mut fixture, dialog, result) = shown(TabStopsDialog::new(Vec::new(), 36.0).bare(true));
    press(&mut fixture, "Tab stop position:");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("100 pt").key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(result.borrow().is_none(), "the Enter that sets the stop does not answer");
    assert_eq!(dialog.borrow_mut().result().stops, [stop(100.0, TabAlign::Left, TabLeader::None)], "Enter set a stop at the typed position");
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    let answer = result.borrow().clone().expect("the next Enter is OK");
    assert_eq!(answer.stops, [stop(100.0, TabAlign::Left, TabLeader::None)]);

    let (mut fixture, dialog, _) = shown(TabStopsDialog::new(Vec::new(), 36.0).bare(true));
    press(&mut fixture, "Tab stop position:");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("90 pt");
    press(&mut fixture, "Set");
    assert_eq!(dialog.borrow_mut().result().stops, [stop(90.0, TabAlign::Left, TabLeader::None)], "Set put the stop at the typed position");
}

/// Which button is the default in the bare dialog: OK when it opens (focus on its first field is not the user's), Set
/// from a click or Tab into the Tab stop position box until Enter there, and neither while another number field holds
/// Enter.
#[trial]
fn the_default_button_follows_the_field_that_holds_enter() {
    let (mut fixture, dialog, _) = shown(TabStopsDialog::new(Vec::new(), 36.0).bare(true));
    let default = |dialog: &Rc<RefCell<TabStopsDialog>>| {
        let dialog = dialog.borrow();
        (dialog.ok_is_default(), dialog.set_is_default())
    };
    assert_eq!(default(&dialog), (true, false), "OK is the default when the dialog opens");
    press(&mut fixture, "Tab stop position:");
    assert_eq!(default(&dialog), (false, true), "a click into the box makes Set the default");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("100 pt").key(Key::Enter, Modifiers::NONE).idle(4);
    assert_eq!(default(&dialog), (true, false), "after Enter in the box, OK is the default again");
    press(&mut fixture, "Default tab stops:");
    assert_eq!(default(&dialog), (false, false), "a click into Default tab stops: Enter takes the interval");
    fixture.key(Key::Tab, Modifiers::NONE).idle(4);
    assert_eq!(default(&dialog), (false, true), "Tab into the box makes Set the default");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("200 pt").key(Key::Escape, Modifiers::NONE).idle(4);
    assert_eq!(default(&dialog), (true, false), "Escape abandoning a typed position gives the keys back");
    press(&mut fixture, "Position");
    assert_eq!(default(&dialog), (false, false), "a click into a stop's position: Enter takes the position");
}

/// The Tab stop position box shows the next free step while no stop is selected (as when the dialog opens: the default
/// interval with no stops), and the selected stop's position, as in Word.
#[trial]
fn the_tab_stop_position_box_shows_the_selected_stop() {
    let stops = vec![stop(72.0, TabAlign::Left, TabLeader::None), stop(144.0, TabAlign::Left, TabLeader::None)];
    let (mut fixture, dialog, _) = shown(TabStopsDialog::new(stops, 36.0).bare(true));
    let shown_in_box = |fixture: &Fixture| {
        let target = fixture.by_label("Tab stop position:").expect("the box is shown");
        fixture.tree().value_of(target.node).unwrap_or_default().to_owned()
    };
    assert!(shown_in_box(&fixture).contains("180"), "the box opens on the next free step: {:?}", shown_in_box(&fixture));
    let (empty, _, _) = shown(TabStopsDialog::new(Vec::new(), 36.0).bare(true));
    assert!(shown_in_box(&empty).starts_with("36"), "with no stops, the default interval: {:?}", shown_in_box(&empty));
    // Set adds a stop where the box shows, and the box moves on to the next free step.
    press(&mut fixture, "Set");
    assert!(shown_in_box(&fixture).contains("216"), "after Set, the next free step: {:?}", shown_in_box(&fixture));
    dialog.borrow_mut().select(Some(1));
    fixture.idle(4);
    assert!(shown_in_box(&fixture).contains("144"), "the box follows the selection: {:?}", shown_in_box(&fixture));
    press(&mut fixture, "Clear");
    // Stops at 72 and 180, 144 cleared: the next free step is 216.
    assert!(shown_in_box(&fixture).contains("216"), "with nothing selected, the box shows the next free step: {:?}", shown_in_box(&fixture));
    // Leaving the box does not take the step it shows as typed: Set adds that step.
    press(&mut fixture, "Tab stop position:");
    press(&mut fixture, "Set");
    let positions: Vec<f32> = dialog.borrow_mut().result().stops.iter().map(|stop| stop.position).collect();
    assert_eq!(positions, vec![72.0, 180.0, 216.0], "Set put a stop at the step the box showed");
}
