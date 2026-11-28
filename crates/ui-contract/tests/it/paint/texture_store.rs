//! `TextureStore` and `TextureLoader`: revisions, eviction, region updates and background jobs.

use craft_behavior::PixelSize;
use craft_host::ManualJobs;
use craft_host::queue::Cancel;
use craft_ui::primitives::{
    LoadFailure, LoadResult, LoadedPicture, LoaderOptions, Priority, Revision, TextureBudget, TextureLoader, TextureState, TextureStore,
    TextureStoreOptions, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails,
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

#[trial]
fn a_region_update_patches_the_existing_texture_in_place_and_advances_revision() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let (loader, jobs) = loader_with(&ctx, 1, 16);
    assert!(!loader.cache().has_texture(&ctx, &1), "initially has no texture");

    // Put initial full picture
    frame(&mut frames, || {
        loader.request(&ctx, &1, Revision::new(1), Priority::VISIBLE, || move |_| Ok(LoadedPicture::Image(square(LARGE, shade(10)))));
    });
    jobs.run_all();
    let mut orig_id = None;
    frame(&mut frames, || {
        if let Some(TextureState::Ready(tex)) = loader.ask(&ctx, &1, Revision::new(1), Priority::VISIBLE) {
            orig_id = Some(tex.id);
        }
    });
    let orig_id = orig_id.expect("revision 1 loaded");
    assert!(loader.cache().has_texture(&ctx, &1), "now has texture resident");
    assert_eq!(loader.cache().texture_revision(&ctx, &1), Some(Revision::new(1)));
    assert_eq!(loader.cache().texture_size(&ctx, &1), Some(PixelSize::new(LARGE as u32, LARGE as u32)));

    // Request region update to revision 2
    frame(&mut frames, || {
        loader.request(&ctx, &1, Revision::new(2), Priority::VISIBLE, || {
            move |_| Ok(LoadedPicture::Region { baseline: Revision::new(1), origin: [0, 2], image: square(SMALL, shade(200)) })
        });
    });
    jobs.run_all();
    frame(&mut frames, || {
        let state = loader.ask(&ctx, &1, Revision::new(2), Priority::VISIBLE).expect("settled");
        match state {
            TextureState::Ready(tex) => {
                assert_eq!(tex.id, orig_id, "the texture handle is patched in place (id stays)");
                assert_eq!(tex.size, PixelSize::new(LARGE as u32, LARGE as u32));
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    });
    assert!(matches!(loader.cache().state(&ctx, &1, Revision::new(2)), TextureState::Ready(_)));
    assert!(matches!(loader.cache().state(&ctx, &1, Revision::new(1)), TextureState::Loading { stand_in: Some(_), .. }));
}

struct TestPatchSource {
    seen_patchable: Cell<bool>,
    seen_resident_revision: Cell<Option<Revision>>,
    render_count: Cell<usize>,
}

impl ThumbSource<u32> for TestPatchSource {
    fn revision(&self, _key: &u32, _request: &ThumbRequest) -> Revision {
        Revision::new(self.render_count.get() as u64)
    }

    fn render(&self, _key: &u32, request: &ThumbRequest) -> ThumbRender {
        self.seen_patchable.set(request.patchable);
        self.seen_resident_revision.set(request.resident_revision);
        let count = self.render_count.get();
        self.render_count.set(count + 1);
        if request.patchable {
            ThumbRender::Region { origin: [0, 2], image: square(SMALL, shade(220)) }
        } else {
            ThumbRender::Ready(square(LARGE, shade(50)))
        }
    }
}

#[trial]
fn thumbnails_routes_region_render_in_place_and_reports_patchable() {
    let frames = Frames::new();
    let ctx = frames.ctx.clone();
    let thumbs = Thumbnails::<u32>::in_context(&ctx, "test-patch", ThumbnailOptions::new());
    let source = TestPatchSource { seen_patchable: Cell::new(false), seen_resident_revision: Cell::new(None), render_count: Cell::new(0) };
    let req = ThumbRequest::for_cell(LARGE as f32, 1.0);

    // Initial render: patchable is false, resident_revision is None
    let initial = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
    assert!(!source.seen_patchable.get(), "first pass is not patchable");
    assert_eq!(source.seen_resident_revision.get(), None, "first pass has no resident revision");
    let orig_id = match initial {
        TextureState::Ready(tex) => tex.id,
        other => panic!("expected Ready, got {other:?}"),
    };

    // Second render: patchable is true and Region patches in place
    let patched = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
    assert!(source.seen_patchable.get(), "second pass is patchable");
    assert!(source.seen_resident_revision.get().is_some(), "second pass reports resident revision");
    match patched {
        TextureState::Ready(tex) => {
            assert_eq!(tex.id, orig_id, "texture patched in place");
            assert_eq!(tex.size, PixelSize::new(LARGE as u32, LARGE as u32));
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

struct PendingBThenInstalledCSource {
    phase: Cell<usize>,
}

impl ThumbSource<u32> for PendingBThenInstalledCSource {
    fn revision(&self, _key: &u32, _request: &ThumbRequest) -> Revision {
        Revision::new(self.phase.get() as u64)
    }

    fn render(&self, _key: &u32, request: &ThumbRequest) -> ThumbRender {
        match self.phase.get() {
            1 => ThumbRender::Ready(square(LARGE, shade(10))),
            2 => ThumbRender::Later(Box::new(|_| Ok(LoadedPicture::Image(square(LARGE, shade(100)))))),
            3 => {
                assert!(request.patchable, "phase 3 must see resident texture");
                assert!(request.resident_revision.is_some(), "phase 3 has resident revision");
                ThumbRender::Region { origin: [0, 2], image: square(SMALL, shade(200)) }
            }
            _ => unreachable!(),
        }
    }
}

#[trial]
fn pending_job_does_not_overwrite_installed_region_update() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let jobs = Arc::new(ManualJobs::new());
    let opts = ThumbnailOptions::new().jobs(jobs.clone());
    let thumbs = Thumbnails::<u32>::in_context(&ctx, "test-pending-b", opts);
    let source = PendingBThenInstalledCSource { phase: Cell::new(1) };
    let req = ThumbRequest::for_cell(LARGE as f32, 1.0);

    // Frame 1: Install A (rev 1) synchronously
    let mut orig_id = None;
    frame(&mut frames, || {
        let state = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
        if let TextureState::Ready(tex) = state {
            orig_id = Some(tex.id);
        }
    });
    let orig_id = orig_id.expect("phase 1 ready");
    let rev_a = thumbs.cache().texture_revision(&ctx, &1).expect("rev a");

    // Frame 2: Request B (rev 2), which queues a background job in `jobs`
    source.phase.set(2);
    frame(&mut frames, || {
        let state = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
        assert!(matches!(state, TextureState::Loading { stand_in: Some(_), .. }), "phase 2 is loading stand-in");
    });
    // Note: jobs.run_all() is NOT called yet. Job B is pending!

    // Frame 3: While job B is still pending, an edit C (rev 3) arrives and renders a Region update
    source.phase.set(3);
    frame(&mut frames, || {
        let state = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
        match state {
            TextureState::Ready(tex) => {
                assert_eq!(tex.id, orig_id, "texture patched in place");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    });
    let rev_c = thumbs.cache().texture_revision(&ctx, &1).expect("rev c");
    assert_ne!(rev_a, rev_c, "revision changed to C");

    // Now job B finishes late in the worker thread:
    jobs.run_all();

    // Drain finished work while held so state() cannot re-render C, and verify job B did NOT overwrite rev C:
    thumbs.hold(true);
    frame(&mut frames, || {
        let _ = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
    });
    assert_eq!(thumbs.cache().texture_revision(&ctx, &1), Some(rev_c), "finished job B did not overwrite revision C before re-render");
    thumbs.hold(false);

    // Frame 5: Run next frame. Verify rev C is still resident!
    frame(&mut frames, || {
        let state = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
        match state {
            TextureState::Ready(tex) => {
                assert_eq!(tex.id, orig_id, "texture is still rev C");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    });
    assert_eq!(thumbs.cache().texture_revision(&ctx, &1), Some(rev_c), "job B did not overwrite revision C");
}

#[trial]
fn put_region_checked_atomically_validates_baseline() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let (loader, jobs) = loader_with(&ctx, 1, 16);

    // Put initial full picture at revision 1
    frame(&mut frames, || {
        loader.request(&ctx, &1, Revision::new(1), Priority::VISIBLE, || move |_| Ok(LoadedPicture::Image(square(LARGE, shade(10)))));
    });
    jobs.run_all();
    frame(&mut frames, || {
        loader.ask(&ctx, &1, Revision::new(1), Priority::VISIBLE);
    });
    assert_eq!(loader.cache().texture_revision(&ctx, &1), Some(Revision::new(1)));

    // Try put_region_checked with mismatched baseline (expecting revision 999 instead of 1)
    let res = loader.cache().put_region_checked(&ctx, &1, Some(Revision::new(999)), Revision::new(2), [0, 0], square(SMALL, shade(200)));
    assert!(res.is_none(), "mismatched baseline must be rejected");
    assert_eq!(loader.cache().texture_revision(&ctx, &1), Some(Revision::new(1)), "texture revision unchanged");

    // Try put_region_checked with matching baseline (expecting revision 1)
    let res = loader.cache().put_region_checked(&ctx, &1, Some(Revision::new(1)), Revision::new(2), [0, 0], square(SMALL, shade(200)));
    assert!(res.is_some(), "matching baseline is accepted");
    assert_eq!(loader.cache().texture_revision(&ctx, &1), Some(Revision::new(2)), "texture revision advanced to 2");

    // 5-argument put_region compatibility check
    let res = loader.cache().put_region(&ctx, &1, Revision::new(3), [0, 0], square(SMALL, shade(150)));
    assert!(res.is_some(), "5-arg put_region succeeds");
    assert_eq!(loader.cache().texture_revision(&ctx, &1), Some(Revision::new(3)), "texture revision advanced to 3");
}

#[trial]
fn cancellation_purges_already_finished_job_from_queue() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let (loader, jobs) = loader_with(&ctx, 1, 16);

    // Frame 1: Submit a job for key 1 at revision 1 that fails
    frame(&mut frames, || {
        loader.submit(&ctx, 1, Revision::new(1), Priority::VISIBLE, |_| Err(LoadFailure::Unreadable));
    });

    // Worker thread completes the job between frames, placing result in the queue buffer
    jobs.run_all();

    // Cancel key 1 before poll / run_frame_work drains the queue
    loader.cancel(&ctx, &1);

    // Frame 2: Submit a replacement job for key 1 at revision 1 that succeeds
    frame(&mut frames, || {
        loader.submit(&ctx, 1, Revision::new(1), Priority::VISIBLE, |_| Ok(LoadedPicture::Image(square(LARGE, shade(100)))));
    });
    jobs.run_all();

    // Verify replacement succeeds and the stale failure from the cancelled job was purged
    frame(&mut frames, || {
        let state = loader.ask(&ctx, &1, Revision::new(1), Priority::VISIBLE).expect("settled");
        assert!(matches!(state, TextureState::Ready(_)), "replacement succeeded, got {state:?}");
    });
    assert_eq!(loader.cache().texture_revision(&ctx, &1), Some(Revision::new(1)));
}

struct FallbackOnRejectSource {
    phase: Cell<usize>,
    saw_fallback_request: Cell<bool>,
}

impl ThumbSource<u32> for FallbackOnRejectSource {
    fn revision(&self, _key: &u32, _request: &ThumbRequest) -> Revision {
        Revision::new(self.phase.get() as u64)
    }

    fn render(&self, _key: &u32, request: &ThumbRequest) -> ThumbRender {
        match self.phase.get() {
            1 => ThumbRender::Ready(square(LARGE, shade(10))),
            2 => {
                if request.patchable {
                    // Try returning a rejected region patch (out of bounds)
                    ThumbRender::Region { origin: [9999, 9999], image: square(SMALL, shade(200)) }
                } else {
                    self.saw_fallback_request.set(true);
                    assert_eq!(request.resident_revision, None);
                    ThumbRender::Ready(square(LARGE, shade(200)))
                }
            }
            _ => unreachable!(),
        }
    }
}

#[trial]
fn patch_rejection_falls_back_to_full_render() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let thumbs = Thumbnails::<u32>::in_context(&ctx, "test-fallback", ThumbnailOptions::new());
    let source = FallbackOnRejectSource { phase: Cell::new(1), saw_fallback_request: Cell::new(false) };
    let req = ThumbRequest::for_cell(LARGE as f32, 1.0);

    // Frame 1: Install rev 1
    let mut rev1 = None;
    frame(&mut frames, || {
        let state = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
        assert!(matches!(state, TextureState::Ready(_)));
        rev1 = thumbs.cache().texture_revision(&ctx, &1);
    });
    let rev1 = rev1.expect("rev 1 installed");

    // Frame 2: Revision 2 attempts region patch, which fails and immediately falls back to full render
    source.phase.set(2);
    frame(&mut frames, || {
        let state = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
        assert!(matches!(state, TextureState::Ready(_)), "settles into Ready on fallback");
    });
    assert!(source.saw_fallback_request.get(), "producer received fallback request with patchable: false");
    let rev2 = thumbs.cache().texture_revision(&ctx, &1).expect("rev 2 installed");
    assert_ne!(rev1, rev2, "texture revision advanced on full render fallback");
}

#[trial]
fn downsampled_texture_is_not_patchable() {
    let frames = Frames::new();
    let ctx = frames.ctx.clone();
    ctx.input_mut(|i| i.max_texture_side = 256);
    let store = TextureStore::<u32>::in_context(&ctx, "test-downsample", TextureStoreOptions::thumbnails());
    let big_image = ColorImage::filled([512, 512], shade(50));
    store.put(&ctx, 1, Revision::new(1), big_image);
    assert!(store.has_texture(&ctx, &1), "has texture");
    assert!(!store.is_patchable(&ctx, &1), "downsampled texture (factor > 1) is not patchable");
}

#[trial]
fn full_put_and_checked_patch_interleaving_preserves_consistent_revision_and_pixels() {
    let frames = Frames::new();
    let ctx = frames.ctx.clone();
    let store = TextureStore::<u32>::in_context(&ctx, "test-put-patch-race", TextureStoreOptions::thumbnails());
    let rev_a = Revision::new(1);
    let rev_b = Revision::new(2);
    let rev_c = Revision::new(3);

    let initial = store.put(&ctx, 1, rev_a, square(LARGE, shade(10)));
    let tex_id = initial.id;
    ctx.tex_manager().write().take_delta().clear();
    let mut pixels = vec![shade(10); LARGE * LARGE];

    let apply_deltas = |pixels: &mut [Color32]| {
        let mut delta = ctx.tex_manager().write().take_delta();
        if let Some(deltas) = delta.set.get(&tex_id) {
            for img_delta in deltas {
                let egui::ImageData::Color(img) = &img_delta.image;
                let [w, h] = img.size;
                let [ox, oy] = img_delta.pos.unwrap_or([0, 0]);
                for y in 0..h {
                    for x in 0..w {
                        pixels[(oy + y) * LARGE + (ox + x)] = img.pixels[y * w + x];
                    }
                }
            }
        }
        delta.clear();
    };

    for _ in 0..24 {
        let reset = store.put(&ctx, 1, rev_a, square(LARGE, shade(10)));
        assert_eq!(reset.id, tex_id);
        apply_deltas(&mut pixels);

        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|s| {
            s.spawn(|| {
                barrier.wait();
                store.put(&ctx, 1, rev_b, square(LARGE, shade(100)));
            });
            s.spawn(|| {
                barrier.wait();
                let _ = store.put_region_checked(&ctx, &1, Some(rev_a), rev_c, [0, 0], square(SMALL, shade(200)));
            });
        });

        apply_deltas(&mut pixels);
        let final_rev = store.texture_revision(&ctx, &1).expect("resident revision");
        assert_eq!(final_rev, rev_b, "both orderings must end at revision B");
        assert!(pixels.iter().all(|&p| p == shade(100)), "Ready(B) must have pure B pixels, no mixed patch pixels");
    }
}

#[trial]
fn put_region_checked_rejects_reduced_texture() {
    let frames = Frames::new();
    let ctx = frames.ctx.clone();
    ctx.input_mut(|i| i.max_texture_side = 256);
    let store = TextureStore::<u32>::in_context(&ctx, "test-reduced-checked", TextureStoreOptions::thumbnails());
    let rev_1 = Revision::new(1);
    let rev_2 = Revision::new(2);
    store.put(&ctx, 1, rev_1, ColorImage::filled([512, 512], shade(50)));

    assert!(store.put_region_checked(&ctx, &1, Some(rev_1), rev_2, [0, 0], square(SMALL, shade(200))).is_none());
    assert_eq!(store.texture_revision(&ctx, &1), Some(rev_1));
    assert!(store.put_region(&ctx, &1, rev_2, [0, 0], square(SMALL, shade(200))).is_some());
    assert_eq!(store.texture_revision(&ctx, &1), Some(rev_2));
}

#[trial]
fn thumbnails_geometry_change_requests_full_render() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let thumbs = Thumbnails::<u32>::in_context(&ctx, "test-geom-change", ThumbnailOptions::new());
    let source = TestPatchSource { seen_patchable: Cell::new(false), seen_resident_revision: Cell::new(None), render_count: Cell::new(0) };

    let req_large = ThumbRequest::for_cell(LARGE as f32, 1.0);
    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req_large, Priority::VISIBLE), TextureState::Ready(_)));
    });
    assert!(!source.seen_patchable.get());

    // Same geometry is patchable
    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req_large, Priority::VISIBLE), TextureState::Ready(_)));
    });
    assert!(source.seen_patchable.get());

    // Different side is NOT patchable
    let req_other_side = ThumbRequest::for_cell((LARGE + 32) as f32, 1.0);
    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req_other_side, Priority::VISIBLE), TextureState::Ready(_)));
    });
    assert!(!source.seen_patchable.get());
    assert_eq!(source.seen_resident_revision.get(), None);

    // Different region crop is NOT patchable
    let req_crop = req_other_side.region(egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.5, 0.5)));
    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req_crop, Priority::VISIBLE), TextureState::Ready(_)));
    });
    assert!(!source.seen_patchable.get());
    assert_eq!(source.seen_resident_revision.get(), None);
}

