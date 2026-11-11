//! Stories of `FlyoutButton`.

use super::FlyoutButton;
use crate::contract::overlay::HOLD;
use crate::contract::{Role, Show as _};
use crate::rails::tool::stories::{BRUSH, tools};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

#[story(FlyoutButton<'static, u8>, covers = [Default], drive = Drive::Activate)]
fn flyout_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let tools = tools(&[BRUSH, BRUSH + 1, BRUSH + 2]);
    FlyoutButton::new(&tools, &BRUSH).show(ui).into()
}

/// Holds the press on the button, which opens the flyout.
fn hold_button(fixture: &mut Fixture) {
    if let Some(button) = fixture.by_role(Role::BUTTON).first().copied() {
        fixture.press(&button);
        fixture.advance(HOLD * 2.0);
        fixture.idle(3);
    }
}

#[story(FlyoutButton<'static, u8>, covers = [Open], drive = Drive::Custom(hold_button))]
fn flyout_button_open(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let tools = tools(&[BRUSH, BRUSH + 1, BRUSH + 2]);
    FlyoutButton::new(&tools, &BRUSH).show(ui).into()
}
