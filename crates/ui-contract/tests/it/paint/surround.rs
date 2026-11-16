//! The canvas surround: a theme's pattern draws the dot texture over the fill; a theme without one, or the preference
//! turned off, draws the fill alone and makes no texture (#4542).

use craft_theme::{Contrast, Mode, Rgba, Size};
use craft_ui::primitives::{CheckerArea, paint_surround};
use craft_ui::style::{SurroundLook, UiTokens};
use craft_ui::testing::{Frames, trial};
use egui::{CornerRadius, Rect, Shape, TextureId, pos2, vec2};
use std::sync::Arc;

/// `base` with its canvas pattern set to `pattern`, or to the canvas fill for none.
fn with_pattern(base: &UiTokens, pattern: Option<Rgba>) -> UiTokens {
    let mut tokens = (**base.tokens()).clone();
    tokens.color.surface.canvas_pattern = pattern.unwrap_or(tokens.color.surface.canvas);
    UiTokens::new(Arc::new(tokens), Contrast::Standard, Size::Compact)
}

#[trial]
fn only_a_theme_pattern_draws_the_canvas_surround_texture() {
    let base = craft_ui::testing::style::tokens(Mode::Dark, Contrast::Standard);
    let plain = with_pattern(&base, None);
    let dot = Rgba([200, 40, 40, 255]);
    assert_ne!(dot, base.color.surface.canvas, "the pattern differs from the fill");
    let dotted = with_pattern(&base, Some(dot));

    let area = Rect::from_min_size(pos2(40.0, 30.0), vec2(200.0, 120.0));
    let mut frames = Frames::new();
    let mut painted = |look: SurroundLook| -> Vec<Shape> {
        let shapes = frames.frame_shapes(|ui| paint_surround(ui.painter(), CheckerArea::Rect(area, CornerRadius::ZERO), &look));
        shapes.into_iter().filter(|shape| shape.visual_bounding_rect() == area).collect()
    };

    let cases = [
        ("a theme whose pattern is its fill", SurroundLook::of(&plain), false),
        ("the same theme with the preference on", SurroundLook::of(&plain).shown(true), false),
        ("a theme with a pattern", SurroundLook::of(&dotted), true),
        ("a theme with a pattern and the preference on", SurroundLook::of(&dotted).shown(true), true),
        ("a theme with a pattern and the preference off", SurroundLook::of(&dotted).shown(false), false),
    ];
    for (case, look, textured) in cases {
        assert_eq!(look.pattern.is_some(), textured, "{case}: the look's pattern");
        let shapes = painted(look);
        let fills = shapes.iter().filter(|shape| shape.texture_id() == TextureId::default()).count();
        let dots = shapes.iter().filter(|shape| shape.texture_id() != TextureId::default()).count();
        assert_eq!((fills, dots), (1, usize::from(textured)), "{case}: one fill, and the dot texture only with a pattern");
    }
}
