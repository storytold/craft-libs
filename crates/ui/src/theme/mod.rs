//! Theme mechanisms and shared interface role colors for egui frontends.
//!
//! This module does not install fonts, store a theme, or change global visuals. Applications
//! adapt shared role colors to their tokens and retain borders, typography, spacing and saved
//! preferences independently.

use egui::{Color32, CornerRadius, Stroke, style::Widgets};

/// Interface role colors shared by the Craft apps, one constant per scheme.
///
/// `ARTCRAFT_DARK` follows the [ArtCraft dark theme contract](https://github.com/storytold/craft-libs/blob/main/docs/artcraft-dark-theme-contract.md)
/// (14 October 2026). Applications map these roles into local tokens; this data does not apply
/// a theme or select a preference. An app can define its own scheme, or override single roles
/// with `SuitePalette { accent, ..SuitePalette::ARTCRAFT_DARK }`.
/// Alternate themes, semantic colors and document/canvas pixels remain application-owned.
/// `on_accent` is the foreground for blue filled controls; `selection_text` is the foreground
/// for selected interface rows. `primary`/`on_primary` support inverted primary buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuitePalette {
    pub window: Color32,
    pub chrome: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub input: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_secondary: Color32,
    pub text_disabled: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub focus: Color32,
    pub on_accent: Color32,
    pub selection: Color32,
    pub selection_text: Color32,
    pub primary: Color32,
    pub on_primary: Color32,
}

impl SuitePalette {
    /// The ArtCraft standard-dark scheme.
    pub const ARTCRAFT_DARK: Self = Self {
        window: Color32::from_rgb(0x12, 0x13, 0x16),
        chrome: Color32::from_rgb(0x0a, 0x0a, 0x0c),
        panel: Color32::from_rgb(0x10, 0x10, 0x14),
        raised: Color32::from_rgb(0x1e, 0x1f, 0x22),
        input: Color32::from_rgb(0x06, 0x06, 0x07),
        hover: Color32::from_rgb(0x32, 0x33, 0x37),
        pressed: Color32::from_rgb(0x41, 0x42, 0x46),
        border: Color32::from_rgb(0x36, 0x37, 0x3a),
        border_strong: Color32::from_rgb(0x77, 0x78, 0x7c),
        text: Color32::from_rgb(0xf2, 0xf1, 0xee),
        text_strong: Color32::from_rgb(0xff, 0xff, 0xff),
        text_secondary: Color32::from_rgb(0xa6, 0xa6, 0xa8),
        text_disabled: Color32::from_rgb(0x71, 0x71, 0x76),
        accent: Color32::from_rgb(0x2d, 0x81, 0xff),
        accent_hover: Color32::from_rgb(0x59, 0xa7, 0xff),
        focus: Color32::from_rgb(0x74, 0xaa, 0xff),
        on_accent: Color32::from_rgb(0x06, 0x06, 0x07),
        selection: Color32::from_rgb(0x20, 0x3b, 0x65),
        selection_text: Color32::from_rgb(0xf2, 0xf1, 0xee),
        primary: Color32::from_rgb(0xff, 0xff, 0xff),
        on_primary: Color32::from_rgb(0x0b, 0x0b, 0x0c),
    };
}

/// Background colors for egui's five widget interaction states.
///
/// These are rendering inputs, not a theme schema: applications construct them from their own
/// tokens. In particular, `noninteractive` describes egui's noninteractive widget state, not a
/// disabled control palette. egui and the application retain their disabled-state behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidgetPalette {
    pub noninteractive: Color32,
    pub inactive: Color32,
    pub hovered: Color32,
    pub active: Color32,
    pub open: Color32,
}

