//! `TextureStore` and `TextureLoader`: revisions, eviction, region updates and background jobs.

use craft_behavior::PixelSize;
use craft_host::ManualJobs;
use craft_host::queue::Cancel;
use craft_ui::primitives::{
    LoadResult, LoadedPicture, LoaderOptions, Priority, Revision, TextureBudget, TextureLoader, TextureState, TextureStore, TextureStoreOptions,
};
use craft_ui::testing::{Frames, trial};
use egui::{Color32, ColorImage, Context, Rect, TextureId, pos2};
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

/// A side four of which fill a page: two to a row, two rows.
const TILE: usize = 1000;

/// The distinct textures of `textures`, in first-seen order.
fn distinct(textures: impl IntoIterator<Item = TextureId>) -> Vec<TextureId> {
    let mut ids = Vec::new();
    for id in textures {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// Pictures share pages; when every page is full and was drawn from in this frame no page starts over (the picture
/// takes a texture of its own); a later frame starts over only the page not drawn from, in place.
#[trial]
fn a_packed_store_shares_pages_and_a_full_store_never_restarts_a_page_drawn_this_pass() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let unit = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let plain = store_with(&ctx, "plain", TextureBudget::new(usize::MAX, usize::MAX));
    assert_eq!(plain.put(&ctx, 0, Revision::new(1), square(SMALL, shade(40))).uv(), unit, "an unpacked texture is drawn whole");

    let budget = TextureBudget::new(usize::MAX, usize::MAX);
    let packed = TextureStore::<u32>::in_context(&ctx, "packed", TextureStoreOptions::thumbnails().packed(TILE as u32).budget(budget));
    let first: Vec<_> = (0..16).map(|key| packed.put(&ctx, key, Revision::new(1), square(TILE, shade(40 + key as u8)))).collect();
    let pages = distinct(first.iter().map(|texture| texture.id));
    assert!(pages.len() < first.len(), "pictures share pages: {} pages for {} pictures", pages.len(), first.len());
    let (a, b) = (first[0], first[1]);
    assert_eq!(a.id, b.id, "the first two pictures share a page");
    assert_ne!(a.uv(), b.uv());
    assert!(unit.contains_rect(a.uv()) && unit.contains_rect(b.uv()), "both lie in the page");
    assert!(!a.uv().intersects(b.uv()), "the two pictures do not overlap");
    assert_eq!(a.size_vec2(), egui::vec2(TILE as f32, TILE as f32), "a packed texture's size is the picture's own");

    // Every page is full and drawn from in this frame: the next picture takes a texture of its own.
    let own = packed.put(&ctx, 16, Revision::new(1), square(TILE, shade(200)));
    assert!(!pages.contains(&own.id), "no page started over while its pictures are on screen");
    assert_eq!(own.uv(), unit);
    assert!((0..16).all(|key| ready(&packed.state(&ctx, &key, Revision::new(1)))), "every picture drawn this pass stays");

    // A later frame draws from every page but the first one's: only that page starts over, in place.
    frames.idle(QUIET_FRAMES, |_| {});
    let page_of = |key: u32| first[key as usize].id;
    let kept: Vec<u32> = (0..16).filter(|key| page_of(*key) != a.id).collect();
    for key in &kept {
        assert!(ready(&packed.state(&ctx, key, Revision::new(1))));
    }
    let restarted = packed.put(&ctx, 17, Revision::new(1), square(TILE, shade(220)));
    assert_eq!(restarted.id, a.id, "the page drawn from longest ago starts over and keeps its texture");
    assert!((0..16).filter(|key| page_of(*key) == a.id).all(|key| packed.state(&ctx, &key, Revision::new(1)) == TextureState::Missing));
    assert!(kept.iter().all(|key| ready(&packed.state(&ctx, key, Revision::new(1)))), "pages drawn from this pass stay");
    assert_eq!(distinct(kept.iter().map(|key| page_of(*key)).chain([restarted.id])).len(), pages.len(), "no page was added");
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

/// A job that finishes after its key went unasked for two frames is discarded like a queued one; asked in the frame
/// before, its result is stored.
#[trial]
fn a_result_finishing_while_its_key_goes_unasked_is_not_stored() {
    for asked_again in [false, true] {
        let mut frames = Frames::new();
        let ctx = frames.ctx.clone();
        let (loader, jobs) = loader_with(&ctx, 1, 16);
        let runs = Arc::new(AtomicUsize::new(0));
        frame(&mut frames, || {
            loader.request(&ctx, &1, Revision::new(1), Priority::VISIBLE, || counted(&runs, SMALL));
        });
        frame(&mut frames, || {
            if asked_again {
                loader.ask(&ctx, &1, Revision::new(1), Priority::VISIBLE);
            }
            loader.request(&ctx, &2, Revision::new(1), Priority::VISIBLE, || counted(&runs, SMALL));
        });
        jobs.run_all();
        assert_eq!(runs.load(Ordering::Relaxed), 2, "both jobs ran");
        let (first, second) = (Cell::new(TextureState::Missing), Cell::new(TextureState::Missing));
        frame(&mut frames, || {
            second.set(loader.ask(&ctx, &2, Revision::new(1), Priority::VISIBLE).expect("settled"));
            first.set(loader.cache().state(&ctx, &1, Revision::new(1)));
        });
        assert!(ready(&second.into_inner()), "the asked key's result is stored");
        let first = first.into_inner();
        if asked_again {
            assert!(ready(&first), "asked in the frame before, the result is stored: {first:?}");
        } else {
            assert!(matches!(first, TextureState::Missing), "unasked for two frames, the result is dropped: {first:?}");
        }
    }
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

/// A job that finishes with no input after it still gets a pass, so its picture is drawn without waiting for the
/// pointer to move.
#[trial]
fn a_finished_render_asks_for_a_repaint() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let (loader, jobs) = loader_with(&ctx, 1, 16);
    let runs = Arc::new(AtomicUsize::new(0));
    // egui's own first frames ask for follow-up passes; the job is asked for once they are over.
    frames.idle(QUIET_FRAMES, |_| {});
    frame(&mut frames, || {
        loader.request(&ctx, &1, Revision::new(1), Priority::VISIBLE, || counted(&runs, SMALL));
    });
    assert!(!frames.repaint_requested(), "a waiting job asks for no pass");
    jobs.run_all();
    assert_eq!(runs.load(Ordering::Relaxed), 1, "the job ran");
    assert!(frames.repaint_requested(), "the finished job asks for a pass");
    let state = Cell::new(TextureState::Missing);
    frame(&mut frames, || state.set(loader.ask(&ctx, &1, Revision::new(1), Priority::VISIBLE).expect("settled")));
    let state = state.into_inner();
    assert!(ready(&state), "the pass it asked for shows the picture: {state:?}");
}
