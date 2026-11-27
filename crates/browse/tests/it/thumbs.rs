//! `FileThumbSource` through `Thumbnails` with `ManualJobs` and a fake decoder.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use craft_browse::thumbs::{DecodeRequest, DecodedThumb, FileStat, FileThumbKey, FileThumbSource, ThumbDecoder, ThumbError};
use craft_host::ManualJobs;
use craft_host::queue::Cancel;
use craft_ui::primitives::textures::{Priority, TextureState, ThumbRequest, ThumbnailOptions, Thumbnails};
use craft_ui::testing::trial;
use egui::Context;

struct Decoder {
    /// Side of the full picture.
    full_side: u32,
    quick_side: Option<u32>,
    decoded: Mutex<Vec<PathBuf>>,
    quick_calls: AtomicUsize,
    /// Answer of the fake stat; the files the tests make are never read.
    stat: Mutex<Result<FileStat, ThumbError>>,
}

impl Decoder {
    fn new(quick_side: Option<u32>) -> Arc<Self> {
        Arc::new(Self {
            full_side: 4,
            quick_side,
            decoded: Mutex::default(),
            quick_calls: AtomicUsize::new(0),
            stat: Mutex::new(Ok(FileStat { len: 7, modified: None })),
        })
    }

    fn decodes(&self) -> usize {
        self.decoded.lock().unwrap().len()
    }
}

fn picture(side: u32) -> DecodedThumb {
    DecodedThumb { width: side, height: side, rgba: vec![255; (side * side * 4) as usize] }
}

impl ThumbDecoder for Decoder {
    fn stat(&self, _: &Path) -> Result<FileStat, ThumbError> {
        self.stat.lock().unwrap().clone()
    }

    fn decode(&self, request: &DecodeRequest, _: &Cancel) -> Result<DecodedThumb, ThumbError> {
        self.decoded.lock().unwrap().push(request.key.path.clone());
        Ok(picture(self.full_side))
    }

    fn quick(&self, _: &DecodeRequest, _: &Cancel) -> Option<DecodedThumb> {
        self.quick_calls.fetch_add(1, Ordering::Relaxed);
        self.quick_side.map(picture)
    }
}

struct Setup {
    ctx: Context,
    jobs: Arc<ManualJobs>,
    thumbs: Thumbnails<FileThumbKey>,
    dir: tempfile::TempDir,
}

impl Setup {
    fn new() -> Self {
        let ctx = Context::default();
        let jobs = Arc::new(ManualJobs::new());
        let options = ThumbnailOptions::new().lanes(1).jobs(jobs.clone());
        let thumbs = Thumbnails::in_context(&ctx, "files", options);
        Self { ctx, jobs, thumbs, dir: tempfile::tempdir().expect("temp dir") }
    }

    fn file(&self, name: &str) -> FileThumbKey {
        let path = self.dir.path().join(name);
        std::fs::write(&path, name).expect("write file");
        FileThumbKey::file(path)
    }

    fn pass(&self, run: impl FnOnce()) {
        let mut run = Some(run);
        self.ctx
            .run_ui(craft_ui::runtime::egui::input::RawInput::default(), |_| {
                if let Some(run) = run.take() {
                    run();
                }
            })
            .drop_without_applying_deltas();
    }

    /// Asks `key` at VISIBLE.
    fn show(&self, source: &FileThumbSource, key: &FileThumbKey) -> TextureState {
        let mut state = TextureState::Missing;
        self.pass(|| state = self.thumbs.state(&self.ctx, source, key, &ThumbRequest::for_cell(32.0, 1.0), Priority::VISIBLE));
        state
    }
}

fn side(state: &TextureState) -> Option<usize> {
    state.shown().map(|texture| texture.size.width as usize)
}

// A decoder with `quick`: the quick picture shows, then stands in while `decode` runs, then the full one.
#[trial]
fn file_source_shows_the_quick_preview_first() {
    let setup = Setup::new();
    let decoder = Decoder::new(Some(2));
    let source = FileThumbSource::new(decoder.clone(), None);
    let key = setup.file("a.png");
    setup.show(&source, &key);
    assert!(setup.jobs.run_next());
    assert_eq!(decoder.decodes(), 0);

    let state = setup.show(&source, &key);
    let TextureState::Loading { stand_in: Some(quick), .. } = state else { panic!("expected the quick picture as stand-in, got {state:?}") };
    assert_eq!(quick.size.width, 2);
    setup.jobs.run_all();
    let state = setup.show(&source, &key);
    assert!(matches!(state, TextureState::Ready(_)));
    assert_eq!(side(&state), Some(4));
    assert_eq!(state.shown().map(|texture| texture.id), Some(quick.id));
    assert_eq!((decoder.decodes(), decoder.quick_calls.load(Ordering::Relaxed)), (1, 1));
    // The full picture is final: no further job.
    setup.show(&source, &key);
    assert_eq!(setup.jobs.waiting(), 0);
}
