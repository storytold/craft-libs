//! Stories of `Panel`.

use super::Panel;
use crate::buttons::Button;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::menu::MenuItem;
use crate::primitives::IconSpec;
use crate::sections::sample::body;
use crate::sections::{HeaderAction, HeaderCommand};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use egui::Ui;

/// The header commands of the stories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "panelStory")]
#[non_exhaustive]
enum CardCommands {
    /// Open.
    #[command(label = "Open", icon = "open")]
    Open,
    /// Reset.
    #[command(label = "Reset", icon = "undo")]
    Reset,
}

#[story(Panel<'static>, covers = [Default], drive = Drive::Never)]
fn panel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Panel::new("Properties").show(ui, |ui, _| body(ui)).output.into()
}

#[story(Panel<'static>, covers = [], drive = Drive::Never)]
fn panel_with_actions_and_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Panel::new("Layers")
        .opened(Bind::bound(data.value("open", true)))
        .subtitle("3 layers")
        .icon(IconSpec::svg(IconId::new("settings")))
        .actions([HeaderAction::Menu, HeaderAction::Float, HeaderAction::Close])
        .menu(|ui, _| {
            MenuItem::new("&New Layer").show(ui);
            MenuItem::new("&Delete Layer").show(ui);
        })
        .show(ui, |ui, _| body(ui));
    shown.output.into()
}

#[story(Panel<'static>, covers = [], drive = Drive::Never)]
fn panel_with_footer(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Panel::new("Layers")
        .footer(|ui, _| {
            Button::new("Apply").show(ui);
        })
        .show(ui, |ui, _| body(ui))
        .output
        .into()
}

#[story(Panel<'static>, covers = [], drive = Drive::Never)]
fn panel_headerless(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Panel::new("Layers").headerless().show(ui, |ui, _| body(ui)).output.into()
}

#[story(Panel<'static>, covers = [], drive = Drive::Never)]
fn panel_selected_with_commands(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Panel::new("Button")
        .selected(true)
        .commands([HeaderCommand::new(IconId::new("open"), CardCommands::Open), HeaderCommand::new(IconId::new("undo"), CardCommands::Reset)])
        .show(ui, |ui, _| body(ui))
        .output
        .into()
}
