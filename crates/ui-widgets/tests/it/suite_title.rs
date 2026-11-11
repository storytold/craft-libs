//! The suite's container-title case: a group with no name of its own is allowed only when it is a field of one
//! control.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::testing::story::{Story, StoryData, StoryOutput};
use craft_ui::testing::suite::title_case;
use craft_ui::testing::trial;
use craft_ui::text::NumberField;
use egui::Ui;

fn group_with_a_field_and_a_button(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    FieldGroup::new("")
        .show(ui, |ui, _| {
            Field::new("Size").show(ui, |ui, _| NumberField::new("Size", Bind::bound(data.value("size", Some(12.0)))).show(ui));
            Button::new("Reset").show(ui);
        })
        .into()
}

#[trial]
fn a_group_with_a_labelled_field_and_an_unlabelled_button_needs_a_name() {
    let failed = title_case(&Story::new("stray-button", group_with_a_field_and_a_button)).expect_err("the button is no part of the field");
    assert!(failed.contains("no name"), "{failed}");
}
