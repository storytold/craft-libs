//! Shortcuts and checked states from the app's craft-keymap [`Keymap`], and the per-frame
//! enabled and checked update.

use craft_keymap::{CommandRef, Keymap};

use crate::model::{Builtin, ItemKind, MenuBar};

/// Give each item the shortcut its command has in `keymap`, the user's overrides included, and
/// make the items of toggle commands ([`craft_keymap::Bindable::is_toggle`]) check items.
///
/// The shortcut is the first single-press binding, one in the command's home scope first; a
/// double press can't be a menu shortcut. A command the keymap binds to nothing loses its
/// shortcut, and an item whose command the keymap doesn't know keeps its own. A toggle's check
/// item keeps its checked state; radio items stay radio items. Call it on the app's bar, before
/// the layout.
pub fn apply_keymap(bar: &mut MenuBar, keymap: &Keymap) {
    bar.for_each_item_mut(|it| {
        let Some(bindable) = keymap.bindable(&it.command) else { return };
        let bindings = keymap.bindings(&it.command);
        let single = || bindings.iter().filter(|b| !b.trigger.double);
        let home = single().find(|b| *b.scope_or(&bindable.scope) == bindable.scope);
        it.shortcut = home.or_else(|| single().next()).map(|b| b.trigger.chord);
        if bindable.is_toggle && it.kind == ItemKind::Plain {
            it.kind = ItemKind::Check(false);
        }
    });
}

/// An item's state for the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemState {
    pub enabled: bool,
    /// Checked or selected; `None` leaves the item's [`ItemKind`] as it is.
    pub checked: Option<bool>,
}

impl Default for ItemState {
    /// Enabled, the kind left as it is.
    fn default() -> ItemState {
        ItemState { enabled: true, checked: None }
    }
}

/// Set every item's enabled and checked state from `state`, called once per item of the app's
/// commands. `checked: Some(..)` makes a plain item a check item and sets a radio item's
/// selection. The built-in items and the open documents are left alone.
pub fn apply_state(bar: &mut MenuBar, mut state: impl FnMut(&CommandRef) -> ItemState) {
    let documents: Vec<CommandRef> = bar.documents.iter().map(|d| d.command.clone()).collect();
    bar.for_each_item_mut(|it| {
        if Builtin::of(&it.command).is_some() || documents.contains(&it.command) {
            return;
        }
        let s = state(&it.command);
        it.enabled = s.enabled;
        if let Some(on) = s.checked {
            it.kind = match it.kind {
                ItemKind::Radio(_) => ItemKind::Radio(on),
                ItemKind::Plain | ItemKind::Check(_) => ItemKind::Check(on),
            };
        }
    });
}
