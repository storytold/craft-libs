//! # craft-guide
//!
//! Interactive onboarding, step-by-step feature walkthroughs, spotlight overlays,
//! hover tooltips, and smart contextual help for all ArtCraft suite applications.
//!
//! Designed for zero-overhead integration across all 15 suite applications:
//! `WordCraft`, `PdfCraft`, `PhotoCraft`, `CADCraft`, `VectorCraft`, `FilmCraft`,
//! `TextureCraft`, `MaterialCraft`, `LightCraft`, `AniCraft`, `DesignCraft`,
//! `FigCraft`, `DeckCraft`, `GridCraft`, `SoundCraft`, and `Craft Launcher`.

pub mod app_catalog;
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
pub struct GuideController {
    /// Active tour currently being guided, if any.
    active_tour: Option<Tour>,
    /// Zero-based index of the current active step in `active_tour`.
    current_step: usize,
    /// Registered bounding rects of UI elements marked in the current frame.
    registered_targets: HashMap<String, Rect>,
    /// Localized hint displayed at the bottom of interactive hover tooltips.
    hover_hint: String,
}

impl Default for GuideController {
    fn default() -> Self {
        Self {
            active_tour: None,
            current_step: 0,
            registered_targets: HashMap::new(),
            hover_hint: "💡 Press F1 or open the interactive guide for step-by-step walkthroughs / לחץ F1 או פתח מדריך לליווי צעד-אחר-צעד".into(),
        }
    }
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

    /// Clears per-frame registered widget bounds before a UI pass.
    pub fn begin_frame(&mut self) {
        self.clear_targets();
    }

    /// Clears all registered spotlight target bounding rectangles.
    pub fn clear_targets(&mut self) {
        self.registered_targets.clear();
    }

    /// Overrides the localized hint shown in interactive hover tooltips.
    pub fn set_hover_hint(&mut self, hint: impl Into<String>) {
        self.hover_hint = hint.into();
    }

    /// Registers the screen bounds of a target UI widget for spotlight highlighting.
    pub fn register_target(&mut self, target_id: &str, rect: Rect) {
        self.registered_targets.insert(target_id.to_string(), rect);
    }

    /// Attaches an interactive step-by-step hover guide to an `egui::Response`.
    pub fn attach_hover_guide(&mut self, response: &Response, target_id: &str, title: &str, explanation: &str) {
        self.register_target(target_id, response.rect);
        let hint = self.hover_hint.clone();
        response.clone().on_hover_ui(|ui| {
            ui.set_max_width(280.0);
            ui.strong(title);
            ui.add_space(2.0);
            ui.label(explanation);
            ui.add_space(4.0);
            ui.label(egui::RichText::new(hint).size(11.0).color(Color32::from_rgb(0, 120, 212)));
        });
    }

