//! Graphite: PdfCraft's Light gray and Dark gray palettes, with the launcher's status colours.

use super::hex;
use crate::{CoreRoles, Variant};

pub const ID: &str = "craft.graphite";
pub const NAME: &str = "Graphite";

pub const LIGHT: CoreRoles = CoreRoles {
    background: hex(0xF1F1F3),
    surface: hex(0xFFFFFF),
    raised: hex(0xFFFFFF),
    text: hex(0x222226),
    text_muted: hex(0x5E5E66),
    line: hex(0xDADADE),
    accent: hex(0x1B63E0),
    on_accent: hex(0xFFFFFF),
    selection: hex(0xE6EEFD),
    focus: hex(0x1B63E0),
    good: hex(0x0A7563),
    warn: hex(0x8A5800),
    bad: hex(0xB3261E),
};

pub const DARK: CoreRoles = CoreRoles {
    background: hex(0x19191C),
    surface: hex(0x26262A),
    raised: hex(0x2E2E33),
    text: hex(0xECECEF),
    text_muted: hex(0xAEAEB6),
    line: hex(0x3C3C43),
    accent: hex(0x4B8BF5),
    // White on this accent is 3.33:1.
    on_accent: hex(0x19191C),
    selection: hex(0x233A63),
    focus: hex(0x4B8BF5),
    good: hex(0x3FD6B8),
    warn: hex(0xFFC14D),
    bad: hex(0xFF6259),
};

/// Fills the roles a user theme leaves out.
pub const fn roles(v: Variant) -> CoreRoles {
    match v {
        Variant::Light => LIGHT,
        Variant::Dark => DARK,
    }
}
