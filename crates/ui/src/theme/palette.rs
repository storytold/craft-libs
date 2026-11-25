//! Shared palette loading for the Craft suite.
//!
//! The suite reads one base16-shaped palette from an external generator and maps it into each
//! application's own tokens. Applications keep their built-in themes; a palette only overrides
//! the core chrome colours.
//!
//! Sources, in discovery order:
//! 1. `CRAFT_PALETTE_FILE`
//! 2. `$XDG_CONFIG_HOME/craft/theme.json`
//! 3. `~/.config/craft/theme.json`
//! 4. `~/.cache/wal/colors.json` (pywal)
//!
//! `CRAFT_NO_PALETTE=1` disables the lookup, for tests and scripted runs.

use egui::Color32;
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

/// Environment variable naming an explicit palette file.
pub const PALETTE_ENV: &str = "CRAFT_PALETTE_FILE";
/// Environment variable that disables palette discovery.
pub const NO_PALETTE_ENV: &str = "CRAFT_NO_PALETTE";
/// Canonical file name inside the suite config directory.
pub const THEME_FILE: &str = "theme.json";

/// How a palette file was recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteSource {
    /// The suite's canonical base16 JSON.
    Canonical,
    /// Pywal's `colors.json`.
    Pywal,
    /// Matugen's Material-3 JSON.
    Matugen,
}

/// A base16 palette plus the suite's optional accent override.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub name: String,
    pub dark: bool,
    /// `base[0]` is `base00`; `base[15]` is `base0F`.
    pub base: [Color32; 16],
    /// An explicit accent. `None` uses `base0D`.
    pub accent: Option<Color32>,
}

/// A palette read from disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPalette {
    pub palette: Palette,
    pub source: PaletteSource,
    pub path: PathBuf,
}

impl Palette {
    /// A neutral dark palette, used to fill missing keys.
    pub fn dark() -> Self {
        Self { name: "craft-dark".into(), dark: true, base: dark_base(), accent: None }
    }

    /// A neutral light palette, used to fill missing keys.
    pub fn light() -> Self {
        Self { name: "craft-light".into(), dark: false, base: light_base(), accent: None }
    }

    /// Parse any supported palette document.
    pub fn from_json(value: &Value) -> Option<(Self, PaletteSource)> {
        if value.get("base00").is_some() {
            return Self::from_canonical_json(value).map(|palette| (palette, PaletteSource::Canonical));
        }
        if value.get("special").is_some() && value.get("colors").is_some() {
            return Self::from_pywal_json(value).map(|palette| (palette, PaletteSource::Pywal));
        }
        if value.get("surface").is_some() || value.pointer("/colors/surface").is_some() {
            return Self::from_matugen_json(value).map(|palette| (palette, PaletteSource::Matugen));
        }
        None
    }

    /// Parse the suite's canonical base16 JSON.
    pub fn from_canonical_json(value: &Value) -> Option<Self> {
        let mut palette = if value.get("dark").and_then(Value::as_bool).unwrap_or(true) { Self::dark() } else { Self::light() };
        let mut any = false;
        for (index, slot) in palette.base.iter_mut().enumerate() {
            let upper = format!("base{index:02X}");
            let lower = format!("base{index:02x}");
            if let Some(color) = value.get(upper.as_str()).or_else(|| value.get(lower.as_str())).and_then(Value::as_str).and_then(parse_hex) {
                *slot = color;
                any = true;
            }
        }
        if !any {
            return None;
        }
        palette.name = value.get("name").and_then(Value::as_str).unwrap_or("custom").to_string();
        palette.accent = value.get("accent").and_then(Value::as_str).and_then(parse_hex);
        palette.dark = value.get("dark").and_then(Value::as_bool).unwrap_or_else(|| is_dark_color(palette.base[0]));
        Some(palette)
    }

    /// Parse pywal's `colors.json`.
    pub fn from_pywal_json(value: &Value) -> Option<Self> {
        let colors = value.get("colors")?.as_object()?;
        let special = value.get("special").and_then(Value::as_object);
        let background = special
            .and_then(|special| special.get("background"))
            .and_then(Value::as_str)
            .and_then(parse_hex)
            .or_else(|| colors.get("color0").and_then(Value::as_str).and_then(parse_hex))?;
        let foreground = special
            .and_then(|special| special.get("foreground"))
            .and_then(Value::as_str)
            .and_then(parse_hex)
            .or_else(|| colors.get("color7").and_then(Value::as_str).and_then(parse_hex))?;
        let mut base = dark_base();
        base[0] = background;
        base[1] = mix(background, foreground, 0.08);
        base[2] = mix(background, foreground, 0.14);
        base[3] = mix(background, foreground, 0.28);
        base[4] = mix(background, foreground, 0.45);
        base[5] = foreground;
        base[6] = mix(foreground, Color32::WHITE, 0.35);
        base[7] = Color32::WHITE;
        for (index, slot) in base.iter_mut().enumerate().skip(8) {
            let color_index = match index {
                8 => 1,
                9 => 3,
                10 => 3,
                11 => 2,
                12 => 6,
                13 => 4,
                14 => 5,
                _ => 9,
            };
            let key = format!("color{color_index}");
            if let Some(color) = colors.get(key.as_str()).and_then(Value::as_str).and_then(parse_hex) {
                *slot = color;
            }
        }
        let dark = is_dark_color(background);
        Some(Self { name: "pywal".into(), dark, base, accent: None })
    }

