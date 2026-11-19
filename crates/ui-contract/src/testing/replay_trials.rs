//! Trials of the replay records: their notes and their comparison.

#![allow(clippy::expect_used, reason = "trial code: clippy.toml allows expect in tests, and trials are compiled into the library")]

use crate::primitives::sensing::{self, MemberSpec};
use crate::runtime::egui::{fixture, repaint};
use crate::runtime::env::ArenaId;
use crate::runtime::focus::{self, FocusPolicy, KeyUse, Stop};
use crate::runtime::replay;
use crate::testing::{Frames, pointer, trial};
use craft_behavior::Sense;
use egui::{Pos2, Rect, Vec2};

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

/// Records `stops` run inside a record, with `entering` the first-pass flag of the layer scope it pushes, inside the
/// scope `outer` when one is given.
fn layer_scope_record(frames: &mut Frames, outer: Option<egui::Id>, entering: bool) -> replay::Record {
    let [scope, stop] = [fixture::named_id("layer scope"), fixture::named_id("stop")];
    let mut record = None;
    frames.frame(Vec::new(), |ui| {
        let ctx = ui.ctx().clone();
        if let Some(outer) = outer {
            focus::push_scope(&ctx, outer, FocusPolicy::NONE, None);
        }
        replay::open(&ctx);
        focus::register_stop(&ctx, Stop::new(fixture::named_id("beside"), KeyUse::CODE));
        focus::push_scope_from(&ctx, scope, FocusPolicy::NONE, None, None, false, Some(entering));
        focus::register_stop(&ctx, Stop::new(stop, KeyUse::CODE));
        focus::pop_scope(&ctx);
        record = Some(replay::close(&ctx, fixture::layer_of(ui)));
        if outer.is_some() {
            focus::pop_scope(&ctx);
        }
    });
    record.expect("the frame ran")
}

// A view recorded in the pass its layer appeared (`entering`) and the real run of the next pass register the same
// once replayed: a replayed scope is no new scope.
#[trial]
fn a_first_pass_transcript_equals_the_next_real_run() {
    let mut frames = Frames::new();
    let first = layer_scope_record(&mut frames, None, true);
    let next = layer_scope_record(&mut frames, None, false);
    assert_eq!(first.focus, next.focus);
}

// The same view shown inside two different scopes registers the same records under each.
#[trial]
fn a_transcript_under_another_open_scope_compares_equal() {
    let mut frames = Frames::new();
    let one = layer_scope_record(&mut frames, Some(fixture::named_id("outer one")), false);
    let other = layer_scope_record(&mut frames, Some(fixture::named_id("outer other")), false);
    assert_eq!(one.focus, other.focus);
}

// A child recorded as the arena's first member and replayed after two members that cover it stacks above them, as a
// real run after them would: its stack positions come from the arena it joins, not from the recorded run.
#[trial]
fn a_replayed_member_stacks_above_the_members_before_it() {
    let mut frames = Frames::new();
    let arena = ArenaId::new(fixture::named_id("arena"));
    let [before, cover, child] = [fixture::named_id("before"), fixture::named_id("cover"), fixture::named_id("child")];
    let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::splat(40.0));
    let spec = |id, rect| MemberSpec { id, rect, sense: Sense::HOVER | Sense::CLICK, parts: &[], enabled: true };
    let mut recorded = None;
    frames.frame(Vec::new(), |ui| {
        let ctx = ui.ctx().clone();
        replay::open(&ctx);
        sensing::sense_members(ui, arena, &[spec(child, rect)]);
        recorded = Some(replay::close(&ctx, fixture::layer_of(ui)));
    });
    let recorded = recorded.expect("the frame ran");
    let shown = |ui: &mut egui::Ui, replayed: bool| {
        sensing::sense_members(ui, arena, &[spec(before, Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0))), spec(cover, rect)]);
        if replayed {
            replay::replay(ui.ctx(), &recorded);
            return None;
        }
        let ctx = ui.ctx().clone();
        replay::open(&ctx);
        sensing::sense_members(ui, arena, &[spec(child, rect)]);
        Some(replay::close(&ctx, fixture::layer_of(ui)))
    };
    let mut real = None;
    frames.frame(Vec::new(), |ui| real = shown(ui, false));
    let real = real.expect("the frame ran");
    assert_eq!(recorded.journal, real.journal, "the join is noted as declared, without its stack positions");
    frames.frame(Vec::new(), |ui| {
        let _ = shown(ui, true);
    });
    let mut hovered = None;
    frames.frame(vec![pointer::moved(rect.center())], |ui| {
        hovered = Some([crate::runtime::pointer::hovers(ui.ctx(), child), crate::runtime::pointer::hovers(ui.ctx(), cover)]);
        let _ = shown(ui, true);
    });
    assert_eq!(hovered, Some([true, false]), "the replayed child is above the member that covers it");
}