    /// Renders the spotlight overlay dimming the background and illuminating the target element.
    pub fn render_spotlight_overlay(&mut self, ctx: &egui::Context) {
        let Some(tour) = &self.active_tour else { return };
        let Some(step) = tour.steps.get(self.current_step) else { return };

        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("guide_spotlight")));
        let screen_rect = ctx.viewport_rect();
        let dim_color = Color32::from_black_alpha(160);

        // Guard against zero-area or off-screen bounding boxes before cutting a spotlight hole.
        let valid_target = self
            .registered_targets
            .get(&step.target_id)
            .copied()
            .filter(|r| r.is_positive() && screen_rect.intersects(*r));
        let Some(target_rect) = valid_target else {
            painter.rect_filled(screen_rect, 0.0, dim_color);
            return;
        };

        let clamped = target_rect.intersect(screen_rect);
        let padded = clamped.expand(6.0);
        let hole = padded.intersect(screen_rect);

        // Dim the four non-overlapping strips around the spotlight cutout so the target stays bright.
        for strip in [
            Rect::from_min_max(screen_rect.min, Pos2::new(screen_rect.right(), hole.top())),
            Rect::from_min_max(Pos2::new(screen_rect.left(), hole.bottom()), screen_rect.max),
            Rect::from_min_max(Pos2::new(screen_rect.left(), hole.top()), Pos2::new(hole.left(), hole.bottom())),
            Rect::from_min_max(Pos2::new(hole.right(), hole.top()), Pos2::new(screen_rect.right(), hole.bottom())),
        ] {
            if strip.is_positive() {
                painter.rect_filled(strip, 0.0, dim_color);
            }
        }
        painter.rect_stroke(padded, 4.0, egui::Stroke::new(2.5, Color32::from_rgb(0, 120, 212)), egui::StrokeKind::Outside);

                // Popover callout card, kept inside the screen on both axes (see
                // `callout_card_rect`): not `f32::clamp`, whose bounds cross on a viewport
                // narrower than the card, and `clamp` panics on that.
        let card_rect = callout_card_rect(screen_rect, padded);

        painter.rect_filled(card_rect, 8.0, Color32::from_rgb(255, 255, 255));
        painter.rect_stroke(card_rect, 8.0, egui::Stroke::new(1.0, Color32::from_gray(210)), egui::StrokeKind::Outside);

        let card_painter = painter.with_clip_rect(card_rect);
        let text_x = card_rect.left() + 12.0;
        card_painter.text(
            Pos2::new(text_x, card_rect.top() + 12.0),
            egui::Align2::LEFT_TOP,
            &step.title,
            egui::FontId::proportional(15.0),
            Color32::from_rgb(24, 24, 27),
        );
        card_painter.text(
            Pos2::new(text_x, card_rect.top() + 34.0),
            egui::Align2::LEFT_TOP,
            &step.description,
            egui::FontId::proportional(13.0),
            Color32::from_gray(64),
        );
        if let Some(tip) = &step.tip {
            card_painter.text(
                Pos2::new(text_x, card_rect.top() + 58.0),
                egui::Align2::LEFT_TOP,
                tip,
                egui::FontId::proportional(12.0),
                Color32::from_rgb(0, 120, 212),
            );
        }
    }
}