    /// Parse matugen's Material-3 JSON.
    pub fn from_matugen_json(value: &Value) -> Option<Self> {
        let surface = matugen_role(value, "surface").or_else(|| matugen_role(value, "background"))?;
        let mut palette = if is_dark_color(surface) { Self::dark() } else { Self::light() };
        palette.name = "matugen".into();
        palette.dark = is_dark_color(surface);
        palette.base[0] = surface;
        for (index, role) in [
            (1, "surface_container"),
            (2, "surface_container_high"),
            (3, "outline"),
            (4, "on_surface_variant"),
            (5, "on_surface"),
            (6, "on_primary"),
            (7, "on_background"),
            (8, "error"),
            (9, "tertiary"),
            (10, "secondary"),
            (11, "secondary"),
            (12, "tertiary"),
            (13, "primary"),
            (14, "tertiary"),
            (15, "error_container"),
        ] {
            if let Some(color) = matugen_role(value, role) {
                palette.base[index] = color;
            }
        }
        palette.accent = matugen_role(value, "primary");
        Some(palette)
    }

    /// Read a palette from a specific file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_from_path(path: &Path) -> Option<LoadedPalette> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("palette {}: {error}", path.display());
                return None;
            }
        };
        let (palette, source) = match Self::from_json(&value) {
            Some(parsed) => parsed,
            None => {
                log::warn!("palette {}: no recognised base16, pywal or matugen keys", path.display());
                return None;
            }
        };
        Some(LoadedPalette { palette, source, path: path.to_path_buf() })
    }

    /// Discover and read the suite palette.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load() -> Option<LoadedPalette> {
        if disabled() {
            return None;
        }
        Self::load_from_path(&discover_path()?)
    }

    /// Window background.
    pub fn background(&self) -> Color32 {
        self.base[0]
    }

    /// Panels and cards.
    pub fn surface(&self) -> Color32 {
        self.base[1]
    }

    /// Raised or alternate surfaces.
    pub fn surface_alt(&self) -> Color32 {
        self.base[2]
    }

    /// Borders and dividers.
    pub fn outline(&self) -> Color32 {
        self.base[3]
    }

    /// Secondary text.
    pub fn text_muted(&self) -> Color32 {
        self.base[4]
    }

    /// Primary text.
    pub fn text(&self) -> Color32 {
        self.base[5]
    }

    /// Accent, selection and focus.
    pub fn accent(&self) -> Color32 {
        self.accent.unwrap_or(self.base[13])
    }

    /// Error and destructive actions.
    pub fn danger(&self) -> Color32 {
        self.base[8]
    }

    /// Warnings.
    pub fn warning(&self) -> Color32 {
        self.base[9]
    }

    /// Success and confirmations.
    pub fn success(&self) -> Color32 {
        self.base[11]
    }

    /// True for a dark palette.
    pub fn is_dark(&self) -> bool {
        self.dark
    }

    /// Black or white, whichever reads better on the accent.
    pub fn contrast_text(&self) -> Color32 {
        if relative_luminance(self.accent()) > 0.179 { Color32::BLACK } else { Color32::WHITE }
    }
}

/// Watch the palette file for changes without a filesystem notification dependency.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
pub struct PaletteWatcher {
    path: Option<PathBuf>,
    stamp: Option<(std::time::SystemTime, u64)>,
    last_check: f64,
    first: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl PaletteWatcher {
    /// Start watching the discovered palette path. The first `poll` loads immediately.
    pub fn from_env() -> Self {
        Self { path: discover_path(), stamp: None, last_check: f64::NEG_INFINITY, first: true }
    }

    /// Return a new palette when the file changed. Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<LoadedPalette> {
        const INTERVAL: f64 = 0.4;
        if disabled() {
            return None;
        }
        let now = ctx.input(|input| input.time);
        if !self.first && now - self.last_check < INTERVAL {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
            return None;
        }
        self.last_check = now;
        if self.path.is_none() {
            self.path = discover_path();
        }
        let Some(path) = self.path.clone() else {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
            return None;
        };
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                self.path = None;
                self.first = true;
                ctx.request_repaint_after(std::time::Duration::from_millis(400));
                return None;
            }
        };
        let stamp = metadata.modified().ok().map(|modified| (modified, metadata.len()));
        if !self.first && stamp == self.stamp {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
            return None;
        }
        self.stamp = stamp;
        self.first = false;
        let loaded = Palette::load_from_path(&path);
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
        loaded
    }
}

