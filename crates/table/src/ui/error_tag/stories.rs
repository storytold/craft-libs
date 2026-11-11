//! Stories of the error tag.

use super::ErrorTag;
use craft_ui::contract::Show as _;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

static FIXES: [Label; 3] = [Label::Static("Convert to Number"), Label::Static("Help on this Error"), Label::Static("Ignore Error")];

#[story(ErrorTag<'static>, covers = [Default], drive = Drive::Activate)]
fn error_tag(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ErrorTag::new("Number Stored as Text", &FIXES).show(ui).into()
}

fn open_menu(fixture: &mut Fixture) {
    if let Some(tag) = fixture.by_label("Error checking options") {
        fixture.click(&tag);
    }
    fixture.idle(2);
}

#[story(ErrorTag<'static>, covers = [Open], drive = Drive::Custom(open_menu))]
fn error_tag_menu(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ErrorTag::new("Number Stored as Text", &FIXES).show(ui).into()
}