#[trial]
fn thumbnails_rejected_patch_respects_render_budget() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let thumbs = Thumbnails::<u32>::in_context(&ctx, "test-budget-fallback", ThumbnailOptions::new().renders_per_pass(1));
    let source = FallbackOnRejectSource { phase: Cell::new(1), saw_fallback_request: Cell::new(false) };
    let req = ThumbRequest::for_cell(LARGE as f32, 1.0);

    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE), TextureState::Ready(_)));
    });

    // Phase 2: patch is rejected, budget (1) is exhausted, so fallback waits for the next pass
    source.phase.set(2);
    frame(&mut frames, || {
        let state = thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE);
        assert!(matches!(state, TextureState::Loading { stand_in: Some(_), .. }), "waits for next pass: {state:?}");
    });
    assert!(!source.saw_fallback_request.get(), "fallback did not run in the same pass");

    // Next pass: force_full gives patchable: false so full render runs
    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE), TextureState::Ready(_)));
    });
    assert!(source.saw_fallback_request.get(), "fallback ran on next pass");
}

#[trial]
fn loader_patch_rejected_reports_rejection_without_unpatching_newer_entry() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let (loader, jobs) = loader_with(&ctx, 1, 16);
    let rev_1 = Revision::new(1);
    let rev_2 = Revision::new(2);
    let rev_3 = Revision::new(3);

    loader.cache().put(&ctx, 1, rev_1, square(LARGE, shade(10)));
    frame(&mut frames, || {
        loader.request(&ctx, &1, rev_2, Priority::VISIBLE, || {
            move |_| Ok(LoadedPicture::Region { baseline: rev_1, origin: [0, 0], image: square(SMALL, shade(200)) })
        });
    });
    // Newer texture rev_3 lands while rev_2's region job is in flight (via another path)
    loader.cache().put(&ctx, 1, rev_3, square(LARGE, shade(50)));
    assert!(loader.cache().is_patchable(&ctx, &1));

    jobs.run_all();
    assert!(loader.patch_rejected(&ctx, &1, rev_2), "loader polls finished work and reports rejected patch for rev_2");
    assert_eq!(loader.cache().texture_revision(&ctx, &1), Some(rev_3), "newer entry rev_3 preserved");
    assert!(loader.cache().is_patchable(&ctx, &1), "newer entry rev_3 stays patchable");
}

