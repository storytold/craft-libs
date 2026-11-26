//! A disabled implicit Select owner naturally retires its held popup before the pointer is released.

use craft_behavior::EditEvent;
use craft_ui::contract::{Bind, Entry, Props as _, Role, Show as _};
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, Room, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Ui};

fn select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let parent_enabled = *data.value("parent_enabled", true);
    let control_enabled = *data.value("control_enabled", true);
    ui.add_enabled_ui(parent_enabled, |ui| {
        let entries = [
            Entry::new(1u8, "One"),
            Entry::new(2u8, "Two"),
            Entry::new(3u8, "Three"),
            Entry::new(4u8, "Four"),
            Entry::new(5u8, "Five"),
            Entry::new(6u8, "Six"),
            Entry::new(7u8, "Seven"),
            Entry::new(8u8, "Eight"),
            Entry::new(9u8, "Nine"),
            Entry::new(10u8, "Ten"),
            Entry::new(11u8, "Eleven"),
            Entry::new(12u8, "Twelve"),
        ];
        let out = Select::new("Pick", Bind::bound(data.value("value", 1u8)), &entries).enabled(control_enabled).show(ui);
        data.note("terminal_edits", Vec::<&'static str>::new()).extend(out.events.iter().filter_map(|event| match event {
            EditEvent::Commit { .. } => Some("commit"),
            EditEvent::Set { .. } => Some("set"),
            _ => None,
        }));
        out.into()
    })
    .inner
}

fn value(fixture: &mut Fixture) -> u8 {
    *fixture.data().value("value", 1u8)
}

fn terminal_edits(fixture: &mut Fixture) -> usize {
    fixture.data().note("terminal_edits", Vec::<&'static str>::new()).len()
}

fn held_popup_retires(disabled: &'static str) {
    let mut fixture = Fixture::story(&Story::new("disabled_implicit_select", select));
    fixture.room(Room::Window.size()).settle();
    let trigger = *fixture.by_role(Role::COMBO_BOX).first().expect("the Select");
    fixture.click(&trigger).settle();
    assert_eq!(fixture.tree().with_role(Role::LIST_BOX_OPTION).len(), 12, "the twelve options are open before the owner is disabled");
    let option = fixture.by_label("Two").expect("the real second option");
    fixture.press(&option);
    assert!(fixture.ctx().input(|input| input.pointer.primary_down()), "the real option press is still physically held");
    assert_eq!(terminal_edits(&mut fixture), 0, "the held option has not committed or set a value");

    *fixture.data().value(disabled, true) = false;
    fixture.idle(2);
    let current = *fixture.by_role(Role::COMBO_BOX).first().expect("the same disabled Select is still installed");
    assert_eq!(current.node, trigger.node, "disabling does not substitute the owner's identity");
    assert!(fixture.tree().is_disabled_key(current.node), "the real disabled state reached the current owner");
    assert!(fixture.ctx().input(|input| input.pointer.primary_down()), "retirement is checked before any pointer release");
    assert_eq!(
        fixture.tree().with_role(Role::LIST_BOX_OPTION).len(),
        0,
        "a disabled implicit owner retires all popup options after two ordinary idle frames, before release"
    );
    assert!(fixture.tree().with_role(Role::LIST_BOX).is_empty(), "the popup shell closes too");
    assert_eq!(value(&mut fixture), 1, "disable preserves the original bound value");
    assert_eq!(terminal_edits(&mut fixture), 0, "retirement produces no Commit or Set");

    fixture.release(&option).idle(2);
    fixture.key(Key::Enter, Modifiers::NONE).idle(2);
    assert_eq!(value(&mut fixture), 1, "late release and Enter cannot commit the retired option");
    assert_eq!(terminal_edits(&mut fixture), 0, "late inputs emit no terminal edit");
    *fixture.data().value(disabled, true) = true;
    fixture.idle(2);
    assert!(fixture.tree().with_role(Role::LIST_BOX_OPTION).is_empty(), "re-enabling does not resurrect the old popup");
    let recovered = *fixture.by_role(Role::COMBO_BOX).first().expect("the re-enabled owner");
    assert_eq!(recovered.node, trigger.node, "normal recovery keeps the same owner identity");
    fixture.click(&recovered).settle();
    let next = fixture.by_label("Two").expect("a newly opened, real option");
    fixture.click(&next).settle();
    assert_eq!(value(&mut fixture), 2, "a later pick still works");
    assert_eq!(terminal_edits(&mut fixture), 1, "only the new complete pick produces a terminal edit");
}

#[trial]
fn a_parent_disabled_select_retires_its_held_popup_before_release() {
    held_popup_retires("parent_enabled");
}

#[trial]
fn an_explicitly_disabled_select_retires_its_held_popup_before_release() {
    held_popup_retires("control_enabled");
}
