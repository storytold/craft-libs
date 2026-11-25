//! Stories of `Tab`.

use super::{Tab, TabKind, TabRole};
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

/// A marked tab: the marker dot after the modified mark, a panel whose content has something new.
#[story(Tab<'static>, covers = [], drive = Drive::Never)]
fn tab_marked(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("Comments").position(SetPosition::new(2).of(3)).detail("3").modified().marked(true).show(ui).into()
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

/// A shown ribbon tab: its label in semibold and the accent text colour, with no underline of its own (the strip
/// slides one).
#[story(Tab<'static>, covers = [], drive = Drive::Never)]
fn tab_ribbon(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("Home").position(SetPosition::new(0).of(4)).kind(TabKind::Ribbon).selected(true).show(ui).into()
}

/// A contextual ribbon tab that is not shown: its label stays in the accent text colour.
#[story(Tab<'static>, covers = [], drive = Drive::Never)]
fn tab_ribbon_contextual(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tab::new("Table Design").position(SetPosition::new(3).of(4)).kind(TabKind::Ribbon).role(TabRole::Contextual).show(ui).into()
}
