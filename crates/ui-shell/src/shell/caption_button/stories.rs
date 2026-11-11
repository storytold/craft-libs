//! Stories of `CaptionButton`: one per family and the maximized state.

use super::CaptionButton;
use crate::contract::Show as _;
use crate::shell::{Caption, CaptionFamily};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(CaptionButton<'static>, covers = [Default], drive = Drive::Never)]
fn caption_button_windows(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    CaptionButton::new(Caption::Close, CaptionFamily::Windows).show(ui).into()
}

#[story(CaptionButton<'static>, covers = [], drive = Drive::Never)]
fn caption_button_windows_restore(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    CaptionButton::new(Caption::Maximize, CaptionFamily::Windows).maximized(true).show(ui).into()
}

#[story(CaptionButton<'static>, covers = [], drive = Drive::Never)]
fn caption_button_adwaita(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    CaptionButton::new(Caption::Maximize, CaptionFamily::Adwaita).show(ui).into()
}

#[story(CaptionButton<'static>, covers = [], drive = Drive::Never)]
fn caption_button_breeze(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    CaptionButton::new(Caption::Minimize, CaptionFamily::Breeze).show(ui).into()
}

#[story(CaptionButton<'static>, covers = [], drive = Drive::Never)]
fn caption_button_generic(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    CaptionButton::new(Caption::Close, CaptionFamily::Generic).show(ui).into()
}
