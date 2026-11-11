//! A control that draws only its mark (a check box, a radio button) takes its label into its own hit target: a
//! press on the label is a press on the control, in the same pass.

use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Rect, Shape, Ui, pos2, vec2};
use std::cell::Cell;

const REGION: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(300.0, 200.0) };

/// The rect `text` was painted in, last frame.
fn text_rect(frames: &Frames, text: &str) -> Rect {
    frames.texts().iter().find(|painted| painted.text == text).unwrap_or_else(|| panic!("no {text} painted")).rect
}

/// The fill colours painted inside `within`.
fn fills(shapes: &[Shape], within: Rect) -> Vec<egui::Color32> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if rect.fill.a() > 0 && within.contains_rect(rect.rect) => Some(rect.fill),
            _ => None,
        })
        .collect()
}

#[trial]
fn a_click_on_a_check_box_label_toggles_it_in_that_pass_and_a_press_there_shows_on_the_box() {
    let mut frames = Frames::new();
    let on = Cell::new(false);
    let shown = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        let mut value = on.get();
        shown.set(in_region(ui, REGION, |ui| Checkbox::new(Bind::bound(&mut value)).label("Snap to grid").show(ui).response.rect));
        on.set(value);
    };
    frames.frame(vec![], &mut show);
    let label = text_rect(&frames, "Snap to grid");
    let rest = frames.frame_shapes(&mut show);
    let mark = shown.get();
    assert!(label.left() > mark.right(), "the label follows the box");
    frames.frame(vec![moved(label.center())], &mut show);
    frames.frame(vec![press(label.center())], &mut show);
    let pressed = frames.frame_shapes(&mut show);
    assert_ne!(fills(&pressed, mark), fills(&rest, mark), "a press on the label shows on the box");
    frames.frame(vec![release(label.center())], &mut show);
    assert!(on.get(), "the release on the label toggles the box in its own pass");
    let gap = pos2((mark.right() + label.left()) / 2.0, label.center().y);
    for event in [moved(gap), press(gap), release(gap)] {
        frames.frame(vec![event], &mut show);
    }
    assert!(!on.get(), "a click in the gap between box and label toggles it too");
}

#[trial]
fn a_click_on_a_radio_label_picks_it_in_that_pass() {
    let mut frames = Frames::new();
    let picked = Cell::new(1u8);
    let mut show = |ui: &mut Ui| {
        let mut value = picked.get();
        in_region(ui, REGION, |ui| {
            RadioGroup::new("Size", Bind::bound(&mut value)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    for (at, label) in [(1u8, "Small"), (2, "Medium"), (3, "Large")] {
                        Radio::new(at).label(label).show(ui);
                    }
                });
            })
        });
        picked.set(value);
    };
    frames.frame(vec![], &mut show);
    let medium = text_rect(&frames, "Medium").center();
    frames.frame(vec![moved(medium)], &mut show);
    frames.frame(vec![press(medium)], &mut show);
    frames.frame(vec![release(medium)], &mut show);
    assert_eq!(picked.get(), 2, "the release on Medium's label picks it in its own pass");
}

#[trial]
fn a_long_label_after_its_box_wraps_with_the_box_on_its_first_line() {
    const LONG: &str = "Show the transform controls on the selected layer whenever the move tool is active";
    let mut frames = Frames::new();
    let shown = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(160.0, 200.0));
        shown.set(in_region(ui, narrow, |ui| Checkbox::new(Bind::given(false)).label(LONG).show(ui).response.rect));
    };
    frames.frame(vec![], &mut show);
    frames.frame(vec![], &mut show);
    let painted = frames.texts().iter().find(|painted| painted.text == LONG).unwrap_or_else(|| panic!("no long label painted"));
    let mark = shown.get();
    assert!(!painted.elided, "the label wraps instead of eliding");
    assert!(painted.rect.height() > mark.height() * 1.5, "the label runs over more than one line");
    assert!(painted.rect.right() <= 160.0, "the label stays inside its room");
    assert!(mark.center().y < painted.rect.center().y && mark.top() < painted.rect.top() + mark.height(), "the box sits on the first line");
}
