//! Trials of the replay records' notes.

use crate::runtime::egui::{fixture, repaint};
use crate::runtime::replay;
use crate::testing::{Frames, trial};

// A job's thread asks for a pass (a finished job's wake, a progress report) while a pass of the same context records a
// region on the UI thread: the request notes nothing in that pass's records and leaves its pass state alone.
#[trial]
fn a_repaint_from_a_job_thread_leaves_the_running_pass_s_records_alone() {
    let mut frames = Frames::new();
    let mut joined = None;
    frames.frame(Vec::new(), |ui| {
        let ctx = ui.ctx().clone();
        replay::open(&ctx);
        let worker = ctx.clone();
        joined = Some(std::thread::spawn(move || repaint::request(&worker)).join().is_ok());
        let _record = replay::close(&ctx, fixture::layer_of(ui));
    });
    assert_eq!(joined, Some(true), "the job thread's repaint request returns");
}
