//! A preset pattern fill, such as a page background's (DrawingML `a:pattFill`, ECMA-376 Part 1
//! §20.1.8.34, whose presets are `ST_PresetPatternVal` in §20.1.10.55; VML `v:fill
//! type="pattern"`, ECMA-376 Part 4 §6.1.2.5).
//!
//! The geometry is the file formats': an 8 × 8 one-bit tile, repeated. Word writes a page
//! pattern as two colours and a tile picture: `v:background/@fillcolor` inks the tile's set bits,
//! `v:fill/@color2` the rest, and `v:fill/@r:id` names a one-bit picture of the preset, whose
//! `o:title` is the preset's label (ECMA-376 Part 4 §2.2.1; Office's `o:` attributes are
//! [MS-ODRAWXML] §2.2). [`PATTERN_PRESETS`] holds the tiles, so a reader of either format and a
//! writer of VML draw the same pattern; [`PagePattern`] keeps what a document model needs: the
//! preset, both colours and the fill's transparency.
//!
//! Transparency is percent (0 = opaque), as elsewhere in this crate.

use serde::{Deserialize, Serialize};

use crate::props::Rgb;

/// A pattern fill for a page background (Design › Page Color › Fill Effects › Pattern).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PagePattern {
    /// The preset's name from [`PATTERN_PRESETS`] (`pct20`, `ltDnDiag`…), or, from a file whose
    /// pattern this build doesn't know, the label it gave the tile: kept so the fill round-trips.
    pub preset: String,
    /// The pattern's ink: the tile's set bits (VML `v:background/@fillcolor`, DrawingML
    /// `a:pattFill/a:fgClr`).
    pub fore: Rgb,
    /// The paper behind it: the tile's clear bits (VML `v:fill/@color2`,
    /// `a:pattFill/a:bgClr`).
    pub back: Rgb,
    /// How much of the fill shows through, percent (100 = invisible).
    pub transparency: f32,
    /// The file's own pattern tile, a [`crate::Document::media`] key, for a `preset` not in
    /// [`PATTERN_PRESETS`]: saved back unchanged instead of being replaced. Empty when the tile
    /// is one this crate knows (a writer builds it from [`PATTERN_PRESETS`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub media: String,
}

impl Default for PagePattern {
    fn default() -> Self {
        PagePattern { preset: DEFAULT_PRESET.into(), fore: Rgb::BLACK, back: Rgb::WHITE, transparency: 0.0, media: String::new() }
    }
}

/// The preset DrawingML uses when `a:pattFill` names none (`ST_PresetPatternVal`'s default).
pub const DEFAULT_PRESET: &str = "pct5";

/// A pattern preset: its `ST_PresetPatternVal` name, Word's label for it (what VML's `o:title`
/// carries), and its 8 × 8 tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternPreset {
    /// The name the OOXML formats give the preset.
    pub name: &'static str,
    /// The label Word shows the preset by, and writes as the fill's `o:title`.
    pub label: &'static str,
    /// The tile, top row first: bit `x` (`1 << x`) of a row is column `x` from the left; 1 inks
    /// the foreground, 0 the background.
    pub bits: [u8; 8],
}

