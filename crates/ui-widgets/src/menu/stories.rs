//! Stories of the menu rows and the menu.

use super::{Menu, MenuItem};
use crate::buttons::Button;
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Trigger};
use crate::display::Divider;
use crate::layout_components::Stack;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use craft_theme::ControlSize;
use egui::Ui;

#[story(MenuItem<'static>, covers = [Default], drive = Drive::Activate)]
fn menu_item(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    MenuItem::new("Save a Copy").shortcut("Ctrl+Shift+S").show(ui).into()
}

#[story(MenuItem<'static>, covers = [], disabled_text = "The clipboard is empty")]
fn menu_item_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    MenuItem::new("Paste").tooltip(data.tooltip("Paste the clipboard")).show(ui).into()
}

#[story(MenuItem<'static>, covers = [LongText], drive = Drive::Never)]
fn menu_item_long_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Stack::new()
        .show(ui, |ui, _| MenuItem::new("Export the selected layers as separate files in the folder of the document").detail("PNG").show(ui))
        .inner
        .map(StoryOutput::from)
        .unwrap_or_default()
}

#[story(MenuItem<'static>, covers = [], drive = Drive::Never)]
fn menu_item_marks_and_submenu(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Stack::new()
        .show(ui, |ui, _| {
            MenuItem::new("Show Rulers").check(true).show(ui);
            MenuItem::new("Snap to Grid").check(false).show(ui);
            MenuItem::new("Fit in Window").radio_mark(true).show(ui);
            MenuItem::submenu("Open Recent").show(ui)
        })
        .inner
        .map(StoryOutput::from)
        .unwrap_or_default()
}

/// Shows `story` in a row as wide as the room: the opener sits at the start edge and the menu hangs from it, so the
/// menu's place mirrors with the direction.
fn in_row(ui: &mut Ui, mut story: impl FnMut(&mut Ui) -> StoryOutput) -> StoryOutput {
    let shown = Stack::new().fill().show(ui, |ui, _| story(ui));
    match shown.inner {
        Some(output) => output,
        None => shown.output.into(),
    }
}

#[story(Menu<'static>, covers = [Default], drive = Drive::Never)]
fn menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let icons = *data.value("icons", false);
    let icon = move |item: MenuItem<'static>, name: &'static str| if icons { item.icon(IconId::new(name)) } else { item };
    in_row(ui, |ui| {
        let opener = Button::new("File").show(ui);
        let mut ran = false;
        let shown = Menu::new("File").size(size).open(Bind::bound(data.value("open", true))).opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            ran |= icon(MenuItem::new("&New").shortcut("Ctrl+N"), "image").show(ui).ran;
            ran |= icon(MenuItem::new("Open").shortcut("Ctrl+O"), "settings").show(ui).ran;
            MenuItem::submenu("Open Recent").show(ui);
            Divider::horizontal().show(ui);
            ran |= MenuItem::new("Show Rulers").check(true).show(ui).ran;
            ran |= MenuItem::new("Revert").enabled(false).show(ui).ran;
        });
        let mut output = StoryOutput::from(shown.output);
        output.clicked |= ran;
        output
    })
}

#[story(Menu<'static>, covers = [], drive = Drive::Never)]
fn menu_of_values(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    in_row(ui, |ui| {
        let opener = Button::new("Zoom").show(ui);
        let entries = [Entry::new(0u8, "Fit in Window").checked(true), Entry::new(1, "Fill Screen"), Entry::separator(), Entry::new(2, "100%")];
        let picked = Menu::new("Zoom").values().open(Bind::bound(data.value("open", true))).opened_by(&opener, Trigger::Click).pick(ui, &entries);
        let ran = picked.picked.is_some();
        let mut output = StoryOutput::from(picked);
        output.clicked |= ran;
        output
    })
}

/// The Edit menu of a context menu: shortcuts, icons, a disabled row, check and radio rows and submenus two deep.
#[story(Menu<'static>, covers = [], drive = Drive::Never)]
fn context_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let icons = *data.value("icons", true);
    let icon = move |item: MenuItem<'static>, name: &'static str| if icons { item.icon(IconId::new(name)) } else { item };
    in_row(ui, |ui| {
        let target = Button::new("Right-click, or focus and press Shift+F10").show(ui);
        let mut ran = false;
        let shown = Menu::context("Edit", &target).show(ui, |ui, _| {
            ran |= icon(MenuItem::new("Undo").shortcut("Ctrl+Z"), "undo").show(ui).ran;
            ran |= icon(MenuItem::new("Redo").shortcut("Ctrl+Shift+Z").enabled(false), "redo").show(ui).ran;
            Divider::horizontal().show(ui);
            ran |= MenuItem::new("Cut").shortcut("Ctrl+X").show(ui).ran;
            ran |= icon(MenuItem::new("Copy").shortcut("Ctrl+C"), "copy").show(ui).ran;
            ran |= MenuItem::new("Paste").shortcut("Ctrl+V").show(ui).ran;
            ran |= icon(MenuItem::new("Delete").shortcut("Delete"), "trash").show(ui).ran;
            Divider::horizontal().show(ui);
            Menu::submenu_row(ui, "View", |ui, _| {
                ran |= MenuItem::new("Rulers").check(true).shortcut("Ctrl+R").show(ui).ran;
                ran |= MenuItem::new("Grid").check(false).show(ui).ran;
                ran |= MenuItem::new("Snap").check(true).enabled(false).show(ui).ran;
                Divider::horizontal().show(ui);
                Menu::submenu_row(ui, "Zoom", |ui, _| {
                    ran |= MenuItem::new("Fit on Screen").shortcut("Ctrl+0").show(ui).ran;
                    ran |= MenuItem::new("50%").show(ui).ran;
                    ran |= MenuItem::new("100%").shortcut("Ctrl+1").show(ui).ran;
                    ran |= MenuItem::new("200%").show(ui).ran;
                });
            });
            Menu::submenu_row(ui, "Mode", |ui, _| {
                ran |= MenuItem::new("RGB Color").radio_mark(true).show(ui).ran;
                ran |= MenuItem::new("CMYK Color").radio_mark(false).show(ui).ran;
                ran |= MenuItem::new("Grayscale").radio_mark(false).show(ui).ran;
            });
            Menu::submenu_row_enabled(ui, "Empty", false, |ui, _| {
                MenuItem::new("Nothing to run").enabled(false).show(ui);
            });
            Divider::horizontal().show(ui);
            ran |= MenuItem::new("Show Guides (stays open)").check(true).keep_open(true).show(ui).ran;
        });
        let mut output = StoryOutput::from(shown.output);
        output.clicked |= ran;
        output
    })
}
