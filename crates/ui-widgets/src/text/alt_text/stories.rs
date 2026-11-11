//! Stories of `AltTextField`.

use super::{AltText, AltTextField};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::{Key, Modifiers, Ui};

const PROMPT: &str = "How would you describe this object and its context to someone who is blind or has low vision?";
const LONG: &str = "A red kite with a long white tail flying over a grey beach on a windy afternoon, with two children holding its string and a dog running behind them";

/// Enter inserts a line break in the multi-line description, so the typed text commits when focus leaves it.
fn type_and_leave(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("A kite");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(2);
}

#[story(AltTextField<'static>, covers = [Default], drive = Drive::Custom(type_and_leave))]
fn alt_text_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AltTextField::new(PROMPT, Bind::bound(data.value("alt", AltText::new("A red kite")))).show(ui).into()
}

#[story(AltTextField<'static>, covers = [LongText], drive = Drive::Never)]
fn alt_text_field_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AltTextField::new(PROMPT, Bind::bound(data.value("long", AltText::new(LONG)))).show(ui).into()
}

#[story(AltTextField<'static>, covers = [Mixed], drive = Drive::Never)]
fn alt_text_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AltTextField::new(PROMPT, Bind::bound_mixed(data.value("mixed", Mixed::<AltText>::Mixed))).show(ui).into()
}

#[story(AltTextField<'static>, covers = [], drive = Drive::Never)]
fn alt_text_field_mixed_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let parts = AltText::parts(Mixed::Mixed, Mixed::Same(false));
    AltTextField::new(PROMPT, Bind::bound(data.value("mixed text", parts))).show(ui).into()
}

#[story(AltTextField<'static>, covers = [Invalid], drive = Drive::Never)]
fn alt_text_field_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_empty = |alt: &AltText| {
        if alt.decorative == Mixed::Same(true) || alt.text != Mixed::Same(String::new()) {
            Validity::Ok
        } else {
            Validity::Refuse(Message::new("A description is required"))
        }
    };
    AltTextField::new(PROMPT, Bind::bound(data.value("empty", AltText::default())).check(refuse_empty)).show(ui).into()
}
