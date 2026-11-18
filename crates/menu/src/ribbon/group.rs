//! `RibbonGroup`: a titled group of controls in the ribbon's row.

use std::hash::Hash;
use std::marker::PhantomData;

use craft_keymap::CommandRef;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;
use egui::Ui;

use super::RibbonRow;

/// A group of the ribbon: a title, an icon for its collapsed form, and an optional launcher. Each
/// group sets its `GroupLevel` for its content; groups are access nodes (role group, named by
/// their title), and a collapsed group opens a popover.
pub struct RibbonGroup<'a> {
    stub: PhantomData<&'a ()>,
}

impl<'a> RibbonGroup<'a> {
    /// A group identified by `key`, titled `title`, with `icon` on its collapsed button.
    pub fn new(key: impl Hash, title: impl Into<Label>, icon: IconSpec<'a>) -> Self {
        let _ = (key, title.into(), icon);
        todo!("rb.R12")
    }

    /// The command of the group's dialog launcher.
    pub fn launcher(self, command: impl Into<CommandRef>) -> Self {
        let _ = (self, command.into());
        todo!("rb.R12")
    }
}

impl RibbonRow<'_> {
    /// Adds `group` to the row, fitting it by production's rule; `add` fills its content.
    pub fn group(&mut self, group: RibbonGroup<'_>, add: impl FnMut(&mut Ui)) -> &mut Self {
        let _ = (group, add);
        todo!("rb.R12")
    }
}
