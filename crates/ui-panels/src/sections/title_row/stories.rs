//! Stories of `TitleRow`.

use super::{HeaderAction, HeaderStrength, TitleRow};
use crate::contract::{Pad, Show as _, ShowContainer as _, Validity};
use crate::display::{Divider, Text};
use crate::layout_components::Stack;
use crate::primitives::IconSpec;
use crate::runtime::env::Message;
use crate::sections::Panel;
use crate::style::SurfaceKind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed};
use egui::Ui;

#[story(TitleRow<'static>, covers = [], drive = Drive::Never)]
fn title_row(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    // The row as a panel shows it: inset in its header band, with a rule across the width under it.
    let mut output = StoryOutput::default();
    Stack::new().fill_width().show(ui, |ui, _| {
        output = Panel::new("Brush settings")
            .band(SurfaceKind::Panel)
            .pad(Pad::None)
            .actions([HeaderAction::Menu, HeaderAction::Float, HeaderAction::Close])
            .show(ui, |ui, _| {
                Divider::horizontal().show(ui);
                Text::new("drag the header to move the panel").show(ui);
            })
            .output
            .into();
    });
    output
}

#[story(TitleRow<'static>, covers = [Default])]
fn title_row_with_disclosure(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TitleRow::new("Layers").disclosure(data.bind("open", false)).show(ui).edit.into()
}

#[story(TitleRow<'static>, covers = [Mixed], drive = Drive::Never)]
fn title_row_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TitleRow::new("Grid").gate(data.bind_mixed("mixed", Mixed::Mixed)).show(ui).edit.into()
}

#[story(TitleRow<'static>, covers = [Invalid], drive = Drive::Never)]
fn title_row_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_on = |on: &bool| if *on { Validity::Refuse(Message::new("Not available")) } else { Validity::Ok };
    TitleRow::new("Grid").gate(data.bind("gate", true).check(refuse_on)).show(ui).edit.into()
}

#[story(TitleRow<'static>, covers = [], drive = Drive::Never)]
fn title_row_with_icon_and_subtitle(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleRow::new("Layer 3").icon(IconSpec::svg(IconId::new("more"))).subtitle("Pixel layer").show(ui).edit.into()
}

#[story(TitleRow<'static>, covers = [], drive = Drive::Never)]
fn title_row_with_note(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleRow::new("Properties").strength(HeaderStrength::Heading).note("Changes apply to the selection").show(ui).edit.into()
}

#[story(TitleRow<'static>, covers = [], drive = Drive::Never)]
fn title_row_caption(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleRow::new("Preset details").strength(HeaderStrength::Caption).show(ui).edit.into()
}

#[story(TitleRow<'static>, covers = [], drive = Drive::Never)]
fn title_row_subheading(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleRow::new("Ellipse Properties:").strength(HeaderStrength::Subheading).show(ui).edit.into()
}

#[story(TitleRow<'static>, covers = [], drive = Drive::Never)]
fn title_row_quiet(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleRow::new("Ellipse").strength(HeaderStrength::Quiet).show(ui).edit.into()
}

#[story(TitleRow<'static>, covers = [], drive = Drive::Never)]
fn title_row_bar(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    // A floating window's drag bar: no title, the Close action at the end.
    TitleRow::new("").strength(HeaderStrength::Bar).actions([HeaderAction::Close]).show(ui).edit.into()
}
