//! # craft-ai-assistant-dialog
//!
//! Universal Floating AI Assistant & FAQ Search Panel integrated into the Ribbon across
//! all 15 ArtCraft applications. Allows users to type in plain language (Hebrew, English, etc.)
//! and immediately triggers interactive step-by-step walkthroughs (`craft-guide`) on the canvas.

use craft_guide::GuideController;
use egui::{Context, RichText, Vec2, Window};
use serde::{Deserialize, Serialize};

/// Search result matching a user's natural language help query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelpSearchResult {
    pub tour_id: String,
    pub title: String,
    pub summary: String,
    pub relevance_score: f32,
}

/// Floating AI Assistant Dialog state.
pub struct AiAssistantPanel {
    pub is_open: bool,
    pub query: String,
    pub search_results: Vec<HelpSearchResult>,
    pub app_id: String,
    pub custom_tours: Option<Vec<craft_guide::Tour>>,
}

impl AiAssistantPanel {
    pub fn new(app_id: impl Into<String>) -> Self {
        Self { is_open: false, query: String::new(), search_results: Vec::new(), app_id: app_id.into(), custom_tours: None }
    }

    pub fn with_tours(app_id: impl Into<String>, tours: Vec<craft_guide::Tour>) -> Self {
        Self { is_open: false, query: String::new(), search_results: Vec::new(), app_id: app_id.into(), custom_tours: Some(tours) }
    }

    /// Toggles the assistant window open/closed (e.g. via F1 or Ribbon button).
    pub fn toggle(&mut self) {
        self.is_open = !self.is_open;
        if self.is_open && self.query.is_empty() {
            self.refresh_results();
        }
    }

    /// Evaluates user query against available tours in the app catalog.
    pub fn refresh_results(&mut self) {
        let tours = self
            .custom_tours
            .clone()
            .unwrap_or_else(|| craft_guide::app_catalog::tours_for_app(&self.app_id));
        let q = self.query.trim().to_lowercase();

        if q.is_empty() {
            self.search_results =
                tours.into_iter().map(|t| HelpSearchResult { tour_id: t.id, title: t.title, summary: t.summary, relevance_score: 1.0 }).collect();
            return;
        }

        self.search_results = tours
            .into_iter()
            .filter_map(|t| {
                let matches_title = t.title.to_lowercase().contains(&q);
                let matches_summary = t.summary.to_lowercase().contains(&q);
                if matches_title || matches_summary {
                    let score = if matches_title { 2.0 } else { 1.0 };
                    Some(HelpSearchResult { tour_id: t.id, title: t.title, summary: t.summary, relevance_score: score })
                } else {
                    None
                }
            })
            .collect();

        self.search_results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// Renders the floating assistant window using `egui`.
    pub fn render(&mut self, ctx: &Context, guide_controller: &mut GuideController) {
        if !self.is_open {
            return;
        }

        let mut open = self.is_open;
        Window::new("🤖 סייען אינטראקטיבי ושאלות נפוצות").open(&mut open).default_size(Vec2::new(420.0, 480.0)).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("🔍");
                let text_edit = ui.text_edit_singleline(&mut self.query);
                if text_edit.changed() {
                    self.refresh_results();
                }
            });

            ui.separator();
            ui.label(RichText::new("מדריכים זמינים לביצוע מהיר:").strong());

            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut triggered_tour_id = None;

                for res in &self.search_results {
                    ui.group(|ui| {
                        ui.heading(&res.title);
                        ui.label(&res.summary);
                        if ui.button("▶ התחל הדרכה על המסך").clicked() {
                            triggered_tour_id = Some(res.tour_id.clone());
                        }
                    });
                }

                if let Some(tid) = triggered_tour_id {
                    let tours = self
                        .custom_tours
                        .clone()
                        .unwrap_or_else(|| craft_guide::app_catalog::tours_for_app(&self.app_id));
                    if let Some(t) = tours.into_iter().find(|t| t.id == tid) {
                        guide_controller.start_tour(t);
                        self.is_open = false;
                    }
                }
            });
        });

        self.is_open = self.is_open && open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_panel_searches_and_triggers_tour() {
        let mut panel = AiAssistantPanel::new("wordcraft");
        panel.refresh_results();
        assert!(!panel.search_results.is_empty(), "WordCraft should have available guides");

        panel.query = "טבלה".into();
        panel.refresh_results();
        assert!(panel.search_results.iter().any(|r| r.title.contains("טבלה")));
    }
}

#[cfg(test)]
mod dialog_behavior_tests {
    use super::*;

    #[test]
    fn english_table_query_matches_wordcraft_insert_table() {
        let mut panel = AiAssistantPanel::new("wordcraft");
        panel.query = "table".into();
        panel.refresh_results();
        assert!(
            panel
                .search_results
                .iter()
                .any(|r| r.tour_id == "wordcraft.insert_table"),
            "expected wordcraft.insert_table in search results for 'table'"
        );
    }

    #[test]
    fn starting_tour_closes_assistant_panel() {
        let ctx = egui::Context::default();
        let tour = craft_guide::Tour {
            id: "test.tour".into(),
            title: "Sample Tour".into(),
            summary: "Sample summary".into(),
            steps: vec![craft_guide::Step {
                target_id: "btn.target".into(),
                title: "Step 1".into(),
                description: "Click target".into(),
                tip: None,
            }],
        };
        let mut panel = AiAssistantPanel::with_tours("wordcraft", vec![tour]);
        panel.is_open = true;
        panel.query = "Sample".into();
        panel.refresh_results();

        let mut guide = craft_guide::GuideController::new();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(800.0, 600.0));

        // Warm up two frames so egui's Window completes its initial sizing pass.
        for _ in 0..2 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| panel.render(ui.ctx(), &mut guide),
            );
            out.textures_delta.clear();
        }

        // Frame 3: find the "Start tour" button center from painted shapes.
        let mut out1 = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| panel.render(ui.ctx(), &mut guide),
        );
        let btn_center = out1
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) if t.galley.text().contains("▶") => {
                    Some(t.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("start tour button text should be painted");
        out1.textures_delta.clear();

        // Frame 2 + 3: press and release primary pointer button on the button center.
        for pressed in [true, false] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events: vec![
                        egui::Event::PointerMoved(btn_center),
                        egui::Event::PointerButton {
                            pos: btn_center,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| panel.render(ui.ctx(), &mut guide),
            );
            out.textures_delta.clear();
        }

        assert!(!panel.is_open, "panel should close when a tour starts");
    }
}
