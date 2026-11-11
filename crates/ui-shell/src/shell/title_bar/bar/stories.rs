//! Stories of `TitleBar`: one per frame mode, and a maximized window.

use super::TitleBar;
use crate::contract::Show as _;
use crate::display::Text;
use crate::shell::Title;
use crate::shell::{Caption, CaptionFamily, Captions, Frame, MacInset};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_window::Layout;
use egui::Ui;

fn captions(family: CaptionFamily, left: &[Caption], right: &[Caption]) -> Frame {
    Frame::Integrated(Captions { family, layout: Layout { left: left.to_vec(), right: right.to_vec() } })
}

fn menu(ui: &mut Ui, _: &mut crate::contract::BodyCx<'_>) {
    Text::new("File  Edit  Image").show(ui);
}

fn tools(ui: &mut Ui, _: &mut crate::contract::BodyCx<'_>) {
    Text::new("Share").show(ui);
}

#[story(TitleBar<'static>, covers = [Default], drive = Drive::Never)]
fn title_bar_windows(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let frame = captions(CaptionFamily::Windows, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    TitleBar::new().frame(frame).show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_windows_maximized(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let frame = captions(CaptionFamily::Windows, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    TitleBar::new().frame(frame).maximized(true).show_titled(ui, Title::unsaved("Landscape.psd"), menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_adwaita(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let frame = captions(CaptionFamily::Adwaita, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    TitleBar::new().frame(frame).show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_inset(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleBar::new().frame(Frame::Inset(MacInset::default())).show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_native(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleBar::new().frame(Frame::Native).title_when_native().show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}
