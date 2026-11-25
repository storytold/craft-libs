//! Shortcuts and checked states from the app's craft-keymap [`Keymap`], and the per-frame
//! enabled and checked update.

use craft_keymap::{CommandKind, CommandRef, Keymap};
use craft_ui::contract::Tracked;

use crate::model::{Builtin, Item, ItemKind, MenuBar};

/// Give each item the shortcut its command has in `keymap`, the user's overrides included, and
/// make the items of toggle commands (kind [`CommandKind::Toggle`]) check items.
///
/// The shortcut is [`Keymap::shown_chord`]: the first single-press binding that runs the command,
/// one in its home scope first, so a chord another command wins is not shown. A command with no
/// such binding loses its shortcut, and an item whose command the keymap doesn't know keeps its
/// own. A toggle's check item keeps its checked state; radio items stay radio items. Call it on
/// the app's bar, before the layout. The bar is written, and its revision moves, only when an item changes.
pub fn apply_keymap(bar: &mut Tracked<MenuBar>, keymap: &Keymap) {
    let wanted = |it: &Item| {
        let bindable = keymap.bindable(&it.command)?;
        let kind = if bindable.kind == CommandKind::Toggle && it.kind == ItemKind::Plain { ItemKind::Check(false) } else { it.kind };
        Some((keymap.shown_chord(&it.command), kind))
    };
    if !bar.items().into_iter().any(|it| wanted(it).is_some_and(|(shortcut, kind)| shortcut != it.shortcut || kind != it.kind)) {
        return;
    }
    bar.edit(|bar| {
        bar.for_each_item_mut(|it| {
            if let Some((shortcut, kind)) = wanted(it) {
                it.shortcut = shortcut;
                it.kind = kind;
            }
        });
    });
}

/// An item's state for the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemState {
    /// Whether the item can be chosen.
    pub enabled: bool,
    /// Checked or selected; `None` leaves the item's [`ItemKind`] as it is.
    pub checked: Option<bool>,
}

impl ItemState {
    /// Set whether the item can be chosen.
    pub fn enabled(mut self, enabled: bool) -> ItemState {
        self.enabled = enabled;
        self
    }

    /// Set the checked state.
    pub fn checked(mut self, checked: Option<bool>) -> ItemState {
        self.checked = checked;
        self
    }
}

impl Default for ItemState {
    /// Enabled, the kind left as it is.
    fn default() -> ItemState {
        ItemState { enabled: true, checked: None }
    }
}

/// Set every item's enabled and checked state from `state`, called once per item of the app's
/// commands. `checked: Some(..)` makes a plain item a check item and sets a radio item's
/// selection. The built-in items and the open documents are left alone. The bar is written, and
/// its revision moves, only when an item changes.
pub fn apply_state(bar: &mut Tracked<MenuBar>, mut state: impl FnMut(&CommandRef) -> ItemState) {
    let skipped = |it: &Item| Builtin::of(&it.command).is_some() || bar.is_document(&it.command);
    // One entry per item, depth first, as `for_each_item_mut` walks them.
    let wanted: Vec<Option<(bool, ItemKind)>> = bar
        .items()
        .into_iter()
        .map(|it| {
            if skipped(it) {
                return None;
            }
            let s = state(&it.command);
            let kind = match (s.checked, it.kind) {
                (None, kind) => kind,
                (Some(on), ItemKind::Radio(_)) => ItemKind::Radio(on),
                (Some(on), ItemKind::Plain | ItemKind::Check(_)) => ItemKind::Check(on),
            };
            Some((s.enabled, kind))
        })
        .collect();
    let differs = bar.items().into_iter().zip(&wanted).any(|(it, want)| want.is_some_and(|(enabled, kind)| enabled != it.enabled || kind != it.kind));
    if !differs {
        return;
    }
    bar.edit(|bar| {
        let mut wanted = wanted.into_iter();
        bar.for_each_item_mut(|it| {
            if let Some((enabled, kind)) = wanted.next().flatten() {
                it.enabled = enabled;
                it.kind = kind;
            }
        });
    });
}
