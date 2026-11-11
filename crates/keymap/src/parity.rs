//! What PhotoCraft's own key handling does that the keymap must match before PhotoCraft drops it
//! (gaps 43, 44, 85): the Return glyph, modifier-only hints, Delete and Backspace as one key,
//! layout position fallback, tools sharing a key and unparseable override texts.
//!
//! "Main shortcut removed, alternates stay" (gap 84) needs nothing new: [`Keymap::remove`] on the
//! first binding writes an override with the others.

use crate::chord::{Chord, Mods, Platform};
use crate::command::CommandRef;
use crate::keymap::{Binding, Keymap};
use crate::layout::Layout;

/// The glyph a Mac shows for the Return key. Apple's menus show ↩ (`Key::Enter` today shows ⏎);
/// which one the suite uses waits for the owner's Mac check (gap 43).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ReturnGlyph {
    /// ↩, as AppKit draws a menu item's Return key equivalent.
    #[default]
    Return,
    /// ⏎.
    Enter,
}

impl ReturnGlyph {
    /// The glyph as text.
    pub fn symbol(self) -> &'static str {
        match self {
            ReturnGlyph::Return => "↩",
            ReturnGlyph::Enter => "⏎",
        }
    }
}

impl Chord {
    /// [`Chord::display`] with `glyph` for Return on macOS; the same as `display` elsewhere.
    // new: gap 43; crate::chord.rs:265 `mac_symbol` maps `Key::Enter` to ⏎.
    pub fn display_with_return(&self, platform: Platform, glyph: ReturnGlyph) -> String {
        self.display_key_with(platform, None, glyph)
    }
}

impl Mods {
    /// The held modifiers alone, for a hint while they are down ("Alt: subtract from selection"):
    /// `⌥⇧` on macOS (Apple's order ⌃⌥⇧⌘), `Alt+Shift` elsewhere (Ctrl, Alt, Shift order as in
    /// [`Chord::display`], without the trailing key). Empty when none is held.
    // pc: crates/ui-egui/src/input/shortcuts.rs:50 `pretty` (`pretty("Shift+Alt")` is "⇧⌥" on
    // macOS, "Shift+Alt" elsewhere; input/shortcuts/tests.rs:225), used by the options bar's
    // selection-mode tooltips (panels/options_bar.rs:272).
    pub fn display(self, platform: Platform) -> String {
        let mut s = self.prefix(platform);
        if !platform.is_mac() {
            s.pop();
        }
        s
    }
}

impl Keymap {
    /// Whether tools of different groups may share a chord ([`Keymap::conflicts`] reports no
    /// conflict between two tools; a press cycles through them in catalog order). Off by default;
    /// PhotoCraft turns it on, as Photoshop allows it.
    // new: gap 85; PhotoCraft's `bindings()` table lets tools share keys.
    pub fn with_shared_tool_keys(mut self, on: bool) -> Keymap {
        self.shared_tool_keys = on;
        self
    }

    /// Whether several tools may share one key. See [`Keymap::with_shared_tool_keys`].
    pub fn shares_tool_keys(&self) -> bool {
        self.shared_tool_keys
    }

    /// The override texts for `command` that don't parse, as the user wrote them, so the dialog
    /// shows them (struck out, with the parse error as tooltip) instead of the default it falls
    /// back to. Empty when every text parses.
    // new: gap 85; PhotoCraft showed the raw text.
    pub fn unparsed_texts(&self, command: &CommandRef) -> Vec<&str> {
        let Some(texts) = self.stored_texts(command) else { return Vec::new() };
        texts.iter().map(String::as_str).filter(|t| Binding::parse(t).is_err()).collect()
    }
}

impl Layout {
    /// Whether the keys at the US `[` and `]` positions stand for the brackets on layouts that
    /// type something else there (German `ü` and `+`, Spanish and Italian `+`): in
    /// [`Layout::candidates`] and in [`Layout::adjust`], which also applies the per-layout symbol
    /// table ([`SYMBOL_TABLE`](crate::layout::SYMBOL_TABLE)). The fallback is off once a real
    /// bracket was typed or a table layout's symbol was seen. No other key falls back: German `+`
    /// is no brush key on a layout with a table row. On by default; with it off, `adjust` still
    /// applies the symbol table.
    // pc: crates/ui-egui/src/input/layout_keys.rs `apply` (the table, the bracket fallback and its
    // two learnt switches); PhotoCraft turned the router's wider position fallback off.
    pub fn with_position_fallback(mut self, on: bool) -> Layout {
        self.position_fallback = on;
        self
    }
}