/// Where a step's callout card goes: under the spotlight, or above it when the screen has no
/// room below, and always inside `screen` (as far as the card's own size allows).
///
/// The bounds are applied with plain comparisons rather than `f32::clamp`, which panics when
/// its lower bound is above its upper one — what a viewport narrower than the card plus its
/// margins (`right - 296 < left + 16`, i.e. under 312 px) makes of the horizontal pair. A
/// small window is not a broken one, and must not crash.
fn callout_card_rect(screen: Rect, padded: Rect) -> Rect {
    const CARD_W: f32 = 280.0;
    const CARD_H: f32 = 110.0;
    const GAP: f32 = 12.0;
    const MARGIN: f32 = 16.0;
    const SLACK: f32 = 8.0;

    // Centred under the spotlight, pulled back inside the side edges. On a screen narrower
    // than the card the left margin wins and the card may run past the right edge: better
    // than placing it off screen, and better than panicking.
    let mut x = padded.center().x - CARD_W / 2.0;
    let max_x = screen.right() - CARD_W - MARGIN;
    if x > max_x {
        x = max_x;
    }
    if x < screen.left() + MARGIN {
        x = screen.left() + MARGIN;
    }

    // Below the spotlight when the card and a little slack fit, above it otherwise, and
    // never past the top or bottom edge.
    let mut y = padded.bottom() + GAP;
    if y + CARD_H + SLACK > screen.bottom() {
        y = padded.top() - GAP - CARD_H;
    }
    let max_y = screen.bottom() - CARD_H - SLACK;
    if y > max_y {
        y = max_y;
    }
    if y < screen.top() + MARGIN {
        y = screen.top() + MARGIN;
    }

    Rect::from_min_size(Pos2::new(x, y), Vec2::new(CARD_W, CARD_H))
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

    /// A controller with one active step that spotlights `target`.
    fn controller_spotlighting(target: Rect) -> GuideController {
        let mut controller = GuideController::new();
        controller.start_tour(Tour {
            id: "photocraft.crop_and_resize".into(),
            title: "חיתוך ושינוי גודל תמונה".into(),
            summary: String::new(),
            steps: vec![Step {
                target_id: "toolbar.crop".into(),
                title: "כלי החיתוך".into(),
                description: String::new(),
                tip: None,
            }],
        });
        controller.register_target("toolbar.crop", target);
        controller
    }

    /// The callout card stays inside a screen with room for it, wherever the spotlight is.
    #[test]
    fn callout_card_clamps_inside_the_screen() {
        let screen = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(800.0, 600.0));
        for target in [
            Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(40.0, 30.0)),
            Rect::from_min_size(Pos2::new(380.0, 300.0), Vec2::new(40.0, 30.0)),
            Rect::from_min_size(Pos2::new(760.0, 570.0), Vec2::new(40.0, 30.0)),
        ] {
            // The steps the overlay takes: clamp the target to the screen, then pad by 6 px.
            let card = callout_card_rect(screen, target.intersect(screen).expand(6.0));
            assert!(screen.contains_rect(card), "card {card:?} for {target:?}");
        }
    }

    /// A viewport smaller than the card must place it, not crash. Regression: the horizontal
    /// `f32::clamp` panicked with `min = left + 16` above `max = right - 296`.
    #[test]
    fn callout_card_survives_a_viewport_smaller_than_the_card() {
        for size in [Vec2::new(300.0, 200.0), Vec2::new(280.0, 120.0), Vec2::new(800.0, 60.0), Vec2::new(120.0, 90.0)] {
            let screen = Rect::from_min_size(Pos2::new(0.0, 0.0), size);
            let padded = Rect::from_min_size(Pos2::new(40.0, 20.0), Vec2::new(20.0, 10.0)).expand(6.0);
            let card = callout_card_rect(screen, padded);
            assert!(card.width() > 0.0 && card.height() > 0.0, "{size:?}");
            assert!(card.left() >= screen.left() + 15.0 && card.top() >= screen.top() + 15.0, "card {card:?} on {size:?}");
        }
    }

    /// The whole overlay draws on a narrow viewport; this panicked before the fix.
    #[test]
    fn spotlight_overlay_draws_in_a_narrow_viewport() {
        let ctx = egui::Context::default();
        let mut controller = controller_spotlighting(Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(50.0, 30.0)));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 200.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| controller.render_spotlight_overlay(ui.ctx()));
        let rects = output.shapes.iter().filter(|s| matches!(&s.shape, egui::epaint::Shape::Rect(_))).count();
        // A headless frame leaves the font atlas delta unapplied, and the fork's epaint
        // panics when one is dropped: clear it, as the ui crate's tests do.
        output.textures_delta.clear();
        assert!(rects >= 4, "dim, spotlight and card: {rects} rect shapes");
    }

    #[test]
    fn spotlight_overlay_renders_callout_text_and_begin_frame_clears_targets() {
        let mut controller = GuideController::new();
        controller.set_hover_hint("Press F1");
        assert_eq!(controller.hover_hint, "Press F1");

        controller.start_tour(Tour {
            id: "photocraft.crop_and_resize".into(),
            title: "Crop".into(),
            summary: String::new(),
            steps: vec![Step {
                target_id: "toolbar.crop".into(),
                title: "Crop Tool".into(),
                description: "Select crop tool".into(),
                tip: Some("Shortcut: C".into()),
            }],
        });
        let target = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(60.0, 40.0));
        controller.register_target("toolbar.crop", target);
        assert!(controller.registered_targets.contains_key("toolbar.crop"));

        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| controller.render_spotlight_overlay(ui.ctx()),
        );
        let texts: Vec<String> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        let has_full_screen_dim = output.shapes.iter().any(|s| match &s.shape {
            egui::epaint::Shape::Rect(r) => r.rect == screen && r.fill != Color32::TRANSPARENT,
            _ => false,
        });
        output.textures_delta.clear();

        assert!(!has_full_screen_dim, "spotlight hole must not be covered by a full-screen dim rect");
        assert!(texts.iter().any(|t| t.contains("Crop Tool")));
        assert!(texts.iter().any(|t| t.contains("Select crop tool")));
        assert!(texts.iter().any(|t| t.contains("Shortcut: C")));

        controller.begin_frame();
        assert!(controller.registered_targets.is_empty());
    }
}
