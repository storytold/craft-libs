//! The dialog's colors and the icon-button style built from them.

use craft_ui::buttons::{ButtonVisuals, IconButtonStyle};
use craft_ui::layout::ResizeHandleStyle;
use egui::{Color32, Stroke, Visuals};

/// The dialog's colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogStyle {
    pub accent: Color32,
    pub warning: Color32,
    pub dim: Color32,
    pub changed: Color32,
    pub app_key: Color32,
    pub scoped_key: Color32,
    pub both_key: Color32,
    pub modifier_key: Color32,
    pub free_key: Color32,
}

impl DialogStyle {
    pub fn from_visuals(v: &Visuals) -> DialogStyle {
        let accent = v.selection.bg_fill;
        DialogStyle {
            accent,
            warning: v.warn_fg_color,
            dim: v.weak_text_color(),
            changed: accent,
            app_key: accent,
            scoped_key: v.warn_fg_color,
            both_key: v.error_fg_color,
            modifier_key: v.widgets.active.bg_fill,
            free_key: v.widgets.inactive.bg_fill,
        }
    }
}

/// Icon buttons (+, ×, ↺, ⌨, ✕) in the colors of the app's visuals.
pub(crate) fn icon_buttons(v: &Visuals) -> IconButtonStyle {
    let w = &v.widgets;
    IconButtonStyle {
        normal: ButtonVisuals { fill: Color32::TRANSPARENT, stroke: Stroke::NONE, icon: w.inactive.fg_stroke.color },
        hovered: ButtonVisuals { fill: w.hovered.weak_bg_fill, stroke: w.hovered.bg_stroke, icon: w.hovered.fg_stroke.color },
        pressed: Some(ButtonVisuals { fill: w.active.weak_bg_fill, stroke: w.active.bg_stroke, icon: w.active.fg_stroke.color }),
        selected: ButtonVisuals { fill: v.selection.bg_fill, stroke: Stroke::NONE, icon: v.selection.stroke.color },
        corner_radius: w.inactive.corner_radius,
        focus_stroke: v.selection.stroke,
    }
}

/// The table/detail divider.
pub(crate) fn divider(v: &Visuals) -> ResizeHandleStyle {
    ResizeHandleStyle { hovered: v.widgets.hovered.bg_fill, dragged: v.widgets.active.bg_fill, focused: v.selection.bg_fill }
}
