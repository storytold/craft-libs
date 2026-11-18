//! Theme mechanisms for egui widgets, with palettes and theme selection owned by applications.
//!
//! This module does not store a theme or change global visuals; [`crate::fonts`] installs fonts.
//! Applications decide their colors, borders, typography, spacing and saved preferences independently.

use egui::{Color32, CornerRadius, Stroke, style::Widgets};

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
}
