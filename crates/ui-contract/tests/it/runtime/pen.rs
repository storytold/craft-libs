//! The pen feed: the tablet's readings pair with the pointer moves of the pass that carries them.

use craft_behavior::{PointerEvent, PointerKind, PointerSample};
use craft_ui::runtime::frame;
use craft_ui::runtime::pen::PenSource;
use craft_ui::runtime::pointer::with_pass_pointer;
use craft_ui::testing::pointer::{self, moved};
use craft_ui::testing::{Frames, install_spec, trial};
use egui::{Event, Pos2, TouchDeviceId, TouchId, TouchPhase, ViewportId};
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
    assert_eq!(seen.take(), [vec![Some(0.5), Some(1.0)]], "the readings spread over the moves, the last move taking the latest");

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
    assert_eq!(seen.take(), [vec![Some(0.25), Some(0.5)]], "the readings that arrived during the rerun frame pair with the next moves");
}

/// The pressure of the running pass's pointer as a whole, and whether it reads as a pen.
fn pass_pressure(ctx: &egui::Context) -> (Option<f32>, bool) {
    with_pass_pointer(ctx, |pass| {
        pass.frame.as_ref().map_or((None, false), |frame| (frame.sample.map(|sample| sample.pressure), frame.kind == PointerKind::Pen))
    })
}

#[trial]
fn as_many_moves_as_readings_take_one_each_and_a_lifted_tap_keeps_its_contact_pressure() {
    let pen = PenSource::default();
    let mut frames = Frames::uninstalled();
    craft_ui::install(&frames.ctx, install_spec().pen(pen.clone())).expect("the runtime installs once");
    frames.idle(2, |_| {});
    let at = Pos2::new(10.0, 10.0);
    let seen = RefCell::new(Vec::new());

    queue(&pen, &[0.2]);
    frames.frame(vec![moved(at)], |_| {});
    queue(&pen, &[0.4, 0.6, 0.8]);
    let three = [20.0, 30.0, 40.0].map(|x| moved(Pos2::new(x, 10.0))).to_vec();
    frames.frame(three, |ui| seen.borrow_mut().push(move_pressures(ui.ctx())));
    assert_eq!(seen.take(), [vec![Some(0.4), Some(0.6), Some(0.8)]], "each move takes its own reading");

    // A tap shorter than a pass: the pen touched, reported pressure 0 as it lifted and left before the pass ran.
    let taps = RefCell::new(Vec::new());
    queue(&pen, &[0.0, 0.6, 0.0]);
    pen.set(None);
    frames.frame(vec![pointer::press(at), pointer::release(at)], |ui| taps.borrow_mut().push(pass_pressure(ui.ctx())));
    assert_eq!(taps.take(), [(Some(0.6), true)], "the tap is the pen's, at its contact pressure");
}

/// A new touch in a pass that also carries an earlier touch's lift supplies the pointer's force.
#[trial]
fn a_new_touch_supplies_the_force_when_an_earlier_touch_lifts_in_the_same_pass() {
    let mut frames = Frames::new();
    frames.idle(2, |_| {});
    let at = Pos2::new(10.0, 10.0);
    let touch = |id, phase, force| Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos: at, force };
    let events = vec![touch(1, TouchPhase::Start, Some(0.6)), touch(1, TouchPhase::End, None), touch(2, TouchPhase::Start, Some(0.2)), moved(at)];
    let mut seen = Vec::new();
    frames.frame(events, |ui| {
        let reading = with_pass_pointer(ui.ctx(), |pass| pass.frame.as_ref().map(|frame| (frame.kind, frame.sample.map(|sample| sample.pressure))));
        seen.push((reading, move_pressures(ui.ctx())));
    });
    assert_eq!(seen, [(Some((PointerKind::Touch, Some(0.2))), vec![Some(0.2)])]);
}
