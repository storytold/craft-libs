pub mod app_catalog;
//! # craft-guide
//!
//! Interactive onboarding, step-by-step feature walkthroughs, spotlight overlays,
//! hover tooltips, and smart contextual help for all ArtCraft suite applications.
//!
//! Designed for zero-overhead integration across all 15 suite applications:
//! `WordCraft`, `PdfCraft`, `PhotoCraft`, `CADCraft`, `VectorCraft`, `FilmCraft`,
//! `TextureCraft`, `MaterialCraft`, `LightCraft`, `AniCraft`, `DesignCraft`,
//! `FigCraft`, `DeckCraft`, `GridCraft`, `SoundCraft`, and `Craft Launcher`.

use egui::{Color32, Pos2, Rect, Response, Vec2};
use std::collections::HashMap;

/// A single step in an interactive walkthrough tour.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Step {
    /// Unique identifier of the UI element/target to highlight (e.g. "toolbar.table.insert").
    pub target_id: String,
    /// Localized title of this step.
    pub title: String,
    /// Plain-language, friendly instruction explaining what to do from scratch.
    pub description: String,
    /// Optional tip or keyboard shortcut reminder.
    pub tip: Option<String>,
}

/// An interactive guide / walkthrough consisting of multiple sequential steps.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tour {
    /// Unique key of the tour (e.g. "wordcraft.create_book_cover", "pdfcraft.sign_document").
    pub id: String,
    /// Title of the tour shown in the Help / FAQ panel.
    pub title: String,
    /// Brief description of the workflow outcome.
    pub summary: String,
    /// Ordered steps guiding the user from zero to completion.
    pub steps: Vec<Step>,
}

/// Runtime controller managing active tours, spotlights, and contextual tooltips.
#[derive(Default)]
pub struct GuideController {
    /// Active tour currently being guided, if any.
    active_tour: Option<Tour>,
    /// Zero-based index of the current active step in `active_tour`.
    current_step: usize,
    /// Registered bounding rects of UI elements marked in the current frame.
    registered_targets: HashMap<String, Rect>,
}

impl GuideController {
    /// Creates a new guide controller.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts an interactive walkthrough tour.
    pub fn start_tour(&mut self, tour: Tour) {
        self.active_tour = Some(tour);
        self.current_step = 0;
    }

    /// Advances to the next step in the tour, or closes it if finished.
    pub fn next_step(&mut self) {
        if let Some(tour) = &self.active_tour {
            if self.current_step + 1 < tour.steps.len() {
                self.current_step += 1;
            } else {
                self.active_tour = None;
                self.current_step = 0;
            }
        }
    }

    /// Dismisses the active tour.
    pub fn dismiss(&mut self) {
        self.active_tour = None;
        self.current_step = 0;
    }

    /// Registers the screen bounds of a target UI widget for spotlight highlighting.
    pub fn register_target(&mut self, target_id: &str, rect: Rect) {
        self.registered_targets.insert(target_id.to_string(), rect);
    }

    /// Attaches an interactive step-by-step hover guide to an `egui::Response`.
    pub fn attach_hover_guide(&mut self, response: &Response, target_id: &str, title: &str, explanation: &str) {
        self.register_target(target_id, response.rect);
        response.clone().on_hover_ui(|ui| {
            ui.set_max_width(280.0);
            ui.strong(title);
            ui.add_space(2.0);
            ui.label(explanation);
            ui.add_space(4.0);
            ui.label(egui::RichText::new("💡 לחץ F1 או פתח מדריך לליווי צעד-אחר-צעד").size(11.0).color(Color32::from_rgb(0, 120, 212)));
        });
    }

    /// Renders the spotlight overlay dimming the background and illuminating the target element.
    pub fn render_spotlight_overlay(&mut self, ctx: &egui::Context) {
        let Some(tour) = &self.active_tour else { return };
        let Some(step) = tour.steps.get(self.current_step) else { return };

        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(egui::IdSalt::new("guide_spotlight"))));
        let screen_rect = ctx.viewport_rect();

        // 1. Dim background
        let dim_color = Color32::from_black_alpha(160);
        painter.rect_filled(screen_rect, 0.0, dim_color);

        // 2. Cutout target spotlight if found
        if let Some(&target_rect) = self.registered_targets.get(&step.target_id) {
            let padded = target_rect.expand(6.0);
            painter.rect_stroke(padded, 4.0, egui::Stroke::new(2.5, Color32::from_rgb(0, 120, 212)), egui::StrokeKind::Outside);

            // Popover callout card next to target
            let card_pos = Pos2::new(padded.center().x - 140.0, (padded.bottom() + 12.0).min(screen_rect.bottom() - 140.0));
            let card_rect = Rect::from_min_size(card_pos, Vec2::new(280.0, 110.0));

            painter.rect_filled(card_rect, 8.0, Color32::from_rgb(255, 255, 255));
            painter.rect_stroke(card_rect, 8.0, egui::Stroke::new(1.0, Color32::from_gray(210)), egui::StrokeKind::Outside);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tour_lifecycle_and_spotlight() {
        let mut controller = GuideController::new();
        assert!(controller.active_tour.is_none());

        let tour = Tour {
            id: "wordcraft.insert_table".into(),
            title: "הוספת טבלה".into(),
            summary: "מדריך להוספת ועיצוב טבלה".into(),
            steps: vec![
                Step {
                    target_id: "ribbon.insert".into(),
                    title: "לחץ על כרטיסיית הוספה".into(),
                    description: "בחר בכרטיסיית הוספה בסרגל העליון".into(),
                    tip: Some("קיצור דרך: Alt+N".into()),
                },
                Step {
                    target_id: "ribbon.table".into(), title: "בחר טבלה".into(), description: "הגדר את מספר העמודות והשורות".into(), tip: None
                },
            ],
        };

        controller.start_tour(tour);
        assert_eq!(controller.current_step, 0);

        controller.register_target("ribbon.insert", Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(50.0, 30.0)));
        assert!(controller.registered_targets.contains_key("ribbon.insert"));

        controller.next_step();
        assert_eq!(controller.current_step, 1);

        controller.next_step();
        assert!(controller.active_tour.is_none(), "Tour finishes after last step");
    }
}
