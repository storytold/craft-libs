//! Stories of `SectionStack`.

use super::SectionStack;
use crate::contract::{Bind, Role, ShowContainer as _};
use crate::sections::Section;
use crate::sections::sample::body;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

const TITLES: [&str; 3] = ["Layers", "Effects", "Paths"];

/// Focuses the first section's disclosure and moves its section down with Alt+Down.
fn move_first_down(fixture: &mut Fixture) {
    let lead = fixture.by_role(Role::BUTTON).into_iter().find(|button| fixture.tree().name_of(button.node) == Some(TITLES[0]));
    if let Some(lead) = lead {
        fixture.click(&lead);
        fixture.key(Key::ArrowDown, Modifiers::ALT);
    }
}

#[story(SectionStack<'static, u8>, covers = [Default], drive = Drive::Custom(move_first_down))]
fn section_stack(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let order = data.value("order", vec![0_u8, 1, 2]).clone();
    let stack = SectionStack::new(&order);
    let rows = stack.rows();
    let shown = stack
        .show(ui, |ui, _| {
            rows.each(ui, |ui, key, slot| Section::new(TITLES[usize::from(*key)]).in_stack(slot).show(ui, |ui, _| body(ui)).output);
        })
        .output;
    let order = data.value("order", vec![0_u8, 1, 2]);
    for edit in &shown.moves {
        if let Some(moved) = edit.value()
            && let Some(from) = order.iter().position(|key| *key == moved.key)
        {
            let key = order.remove(from);
            order.insert(moved.to.min(order.len()), key);
        }
    }
    shown.into()
}

#[story(SectionStack<'static, u8>, covers = [], drive = Drive::Never)]
fn section_stack_closed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let order = [0_u8, 1, 2];
    let stack = SectionStack::new(&order);
    let rows = stack.rows();
    stack
        .show(ui, |ui, _| {
            rows.each(ui, |ui, key, slot| {
                let open = Bind::bound(data.value(TITLES[usize::from(*key)], *key == 0));
                Section::new(TITLES[usize::from(*key)]).open(open).in_stack(slot).show(ui, |ui, _| body(ui)).output
            });
        })
        .output
        .into()
}
