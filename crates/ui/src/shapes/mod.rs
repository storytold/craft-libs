//! Shared shape-choice semantics and procedural previews, without document insertion commands.

use crate::buttons::{IconButton, IconButtonStyle};
use egui::{Align2, FontId, Ui, Vec2};

mod paint;
pub use paint::paint_preset;

/// A menu choice, not a document shape model. Connector routing/attachment and text editing stay
/// in each app's engine. Stable keys are suitable for IDs and explicit adapters, not localized text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShapePreset {
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Triangle,
    Diamond,
    Line,
    Arrow,
    Star,
    Heart,
    TextBox,
    StraightConnector,
    StraightArrowConnector,
    ElbowConnector,
    ElbowArrowConnector,
    CurvedConnector,
    CurvedArrowConnector,
}

/// Default presentation order. Filter by actual application capabilities before showing choices;
/// having a preview here never means an application's importer/renderer supports the shape.
pub const PRESETS: &[ShapePreset] = &[
    ShapePreset::Rectangle,
    ShapePreset::RoundedRectangle,
    ShapePreset::Ellipse,
    ShapePreset::Triangle,
    ShapePreset::Diamond,
    ShapePreset::Line,
    ShapePreset::Arrow,
    ShapePreset::Star,
    ShapePreset::Heart,
    ShapePreset::TextBox,
    ShapePreset::StraightConnector,
    ShapePreset::StraightArrowConnector,
    ShapePreset::ElbowConnector,
    ShapePreset::ElbowArrowConnector,
    ShapePreset::CurvedConnector,
    ShapePreset::CurvedArrowConnector,
];

impl ShapePreset {
    /// Stable semantic choice key, independent of labels and any application's command protocol.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Rectangle => "rectangle",
            Self::RoundedRectangle => "roundedRectangle",
            Self::Ellipse => "ellipse",
            Self::Triangle => "triangle",
            Self::Diamond => "diamond",
            Self::Line => "line",
            Self::Arrow => "arrow",
            Self::Star => "star",
            Self::Heart => "heart",
            Self::TextBox => "textBox",
            Self::StraightConnector => "straightConnector",
            Self::StraightArrowConnector => "straightArrowConnector",
            Self::ElbowConnector => "elbowConnector",
            Self::ElbowArrowConnector => "elbowArrowConnector",
            Self::CurvedConnector => "curvedConnector",
            Self::CurvedArrowConnector => "curvedArrowConnector",
        }
    }

    /// English source string for the application's localization catalog. The menu always asks
    /// the caller for a label; it never installs a translation service or uses a label as identity.
    pub const fn source_label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::RoundedRectangle => "Rounded Rectangle",
            Self::Ellipse => "Oval",
            Self::Triangle => "Triangle",
            Self::Diamond => "Diamond",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::Star => "Star",
            Self::Heart => "Heart",
            Self::TextBox => "Text Box",
            Self::StraightConnector => "Straight Connector",
            Self::StraightArrowConnector => "Straight Arrow Connector",
            Self::ElbowConnector => "Elbow Connector",
            Self::ElbowArrowConnector => "Elbow Arrow Connector",
            Self::CurvedConnector => "Curved Connector",
            Self::CurvedArrowConnector => "Curved Arrow Connector",
        }
    }

    pub const fn is_connector(self) -> bool {
        matches!(
            self,
            Self::StraightConnector
                | Self::StraightArrowConnector
                | Self::ElbowConnector
                | Self::ElbowArrowConnector
                | Self::CurvedConnector
                | Self::CurvedArrowConnector
        )
    }

    /// Whether this choice has an arrow at its end. An ordinary arrow remains a different
    /// choice from a connector with an arrow; the application decides its attachment semantics.
    pub const fn has_end_arrow(self) -> bool {
        matches!(self, Self::Arrow | Self::StraightArrowConnector | Self::ElbowArrowConnector | Self::CurvedArrowConnector)
    }
}

/// Show the default shape list, returning the chosen semantic preset without executing commands.
///
/// `supported` hides capabilities absent from the host. `label` supplies localized accessible
/// text; IDs remain stable even for duplicate translations. The caller owns the popup, theme,
/// font, optional canvas/gallery actions and command mapping. Wrap repeated menus in `push_id`.
/// Uses the existing [`IconButton`] for keyboard/pointer/accessibility and disabled-state behavior.
/// Rows clip long labels; widths must be finite, positive and at most 4096 points or no rows show.
pub fn menu(
    ui: &mut Ui,
    width: f32,
    font: FontId,
    style: &IconButtonStyle,
    mut supported: impl FnMut(ShapePreset) -> bool,
    mut label: impl FnMut(ShapePreset) -> String,
) -> Option<ShapePreset> {
    if !width.is_finite() || width <= 0.0 || width > 4096.0 {
        return None;
    }
    let font = FontId::new(if font.size.is_finite() { font.size.clamp(1.0, 128.0) } else { 12.0 }, font.family);
    let height = (font.size + 10.0).max(26.0);
    let mut chosen = None;
    let mut previous_connector = false;
    let mut have_item = false;
    for &preset in PRESETS {
        if !supported(preset) {
            continue;
        }
        if have_item && preset.is_connector() && !previous_connector {
            ui.separator();
        }
        previous_connector = preset.is_connector();
        have_item = true;
        let label = label(preset);
        ui.push_id(preset.key(), |ui| {
            let response = IconButton::new(&label, Vec2::new(width, height), style).show(ui, |ui, rect, tint| {
                let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
                let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 14.0, rect.center().y), Vec2::splat(20.0));
                paint_preset(&painter, icon, preset, egui::Stroke::new(1.4, tint), tint.gamma_multiply(0.15));
                let text_rect = egui::Rect::from_min_max(egui::pos2((rect.left() + 30.0).min(rect.right()), rect.top()), rect.max);
                painter.with_clip_rect(ui.clip_rect().intersect(text_rect)).text(
                    text_rect.left_center(),
                    Align2::LEFT_CENTER,
                    &label,
                    font.clone(),
                    tint,
                );
            });
            if response.clicked() {
                chosen = Some(preset);
            }
        });
    }
    chosen
}

#[cfg(test)]
mod tests;
