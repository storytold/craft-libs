//! The Keyboard Shortcuts dialog, driven pass by pass through `craft_ui::testing::Fixture`.
mod fixture;
pub(crate) mod hold;
pub(crate) mod import_offer;
pub(crate) mod lists;
pub(crate) mod recorder;
pub(crate) mod review;
pub(crate) mod rows;
pub(crate) mod sets;
pub(crate) mod tabs;

use std::cell::Cell;
use std::rc::Rc;

use craft_keymap::{Binding, CommandDef, CommandRef, Keymap, Platform, Policy, Scope, Scopes, Trigger};
use craft_menu::shortcuts::HoldSetting;
use craft_ui::contract::Role;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};
use serde_json::json;

use fixture::Rig;

#[trial]
fn automation_replaces_a_shortcut_in_its_scope_as_one_undo_step() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("type.bold")).unwrap();
    let original = rig.bindings("type.bold");
    let old = &original[0];

    assert!(rig.set_json_field("replace", json!({"old": old, "new": "not a trigger"})).is_err());
    assert!(rig.set_json_field("replace", json!({"old": "Cmd+B", "new": "Cmd+K"})).is_err());
    assert!(rig.set_json_field("replace", json!({"old": old, "new": "Hold K"})).is_err());
    assert_eq!(rig.bindings("type.bold"), original);

    rig.set_json_field("replace", json!({"old": old, "new": "Cmd+K"})).unwrap();
    assert_eq!(rig.bindings("type.bold"), ["Text: Cmd+K"]);
    rig.click("Undo");
    assert_eq!(rig.bindings("type.bold"), original);
    let shared = rig.shared.borrow();
    assert!(!shared.state.snapshot(&shared.km).unwrap().can_undo);
}

#[trial]
fn an_automation_replacement_waits_for_conflict_confirmation() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).unwrap();
    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
    assert_eq!(rig.bindings("file.export"), ["Cmd+E"]);
    assert!([Role::STATUS, Role::ALERT].into_iter().any(|role| !rig.names_with(role, "for Save (Application) with").is_empty()));

    rig.click("Take It");
    assert_eq!(rig.bindings("file.save"), ["Cmd+E"]);
    assert!(rig.bindings("file.export").is_empty());
    rig.click("Undo");
    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
    assert_eq!(rig.bindings("file.export"), ["Cmd+E"]);
}

#[trial]
fn confirming_an_automation_replacement_preserves_shortcuts_that_changed_after_the_prompt() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).unwrap();
    let third = CommandRef::from("object.front");
    rig.shared.borrow_mut().km.assign(&third, Binding::parse("Cmd+E").unwrap(), Policy::Keep).unwrap();
    rig.pass();
    rig.answer("Take It");

    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
    assert_eq!(rig.bindings("file.export"), ["Cmd+E"]);
    assert_eq!(rig.bindings("object.front"), ["Cmd+E"]);
    let shared = rig.shared.borrow();
    assert!(shared.state.snapshot(&shared.km).unwrap().conflict.is_none());
}

#[trial]
fn confirming_an_automation_replacement_cannot_change_shortcuts_in_a_new_scope() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).unwrap();
    let mut catalog = rig.shared.borrow().km.catalog().to_vec();
    for command in &mut catalog {
        if matches!(command.command.key().as_str(), "file.save" | "file.export") {
            command.scope = Scope::new("Text").unwrap();
        }
    }
    rig.shared.borrow_mut().km = Keymap::new(catalog, Scopes::new().with_context("Text"), Platform::Mac);
    rig.pass();
    rig.answer("Take It");
    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
    assert_eq!(rig.bindings("file.export"), ["Cmd+E"]);
    let shared = rig.shared.borrow();
    assert!(shared.state.snapshot(&shared.km).unwrap().conflict.is_none());
}

