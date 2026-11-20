//! `StripEnd`: an item at the reading end of a tab strip, beside the tabs.
//!
//! The strip fits, places and runs the items; this file gives each item its two faces (with its label, and icon only),
//! its measure for the fit, its part's state and its menu.

use crate::buttons::{Button, IconButton};
use crate::contract::{Component, Entry, MeasureCx, PaintUi, Props as _};
use crate::menu::entry_rows;
use crate::primitives::IconSpec;
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use crate::style::StyleCx;
use craft_behavior::PartRef;
use craft_behavior::fit::{FitItem, FitKeep};
use craft_theme::Variant;
use egui::Ui;

/// What an end item does when pressed.
#[derive(Clone)]
enum EndAction<'a> {
    Command(CommandRef),
    Menu(&'a [Entry<CommandRef>]),
}

/// An item at the reading end of a `TabStrip`: a button that runs a command or opens a menu. It drops its label before
/// the tabs overflow.
#[derive(Clone)]
pub struct StripEnd<'a> {
    label: Label,
    icon: IconSpec<'a>,
    action: EndAction<'a>,
    variant: Variant,
}

impl<'a> StripEnd<'a> {
    /// An item that runs `command`, named `label`, with `icon`. It is enabled while the command is.
    pub fn command(label: impl Into<Label>, icon: IconSpec<'a>, command: impl Into<CommandRef>) -> Self {
        StripEnd { label: label.into(), icon, action: EndAction::Command(command.into()), variant: Variant::GHOST }
    }

    /// An item that opens a menu of `entries`, named `label`, with `icon`.
    pub fn menu(label: impl Into<Label>, icon: IconSpec<'a>, entries: &'a [Entry<CommandRef>]) -> Self {
        StripEnd { label: label.into(), icon, action: EndAction::Menu(entries), variant: Variant::GHOST }
    }

    /// Sets the button variant of the item.
    #[must_use]
    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }

    /// The item's name: the label its full face shows, and its accessible name in either form.
    pub(super) fn label(&self) -> &Label {
        &self.label
    }

    /// The command the item runs when pressed; `None` for a menu item, which opens its menu instead.
    pub(super) fn command_ref(&self) -> Option<&CommandRef> {
        match &self.action {
            EndAction::Command(command) => Some(command),
            EndAction::Menu(_) => None,
        }
    }

    /// Whether the item opens a menu.
    pub(super) fn opens_menu(&self) -> bool {
        matches!(self.action, EndAction::Menu(_))
    }

    /// Whether the item can be pressed: a command item while its command is enabled, a menu item always.
    pub(super) fn enabled<C: Component>(&self, cx: &MeasureCx<'_, C>) -> bool {
        self.command_ref().is_none_or(|command| cx.resolve_command(command.clone()).is_none_or(|resolved| resolved.enabled))
    }

    /// The face with the label beside the icon; a menu item's ends with the chevron of a button that opens a menu.
    fn full_face(&self) -> Button<'a> {
        let face = Button::new(self.label.clone()).icon(self.icon.clone()).variant(self.variant);
        match self.command_ref() {
            Some(command) => face.command(command.clone()).label(self.label.clone()),
            None => face.trailing_icon(crate::controls::disclosure::DisclosureGlyph::menu_button()),
        }
    }

    /// The face with the icon only; the label stays its name and tooltip.
    fn short_face(&self) -> IconButton<'a> {
        let face = IconButton::new(self.icon.clone()).variant(self.variant);
        match self.command_ref() {
            Some(command) => face.command(command.clone()).label(self.label.clone()),
            None => face.label(self.label.clone()),
        }
    }

    /// The item as the fit sees it: its full width, and its icon-only width as the short form. It never leaves the
    /// strip, since the tabs overflow before the end items give up their room.
    pub(super) fn fit_item(&self, cx: &StyleCx<'_>) -> FitItem {
        let full = cx.face(self.full_face()).x;
        let short = cx.face(self.short_face()).x;
        FitItem::new(full, 0, FitKeep::Pinned).short(short)
    }

    /// Paints the item's face on `part`, icon only when the fit shortened it.
    pub(super) fn paint(&self, ui: &mut PaintUi<'_>, part: PartRef, short: bool) {
        if short {
            ui.face(part, self.short_face());
        } else {
            ui.face(part, self.full_face());
        }
    }

    /// The rows of a menu item's menu, shown as every menu shows its entries (`entry_rows`), and the command of the
    /// row that ran this pass.
    pub(super) fn menu_rows(&self, ui: &mut Ui) -> Option<CommandRef> {
        let EndAction::Menu(entries) = &self.action else { return None };
        let ran = entry_rows(ui, entries)?;
        entries.get(ran)?.value().cloned()
    }
}