struct BackgroundGeomSource {
    phase: Cell<u64>,
    seen_patchable: Cell<bool>,
}

impl ThumbSource<u32> for BackgroundGeomSource {
    fn revision(&self, _key: &u32, _request: &ThumbRequest) -> Revision {
        Revision::new(self.phase.get())
    }

    fn render(&self, _key: &u32, request: &ThumbRequest) -> ThumbRender {
        self.seen_patchable.set(request.patchable);
        let side = request.side as usize;
        if self.phase.get() == 1 {
            ThumbRender::Ready(square(side, shade(10)))
        } else if self.phase.get() == 2 {
            ThumbRender::Later(Box::new(move |_| Ok(LoadedPicture::Image(square(side, shade(50))))))
        } else if request.patchable {
            ThumbRender::Region { origin: [0, 0], image: square(SMALL, shade(200)) }
        } else {
            ThumbRender::Ready(square(side, shade(100)))
        }
    }
}

#[trial]
fn thumbnails_background_geometry_reconciles_before_next_request() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let jobs = Arc::new(ManualJobs::new());
    let thumbs = Thumbnails::<u32>::in_context(
        &ctx,
        "test-bg-geom-reconcile",
        ThumbnailOptions::new().max_side(512).jobs(Arc::clone(&jobs) as Arc<dyn craft_host::Jobs>),
    );
    let source = BackgroundGeomSource { phase: Cell::new(1), seen_patchable: Cell::new(false) };
    let req_small = ThumbRequest::for_cell(16.0, 1.0);
    let req_large = ThumbRequest::for_cell(64.0, 1.0);

    // Frame 1: install small texture
    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req_small, Priority::VISIBLE), TextureState::Ready(_)));
    });

    // Frame 2: queue large background render
    source.phase.set(2);
    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req_large, Priority::VISIBLE), TextureState::Loading { .. }));
    });
    jobs.run_all();

    // Frame 3: request small again without ever asking for the large revision;
    // completed large job's geometry must reconcile before matches_geom is evaluated.
    source.phase.set(3);
    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req_small, Priority::VISIBLE), TextureState::Ready(_)));
    });
    assert!(!source.seen_patchable.get(), "small request must not patch against newly installed large texture");
}

