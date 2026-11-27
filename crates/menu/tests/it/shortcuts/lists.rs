//! The Customize Keyboard layout: each change is applied at once and Escape closes without rolling back.

use craft_keymap::{Binding, CommandDef, CommandRef, Keymap, Platform, Scope, Scopes, Trigger};
use craft_menu::shortcuts::{Layout, Outcome};
use craft_ui::contract::Role;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::{Rig, options};

/// Records `key` with the command modifier in the New Shortcut field.
fn record(rig: &mut Rig, key: Key) {
    rig.click("New shortcut: None");
    rig.press(Modifiers::COMMAND, key);
}

/// The bindings of Bring to Front, compared as bindings rather than as their text.
fn keys(rig: &Rig) -> Vec<Binding> {
    rig.shared.borrow().km.bindings(&CommandRef::from("object.front")).to_vec()
}

#[trial]
fn remove_identifies_the_retained_shortcut_that_blocks_saving() {
    let context = "Text".repeat(16);
    let trigger = Trigger::parse("Cmd+Ctrl+Alt+Shift+IntlBackslash Cmd+Ctrl+Alt+Shift+IntlBackslash").unwrap();
    let retained = Binding::in_scope(trigger, Scope::new(&context).unwrap());
    let command =
        CommandDef::new("text.save", "Save Text", Scope::app()).with_default(retained.clone()).with_default(Binding::parse("Cmd+S").unwrap());
    let keymap = Keymap::new(vec![command], Scopes::new().with_context(&context), Platform::Mac);
    let mut options = options();
    options.layout = Layout::CommandLists;
    let mut rig = Rig::new(keymap, options, None);

    rig.click("Save Text");
    rig.click("⌘S");
    rig.click("Remove");
    assert_eq!(rig.bindings("text.save"), [retained.to_string(), "Cmd+S".into()]);
    assert!([Role::STATUS, Role::ALERT].into_iter().any(|role| !rig.names_with(role, "for Save Text cannot be saved").is_empty()));
}

/// Production applies each change at once and has no Cancel: an Escape that rolled back would silently lose them.
#[trial]
fn command_lists_assign_remove_and_close_apply_each_change_at_once() {
    let mut rig = Rig::tuned(Platform::Mac, |options| options.layout = Layout::CommandLists);
    rig.click("Bring to Front");
    record(&mut rig, Key::E);
    let snapshot = {
        let shared = rig.shared.borrow();
        shared.state.snapshot(&shared.km)
    }
    .expect("the dialog has shown");
    assert_eq!(snapshot.selected, Some(CommandRef::from("object.front")));
    assert_eq!(snapshot.holders, [CommandRef::from("file.export")], "the chord's holder is reported");
    assert!(snapshot.pressed.is_some(), "the field holds the chord");
    assert!(rig.bindings("object.front").is_empty());

    rig.click("Assign");
    assert_eq!(rig.bindings("object.front").len(), 1, "Assign gives the command the chord");
    assert!(rig.bindings("file.export").is_empty(), "the chord is taken from its holder");
    assert!(rig.has_outcome(|o| matches!(o, Outcome::Changed)), "Assign reports Changed");

    record(&mut rig, Key::J);
    rig.click("Assign");
    assert_eq!(rig.bindings("object.front").len(), 2);

    let both = keys(&rig);
    let changes = rig.outcomes().iter().filter(|o| matches!(o, Outcome::Changed)).count();
    rig.click("⌘E");
    rig.click("Remove");
    let left = keys(&rig);
    assert_eq!(left, [both[1].clone()], "Remove takes away the picked key only");
    let after = rig.outcomes().iter().filter(|o| matches!(o, Outcome::Changed)).count();
    assert!(after > changes, "Remove reports Changed");

    rig.press(Modifiers::NONE, Key::Escape);
    assert!(!rig.is_open(), "Escape closes the dialog");
    assert!(rig.has_outcome(|o| matches!(o, Outcome::Ok)), "Escape closes as OK");
    assert!(!rig.has_outcome(|o| matches!(o, Outcome::Cancel)), "there is no Cancel");
    assert_eq!(keys(&rig), left, "the keymap keeps the edits");
    assert!(rig.bindings("file.export").is_empty(), "the taken chord is not given back");
}
