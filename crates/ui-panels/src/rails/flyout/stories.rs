//! Stories of `FlyoutButton`.

use super::FlyoutButton;
use crate::contract::overlay::HOLD;
use crate::contract::{Props as _, Role, Show as _};
use crate::rails::tool::stories::{BRUSH, tools};
use crate::style::GlyphRole;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

#[story(FlyoutButton<'static, u8>, covers = [Default], drive = Drive::Activate)]
fn flyout_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let tools = tools(&[BRUSH, BRUSH + 1, BRUSH + 2]);
    // The button as a tool rail shows it, holding the current tool.
    FlyoutButton::new(&tools, &BRUSH).glyphs(GlyphRole::ToolRail).selected(true).show(ui).into()
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

/// The flyout with its tear-off bar down its end edge; the readout tells where the last tear-off left the bar.
#[story(FlyoutButton<'static, u8>, covers = [], drive = Drive::Custom(hold_button))]
fn flyout_button_tear_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tools = tools(&[BRUSH, BRUSH + 1, BRUSH + 2]);
    let out = FlyoutButton::new(&tools, &BRUSH).tear_off().show(ui);
    let note = data.note("torn off", "Not torn off".to_owned());
    if let Some(at) = out.torn_off {
        *note = format!("torn off at {:.0}, {:.0}", at.x, at.y);
    }
    let readout = note.clone();
    StoryOutput::from(out).readout(readout)
}
