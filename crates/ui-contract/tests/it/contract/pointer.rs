//! Pointer and focus rules the pipeline keeps between the services: one press cancel path, parts and their focus.

use craft_behavior::{PressPhase, Sense};
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{Panel, Probe, Seen};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
}

fn releases(seen: &Seen) -> usize {
    seen.press.iter().filter(|phase| matches!(phase, PressPhase::Release { .. })).count()
}

/// A tool on a canvas: press, drag, then release in a frame egui discards and repeats.
#[trial]
fn canvas_tool_release_in_a_discarded_pass_reaches_the_tool_once() {
    let mut frames = Frames::new();
    let canvas = RefCell::new(Seen::default());
    let discard = Cell::new(false);
    let mut show = |ui: &mut Ui| {
        let tool = RefCell::new(Seen::default());
        Panel::new().arena(true).sense(Sense::HOVER | Sense::DRAG).seen(&canvas).id_salt("canvas").show_at(
            ui,
            rect(10.0, 10.0, 300.0, 200.0),
            |ui, _| {
                Probe::new(&tool).size(Vec2::new(40.0, 20.0)).id_salt("control").show_at(ui, rect(20.0, 20.0, 40.0, 20.0));
            },
        );
        if discard.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the release frame runs twice");
        }
    };
    let from = Pos2::new(200.0, 150.0);
    frames.frame(vec![], &mut show);
    frames.frame(vec![moved(from)], &mut show);
    frames.frame(vec![press(from)], &mut show);
    frames.frame(vec![moved(from + Vec2::new(30.0, 0.0))], &mut show);
    discard.set(true);
    let passes = frames.frame(vec![release(from + Vec2::new(30.0, 0.0))], &mut show);
    discard.set(false);
    frames.frame(vec![], &mut show);
    assert_eq!(passes, 2, "the release frame ran twice");
    assert_eq!(releases(&canvas.borrow()), 1, "the tool saw its release once: {:?}", canvas.borrow().press);
}