impl WidgetPalette {
    /// Set each state's strong and weak background fill, with a common radius and foreground.
    ///
    /// Borders, expansion and every other widget property are retained. Apply per-state
    /// exceptions afterward, e.g. transparent weak fill for outline-only inactive buttons.
    /// This deliberately does not choose selection colors, surfaces, shadows or text styles.
    pub fn apply_to(self, widgets: &mut Widgets, radius: CornerRadius, foreground: Stroke) {
        for (widget, fill) in [
            (&mut widgets.noninteractive, self.noninteractive),
            (&mut widgets.inactive, self.inactive),
            (&mut widgets.hovered, self.hovered),
            (&mut widgets.active, self.active),
            (&mut widgets.open, self.open),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.corner_radius = radius;
            widget.fg_stroke = foreground;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artcraft_roles_match_the_dark_theme_contract() {
        let palette = SuitePalette::ARTCRAFT_DARK;
        let roles = [
            ("window", palette.window, [0x12, 0x13, 0x16, 0xff]),
            ("chrome", palette.chrome, [0x0a, 0x0a, 0x0c, 0xff]),
            ("panel", palette.panel, [0x10, 0x10, 0x14, 0xff]),
            ("raised", palette.raised, [0x1e, 0x1f, 0x22, 0xff]),
            ("input", palette.input, [0x06, 0x06, 0x07, 0xff]),
            ("hover", palette.hover, [0x32, 0x33, 0x37, 0xff]),
            ("pressed", palette.pressed, [0x41, 0x42, 0x46, 0xff]),
            ("border", palette.border, [0x36, 0x37, 0x3a, 0xff]),
            ("border_strong", palette.border_strong, [0x77, 0x78, 0x7c, 0xff]),
            ("text", palette.text, [0xf2, 0xf1, 0xee, 0xff]),
            ("text_strong", palette.text_strong, [0xff, 0xff, 0xff, 0xff]),
            ("text_secondary", palette.text_secondary, [0xa6, 0xa6, 0xa8, 0xff]),
            ("text_disabled", palette.text_disabled, [0x71, 0x71, 0x76, 0xff]),
            ("accent", palette.accent, [0x2d, 0x81, 0xff, 0xff]),
            ("accent_hover", palette.accent_hover, [0x59, 0xa7, 0xff, 0xff]),
            ("focus", palette.focus, [0x74, 0xaa, 0xff, 0xff]),
            ("on_accent", palette.on_accent, [0x06, 0x06, 0x07, 0xff]),
            ("selection", palette.selection, [0x20, 0x3b, 0x65, 0xff]),
            ("selection_text", palette.selection_text, [0xf2, 0xf1, 0xee, 0xff]),
            ("primary", palette.primary, [0xff, 0xff, 0xff, 0xff]),
            ("on_primary", palette.on_primary, [0x0b, 0x0b, 0x0c, 0xff]),
        ];
        for (role, color, expected) in roles {
            assert_eq!(color.to_array(), expected, "{role}");
        }
    }

    fn contrast(foreground: Color32, background: Color32) -> f64 {
        fn luminance(color: Color32) -> f64 {
            let [r, g, b, _] = color.to_array();
            let linear = |value: u8| {
                let value = f64::from(value) / 255.0;
                if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
        }
        let foreground = luminance(foreground);
        let background = luminance(background);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    }

    #[test]
    fn artcraft_intended_foregrounds_have_readable_contrast() {
        let palette = SuitePalette::ARTCRAFT_DARK;
        for surface in [palette.window, palette.chrome, palette.panel, palette.raised, palette.input, palette.hover, palette.pressed] {
            assert!(contrast(palette.text, surface) >= 4.5);
            assert!(contrast(palette.text_strong, surface) >= 4.5);
        }
        // Secondary labels are intended for resting/hovered surfaces, not pressed controls.
        for surface in [palette.window, palette.chrome, palette.panel, palette.raised, palette.input, palette.hover] {
            assert!(contrast(palette.text_secondary, surface) >= 4.5);
        }
        assert!(contrast(palette.selection_text, palette.selection) >= 4.5);
        assert!(contrast(palette.on_accent, palette.accent) >= 4.5);
        assert!(contrast(palette.on_accent, palette.accent_hover) >= 4.5);
        assert!(contrast(palette.on_primary, palette.primary) >= 4.5);
        assert!(contrast(palette.focus, palette.raised) >= 3.0);
        assert!(contrast(palette.border_strong, palette.raised) >= 3.0);
        // Disabled text and subtle borders intentionally do not promise body-text contrast.
    }

    #[test]
    fn switching_palette_retains_custom_borders_and_expansion() {
        let mut widgets = egui::Visuals::light().widgets;
        widgets.active.bg_stroke = Stroke::new(3.0, Color32::YELLOW);
        widgets.hovered.expansion = 7.0;
        widgets.noninteractive.expansion = 1.0;
        widgets.inactive.expansion = 2.0;
        widgets.active.expansion = 3.0;
        widgets.open.expansion = 4.0;
        let original_expansions = [
            widgets.noninteractive.expansion,
            widgets.inactive.expansion,
            widgets.hovered.expansion,
            widgets.active.expansion,
            widgets.open.expansion,
        ];
        let original_borders = [
            widgets.noninteractive.bg_stroke,
            widgets.inactive.bg_stroke,
            widgets.hovered.bg_stroke,
            widgets.active.bg_stroke,
            widgets.open.bg_stroke,
        ];
        let palette = WidgetPalette {
            noninteractive: Color32::BLACK,
            inactive: Color32::RED,
            hovered: Color32::GREEN,
            active: Color32::BLUE,
            open: Color32::TRANSPARENT,
        };
        let foreground = Stroke::new(2.0, Color32::WHITE);
        palette.apply_to(&mut widgets, CornerRadius::same(4), foreground);
        assert_eq!(widgets.noninteractive.bg_fill, Color32::BLACK);
        assert_eq!(widgets.inactive.bg_fill, Color32::RED);
        assert_eq!(widgets.hovered.bg_fill, Color32::GREEN);
        assert_eq!(widgets.active.bg_fill, Color32::BLUE);
        assert_eq!(widgets.open.bg_fill, Color32::TRANSPARENT);
        for widget in [&widgets.noninteractive, &widgets.inactive, &widgets.hovered, &widgets.active, &widgets.open] {
            assert_eq!(widget.weak_bg_fill, widget.bg_fill);
            assert_eq!(widget.corner_radius, CornerRadius::same(4));
            assert_eq!(widget.fg_stroke, foreground);
        }
        assert_eq!(
            [
                widgets.noninteractive.expansion,
                widgets.inactive.expansion,
                widgets.hovered.expansion,
                widgets.active.expansion,
                widgets.open.expansion
            ],
            original_expansions
        );
        assert_eq!(
            [
                widgets.noninteractive.bg_stroke,
                widgets.inactive.bg_stroke,
                widgets.hovered.bg_stroke,
                widgets.active.bg_stroke,
                widgets.open.bg_stroke
            ],
            original_borders
        );
        // A second theme application replaces prior fills, including an app's outline override.
        widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        WidgetPalette { inactive: Color32::GRAY, ..palette }.apply_to(&mut widgets, CornerRadius::ZERO, foreground);
        assert_eq!(widgets.inactive.bg_fill, Color32::GRAY);
        assert_eq!(widgets.inactive.weak_bg_fill, Color32::GRAY);
        assert_eq!(widgets.active.bg_stroke, Stroke::new(3.0, Color32::YELLOW));
    }
}