/// DrawingML's 48 preset patterns in Word's order (ECMA-376 Part 1 §20.1.10.55).
pub const PATTERN_PRESETS: [PatternPreset; 48] = [
    PatternPreset { name: "pct5", label: "5%", bits: [0x11, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00] },
    PatternPreset { name: "pct10", label: "10%", bits: [0x11, 0x00, 0x44, 0x00, 0x11, 0x00, 0x40, 0x00] },
    PatternPreset { name: "pct20", label: "20%", bits: [0x55, 0x00, 0x45, 0x00, 0x55, 0x00, 0x44, 0x00] },
    PatternPreset { name: "pct25", label: "25%", bits: [0x55, 0x00, 0x55, 0x00, 0x55, 0x00, 0x55, 0x00] },
    PatternPreset { name: "pct30", label: "30%", bits: [0x55, 0x22, 0x55, 0x00, 0x55, 0x22, 0x55, 0x00] },
    PatternPreset { name: "pct40", label: "40%", bits: [0x55, 0x2a, 0x55, 0x88, 0x55, 0xa2, 0x55, 0x88] },
    PatternPreset { name: "pct50", label: "50%", bits: [0x55, 0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55, 0xaa] },
    PatternPreset { name: "pct60", label: "60%", bits: [0x77, 0xaa, 0xdd, 0xaa, 0x77, 0xaa, 0xd5, 0xaa] },
    PatternPreset { name: "pct70", label: "70%", bits: [0xff, 0xaa, 0xdf, 0xaa, 0xff, 0xaa, 0xdd, 0xaa] },
    PatternPreset { name: "pct75", label: "75%", bits: [0xff, 0xaa, 0xff, 0xaa, 0xff, 0xaa, 0xff, 0xaa] },
    PatternPreset { name: "pct80", label: "80%", bits: [0xff, 0xbb, 0xff, 0xaa, 0xff, 0xbb, 0xff, 0xaa] },
    PatternPreset { name: "pct90", label: "90%", bits: [0xff, 0xbf, 0xff, 0xee, 0xff, 0xfb, 0xff, 0xee] },
    PatternPreset { name: "ltDnDiag", label: "Light downward diagonal", bits: [0x11, 0x22, 0x44, 0x88, 0x11, 0x22, 0x44, 0x88] },
    PatternPreset { name: "ltUpDiag", label: "Light upward diagonal", bits: [0x11, 0x88, 0x44, 0x22, 0x11, 0x88, 0x44, 0x22] },
    PatternPreset { name: "dkDnDiag", label: "Dark downward diagonal", bits: [0x33, 0x66, 0xcc, 0x99, 0x33, 0x66, 0xcc, 0x99] },
    PatternPreset { name: "dkUpDiag", label: "Dark upward diagonal", bits: [0x33, 0x99, 0xcc, 0x66, 0x33, 0x99, 0xcc, 0x66] },
    PatternPreset { name: "wdDnDiag", label: "Wide downward diagonal", bits: [0x07, 0x0e, 0x1c, 0x38, 0x70, 0xe0, 0xc1, 0x83] },
    PatternPreset { name: "wdUpDiag", label: "Wide upward diagonal", bits: [0x07, 0x83, 0xc1, 0xe0, 0x70, 0x38, 0x1c, 0x0e] },
    PatternPreset { name: "ltVert", label: "Light vertical", bits: [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11] },
    PatternPreset { name: "ltHorz", label: "Light horizontal", bits: [0xff, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00] },
    PatternPreset { name: "narVert", label: "Narrow vertical", bits: [0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55] },
    PatternPreset { name: "narHorz", label: "Narrow horizontal", bits: [0xff, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff, 0x00] },
    PatternPreset { name: "dkVert", label: "Dark vertical", bits: [0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33] },
    PatternPreset { name: "dkHorz", label: "Dark horizontal", bits: [0xff, 0xff, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00] },
    PatternPreset { name: "dashDnDiag", label: "Dashed downward diagonal", bits: [0x11, 0x22, 0x44, 0x88, 0x00, 0x00, 0x00, 0x00] },
    PatternPreset { name: "dashUpDiag", label: "Dashed upward diagonal", bits: [0x11, 0x88, 0x44, 0x22, 0x00, 0x00, 0x00, 0x00] },
    PatternPreset { name: "dashHorz", label: "Dashed horizontal", bits: [0x0f, 0x00, 0x00, 0x00, 0xf0, 0x00, 0x00, 0x00] },
    PatternPreset { name: "dashVert", label: "Dashed vertical", bits: [0x01, 0x01, 0x01, 0x01, 0x10, 0x10, 0x10, 0x10] },
    PatternPreset { name: "smConfetti", label: "Small confetti", bits: [0x01, 0x10, 0x00, 0x84, 0x00, 0x20, 0x02, 0x48] },
    PatternPreset { name: "lgConfetti", label: "Large confetti", bits: [0x03, 0x63, 0x60, 0x00, 0x0c, 0xcc, 0xc0, 0x00] },
    PatternPreset { name: "zigZag", label: "Zigzag", bits: [0x11, 0xaa, 0x44, 0x00, 0x11, 0xaa, 0x44, 0x00] },
    PatternPreset { name: "wave", label: "Wave", bits: [0x83, 0x44, 0x38, 0x00, 0x83, 0x44, 0x38, 0x00] },
    PatternPreset { name: "diagBrick", label: "Diagonal brick", bits: [0x01, 0x82, 0x40, 0x20, 0x10, 0x28, 0x04, 0x02] },
    PatternPreset { name: "horzBrick", label: "Horizontal brick", bits: [0xff, 0x01, 0x01, 0x01, 0xff, 0x10, 0x10, 0x10] },
    PatternPreset { name: "weave", label: "Weave", bits: [0x11, 0x28, 0x44, 0x82, 0x11, 0x82, 0x44, 0x28] },
    PatternPreset { name: "plaid", label: "Plaid", bits: [0x57, 0x57, 0x03, 0x00, 0x03, 0x00, 0x03, 0x00] },
    PatternPreset { name: "divot", label: "Divot", bits: [0x00, 0x0a, 0x04, 0x00, 0x00, 0xa0, 0x40, 0x00] },
    PatternPreset { name: "dotGrid", label: "Dotted grid", bits: [0x55, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00] },
    PatternPreset { name: "dotDmnd", label: "Dotted diamond", bits: [0x01, 0x00, 0x44, 0x00, 0x10, 0x00, 0x44, 0x00] },
    PatternPreset { name: "shingle", label: "Shingle", bits: [0x3f, 0x40, 0x00, 0x00, 0x3f, 0x40, 0x00, 0x00] },
    PatternPreset { name: "trellis", label: "Trellis", bits: [0x11, 0xaa, 0x44, 0xaa, 0x11, 0xaa, 0x44, 0xaa] },
    PatternPreset { name: "sphere", label: "Sphere", bits: [0x18, 0x7e, 0x42, 0xc3, 0xc3, 0x42, 0x7e, 0x18] },
    PatternPreset { name: "smGrid", label: "Small grid", bits: [0xff, 0x11, 0x11, 0x11, 0xff, 0x11, 0x11, 0x11] },
    PatternPreset { name: "lgGrid", label: "Large grid", bits: [0xff, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01] },
    PatternPreset { name: "smCheck", label: "Small checker board", bits: [0x33, 0x33, 0xcc, 0xcc, 0x33, 0x33, 0xcc, 0xcc] },
    PatternPreset { name: "lgCheck", label: "Large checker board", bits: [0x0f, 0x0f, 0x0f, 0x0f, 0xf0, 0xf0, 0xf0, 0xf0] },
    PatternPreset { name: "openDmnd", label: "Outlined diamond", bits: [0x01, 0x82, 0x44, 0x28, 0x10, 0x28, 0x44, 0x82] },
    PatternPreset { name: "solidDmnd", label: "Solid diamond", bits: [0x00, 0x18, 0x3c, 0x7e, 0x7e, 0x3c, 0x18, 0x00] },
];

