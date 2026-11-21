//! Stories of the page setup dialog and the layout menus.

use super::{PageLayoutMenus, PageSetupDialog};
use crate::setup::PageSetup;
use crate::setup::{DocumentGrid, GridKind};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Bind, Props as _, Show as _, clicked};
use craft_ui::dialog::HeldAnswer;
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(PageSetupDialog<'static>, covers = [], drive = Drive::Never)]
fn page_setup(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (setup, setup_at) = data.value_at("setup", PageSetup::default());
    PageSetupDialog::new(setup_at.bind(setup)).layout().direction().opened(Bind::given(true)).show(ui);
    StoryOutput::default()
}

#[story(PageSetupDialog<'static>, covers = [], drive = Drive::Never)]
fn document_setup(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = PageSetup { pages: Some(8), mirrored: true, ..PageSetup::default() };
    let (setup, setup_at) = data.value_at("setup", start);
    PageSetupDialog::new(setup_at.bind(setup)).document_setup().unit(UnitKind::Millimetres).opened(Bind::given(true)).show(ui);
    StoryOutput::default()
}

/// A setup with a line and character grid.
fn gridded() -> PageSetup {
    PageSetup { grid: Some(DocumentGrid::new(GridKind::LinesAndChars, 40, 30)), ..PageSetup::default() }
}

/// WordCraft's Page Setup: a Document Grid with the section direction and the vertical alignment in its box, then
/// Headers and footers; the margins, paper sides and grid counts two to a line.
#[story(PageSetupDialog<'static>, covers = [], drive = Drive::Never)]
fn page_setup_word(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = PageSetup { grid: Some(DocumentGrid::new(GridKind::None, 36, 39)), ..PageSetup::default() };
    let (setup, setup_at) = data.value_at("setup", start);
    PageSetupDialog::new(setup_at.bind(setup)).layout().direction().opened(Bind::given(true)).show(ui);
    StoryOutput::default()
}

/// The dialog over a setup with a Document grid, opened by its button; the kind radios switch which counts apply.
#[story(PageSetupDialog<'static>, covers = [], drive = Drive::Never)]
fn page_setup_grid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut open = *data.value("open", false);
    if clicked(&Button::new("Page Setup\u{2026}").show(ui)) {
        open = true;
    }
    let (setup, setup_at) = data.value_at("setup", gridded());
    PageSetupDialog::new(setup_at.bind(setup)).opened(Bind::bound(&mut open)).show(ui);
    *data.value("open", false) = open;
    StoryOutput::default()
}

/// The host refuses every OK, so the dialog stays open with the reason under its fields.
#[story(PageSetupDialog<'static>, covers = [], drive = Drive::Never)]
fn page_setup_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut open = *data.value("open", false);
    if clicked(&Button::new("Page Setup\u{2026}").show(ui)) {
        open = true;
    }
    let state = data.note("refused", (gridded(), HeldAnswer::<PageSetup>::new()));
    let (setup, held) = (&mut state.0, &mut state.1);
    PageSetupDialog::new(Bind::bound(setup)).opened(Bind::bound(&mut open)).held(&mut *held).show(ui);
    if held.take().is_some() {
        held.answer(Err(Label::from("This document cannot take a grid.".to_owned())));
    }
    *data.value("open", false) = open;
    StoryOutput::default()
}

#[story(PageLayoutMenus<'static>, covers = [Open], drive = Drive::Never)]
fn margins_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let setup = data.value("setup", PageSetup::default()).clone();
    let opener = IconButton::new(craft_ui::controls::disclosure::DisclosureGlyph::menu_button()).label("Margins").show(ui);
    PageLayoutMenus::new(&setup).opened(Bind::given(true)).margins(ui, &opener);
    StoryOutput::default()
}
