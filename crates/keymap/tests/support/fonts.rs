//! Deterministic app-supplied fonts for dialog tests, including their first layout pass.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use egui::{FontData, FontDefinitions, FontId};
use sha2::{Digest, Sha256};

// Assets from craft-fonts b076417b5880bd4154bd22f00695b1d28c6bea83. Keep the
// input outside this repository; do not fall back to machine-installed fonts.
const INPUTS: [(&str, &str, &str); 2] = [
    ("keymap-symbols", "fonts/noto-sans-symbols/NotoSansSymbols.ttf", "f7e7e04b4a24b6c78893d50cbfd2b2f6cae49617ab047bfef668d252adb128f7"),
    (
        "keymap-symbols-2",
        "fonts/noto-sans-symbols-2/NotoSansSymbols2-Regular.ttf",
        "7d5fb73b7ca67a6798101741f5d280a3d016a56a197afcd4199dbb57b4b82a21",
    ),
];

/// Load the pinned craft-fonts inputs once for all dialog tests.
///
/// Requires an absolute `CRAFT_FONTS_DIR`, verifies the SHA-256 of both symbol faces, refuses any
/// installed-font fallback, and appends the two pinned faces plus egui's Hack to every family.
fn definitions() -> FontDefinitions {
    static FONTS: OnceLock<FontDefinitions> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let root = PathBuf::from(std::env::var_os("CRAFT_FONTS_DIR").unwrap_or_else(|| {
                panic!("UI tests require absolute CRAFT_FONTS_DIR at craft-fonts b076417b5880bd4154bd22f00695b1d28c6bea83; see README.md")
            }));
            assert!(root.is_absolute(), "CRAFT_FONTS_DIR must be absolute: {}", root.display());
            let mut fonts = FontDefinitions::default();
            assert!(fonts.font_data.contains_key("Hack"), "UI tests require egui default_fonts");
            for (name, relative, expected) in INPUTS {
                let path = root.join(relative);
                let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("required font {}: {e}", path.display()));
                assert_eq!(format!("{:x}", Sha256::digest(&bytes)), expected, "unreviewed font input: {}", path.display());
                fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            }
            for family in fonts.families.values_mut() {
                for name in ["keymap-symbols", "keymap-symbols-2", "Hack"] {
                    if !family.iter().any(|existing| existing == name) {
                        family.push(name.into());
                    }
                }
            }
            fonts
        })
        .clone()
}

/// Install the deterministic fonts on `ctx` before its first layout pass.
pub fn configure(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

/// All printable ASCII plus the key symbols, dialog icons and accented pseudolocale
/// repertoire. Direction marks are intentionally invisible, rather than drawn glyphs.
pub fn assert_coverage(ctx: &egui::Context) {
    const SYMBOLS: &str = "×·—“”…‹›←↑→↓↩↺⇥⇧⌃⌘⌥⌦⌫⎋⏎⌨▾⚠✕⟦⟧ÅÉØáëïöüÜ";
    ctx.fonts_mut(|fonts| {
        for font in [FontId::proportional(16.0), FontId::monospace(16.0)] {
            for c in (' '..='~').chain(SYMBOLS.chars()) {
                assert!(fonts.has_glyph(&font, c), "test font {:?} lacks {c} (U+{:04X})", font.family, u32::from(c));
            }
        }
    });
}
