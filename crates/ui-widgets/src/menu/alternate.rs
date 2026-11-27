//! `Alternate`: the command a row runs while modifiers are held as it is chosen.

use crate::runtime::commands::CommandRef;
use craft_keymap::Mods;

/// The command a row runs while `mods` are held as it is chosen (Alt on Merge Visible runs Stamp Visible).
#[derive(Clone, Debug, PartialEq, crate::contract::MemoProps, crate::contract::PlainData)]
#[non_exhaustive]
pub struct Alternate {
    /// The modifiers that select the alternate.
    pub mods: Mods,
    /// The command a pick runs while they are held.
    pub command: CommandRef,
}

impl Alternate {
    /// The alternate that runs `command` while `mods` are held.
    pub fn new(mods: Mods, command: impl Into<CommandRef>) -> Alternate {
        Alternate { mods, command: command.into() }
    }

    /// Whether `held` includes every modifier of the alternate. The one rule the in-window row and the native
    /// dispatch share.
    pub fn selected_by(&self, held: Mods) -> bool {
        let m = self.mods;
        (!m.cmd || held.cmd) && (!m.ctrl || held.ctrl) && (!m.alt || held.alt) && (!m.shift || held.shift)
    }
}
