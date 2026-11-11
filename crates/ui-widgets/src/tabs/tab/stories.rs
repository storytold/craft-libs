//! Stories of `Tab`.

use super::{Tab, TabKind};
use crate::contract::{Props as _, SetPosition, Show as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use egui::Ui;

#[story(Tab<'static>, covers = [Default], drive = Drive::Activate)]
fn tab(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("Background").position(SetPosition::new(1).of(3)).detail("RGB/8").show(ui).into()
}

#[story(Tab<'static>, covers = [LongText], drive = Drive::Never)]
fn tab_long_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("A title far too long for the tab to show whole").position(SetPosition::new(1).of(3)).detail("50%").closable(true).show(ui).into()
}

#[story(Tab<'static>, covers = [], drive = Drive::Never)]
fn tab_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("Photo").position(SetPosition::new(1).of(3)).selected(true).icon(IconId::new("image")).modified().show(ui).into()
}

#[story(Tab<'static>, covers = [], drive = Drive::Never)]
fn tab_opening(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("Opening").position(SetPosition::new(1).of(3)).progress(0.4).close_label("Cancel opening").show(ui).into()
}

#[story(Tab<'static>, covers = [], drive = Drive::Never)]
fn tab_panel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("Layers").position(SetPosition::new(1).of(3)).kind(TabKind::Panel).selected(true).show(ui).into()
}

#[story(Tab<'static>, covers = [], drive = Drive::Never)]
fn tab_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("General").position(SetPosition::new(1).of(3)).kind(TabKind::Text).selected(true).show(ui).into()
}
