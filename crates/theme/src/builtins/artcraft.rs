//! ArtCraft, the suite's default theme: PhotoCraft's Pro Light and Pro Medium palettes
//! (`crates/ui-egui/src/theme.rs` at dacecbfea1).

use super::hex;
use crate::CoreRoles;

pub const ID: &str = crate::SUITE_DEFAULT;
pub const NAME: &str = "ArtCraft";

pub const LIGHT: CoreRoles = CoreRoles {
    background: hex(0xE2E2E2),
    surface: hex(0xF0F0F0),
    raised: hex(0xFFFFFF),
    text: hex(0x141414),
    text_muted: hex(0x404040),
    line: hex(0xD6D6D6),
    accent: hex(0x1062C4),
    on_accent: hex(0xFFFFFF),
    selection: hex(0xCCCCCC),
    focus: hex(0x1062C4),
    good: hex(0x4E712E),
    warn: hex(0x99780C),
    bad: hex(0xC42B1C),
};

pub const DARK: CoreRoles = CoreRoles {
    background: hex(0x424242),
    surface: hex(0x535353),
    raised: hex(0x454545),
    text: hex(0xEEEEEE),
    text_muted: hex(0xD4D4D4),
    line: hex(0x3E3E3E),
    accent: hex(0x78ADF6),
    on_accent: hex(0x141414),
    selection: hex(0x6B6B6B),
    focus: hex(0x78ADF6),
    good: hex(0x85DC6A),
    warn: hex(0xE8B046),
    bad: hex(0xFF8080),
};
