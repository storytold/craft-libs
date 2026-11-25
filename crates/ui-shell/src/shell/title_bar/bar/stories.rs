//! Stories of `TitleBar`: one per frame mode, and a maximized window.

use super::TitleBar;
use crate::buttons::Button;
use crate::contract::{Show as _, ShowContainer as _};
use crate::display::Text;
use crate::layout_components::Row;
use crate::shell::Title;
use crate::shell::{Caption, CaptionFamily, Captions, Frame, MacInset};
use crate::style::SurfaceKind;
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

/// The width of the narrow fitted bar: the menus and Share fit, the ranked end controls do not.
const FITTED_NARROW: f32 = 420.0;

/// End controls that leave a short bar by rank, then one that stays.
fn ranked_tools(ui: &mut Ui, _: &mut crate::contract::BodyCx<'_>) {
    let _ = Row::new().fit_rank(1).show(ui, |ui, _| {
        Button::new("Community").show(ui);
    });
    let _ = Row::new().fit_rank(2).show(ui, |ui, _| {
        Button::new("Learn").show(ui);
    });
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

/// A title that is a button: it reads as text until hovered, and a click or Enter reports it in
/// `TitleBarOutput::title_activated` (a document's rename and location popup).
#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_action_title(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let frame = captions(CaptionFamily::Windows, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    let title = Title::unsaved("Landscape.psd").action("Rename or move the document");
    TitleBar::new().frame(frame).show_titled(ui, title, menu, tools).into()
}

#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_native(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TitleBar::new().frame(Frame::Native).title_when_native().show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}

/// A title bar on the window's background surface instead of the chrome surface (an app whose title bar is darker
/// than its other bars).
#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_on_background(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let frame = captions(CaptionFamily::Windows, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    TitleBar::new().frame(frame).surface(SurfaceKind::Background).show_titled(ui, Title::new("Landscape.psd"), menu, tools).into()
}

/// A bar whose end controls fit after its menus: in a narrow window the menus stay whole and the end controls leave by
/// rank, the lowest first (Community, then Learn), while the unranked Share stays at the end, before the captions.
#[story(TitleBar<'static>, covers = [], drive = Drive::Never)]
fn title_bar_fitted_narrow(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ui.set_max_width(FITTED_NARROW);
    let frame = captions(CaptionFamily::Windows, &[], &[Caption::Minimize, Caption::Maximize, Caption::Close]);
    TitleBar::new().frame(frame).show_fitted(ui, menu, ranked_tools).into()
}