/// Longest preset name or label kept from a file.
const MAX_PRESET: usize = 64;

impl PagePattern {
    /// Whether `preset` is a name in [`PATTERN_PRESETS`].
    pub fn known(preset: &str) -> bool {
        Self::bits(preset).is_some()
    }

    /// The preset `name` (a [`PATTERN_PRESETS`] name, any case) or label (Word's `o:title`).
    pub fn preset_named(name: &str) -> Option<&'static str> {
        let name = name.trim();
        PATTERN_PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name) || p.label.eq_ignore_ascii_case(name)).map(|p| p.name)
    }

    /// The 8 × 8 tile of preset `name`, or `None` for a name DrawingML lacks.
    pub fn bits(preset: &str) -> Option<[u8; 8]> {
        let name = Self::preset_named(preset)?;
        PATTERN_PRESETS.iter().find(|p| p.name == name).map(|p| p.bits)
    }

    /// Word's label for preset `name` (its `o:title`), or `None` for a name it lacks.
    pub fn label(preset: &str) -> Option<&'static str> {
        let name = Self::preset_named(preset)?;
        PATTERN_PRESETS.iter().find(|p| p.name == name).map(|p| p.label)
    }

    /// Finite and in range: transparency 0–100%, the preset a known name (by name or label) or
    /// the file's own label, shortened.
    pub fn sanitized(&self) -> PagePattern {
        let preset = match Self::preset_named(&self.preset) {
            Some(name) => name.to_string(),
            None => self.preset.chars().take(MAX_PRESET).collect(),
        };
        let preset = if preset.is_empty() && self.media.is_empty() { DEFAULT_PRESET.to_string() } else { preset };
        PagePattern {
            preset,
            fore: self.fore,
            back: self.back,
            transparency: if self.transparency.is_finite() { self.transparency.clamp(0.0, 100.0) } else { 0.0 },
            media: self.media.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_a_label_and_a_tile_that_is_not_empty() {
        assert_eq!(PATTERN_PRESETS.len(), 48);
        for p in PATTERN_PRESETS {
            assert!(!p.label.is_empty(), "{} has no label", p.name);
            assert!(p.bits.iter().any(|&b| b != 0), "{} draws nothing", p.name);
            assert_eq!(PagePattern::bits(p.name), Some(p.bits));
            assert_eq!(PagePattern::preset_named(p.label), Some(p.name), "{} by its label", p.name);
        }
        // The names are unique, case-insensitively.
        for (i, a) in PATTERN_PRESETS.iter().enumerate() {
            assert!(!PATTERN_PRESETS[i + 1..].iter().any(|b| b.name.eq_ignore_ascii_case(a.name)), "{} twice", a.name);
        }
    }

    #[test]
    fn unknown_presets_keep_their_label_and_sanitize() {
        let p = PagePattern { preset: "Hostile \"pattern\"".into(), transparency: f32::NAN, media: "tile.bmp".into(), ..Default::default() };
        let s = p.sanitized();
        assert_eq!((s.preset.as_str(), s.transparency, s.media.as_str()), ("Hostile \"pattern\"", 0.0, "tile.bmp"));
        assert_eq!(PagePattern { preset: "light downward diagonal".into(), ..Default::default() }.sanitized().preset, "ltDnDiag");
        assert_eq!(PagePattern { transparency: 250.0, ..Default::default() }.sanitized().transparency, 100.0);
        assert_eq!(PagePattern { preset: String::new(), ..Default::default() }.sanitized().preset, DEFAULT_PRESET);
        assert_eq!(PagePattern::bits("notAPreset"), None);
    }
}