#[trial]
fn packed_cancelled_job_does_not_publish_over_replacement() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let jobs = Arc::new(ManualJobs::new());
    let loader = TextureLoader::<u32>::in_context(
        &ctx,
        "test-packed-cancel",
        LoaderOptions::new()
            .store(TextureStoreOptions::thumbnails().packed(64))
            .jobs(Arc::clone(&jobs) as Arc<dyn craft_host::Jobs>),
    );
    let rev = Revision::new(1);

    // Submit a packed-size job, then cancel and replace at the exact same revision before it runs.
    frame(&mut frames, || {
        loader.submit(&ctx, 1, rev, Priority::VISIBLE, |_| Ok(LoadedPicture::Image(square(SMALL, shade(10)))));
    });
    loader.cancel(&ctx, &1);
    frame(&mut frames, || {
        loader.submit(&ctx, 1, rev, Priority::VISIBLE, |_| Ok(LoadedPicture::Image(square(SMALL, shade(200)))));
    });
    jobs.run_all();
    frame(&mut frames, || {
        assert!(matches!(loader.ask(&ctx, &1, rev, Priority::VISIBLE), Some(TextureState::Ready(_))));
    });
    assert!(matches!(loader.cache().state(&ctx, &1, rev), TextureState::Ready(_)));

    // Also verify put_unless_cancelled / packed cancellation stops before publishing when no replacement exists.
    let rev_2 = Revision::new(2);
    frame(&mut frames, || {
        loader.submit(&ctx, 2, rev_2, Priority::VISIBLE, |_| Ok(LoadedPicture::Image(square(SMALL, shade(50)))));
    });
    loader.cancel(&ctx, &2);
    jobs.run_all();
    frame(&mut frames, || {
        assert_eq!(loader.ask(&ctx, &2, rev_2, Priority::VISIBLE), None);
    });
    assert!(matches!(loader.cache().state(&ctx, &2, rev_2), TextureState::Missing));
}

