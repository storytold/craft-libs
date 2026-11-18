//! `StripEnd`: an item at the reading end of a tab strip, beside the tabs.

use crate::contract::Entry;
use crate::primitives::IconSpec;
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use craft_theme::Variant;

/// What an end item does when pressed.
#[allow(dead_code)] // P3(ribbon-w1): rb.R3
enum EndAction<'a> {
    Command(CommandRef),
    Menu(&'a [Entry<CommandRef>]),
}

/// An item at the reading end of a `TabStrip`: a button that runs a command or opens a menu. It drops its label before
/// the tabs overflow.
#[allow(dead_code)] // P3(ribbon-w1): rb.R3
pub struct StripEnd<'a> {
    label: Label,
    icon: IconSpec<'a>,
    action: EndAction<'a>,
    variant: Variant,
}

impl<'a> StripEnd<'a> {
    /// An item that runs `command`, named `label`, with `icon`.
    pub fn command(label: impl Into<Label>, icon: IconSpec<'a>, command: impl Into<CommandRef>) -> Self {
        let _ = (label.into(), icon, command.into());
        todo!("rb.R3")
    }

    /// An item that opens a menu of `entries`, named `label`, with `icon`.
    pub fn menu(label: impl Into<Label>, icon: IconSpec<'a>, entries: &'a [Entry<CommandRef>]) -> Self {
        let _ = (label.into(), icon, entries);
        todo!("rb.R3")
    }

    /// Sets the button variant of the item.
    #[must_use]
    pub fn variant(self, variant: Variant) -> Self {
        let _ = variant;
        todo!("rb.R3")
    }
}
