//! Stories of the type bar.

use super::TypeBar;
use crate::ui::character::{CharacterValues, ParagraphValues, TextAlign};
use crate::ui::type_options::{TextAntiAlias, TextOrientation};
use craft_theme::Rgba;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn families() -> Vec<String> {
    ["Hack", "Ubuntu", "Noto Serif"].map(str::to_owned).to_vec()
}

fn styles() -> Vec<String> {
    ["Regular", "Bold", "Italic"].map(str::to_owned).to_vec()
}

fn single() -> (CharacterValues, ParagraphValues) {
    let character = CharacterValues {
        family: Some("Ubuntu".to_owned()),
        style: Some("Bold".to_owned()),
        size: Some(24.0),
        color: Some(Rgba([200, 60, 40, 255])),
        ..CharacterValues::default()
    };
    let paragraph = ParagraphValues { align: Some(TextAlign::Center), ..ParagraphValues::default() };
    (character, paragraph)
}

/// No text is targeted: the tool's defaults.
#[story(TypeBar<'static>, covers = [], drive = Drive::Never)]
fn idle(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (character, paragraph) = (CharacterValues::default(), ParagraphValues::default());
    let (families, styles) = (families(), styles());
    TypeBar::new(&character, &paragraph, &families, &styles)
        .orientation(TextOrientation::Horizontal)
        .anti_alias(TextAntiAlias::Smooth)
        .show(ui)
        .into()
}

/// The bar in the options bar of a text edit, with commit and cancel.
#[story(TypeBar<'static>, covers = [], drive = Drive::Never)]
fn editing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (character, paragraph) = single();
    let (families, styles) = (families(), styles());
    let mut shown = None;
    Bar::new("Type", BarKind::Options).commit_cancel("Commit any current edits", "Cancel any current edits").show(ui, |ui, _| {
        shown = Some(
            TypeBar::new(&character, &paragraph, &families, &styles)
                .orientation(TextOrientation::Horizontal)
                .anti_alias(TextAntiAlias::Crisp)
                .can_warp(true)
                .show(ui),
        );
    });
    shown.map(Into::into).unwrap_or_default()
}

/// Vertical text: the alignment buttons are Top, Middle and Bottom.
#[story(TypeBar<'static>, covers = [], drive = Drive::Never)]
fn vertical(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (character, paragraph) = single();
    let (families, styles) = (families(), styles());
    TypeBar::new(&character, &paragraph, &families, &styles).orientation(TextOrientation::Vertical).anti_alias(TextAntiAlias::Sharp).show(ui).into()
}

/// A selection that mixes orientation, anti-aliasing and character values: both props stay unset.
#[story(TypeBar<'static>, covers = [], drive = Drive::Never)]
fn mixed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let character = CharacterValues { size: Some(12.0), ..CharacterValues::default() };
    let paragraph = ParagraphValues::default();
    let (families, styles) = (families(), styles());
    TypeBar::new(&character, &paragraph, &families, &styles).show(ui).into()
}
