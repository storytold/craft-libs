//! `contract/keys`: raw key reads and key events outside `runtime::egui`.

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/keys";

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    cx.flag_methods(out, ID, ID, "reads raw keys; key events arrive through the input hook as craft-keymap keys", |name, _| {
        matches!(name, "key_pressed" | "key_down" | "key_released" | "consume_key" | "consume_shortcut" | "any_key_pressed")
    });
    cx.flag_paths(out, ID, ID, "matches a raw egui key, text or IME event; use the input hook's events", |p| {
        p.windows(2).any(|w| w[0] == "Event" && matches!(w[1].as_str(), "Key" | "Text" | "Ime"))
    });
}
