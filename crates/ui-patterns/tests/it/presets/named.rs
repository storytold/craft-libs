//! `NamedPresetRow`: F2 on the picker renames a user preset and nothing else; New… opens the name prompt.

use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::presets::{NamedPresetEvent, NamedPresetRow, NamedPresetStyle, PresetPicker, PresetRow, PresetState};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

const LINEAR: u32 = 1;
const MINE: u32 = 2;

struct Rig {
    fixture: Fixture,
    events: Rc<RefCell<Vec<NamedPresetEvent<u32>>>>,
}

/// A row in `style` on the preset `current`: Linear is built in, Mine is the user's.
fn rig(style: NamedPresetStyle, current: u32) -> Rig {
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let rows = vec![PresetRow::item(LINEAR, "Linear", 0), PresetRow::item(MINE, "Mine", 0)];
    let mut value = Some(current);
    let mut state = PresetState::<u32>::new();
    let mut fixture = Fixture::new(move |ui| {
        let picker = PresetPicker::new("Contour", Bind::bound(&mut value), &rows, Bind::bound(&mut state));
        let out = NamedPresetRow::new("Contour preset", picker).style(style).suggest_name("Contour 3").built_in(|key| *key == LINEAR).show(ui);
        seen.borrow_mut().extend(out.events);
    });
    fixture.settle();
    Rig { fixture, events }
}

impl Rig {
    /// The name prompt's text, when the prompt is open.
    fn prompt(&self) -> Option<String> {
        let field = self.fixture.by_role(Role::TEXT_INPUT).into_iter().next()?;
        Some(self.fixture.tree().value_of(field.node).unwrap_or_default().to_owned())
    }
}

#[trial]
fn f2_on_the_picker_renames_a_user_preset_and_enter_reports_the_name() {
    let mut rig = rig(NamedPresetStyle::Buttons, MINE);
    assert_eq!(rig.prompt(), None, "the row opens without the prompt");
    rig.fixture.key(Key::Tab, Modifiers::NONE).settle();
    rig.fixture.key(Key::F2, Modifiers::NONE).settle();
    assert_eq!(rig.prompt().as_deref(), Some("Mine"), "F2 opened the prompt with the preset's name");
    rig.fixture.type_text("Ridge").settle();
    rig.fixture.key(Key::Enter, Modifiers::NONE).settle();
    let events = rig.events.borrow();
    assert!(
        events.iter().any(|event| matches!(event, NamedPresetEvent::Rename { key: MINE, text, .. } if text == "Ridge")),
        "Enter renamed the preset: {events:?}"
    );
}

#[trial]
fn f2_on_a_built_in_preset_opens_nothing() {
    let mut rig = rig(NamedPresetStyle::Menu, LINEAR);
    rig.fixture.key(Key::Tab, Modifiers::NONE).settle();
    rig.fixture.key(Key::F2, Modifiers::NONE).settle();
    assert_eq!(rig.prompt(), None, "a built-in preset has no rename");
}

#[trial]
fn the_new_button_opens_the_prompt_with_the_suggestion() {
    let mut rig = rig(NamedPresetStyle::Buttons, LINEAR);
    let new = rig.fixture.by_label("New…").expect("the Buttons style shows New…");
    rig.fixture.click(&new).settle();
    assert_eq!(rig.prompt().as_deref(), Some("Contour 3"), "New… opened the prompt with the suggested name");
}
