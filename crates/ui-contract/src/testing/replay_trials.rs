//! Trials of the replay records: their notes and their comparison.

use crate::runtime::egui::{fixture, repaint};
use crate::runtime::focus::{self, KeyUse, Stop};
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

/// Records one frame's run of `run` in `frames`.
fn record(frames: &mut Frames, mut run: impl FnMut(&egui::Context)) -> replay::Record {
    let mut record = None;
    frames.frame(Vec::new(), |ui| {
        let ctx = ui.ctx().clone();
        replay::open(&ctx);
        run(&ctx);
        record = Some(replay::close(&ctx, fixture::layer_of(ui)));
    });
    record.expect("the frame ran")
}

fn note_a(_: &egui::Context, _: &(egui::Id, u32)) {}

fn note_b(_: &egui::Context, _: &(egui::Id, u32)) {}

// A journal entry is the same registration as another only when both come from the same call with equal arguments: a
// replay and a real run compare by it.
#[trial]
fn journal_entries_compare_by_their_call_and_arguments() {
    let mut frames = Frames::new();
    let id = fixture::named_id("owner");
    let notes = |calls: [(bool, u32); 2]| {
        move |ctx: &egui::Context| {
            for (a, n) in calls {
                if a {
                    replay::journal(ctx, || (id, n), note_a);
                } else {
                    replay::journal(ctx, || (id, n), note_b);
                }
            }
        }
    };
    let first = record(&mut frames, notes([(true, 1), (true, 2)]));
    let again = record(&mut frames, notes([(true, 1), (true, 2)]));
    let changed_argument = record(&mut frames, notes([(true, 1), (true, 3)]));
    let other_call = record(&mut frames, notes([(true, 1), (false, 2)]));
    assert_eq!(first.journal.len(), 2, "both notes are in the record");
    assert_eq!(first.journal, again.journal, "the same calls with the same arguments");
    assert_ne!(first.journal[1], changed_argument.journal[1], "one argument changed");
    assert_eq!(first.journal[0], changed_argument.journal[0]);
    assert_ne!(first.journal[1], other_call.journal[1], "another call with the same arguments");
}

// A focus transcript with one changed stop compares unequal to the run before it; one with the same stops equal.
#[trial]
fn a_transcript_with_one_changed_stop_compares_unequal() {
    let mut frames = Frames::new();
    let [one, two] = [fixture::named_id("one"), fixture::named_id("two")];
    let stops = |second: KeyUse| {
        move |ctx: &egui::Context| {
            focus::register_stop(ctx, Stop::new(one, KeyUse::CODE));
            focus::register_stop(ctx, Stop::new(two, second));
        }
    };
    let first = record(&mut frames, stops(KeyUse::CODE));
    let again = record(&mut frames, stops(KeyUse::CODE));
    let changed = record(&mut frames, stops(KeyUse::TEXT_MULTILINE));
    assert_eq!(first.focus.stop_ids().count(), 2, "both stops are in the transcript");
    assert_eq!(first.focus, again.focus);
    assert_ne!(first.focus, changed.focus, "the second stop's keys changed");
}