/// Parse `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn parse_hex(value: &str) -> Option<Color32> {
    let hex = value.trim().trim_start_matches('#');
    let component = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    match hex.len() {
        3 => {
            let digit = |index: usize| {
                let value = hex.get(index..index + 1)?;
                u8::from_str_radix(value, 16).ok().map(|nibble| nibble * 17)
            };
            Some(Color32::from_rgb(digit(0)?, digit(1)?, digit(2)?))
        }
        6 => Some(Color32::from_rgb(component(0..2)?, component(2..4)?, component(4..6)?)),
        8 => Some(Color32::from_rgba_unmultiplied(component(0..2)?, component(2..4)?, component(4..6)?, component(6..8)?)),
        _ => None,
    }
}

/// Discover the palette path without reading it.
#[cfg(not(target_arch = "wasm32"))]
pub fn discover_path() -> Option<PathBuf> {
    let explicit = std::env::var_os(PALETTE_ENV).filter(|value| !value.is_empty()).map(PathBuf::from);
    let configured = config_dir().map(|dir| dir.join("craft").join(THEME_FILE));
    let pywal = home_dir().map(|home| home.join(".cache").join("wal").join("colors.json"));
    [explicit, configured, pywal].into_iter().flatten().find(|path| path.is_file())
}

#[cfg(not(target_arch = "wasm32"))]
fn disabled() -> bool {
    std::env::var_os(NO_PALETTE_ENV).is_some_and(|value| matches!(value.to_string_lossy().as_ref(), "1" | "true" | "yes" | "on"))
}

#[cfg(not(target_arch = "wasm32"))]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()).map(PathBuf::from).or_else(|| home_dir().map(|home| home.join(".config")))
}

#[cfg(not(target_arch = "wasm32"))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn matugen_role(value: &Value, role: &str) -> Option<Color32> {
    let entry = value.get(role).or_else(|| value.pointer(&format!("/colors/{role}")))?;
    entry.as_str().and_then(parse_hex).or_else(|| entry.get("color").and_then(Value::as_str).and_then(parse_hex))
}

fn mix(from: Color32, to: Color32, amount: f32) -> Color32 {
    from.lerp_to_gamma(to, amount.clamp(0.0, 1.0))
}

fn is_dark_color(color: Color32) -> bool {
    relative_luminance(color) < 0.5
}