#[trial]
fn a_holder_binding_change_retires_its_pending_replacement_confirmation() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).unwrap();
    rig.shared.borrow_mut().km.assign(&CommandRef::from("file.export"), Binding::parse("Cmd+K").unwrap(), Policy::Keep).unwrap();
    rig.pass();
    rig.answer("Take It");

    assert_eq!(rig.bindings("file.save"), ["Cmd+S"], "a stale confirmation cannot replace the original shortcut");
    assert_eq!(rig.bindings("file.export"), ["Cmd+E", "Cmd+K"], "the changed holder keeps all its current shortcuts");
    let shared = rig.shared.borrow();
    assert!(shared.state.snapshot(&shared.km).unwrap().conflict.is_none());
}

#[trial]
fn a_superseding_replacement_has_its_own_confirmation_actions() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).unwrap();
    let old_action = rig.find("Take It").node;
    rig.shared.borrow_mut().options.allow_keep = false;
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+Shift+S"})).unwrap();
    assert_ne!(rig.find("Take It").node, old_action);
    assert!(!rig.has("Keep Both"));
    let dismiss = rig.find("Undo Changes").node;
    rig.shared.borrow_mut().options.allow_keep = true;
    rig.pass();
    assert_ne!(rig.find("Keep Both").node, dismiss);
}

#[trial]
fn escape_dismisses_a_visible_prompt_before_a_hidden_replacement() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+K"})).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+K", "new": "Cmd+E"})).unwrap();
    assert!(rig.has("Take It"));
    rig.click("Reset All…");
    assert!(rig.has("Reset All"));
    assert!(!rig.has("Take It"));

    rig.press(Modifiers::NONE, Key::Escape);
    assert!(!rig.has("Reset All"));
    assert!(rig.has("Take It"));
    assert_eq!(rig.bindings("file.save"), ["Cmd+K"]);
}

#[trial]
fn automation_offers_only_keep_both_when_taking_the_shortcut_cannot_be_saved() {
    let scope = "Text".repeat(16);
    let trigger = Trigger::parse("Cmd+Ctrl+Alt+Shift+IntlBackslash Cmd+Ctrl+Alt+Shift+IntlBackslash").unwrap();
    let retained = Binding::in_scope(trigger, Scope::new(&scope).unwrap());
    let save = CommandDef::new("file.save", "Save", Scope::app()).with_default(Binding::parse("Cmd+S").unwrap());
    let export = CommandDef::new("file.export", "Export", Scope::app()).with_default(retained.clone()).with_default(Binding::parse("Cmd+E").unwrap());
    let keymap = Keymap::new(vec![save, export], Scopes::new().with_context(&scope), Platform::Mac);
    let mut rig = Rig::new(keymap, fixture::options(), None);
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).unwrap();
    assert!(!rig.has("Take It"));
    assert!(!rig.has("Accept and Go to Conflict"));
    rig.click("Keep Both");
    assert_eq!(rig.bindings("file.save"), ["Cmd+E"]);
    assert_eq!(rig.bindings("file.export"), [retained.to_string(), "Cmd+E".into()]);
}

#[trial]
fn automation_replacement_preserves_the_shortcut_when_the_app_rejects_it() {
    let mut rig = Rig::tuned(Platform::Mac, |options| options.accept = Some(Box::new(|_, _| Err("That shortcut is unavailable".into()))));
    rig.set_json_field("command", json!("file.save")).unwrap();
    let result = rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+K"}));
    assert_eq!(result, Err("That shortcut is unavailable".into()));
    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
}

#[trial]
fn confirming_an_automation_replacement_checks_the_current_app_rule() {
    let accepted = Rc::new(Cell::new(true));
    let rule = accepted.clone();
    let mut rig = Rig::tuned(Platform::Mac, |options| {
        options.accept = Some(Box::new(move |_, _| if rule.get() { Ok(()) } else { Err("That shortcut is unavailable".into()) }));
    });
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).unwrap();
    accepted.set(false);
    rig.answer("Take It");
    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
    assert_eq!(rig.bindings("file.export"), ["Cmd+E"]);
    let shared = rig.shared.borrow();
    assert!(shared.state.snapshot(&shared.km).unwrap().conflict.is_none());
}

