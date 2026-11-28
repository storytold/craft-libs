//! Hold shortcuts in the dialog: the switch, the Hold column, and what Cancel and OK leave of the setting.

use craft_keymap::{Binding, CommandRef, Platform, Policy};
use craft_menu::shortcuts::{HoldSetting, Outcome};
use craft_ui::contract::Role;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::{self, Rig};

/// The setting a trial starts from, at a delay inside its range.
fn setting(on: bool) -> HoldSetting {
    HoldSetting::new(on, 0.5, 0.2..=2.0)
}

/// The hold texts the keymap's overrides hold for the command `key`.
fn holds_of(rig: &Rig, key: &str) -> Vec<String> {
    let shared = rig.shared.borrow();
    shared.km.overrides().get(key).map(|texts| texts.iter().filter(|t| t.starts_with("Hold ")).cloned().collect()).unwrap_or_default()
}

/// Whether the message under the table says `part`.
fn says(rig: &Rig, part: &str) -> bool {
    [Role::STATUS, Role::ALERT].into_iter().any(|role| !rig.names_with(role, part).is_empty())
}

#[trial]
fn the_hold_column_records_a_hold_and_cancel_restores_the_setting() {
    let mut rig = Rig::with_holds(fixture::keymap(Platform::Mac), fixture::options(), setting(false));
    assert!(!rig.has("Add a hold shortcut to Save"), "no Hold column while holds are off");

    rig.click("Hold Shortcuts");
    assert!(rig.shared.borrow().holds.as_ref().is_some_and(|h| h.on), "the switch turns the setting on");
    assert!(rig.shared.borrow().km.holds(), "and the keymap's holds with it");
    assert!(rig.has("Add a hold shortcut to Save"), "the Hold column shows");

    rig.click("Add a hold shortcut to Save");
    rig.press(Modifiers::NONE, Key::K);
    assert_eq!(holds_of(&rig, "file.save"), ["Hold K"], "a letter recorded in the hold cell is a hold");

    rig.click("Add a hold shortcut to Save");
    rig.press(Modifiers::COMMAND, Key::K);
    assert_eq!(holds_of(&rig, "file.save"), ["Hold K"], "Cmd+K cannot be held, so nothing is written");
    assert!(says(&rig, "can be held"), "the refusal is said under the table");

    let field = rig.find("Hold delay (seconds):");
    rig.fixture.click(&field);
    rig.press(Modifiers::COMMAND, Key::A);
    rig.fixture.type_text("1.5");
    rig.press(Modifiers::NONE, Key::Enter);
    assert_eq!(rig.shared.borrow().holds.as_ref().map(|h| h.delay), Some(1.5), "the delay field edits the setting");

    rig.click("Cancel");
    {
        let shared = rig.shared.borrow();
        assert_eq!(shared.holds, Some(setting(false)), "Cancel puts the setting back, on and delay");
        assert!(!shared.km.holds(), "and the keymap's holds");
        assert!(shared.km.overrides().get("file.save").is_none(), "and drops the hold");
        assert!(shared.outcomes.iter().any(|o| matches!(o, Outcome::Cancel)), "the dialog closed with Cancel");
    }

    let mut rig = Rig::with_holds(fixture::keymap(Platform::Mac), fixture::options(), setting(false));
    rig.click("Hold Shortcuts");
    rig.click("OK");
    assert!(rig.has_outcome(|o| matches!(o, Outcome::Ok)), "the dialog closed with OK");
    assert!(rig.shared.borrow().holds.as_ref().is_some_and(|h| h.on), "OK leaves the edited setting for the app to save");
    assert!(rig.shared.borrow().km.holds(), "and the keymap's holds as the setting says");
}

/// The row cache is keyed on the overrides, which the switch does not change: the rows follow the switch all the
/// same.
#[trial]
fn toggling_holds_rebuilds_the_rows_and_the_keymap_view() {
    let mut km = fixture::keymap(Platform::Mac);
    km.set_holds(true);
    km.assign(&CommandRef::from("file.save"), Binding::parse("Hold K").expect("a hold"), Policy::Keep).expect("assigns");
    let mut rig = Rig::with_holds(km, fixture::options(), setting(true));
    assert!(rig.has("Save: Hold K"), "the hold shows in Save's row");

    rig.click("Hold Shortcuts");
    assert!(!rig.has("Save: Hold K"), "switched off, the row shows no hold");

    rig.click("Hold Shortcuts");
    assert!(rig.has("Save: Hold K"), "switched on again, the hold is back");
}

#[trial]
fn replacing_a_hold_in_the_detail_pane_records_another_hold() {
    let command = CommandRef::from("file.save");
    let mut km = fixture::keymap(Platform::Mac);
    km.set_holds(true);
    km.assign(&command, Binding::parse("Hold K").unwrap(), Policy::Keep).unwrap();
    let mut options = fixture::options();
    options.double_press = true;
    let mut rig = Rig::with_holds(km, options, setting(true));
    rig.shared.borrow_mut().state.select(&command);
    rig.pass();

    rig.click("Replace Hold K for Save");
    {
        let shared = rig.shared.borrow();
        assert_eq!(shared.state.snapshot(&shared.km).unwrap().recording_hold, Some(command));
    }
    rig.press(Modifiers::NONE, Key::J);
    assert_eq!(holds_of(&rig, "file.save"), ["Hold J"]);
}