#[trial]
fn put_region_checked_honors_mark_unpatchable_and_texture_size_reflects_reduction() {
    let frames = Frames::new();
    let ctx = frames.ctx.clone();
    ctx.input_mut(|i| i.max_texture_side = 256);
    let store = TextureStore::<u32>::in_context(&ctx, "test-unpatchable-and-size", TextureStoreOptions::thumbnails());
    let rev_1 = Revision::new(1);
    let rev_2 = Revision::new(2);

    store.put(&ctx, 1, rev_1, ColorImage::filled([512, 512], shade(50)));
    assert_eq!(store.texture_size(&ctx, &1), Some(PixelSize::new(256, 256)));

    store.put(&ctx, 2, rev_1, square(LARGE, shade(50)));
    store.mark_unpatchable(&ctx, &2);
    assert!(store.put_region_checked(&ctx, &2, Some(rev_1), rev_2, [0, 0], square(SMALL, shade(200))).is_none());
    assert_eq!(store.texture_revision(&ctx, &2), Some(rev_1));
}

#[trial]
fn thumbnails_geometry_invalidated_when_cache_put_directly() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let thumbs = Thumbnails::<u32>::in_context(&ctx, "test-direct-put-geom", ThumbnailOptions::new());
    let source = TestPatchSource { seen_patchable: Cell::new(false), seen_resident_revision: Cell::new(None), render_count: Cell::new(0) };
    let req = ThumbRequest::for_cell(LARGE as f32, 1.0);

    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE), TextureState::Ready(_)));
    });
    // Replace directly in the store at a different revision
    thumbs.cache().put(&ctx, 1, Revision::new(999), square(64, shade(90)));

    frame(&mut frames, || {
        assert!(matches!(thumbs.state(&ctx, &source, &1, &req, Priority::VISIBLE), TextureState::Ready(_)));
    });
    assert!(!source.seen_patchable.get(), "direct store put must invalidate old geometry");
}
