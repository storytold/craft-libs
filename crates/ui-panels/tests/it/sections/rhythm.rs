//! `SectionRhythm`: the room around the rule above a section's header.

use craft_ui::contract::{SectionRhythm, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::{Env, RootEnv};
use craft_ui::sections::Section;
use craft_ui::testing::{Frames, trial};
use egui::{Rect, Ui};

/// What a stack of a row and a fixed section shows under `rhythm`: the room from the row to the section's rule, the room
/// from the rule to the section's body, and `space.gap.section`.
fn rule_room(rhythm: SectionRhythm) -> (f32, f32, f32) {
    let mut frames = Frames::new();
    let mut root = RootEnv::default();
    root.values.set(rhythm);
    frames.root(root);
    let (mut row, mut body, mut section_gap) = (Rect::NOTHING, Rect::NOTHING, 0.0);
    let mut show = |ui: &mut Ui| {
        section_gap = Env::of(ui).tokens.space.gap.section;
        Stack::new().show(ui, |ui, _| {
            row = Text::new("Fill").show(ui).rect;
            Section::fixed("Stroke").show(ui, |ui, _| body = Text::new("Width").show(ui).rect);
        });
    };
    frames.idle(2, &mut show);
    let rule = frames
        .frame_shapes(&mut show)
        .iter()
        .map(egui::Shape::visual_bounding_rect)
        .find(|line| {
            line.is_finite() && line.height() <= 2.0 && line.width() > body.width() && line.top() >= row.bottom() && line.bottom() <= body.top()
        })
        .expect("the section draws its rule between the row and its body");
    (rule.top() - row.bottom(), body.top() - rule.bottom(), section_gap)
}

/// Regression: a ruled section in a stack took only the stack's gap above its rule and its header right under it, so an
/// app could not keep production's room around its panel dividers. `Spaced` stands the rule `space.gap.section` off the
/// row above and the header further from the rule; `Tight` keeps the stack's rhythm.
#[trial]
fn a_spaced_section_stands_its_rule_off_the_row_above_and_its_header() {
    let (tight_above, tight_below, _) = rule_room(SectionRhythm::Tight);
    let (spaced_above, spaced_below, section_gap) = rule_room(SectionRhythm::Spaced);
    assert!(spaced_above > tight_above, "the spaced rule stands further off the row ({spaced_above}) than the tight one ({tight_above})");
    assert!(spaced_above >= section_gap - 0.5, "the spaced rule stands a section gap ({section_gap}) off the row, not {spaced_above}");
    assert!(spaced_below > tight_below, "the spaced header stands further below its rule ({spaced_below}) than the tight one ({tight_below})");
}
