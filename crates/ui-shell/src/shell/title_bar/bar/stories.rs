//! Stories of `TitleBar`: one per frame mode, and a maximized window, each with the app's menus at its leading end and
//! icon buttons at its trailing end, so the title's squeeze shows.

use super::TitleBar;
use crate::buttons::IconButton;
use crate::contract::{Props as _, Show as _};
use crate::menu::{MenuBar, MenuItem, MenuTitle};
use crate::shell::Title;
use crate::shell::{Caption, CaptionFamily, Captions, Frame, MacInset};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use craft_window::Layout;
use egui::Ui;

fn captions(family: CaptionFamily, left: &[Caption], right: &[Caption]) -> Frame {
    Frame::Integrated(Captions { family, layout: Layout { left: left.to_vec(), right: right.to_vec() } })
}

/// A menu of two commands.
fn items(first: &'static str, second: &'static str) -> impl FnMut(&mut Ui) {
    move |ui| {
        MenuItem::new(first).show(ui);
        MenuItem::new(second).show(ui);
    }
}

/// The app's menus at the bar's leading end.
fn menu(ui: &mut Ui, _: &mut crate::contract::BodyCx<'_>) {
    let titles = vec![
        MenuTitle::new("&File", items("&New\u{2026}", "&Open\u{2026}")),
        MenuTitle::new("&Edit", items("&Undo", "&Redo")),
        MenuTitle::new("&Image", items("Image &Size\u{2026}", "&Canvas Size\u{2026}")),
        MenuTitle::new("&Window", items("&Workspace", "&Layers")),
        MenuTitle::new("&Help", items("&Keyboard Shortcuts", "&About")),
    ];
    MenuBar::new("Main menu", titles).show(ui);
}

/// Icon buttons at the bar's trailing end, before the captions.
fn tools(ui: &mut Ui, _: &mut crate::contract::BodyCx<'_>) {
    IconButton::new(IconId::new("search")).label("Search").show(ui);
    IconButton::new(IconId::new("plus")).label("New").show(ui);
}

#[story(TitleBar<'static>, covers = [Default], drive = Drive::Never)]
fn title_bar_windows(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let frame = captions(CaptionFamily::Windows, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    TitleBar::new().separator().frame(frame).show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_windows_maximized(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let frame = captions(CaptionFamily::Windows, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    TitleBar::new().separator().frame(frame).maximized(true).show_titled(ui, Title::unsaved("Landscape.psd"), menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_adwaita(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let frame = captions(CaptionFamily::Adwaita, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    TitleBar::new().separator().frame(frame).show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_inset(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleBar::new().separator().frame(Frame::Inset(MacInset::default())).show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_native(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleBar::new().separator().frame(Frame::Native).title_when_native().show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}
