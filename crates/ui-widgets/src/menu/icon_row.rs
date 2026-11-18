//! `MenuIconRow`: one stop in a menu that holds a row of icons.

use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use egui::{Response, Ui};

/// One icon of a `MenuIconRow`.
#[allow(dead_code)] // P3(pdf-edit-w5): ED-F11
pub struct MenuIcon<'a> {
    icon: IconSpec<'a>,
    label: Label,
    enabled: bool,
}

impl<'a> MenuIcon<'a> {
    /// An icon `icon` named `label`.
    pub fn new(icon: impl Into<IconSpec<'a>>, label: impl Into<Label>) -> Self {
        let _ = (icon, label);
        todo!("ED-F11")
    }

    /// Whether the icon can be picked.
    #[must_use]
    pub fn enabled(self, enabled: bool) -> Self {
        let _ = enabled;
        todo!("ED-F11")
    }
}

/// A menu row of icons that is one stop: Left and Right rove among the icons, Up and Down leave the row to the
/// neighbouring menu rows as from any row. Picking an icon runs it and closes the menu chain by the menu item rule.
#[allow(dead_code)] // P3(pdf-edit-w5): ED-F11
pub struct MenuIconRow<'a> {
    label: Label,
    icons: &'a [MenuIcon<'a>],
}

impl<'a> MenuIconRow<'a> {
    /// A row named `label` of `icons`.
    pub fn new(label: impl Into<Label>, icons: &'a [MenuIcon<'a>]) -> Self {
        let _ = (label, icons);
        todo!("ED-F11")
    }

    /// Shows the row in the menu.
    pub fn show(self, ui: &mut Ui) -> MenuIconRowOutput {
        let _ = ui;
        todo!("ED-F11")
    }
}

/// What a `MenuIconRow` returns.
#[non_exhaustive]
pub struct MenuIconRowOutput {
    /// The row's response.
    pub response: Response,
    /// The index of the icon picked in this pass.
    pub picked: Option<usize>,
}