fn relative_luminance(color: Color32) -> f32 {
    fn linear(channel: u8) -> f32 {
        let channel = f32::from(channel) / 255.0;
        if channel <= 0.04045 { channel / 12.92 } else { ((channel + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

fn dark_base() -> [Color32; 16] {
    [
        Color32::from_rgb(0x1c, 0x1c, 0x1c),
        Color32::from_rgb(0x2d, 0x2d, 0x2d),
        Color32::from_rgb(0x3a, 0x3a, 0x3a),
        Color32::from_rgb(0x5c, 0x5c, 0x5c),
        Color32::from_rgb(0x8e, 0x8e, 0x8e),
        Color32::from_rgb(0xe2, 0xe2, 0xe2),
        Color32::from_rgb(0xf0, 0xf0, 0xf0),
        Color32::from_rgb(0xff, 0xff, 0xff),
        Color32::from_rgb(0xe0, 0x4a, 0x4a),
        Color32::from_rgb(0xe3, 0xa8, 0x3c),
        Color32::from_rgb(0xe2, 0xc4, 0x4a),
        Color32::from_rgb(0x58, 0xb0, 0x5c),
        Color32::from_rgb(0x4a, 0xa8, 0xb0),
        Color32::from_rgb(0x01, 0x65, 0xdd),
        Color32::from_rgb(0x9e, 0x64, 0xd2),
        Color32::from_rgb(0xb0, 0x6a, 0x4a),
    ]
}

fn light_base() -> [Color32; 16] {
    [
        Color32::from_rgb(0xfa, 0xfa, 0xfa),
        Color32::from_rgb(0xf0, 0xf0, 0xf0),
        Color32::from_rgb(0xe6, 0xe6, 0xe6),
        Color32::from_rgb(0xb8, 0xb8, 0xb8),
        Color32::from_rgb(0x70, 0x70, 0x70),
        Color32::from_rgb(0x1c, 0x1c, 0x1c),
        Color32::from_rgb(0x11, 0x11, 0x11),
        Color32::from_rgb(0x00, 0x00, 0x00),
        Color32::from_rgb(0xc8, 0x34, 0x34),
        Color32::from_rgb(0xb0, 0x76, 0x1a),
        Color32::from_rgb(0xb0, 0x96, 0x1a),
        Color32::from_rgb(0x2f, 0x86, 0x36),
        Color32::from_rgb(0x1f, 0x7f, 0x86),
        Color32::from_rgb(0x0a, 0x62, 0xd0),
        Color32::from_rgb(0x7a, 0x45, 0xa8),
        Color32::from_rgb(0x8a, 0x4f, 0x36),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_palette_reads_base16() {
        let palette = Palette::from_canonical_json(&json!({
            "name": "test",
            "dark": true,
            "base00": "#101010",
            "base01": "#202020",
            "base05": "#e0e0e0",
            "base0D": "#3366ff",
            "accent": "#ff6600"
        }))
        .unwrap();
        assert_eq!(palette.name, "test");
        assert!(palette.dark);
        assert_eq!(palette.background(), Color32::from_rgb(0x10, 0x10, 0x10));
        assert_eq!(palette.surface(), Color32::from_rgb(0x20, 0x20, 0x20));
        assert_eq!(palette.text(), Color32::from_rgb(0xe0, 0xe0, 0xe0));
        assert_eq!(palette.accent(), Color32::from_rgb(0xff, 0x66, 0x00));
    }

    #[test]
    fn canonical_lowercase_keys_work() {
        let palette = Palette::from_canonical_json(&json!({ "base00": "#000000", "base0d": "#123456" })).unwrap();
        assert_eq!(palette.accent(), Color32::from_rgb(0x12, 0x34, 0x56));
    }

    #[test]
    fn pywal_palette_uses_special_background_and_foreground() {
        let palette = Palette::from_pywal_json(&json!({
            "special": { "background": "#101010", "foreground": "#f0f0f0" },
            "colors": {
                "color0": "#000000",
                "color1": "#ff0000",
                "color2": "#00ff00",
                "color3": "#ffff00",
                "color4": "#0000ff",
                "color5": "#ff00ff",
                "color6": "#00ffff",
                "color7": "#ffffff",
                "color9": "#ff5555"
            }
        }))
        .unwrap();
        assert_eq!(palette.background(), Color32::from_rgb(0x10, 0x10, 0x10));
        assert_eq!(palette.text(), Color32::from_rgb(0xf0, 0xf0, 0xf0));
        assert_eq!(palette.danger(), Color32::from_rgb(0xff, 0x00, 0x00));
        assert!(palette.dark);
    }

    #[test]
    fn matugen_palette_reads_flat_and_nested_roles() {
        let flat = Palette::from_matugen_json(&json!({
            "surface": "#101010",
            "on_surface": "#eeeeee",
            "primary": "#3366ff",
            "error": "#ff0000"
        }))
        .unwrap();
        assert_eq!(flat.background(), Color32::from_rgb(0x10, 0x10, 0x10));
        assert_eq!(flat.text(), Color32::from_rgb(0xee, 0xee, 0xee));
        assert_eq!(flat.accent(), Color32::from_rgb(0x33, 0x66, 0xff));
        assert_eq!(flat.danger(), Color32::from_rgb(0xff, 0x00, 0x00));

        let nested = Palette::from_matugen_json(&json!({
            "colors": {
                "surface": { "color": "#202020" },
                "on_surface": { "color": "#dddddd" },
                "primary": { "color": "#00aa88" }
            }
        }))
        .unwrap();
        assert_eq!(nested.background(), Color32::from_rgb(0x20, 0x20, 0x20));
        assert_eq!(nested.accent(), Color32::from_rgb(0x00, 0xaa, 0x88));
    }

    #[test]
    fn hex_parser_accepts_short_long_and_alpha() {
        assert_eq!(parse_hex("#fff"), Some(Color32::from_rgb(0xff, 0xff, 0xff)));
        assert_eq!(parse_hex("3366ff"), Some(Color32::from_rgb(0x33, 0x66, 0xff)));
        assert_eq!(parse_hex("#3366ff80"), Some(Color32::from_rgba_unmultiplied(0x33, 0x66, 0xff, 0x80)));
        assert_eq!(parse_hex("nope"), None);
    }

    #[test]
    fn unknown_document_is_rejected() {
        assert!(Palette::from_json(&json!({ "name": "nothing" })).is_none());
    }

    #[test]
    fn contrast_text_reads_on_the_accent() {
        let dark = Palette::dark();
        assert_eq!(dark.contrast_text(), Color32::WHITE);
        let mut light = Palette::light();
        light.accent = Some(Color32::WHITE);
        assert_eq!(light.contrast_text(), Color32::BLACK);
    }
}
