//! The pen feed: the tablet's readings pair with the pointer moves of the pass that carries them.

use craft_behavior::{PointerEvent, PointerSample};
use craft_ui::runtime::frame;
use craft_ui::runtime::pen::PenSource;
use craft_ui::runtime::pointer::with_pass_pointer;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Frames, install_spec, trial};
use egui::{Event, Pos2, ViewportId};
use std::cell::RefCell;

/// The pressure each pointer move of the running pass carries.
fn move_pressures(ctx: &egui::Context) -> Vec<Option<f32>> {
    with_pass_pointer(ctx, |pass| {
        pass.events
            .iter()
            .filter_map(|event| match event {
                PointerEvent::Moved { sample, .. } => Some(sample.map(|sample| sample.pressure)),
                _ => None,
            })
            .collect()
    })
}

/// Queues one tablet reading per pressure, in order.
fn queue(pen: &PenSource, pressures: &[f32]) {
    for &pressure in pressures {
        pen.set(Some(PointerSample::new(pressure)));
    }
}

fn two_moves() -> Vec<Event> {
    vec![moved(Pos2::new(10.0, 10.0)), moved(Pos2::new(20.0, 10.0))]
}

#[trial]
fn pen_readings_wait_for_the_pass_that_carries_the_moves() {
    let pen = PenSource::default();
    let mut frames = Frames::uninstalled();
    craft_ui::install(&frames.ctx, install_spec().pen(pen.clone())).expect("the runtime installs once");
    frames.idle(2, |_| {});
    let seen = RefCell::new(Vec::new());
    let mut record = |ui: &mut egui::Ui| seen.borrow_mut().push(move_pressures(ui.ctx()));

    // A pass with no pointer event, of this window or of another, leaves the readings for the moves.
    queue(&pen, &[0.25, 0.5, 0.75, 1.0]);
    frames.idle(1, |_| {});
    frames.frame_in_viewport(ViewportId::from_hash_of("pen child window"), Vec::new(), |_| {});
    frames.frame(two_moves(), &mut record);
    assert_eq!(seen.take(), [vec![Some(0.625), Some(1.0)]], "the readings spread over the moves, the last move taking the latest");

    // A pass the frame reruns gets no events of its own: readings that arrive during the frame wait for the next moves.
    let mut reran = false;
    let passes = frames.frame(two_moves(), |ui| {
        if !reran && frame::discardable(ui.ctx()) {
            reran = true;
            queue(&pen, &[0.25, 0.5]);
            frame::discard_pass(ui.ctx(), "pen trial: a rerun pass");
        }
    });
    assert_eq!(passes, 2, "the frame reran its pass");
    frames.frame(two_moves(), &mut record);
    assert_eq!(seen.take(), [vec![Some(0.375), Some(0.5)]], "the readings that arrived during the rerun frame pair with the next moves");
}
