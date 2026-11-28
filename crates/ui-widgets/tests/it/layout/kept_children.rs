//! A composite measures its children (`MeasureCx::child`) inside its own measure; the body around it takes the
//! composite's call, or the rect it is shown at. Children with ids the body's kept transcript never held (rows salted by
//! the document they show, after the document changes) are new to the body, not a change of it: they run no discard
//! pass.

use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::SliderField;
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Frames, trial};
use egui::{Rect, Ui, Vec2};
use std::cell::Cell;

const ROW: Vec2 = Vec2::new(300.0, 48.0);

/// Three slider rows of the document `photo`, each shown at its own rect in a stack.
fn rows(ui: &mut Ui, photo: u32) {
    let at = Rect::from_min_size(ui.cursor().min, Vec2::new(ROW.x, ROW.y * 3.0));
    let _ = Stack::new().id_salt("rows").show_at(ui, at, |ui, _| {
        let top = ui.cursor().min;
        for (row, label) in ["Exposure", "Contrast", "Highlights"].into_iter().enumerate() {
            let rect = Rect::from_min_size(top + Vec2::new(0.0, ROW.y * row as f32), ROW);
            SliderField::new(label, Bind::given(0.5), -5.0..=5.0).step(0.01).id_salt((photo, row)).show_at(ui, rect);
        }
    });
}

#[trial]
fn a_composites_children_under_new_ids_run_no_discard_pass() {
    let photo = Cell::new(1u32);
    let show = |ui: &mut Ui| rows(ui, photo.get());
    let mut frames = Frames::new();
    frames.idle(4, show);
    assert_eq!(frames.frame(vec![], show), 1, "a settled frame runs one pass");
    photo.set(2);
    let changed = frames.frame(vec![], show);
    frames.idle(2, show);
    assert_eq!(frames.frame(vec![], show), 1, "the rows settle again");
    assert_eq!(changed, 1, "rows of another document, same sizes, run no discard pass");
}
