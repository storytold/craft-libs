//! A picture that failed to load is asked for again only when its revision changes.

use craft_host::ManualJobs;
use craft_host::queue::Cancel;
use craft_ui::primitives::{
    LoadFailure, LoadResult, LoaderOptions, Priority, Revision, TextureBudget, TextureLoader, TextureState, TextureStoreOptions,
};
use craft_ui::testing::{Frames, trial};
use egui::Context;
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The frames the failed revision is asked for after its failure is stored.
const ASKED_AGAIN: usize = 4;

fn loader_with(ctx: &Context, lanes: usize, uploads: usize) -> (TextureLoader<u32>, Arc<ManualJobs>) {
    let jobs = Arc::new(ManualJobs::new());
    let budget = TextureBudget::new(usize::MAX, usize::MAX).uploads(uploads, usize::MAX);
    let options = LoaderOptions::new().store(TextureStoreOptions::thumbnails().budget(budget)).lanes(lanes).jobs(jobs.clone());
    (TextureLoader::in_context(ctx, "loader", options), jobs)
}

/// A job that counts its run in `runs` and fails, as a page that cannot render does.
fn failing(runs: &Arc<AtomicUsize>) -> impl FnOnce(&Cancel) -> LoadResult + Send + 'static {
    let runs = Arc::clone(runs);
    move |_| {
        runs.fetch_add(1, Ordering::Relaxed);
        Err(LoadFailure::Unreadable)
    }
}

/// One frame in which `run` asks the loader.
fn frame(frames: &mut Frames, run: impl FnOnce()) {
    let mut run = Some(run);
    frames.frame(Vec::new(), |_| {
        if let Some(run) = run.take() {
            run();
        }
    });
}

/// A page whose picture fails is not made again on every frame that shows it: the store keeps the failure for the
/// revision that failed, so asking for that revision queues no job. A new revision is missing, not failed, and is
/// made once.
#[trial]
fn a_failed_thumbnail_is_asked_again_only_after_its_revision_changes() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let (loader, jobs) = loader_with(&ctx, 1, 16);
    let runs = Arc::new(AtomicUsize::new(0));
    let ask = |revision: u64| loader.request(&ctx, &1, Revision::new(revision), Priority::VISIBLE, || failing(&runs));

    frame(&mut frames, || {
        ask(1);
    });
    jobs.run_all();
    assert_eq!(runs.load(Ordering::Relaxed), 1, "the first ask makes the picture");

    for _ in 0..ASKED_AGAIN {
        let state = Cell::new(TextureState::Missing);
        frame(&mut frames, || state.set(ask(1)));
        jobs.run_all();
        let state = state.into_inner();
        assert!(matches!(state, TextureState::Failed { .. }), "the failed revision shows its failure: {state:?}");
    }
    assert_eq!(runs.load(Ordering::Relaxed), 1, "the failed revision is not made again");
    assert_eq!(loader.cache().state(&ctx, &1, Revision::new(2)), TextureState::Missing, "a later revision is missing, not failed");

    frame(&mut frames, || {
        ask(2);
    });
    jobs.run_all();
    frame(&mut frames, || {
        ask(2);
    });
    jobs.run_all();
    assert_eq!(runs.load(Ordering::Relaxed), 2, "a new revision is made once");
}
