//! Theme mechanisms for egui widgets, with palettes and theme selection owned by applications.
//!
//! [`WidgetPalette`] colors egui's widget states. [`color32`] and [`install`] bridge the suite-theme
//! contract in `craft_theme` to egui: an app maps a resolved theme's roles to its own tokens and
//! styles, then installs one style per egui theme. This module stores no theme and keeps nothing in
//! egui's context; [`crate::fonts`] installs fonts. Applications decide their colors, borders,
//! typography, spacing and saved preferences independently.

use egui::{Color32, CornerRadius, Stroke, Style, Theme, ThemePreference, style::Widgets};

/// A suite-theme colour as egui's (premultiplied) colour.
pub fn color32(c: craft_theme::Rgba) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Sets the style egui uses in light and in dark, and lets egui pick between them from the
/// system's answer ([`ThemePreference::System`]).
///
/// An app with a fixed mode passes its chosen variant's style twice; one following the system
/// passes each variant's, and egui switches without a rebuild.
pub fn install(ctx: &egui::Context, light: Style, dark: Style) {
    ctx.set_style_of(Theme::Light, light);
    ctx.set_style_of(Theme::Dark, dark);
    ctx.set_theme(ThemePreference::System);
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
    fn switching_palette_retains_custom_borders_and_expansion() {
        let mut widgets = egui::Visuals::light().widgets;
        widgets.active.bg_stroke = Stroke::new(3.0, Color32::YELLOW);
        widgets.hovered.expansion = 7.0;
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
        assert_eq!(widgets.hovered.expansion, 7.0);
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

    #[test]
    fn bridge_converts_colors_and_installs_both_styles() {
        assert_eq!(color32(craft_theme::Rgba::from_rgb_u32(0x2D81FF)), Color32::from_rgb(0x2D, 0x81, 0xFF));
        assert_eq!(color32(craft_theme::Rgba([255, 0, 0, 128])), Color32::from_rgba_unmultiplied(255, 0, 0, 128));

        let ctx = egui::Context::default();
        let mut light = Style::default();
        light.visuals = egui::Visuals::light();
        light.visuals.panel_fill = Color32::RED;
        let mut dark = Style::default();
        dark.visuals.panel_fill = Color32::BLUE;
        ctx.set_theme(ThemePreference::Light);
        install(&ctx, light, dark);
        assert_eq!(ctx.style_of(Theme::Light).visuals.panel_fill, Color32::RED);
        assert_eq!(ctx.style_of(Theme::Dark).visuals.panel_fill, Color32::BLUE);
        assert_eq!(ctx.options(|o| o.theme_preference), ThemePreference::System);
    }
}
