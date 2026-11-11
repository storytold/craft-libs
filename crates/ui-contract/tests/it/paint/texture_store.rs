//! `TextureStore` and `TextureLoader`: revisions, eviction, region updates and background jobs.

use craft_behavior::PixelSize;
use craft_host::ManualJobs;
use craft_host::queue::Cancel;
use craft_ui::primitives::{
    LoadResult, LoadedPicture, LoaderOptions, Priority, Revision, TextureBudget, TextureLoader, TextureState, TextureStore, TextureStoreOptions,
};
use craft_ui::testing::{Frames, trial};
use egui::{Color32, ColorImage, Context};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SMALL: usize = 4;

const LARGE: usize = 8;

const KEPT_ENTRIES: usize = 2;

const QUIET_FRAMES: usize = 3;

fn shade(level: u8) -> Color32 {
    Color32::from_gray(level)
}

fn square(side: usize, color: Color32) -> ColorImage {
    ColorImage::filled([side, side], color)
}

fn next_frame(frames: &mut Frames) {
    frames.idle(1, |_| {});
}

fn ready(state: &TextureState) -> bool {
    matches!(state, TextureState::Ready(_))
}

fn store_with(ctx: &Context, salt: &str, budget: TextureBudget) -> TextureStore<u32> {
    TextureStore::in_context(ctx, salt, TextureStoreOptions::thumbnails().budget(budget))
}

#[trial]
fn over_budget_the_oldest_unseen_entry_goes_first_and_a_fresh_entry_stays() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let by_count = store_with(&ctx, "by-count", TextureBudget::new(usize::MAX, KEPT_ENTRIES));
    for key in 0..4 {
        by_count.put(&ctx, key, Revision::new(1), square(2, shade(40)));
        frames.idle(QUIET_FRAMES, |_| {});
    }
    let stats = by_count.statistics(&ctx);
    assert_eq!((stats.entries, stats.evicted), (KEPT_ENTRIES, 2));
    assert_eq!(by_count.state(&ctx, &0, Revision::new(1)), TextureState::Missing);
    assert!(ready(&by_count.state(&ctx, &3, Revision::new(1))));

    let crowded = store_with(&ctx, "crowded", TextureBudget::new(1, 1));
    for key in 0..3 {
        crowded.put(&ctx, key, Revision::new(1), square(2, shade(40)));
    }
    assert_eq!(crowded.statistics(&ctx).entries, 3, "entries put in this frame stay past the budget");
    next_frame(&mut frames);
    crowded.put(&ctx, 3, Revision::new(1), square(2, shade(40)));
    assert_eq!(crowded.statistics(&ctx).entries, 4, "entries of the previous frame stay too");
}

fn loader_with(ctx: &Context, lanes: usize, uploads: usize) -> (TextureLoader<u32>, Arc<ManualJobs>) {
    let jobs = Arc::new(ManualJobs::new());
    let budget = TextureBudget::new(usize::MAX, usize::MAX).uploads(uploads, usize::MAX);
    let options = LoaderOptions::new().store(TextureStoreOptions::thumbnails().budget(budget)).lanes(lanes).jobs(jobs.clone());
    (TextureLoader::in_context(ctx, "loader", options), jobs)
}

fn counted(runs: &Arc<AtomicUsize>, side: usize) -> impl FnOnce(&Cancel) -> LoadResult + Send + 'static {
    let runs = Arc::clone(runs);
    move |_| {
        runs.fetch_add(1, Ordering::Relaxed);
        Ok(LoadedPicture::Image(ColorImage::filled([side, side], shade(40))))
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

#[trial]
fn a_result_of_an_older_revision_is_discarded() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    // One upload per frame keeps key 2's finished revision-1 result waiting behind key 1's.
    let (loader, jobs) = loader_with(&ctx, 1, 1);
    let runs = Arc::new(AtomicUsize::new(0));
    frame(&mut frames, || {
        loader.request(&ctx, &1, Revision::new(1), Priority::VISIBLE, || counted(&runs, SMALL));
        loader.request(&ctx, &2, Revision::new(1), Priority::NEAR, || counted(&runs, SMALL));
    });
    jobs.run_all();
    frame(&mut frames, || {
        loader.ask(&ctx, &1, Revision::new(1), Priority::VISIBLE);
        loader.request(&ctx, &2, Revision::new(2), Priority::NEAR, || counted(&runs, LARGE));
    });
    frame(&mut frames, || {
        loader.ask(&ctx, &2, Revision::new(2), Priority::NEAR);
    });
    assert_eq!(loader.cache().statistics(&ctx).entries, 1, "the revision-1 result of key 2 was not stored");
    jobs.run_all();
    let state = Cell::new(TextureState::Missing);
    frame(&mut frames, || state.set(loader.ask(&ctx, &2, Revision::new(2), Priority::NEAR).expect("settled")));
    let size = PixelSize::new(LARGE as u32, LARGE as u32);
    assert!(matches!(state.into_inner(), TextureState::Ready(texture) if texture.size == size), "the revision-2 result is stored");
    assert_eq!(runs.load(Ordering::Relaxed), 3);
}
