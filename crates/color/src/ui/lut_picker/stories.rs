//! Stories of the LUT picker.

use super::{LutChoice, LutPicker};
use craft_ui::contract::{Bind, Props as _, Role, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

/// Opens the select and takes the row after the first with the keys.
fn pick_second(fixture: &mut Fixture) {
    if let Some(field) = fixture.by_role(Role::COMBO_BOX).first().copied() {
        fixture.click(&field);
    }
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(2);
}

#[story(LutPicker<'static>, covers = [Default], drive = Drive::Custom(pick_second))]
fn lut_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    LutPicker::new(Bind::bound(data.value("lut", LutChoice::Builtin("warm")))).label("3D LUT File").show(ui).into()
}

#[story(LutPicker<'static>, covers = [], drive = Drive::Never)]
fn lut_picker_loaded(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    LutPicker::new(Bind::bound(data.value("lut", LutChoice::loaded("Film", 33)))).label("3D LUT File").show(ui).into()
}

#[story(LutPicker<'static>, covers = [Invalid], drive = Drive::Never)]
fn lut_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_builtin = |choice: &LutChoice| match choice {
        LutChoice::Builtin(_) => Validity::Refuse(Message::new("Load a file")),
        _ => Validity::Ok,
    };
    LutPicker::new(Bind::bound(data.value("lut", LutChoice::Builtin("warm"))).check(refuse_builtin)).label("3D LUT File").show(ui).into()
}