#[trial]
fn a_pending_automation_conflict_cannot_be_applied_from_a_read_only_dialog() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).unwrap();
    assert!(rig.has("Take It"));

    rig.shared.borrow_mut().options.editable = false;
    rig.pass();
    assert!(!rig.has("Take It"));
    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
    assert_eq!(rig.bindings("file.export"), ["Cmd+E"]);
    rig.press(Modifiers::NONE, Key::Escape);
    assert!(!rig.is_open());
}

#[trial]
fn automation_cannot_replace_a_shortcut_while_reset_confirmation_is_open() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+K"})).unwrap();
    rig.click("Reset All…");
    assert!(rig.set_json_field("replace", json!({"old": "Cmd+K", "new": "Cmd+B"})).is_err());
    assert_eq!(rig.bindings("file.save"), ["Cmd+K"]);
}

#[trial]
fn an_automation_hold_replacement_is_dismissed_when_holds_are_disabled() {
    let mut keymap = fixture::keymap(Platform::Mac);
    keymap.set_holds(true);
    for (command, binding) in [("file.save", "Hold K"), ("file.export", "Hold L")] {
        keymap.assign(&CommandRef::from(command), Binding::parse(binding).unwrap(), Policy::Keep).unwrap();
    }
    let mut rig = Rig::with_holds(keymap, fixture::options(), HoldSetting::new(true, 0.5, 0.2..=2.0));
    rig.set_json_field("command", json!("file.save")).unwrap();
    let original = rig.bindings("file.save");
    assert!(rig.set_json_field("replace", json!({"old": "Hold K", "new": "Cmd+E"})).is_err());
    rig.set_json_field("replace", json!({"old": "Hold K", "new": "Hold L"})).unwrap();
    assert!(rig.has("Take It"));

    rig.click("Hold Shortcuts");
    assert!(!rig.has("Take It"));
    rig.click("Hold Shortcuts");
    assert_eq!(rig.bindings("file.save"), original);
}

