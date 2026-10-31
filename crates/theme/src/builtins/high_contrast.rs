//! High contrast: black on white with a dark blue accent, and white on black with a yellow
//! accent after DesignCraft's high-contrast brightness. Text pairs reach 7:1 (WCAG AAA); the
//! lowest muted pair is dark text_muted on selection at 5.64:1.

use super::hex;
use crate::CoreRoles;

pub const ID: &str = "craft.high-contrast";
pub const NAME: &str = "High contrast";

pub const LIGHT: CoreRoles = CoreRoles {
    background: hex(0xF0F0F0),
    surface: hex(0xFFFFFF),
    raised: hex(0xFFFFFF),
    text: hex(0x000000),
    text_muted: hex(0x3A3A3A),
    line: hex(0x000000),
    accent: hex(0x0040B0),
    on_accent: hex(0xFFFFFF),
    selection: hex(0xC7DBFF),
    focus: hex(0x0040B0),
    good: hex(0x005C33),
    warn: hex(0x6B4400),
    bad: hex(0xA50000),
};

pub const DARK: CoreRoles = CoreRoles {
    background: hex(0x1A1A1A),
    surface: hex(0x000000),
    raised: hex(0x000000),
    text: hex(0xFFFFFF),
    text_muted: hex(0xE6E6E6),
    line: hex(0xFFFFFF),
    accent: hex(0xFFEB3B),
    on_accent: hex(0x000000),
    selection: hex(0x5C5C00),
    focus: hex(0xFFEB3B),
    good: hex(0x6EF2A4),
    warn: hex(0xFFB84D),
    bad: hex(0xFF8F8F),
};
