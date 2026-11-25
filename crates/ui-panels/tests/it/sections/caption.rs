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

/// An overline section reads its title in capitals, with no rule above it.
#[trial]
fn an_overline_section_shows_its_title_in_capitals_with_no_rule() {
    let (overline, lines) = rules("Pages to print", HeaderStrength::Overline);
    assert!(overline.texts().iter().any(|run| run.text == "PAGES TO PRINT"), "the overline is in capitals");
    let (_, section_lines) = rules("Pages to print", HeaderStrength::Section);
    assert!(section_lines > lines, "no rule above an overline");
}

/// The height of a sub-heading's title row under `row`.
fn subheading_height(row: craft_ui::sections::QuietTitleRow) -> f32 {
    let mut fixture = Fixture::new(|ui| {
        Section::fixed("Ellipse Properties").strength(HeaderStrength::Subheading).show(ui, |ui, _| {
            ui.label("body");
        });
    });
    fixture.root_value(row).room(ROOM).settle();
    fixture.by_role(craft_ui::contract::Role::HEADING).first().expect("the sub-heading is a heading").rect.height()
}

/// An app whose sub-headings are a line of text over their fields (`QuietTitleRow::Text`) gets a title row shorter
/// than the control-high one other apps keep.
#[trial]
fn a_text_row_subheading_stands_lower_than_a_control_high_one() {
    use craft_ui::sections::QuietTitleRow;
    let (control, text) = (subheading_height(QuietTitleRow::Control), subheading_height(QuietTitleRow::Text));
    assert!(text < control, "the text row ({text}) is lower than the control-high row ({control})");
}