#[trial]
fn automation_does_not_replace_a_binding_while_the_recorder_owns_it() {
    let mut rig = Rig::mac();
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.click("Save: ⌘S");
    assert!(rig.shared.borrow().state.is_recording());
    assert!(rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+K"})).is_err());
    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
    rig.press(Modifiers::COMMAND, Key::L);
    assert_eq!(rig.bindings("file.save"), ["Cmd+L"]);
    assert!(!rig.shared.borrow().state.is_recording());
}

#[trial]
fn automation_rejects_oversize_replacements_but_can_replace_an_oversize_default() {
    let scope = "Text".repeat(16);
    let original = Binding::in_scope(Trigger::parse("Cmd+S").unwrap(), Scope::new(&scope).unwrap());
    let trigger = "Cmd+Ctrl+Alt+Shift+IntlBackslash Cmd+Ctrl+Alt+Shift+IntlBackslash";
    let oversize = Binding::in_scope(Trigger::parse(trigger).unwrap(), Scope::new(&scope).unwrap());
    let command = CommandDef::new("text.save", "Save Text", Scope::app()).with_default(original.clone());
    let repair = CommandDef::new("text.fix", "Fix Text", Scope::app()).with_default(oversize.clone()).with_default(Binding::parse("Cmd+K").unwrap());
    let export = CommandDef::new("text.export", "Export Text", Scope::app()).with_default(Binding::parse("Cmd+L").unwrap());
    let keymap = Keymap::new(vec![command, repair, export], Scopes::new().with_context(&scope), Platform::Mac);
    let mut rig = Rig::new(keymap, fixture::options(), None);
    rig.set_json_field("command", json!("text.save")).unwrap();

    assert!(rig.set_json_field("replace", json!({"old": original.to_string(), "new": trigger})).is_err());
    assert_eq!(rig.bindings("text.save"), [original.to_string()]);
    rig.set_json_field("command", json!("text.fix")).unwrap();
    assert!(rig.set_json_field("replace", json!({"old": "Cmd+K", "new": "Cmd+L"})).is_err());
    assert!(!rig.has("Take It"));
    assert_eq!(rig.bindings("text.fix"), [oversize.to_string(), "Cmd+K".into()]);
    let adjusted = format!("{scope}: Cmd+J");
    rig.set_json_field("replace", json!({"old": oversize.to_string(), "new": "Cmd+J"})).unwrap();
    assert_eq!(rig.bindings("text.fix"), [adjusted.clone(), "Cmd+K".into()]);
    rig.set_json_field("replace", json!({"old": adjusted, "new": trigger})).unwrap();
    assert_eq!(rig.bindings("text.fix"), [oversize.to_string(), "Cmd+K".into()]);
    assert!(rig.shared.borrow().km.overrides().is_empty());
}

#[trial]
fn automation_rejects_a_shortcut_that_would_reload_in_a_different_scope() {
    let scope = "Cmd+";
    let original = Binding::in_scope(Trigger::parse("K").unwrap(), Scope::new(scope).unwrap());
    let command = CommandDef::new("text.save", "Save Text", Scope::app()).with_default(original.clone());
    let keymap = Keymap::new(vec![command], Scopes::new().with_context(scope), Platform::Mac);
    let mut rig = Rig::new(keymap, fixture::options(), None);
    rig.set_json_field("command", json!("text.save")).unwrap();
    assert!(rig.set_json_field("replace", json!({"old": original.to_string(), "new": "Cmd+:"})).is_err());
    assert_eq!(rig.bindings("text.save"), [original.to_string()]);
}

#[trial]
fn an_automation_replacement_with_the_same_effective_shortcut_is_a_noop() {
    let mut rig = Rig::tuned(Platform::Windows, |_| {});
    rig.set_json_field("command", json!("file.save")).unwrap();
    rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Ctrl+S"})).unwrap();
    assert_eq!(rig.bindings("file.save"), ["Cmd+S"]);
    let shared = rig.shared.borrow();
    assert!(!shared.state.snapshot(&shared.km).unwrap().can_undo);
}

#[trial]
fn an_automation_replacement_does_not_offer_to_take_a_shortcut_the_command_already_has() {
    let mut keymap = fixture::keymap(Platform::Mac);
    let save = CommandRef::from("file.save");
    keymap.assign(&save, Binding::parse("Cmd+E").unwrap(), Policy::Keep).unwrap();
    let mut rig = Rig::new(keymap, fixture::options(), None);
    rig.set_json_field("command", json!("file.save")).unwrap();
    let original = rig.bindings("file.save");

    assert!(rig.set_json_field("replace", json!({"old": "Cmd+S", "new": "Cmd+E"})).is_err());
    assert!(!rig.has("Take It"));
    assert_eq!(rig.bindings("file.save"), original);
    assert_eq!(rig.bindings("file.export"), ["Cmd+E"]);
}

/// A read-only shortcuts reference does not stretch to the window's height: it opens at a moderate, fixed height and
/// scrolls inside it.
#[trial]
fn a_read_only_dialog_opens_at_a_moderate_height() {
    let rig = Rig::tuned(Platform::Windows, |options| options.editable = false);
    let close = rig.fixture.by_label("Close").expect("the close button");
    assert!(close.rect.bottom() < 560.0, "the dialog's footer sits within a moderate height, not the 700-point window: {:?}", close.rect);
}

/// The editable dialog's bounds, opened with `size` in the rig's 1000 by 700 point window.
fn dialog_bounds(size: Option<egui::Vec2>) -> egui::Rect {
    let rig = Rig::tuned(Platform::Windows, |options| options.size = size);
    rig.fixture.by_role(craft_ui::contract::Role::DIALOG).first().expect("the dialog").rect
}

/// An editable dialog opens at the size the app gives, larger or smaller than its own, and ignores a size that is not
/// finite.
#[trial]
fn an_editable_dialog_opens_at_the_size_the_app_gives() {
    let own = dialog_bounds(None).size();
    for asked in [own + egui::vec2(200.0, 120.0), egui::vec2(own.x + 80.0, own.y - 60.0)] {
        let opened = dialog_bounds(Some(asked)).size();
        assert!((opened - asked).abs().max_elem() < 1.0, "asked {asked:?}, opened {opened:?}, own {own:?}");
    }
    let endless = dialog_bounds(Some(egui::vec2(f32::INFINITY, f32::NAN))).size();
    assert_eq!(endless, own, "a size that is not finite opens the dialog as without one");
}
