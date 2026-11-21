//! What the report step builds for a component in each pass: its AccessKit data only for a reader (a node, a merged
//! role, a role picked from alternatives), and its tooltip text only for a tooltip the pointer or keyboard wants or
//! one still shown.

use craft_behavior::IconId;
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::Field;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Frames, last_pass_access_builds, last_pass_tip_texts, trial};
use craft_ui::text::NumberField;
use egui::{Rect, Ui};
use std::cell::RefCell;

/// The rows of each scene.
const ROWS: usize = 40;

/// Count relation: with AccessKit on, every Field row's node reads its control's access data; with it off, a pass of
/// the same rows builds none.
#[trial]
fn reports_build_access_data_only_for_a_reader() {
    let labels: Vec<String> = (0..ROWS).map(|row| format!("Row {row}")).collect();
    let show = |ui: &mut Ui| {
        for label in &labels {
            Field::new(label.clone()).show(ui, |ui, _| NumberField::new(label.clone(), Bind::given(Some(1.0))).show(ui));
        }
    };
    let mut frames = Frames::new();
    frames.idle(3, &show);
    assert!(last_pass_access_builds(&frames.ctx) >= ROWS as u32, "with AccessKit on, each row's node reads its access data");
    frames.ctx.disable_accesskit();
    frames.idle(3, &show);
    assert_eq!(last_pass_access_builds(&frames.ctx), 0, "with AccessKit off, no row builds access data");
}

/// Count relation: icon buttons whose label is their tooltip build no tooltip text while the pointer is away; the one
/// the pointer rests on builds its own once a pass, before and after its tooltip shows, and the others still none.
#[trial]
fn tooltip_texts_are_built_only_for_a_wanted_tooltip() {
    let labels: Vec<String> = (0..ROWS).map(|row| format!("Tool {row}")).collect();
    let rects = RefCell::new(Vec::new());
    let show = |ui: &mut Ui| {
        let mut shown = rects.borrow_mut();
        shown.clear();
        for label in &labels {
            shown.push(IconButton::new(IconId::new("play")).label(label.clone()).show(ui).response.rect);
        }
    };
    let mut frames = Frames::new();
    frames.idle(3, &show);
    assert_eq!(last_pass_tip_texts(&frames.ctx), 0, "no tooltip is wanted, so no text is built");
    let rested: Rect = rects.borrow()[ROWS / 2];
    frames.frame(vec![moved(rested.center())], &show);
    frames.idle(2, &show);
    assert_eq!(last_pass_tip_texts(&frames.ctx), 1, "only the button under the pointer builds its text");
    frames.advance(craft_ui::contract::tooltip::DELAY);
    frames.idle(3, &show);
    assert_eq!(last_pass_tip_texts(&frames.ctx), 1, "the shown tooltip's text is built once a pass");
}
