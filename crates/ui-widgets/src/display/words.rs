//! Built-in words of the display family.

craft_ui::words! {
    /// The words of the display components.
    pub struct DisplayWords {
        kbd_up: "kbd.up" = "Up";
        kbd_down: "kbd.down" = "Down";
        kbd_left: "kbd.left" = "Left";
        kbd_right: "kbd.right" = "Right";
        kbd_page_up: "kbd.pageUp" = "Page Up";
        kbd_page_down: "kbd.pageDown" = "Page Down";
        kbd_enter: "kbd.enter" = "Enter";
        kbd_escape: "kbd.escape" = "Escape";
        kbd_tab: "kbd.tab" = "Tab";
        kbd_space: "kbd.space" = "Space";
        kbd_backspace: "kbd.backspace" = "Backspace";
        kbd_delete: "kbd.delete" = "Delete";
        kbd_insert: "kbd.insert" = "Insert";
        kbd_home: "kbd.home" = "Home";
        kbd_end: "kbd.end" = "End";
    }
}

impl DisplayWords {
    /// The name of a key that is a word on `os`: Enter, Escape, Tab, Space, Backspace, Delete, Insert, Home, End,
    /// the arrows and the page keys. `None` for a key shown as its character and for a key macOS shows as a
    /// glyph.
    pub fn key_name(&self, key: craft_keymap::Key, os: craft_host::target::Os) -> Option<&str> {
        use craft_keymap::Key;
        let word = match key {
            Key::Up if !os.is_mac() => &self.kbd_up,
            Key::Down if !os.is_mac() => &self.kbd_down,
            Key::Left if !os.is_mac() => &self.kbd_left,
            Key::Right if !os.is_mac() => &self.kbd_right,
            Key::Enter if !os.is_mac() => &self.kbd_enter,
            Key::Escape if !os.is_mac() => &self.kbd_escape,
            Key::Tab if !os.is_mac() => &self.kbd_tab,
            Key::Backspace if !os.is_mac() => &self.kbd_backspace,
            Key::Delete if !os.is_mac() => &self.kbd_delete,
            Key::PageUp => &self.kbd_page_up,
            Key::PageDown => &self.kbd_page_down,
            Key::Space => &self.kbd_space,
            Key::Insert => &self.kbd_insert,
            Key::Home => &self.kbd_home,
            Key::End => &self.kbd_end,
            _ => return None,
        };
        Some(word.as_str())
    }
}
