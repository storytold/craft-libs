//! A caption-strength section is a quiet label over a group of controls: its title reads as written, with no rule
//! above it, while a section-strength one sits under a rule.

use craft_ui::contract::ShowContainer as _;
use craft_ui::sections::{HeaderStrength, Section};
use craft_ui::testing::{Fixture, trial};
use egui::{Shape, Vec2};

const ROOM: Vec2 = Vec2::new(400.0, 300.0);

/// The section titled `title` at `strength`, and the horizontal lines across most of its width that a frame paints.
fn rules(title: &'static str, strength: HeaderStrength) -> (Fixture, usize) {
    let mut fixture = Fixture::new(move |ui| {
        Section::fixed(title).strength(strength).show(ui, |ui, _| {
            ui.label("body");
        });
    });
    fixture.room(ROOM).settle();
    let lines = fixture
        .frame_shapes()
        .iter()
        .filter(|shape| !matches!(shape, Shape::Text(_)))
        .map(Shape::visual_bounding_rect)
        .filter(|rect| rect.is_finite() && rect.height() <= 2.0 && rect.width() >= ROOM.x / 2.0)
        .count();
    (fixture, lines)
}

#[trial]
fn a_caption_section_shows_its_title_as_written_with_no_rule_while_a_section_has_one() {
    let (caption, caption_lines) = rules("Blend mode", HeaderStrength::Caption);
    assert!(caption.texts().iter().any(|run| run.text == "Blend mode"), "the caption reads as written, not in capitals");
    let (_, section_lines) = rules("Blend mode", HeaderStrength::Section);
    assert!(section_lines > caption_lines, "a section draws a rule above its title; a caption draws none");
}
