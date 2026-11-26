//! `FileThumbSource`: a `ThumbSource` over files, decoded by the app's `ThumbDecoder` in jobs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use craft_host::queue::Cancel;
use craft_ui::primitives::textures::{LoadFailure, LoadedPicture, Revision, ThumbRender, ThumbRequest, ThumbSource};
use egui::ColorImage;

use super::{DecodeRequest, DecodedThumb, FileThumbKey, StoreKey, ThumbDecoder, ThumbError, ThumbStore};

/// How a [`FileThumbSource`] uses the decoder and the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileThumbOptions {
    /// Ask [`ThumbDecoder::quick`] first and show its picture until `decode` lands. Default on.
    pub quick_previews: bool,
    /// Save what `decode` makes to the store. Default on.
    pub save_to_store: bool,
}

impl Default for FileThumbOptions {
    fn default() -> Self {
        Self { quick_previews: true, save_to_store: true }
    }
}

/// Where a key's pictures are: before the first job, after a quick preview, or done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Fresh,
    NeedsFull,
    Done,
}

/// File thumbnails as a [`ThumbSource`]: keyed by [`FileThumbKey`] directly, or by the widget's
/// own key through [`keyed`](Self::keyed). App state, one per library; cloning shares it.
///
/// - `revision`: `Revision::of(&(generation, full))`, where `generation` counts the
///   [`invalidate`](Self::invalidate) calls for the path and [`clear`](Self::clear) calls, and
///   `full` is 1 once a quick preview has landed and the full picture is wanted (0 before, and
///   when the first job already made the full picture). So the first picture is never asked
///   twice, and after a quick preview the cache keeps it as the stand-in while the full job runs.
/// - Stages are kept per `(key, side)`, and a job records its stage when its picture is made
///   (after the buffer check), so a zoom change starts from the store and the quick preview again.
/// - `render` always returns `ThumbRender::Later`: statting (length and modification time),
///   store reads, decoding and saving happen in the job, never on the UI thread. The first job
///   stats the file with [`ThumbDecoder::stat`] (failure: that error, `Missing` is
///   `LoadFailure::Missing`), loads the store at
///   `StoreKey { len, modified, side }` (a hit is final), else with `quick_previews` asks
///   `quick` (a picture: stage `NeedsFull`; the loader repaints when it stores the picture, so the next pass asks the
///   new revision), else decodes, saves, and is final. The second job (stage `NeedsFull`)
///   decodes and saves.
/// - `ThumbError::Unsupported` ends the job with `Ok(LoadedPicture::Blank)`, which the cache
///   stores as `Empty` (the tile shows the host's own icon); `Failed(message)` is the failure
///   tooltip (`LoadFailure::Message`); a decoder's own `Cancelled` and a buffer that is not
///   `width × height × 4` bytes are `LoadFailure::Unreadable`; a zero-sized picture is `Blank`. A failure is remembered by the cache for its revision, so it is not retried until
///   the file changes (`invalidate`) (the widget asks `Thumbnails::state` before it
///   paints, the source never draws).
#[derive(Clone)]
pub struct FileThumbSource {
    decoder: Arc<dyn ThumbDecoder>,
    store: Option<Arc<dyn ThumbStore>>,
    options: FileThumbOptions,
    generations: Arc<Mutex<HashMap<PathBuf, u64>>>,
    stages: Arc<Mutex<HashMap<(FileThumbKey, u32), Stage>>>,
    epoch: Arc<Mutex<u64>>,
}

impl FileThumbSource {
    /// A source over `decoder`, with the app's `store` if it has one.
    pub fn new(decoder: Arc<dyn ThumbDecoder>, store: Option<Arc<dyn ThumbStore>>) -> Self {
        Self { decoder, store, options: FileThumbOptions::default(), generations: Arc::default(), stages: Arc::default(), epoch: Arc::default() }
    }

    /// Replaces the options.
    #[must_use]
    pub fn options(self, options: FileThumbOptions) -> Self {
        Self { options, ..self }
    }

    /// The source seen through the widget's key: `locate` turns a tile key into the file's key
    /// (`None`: no file, the tile shows no picture). Built per pass; borrows `self`.
    pub fn keyed<K, F: Fn(&K) -> Option<FileThumbKey>>(&self, locate: F) -> KeyedFileThumbs<'_, F> {
        KeyedFileThumbs { source: self, locate }
    }

    /// `path` changed on disk: every variant's revision moves on, so the next pass decodes again
    /// and shows the old picture until the new one lands.
    pub fn invalidate(&self, path: &Path) {
        *lock(&self.generations).entry(path.to_path_buf()).or_insert(0) += 1;
        lock(&self.stages).retain(|(key, _), _| key.path != path);
    }

    /// Another library opened: every revision moves on and the stages are forgotten.
    pub fn clear(&self) {
        *lock(&self.epoch) += 1;
        lock(&self.stages).clear();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl FileThumbSource {
    /// The key's generation (path count plus library count) and its stage at `side`.
    fn state_of(&self, key: &FileThumbKey, side: u32) -> (u64, Stage) {
        let generation = self.state_of_generation(key);
        let stage = lock(&self.stages).get(&(key.clone(), side)).copied().unwrap_or(Stage::Fresh);
        (generation, stage)
    }
}

/// One decode job's inputs, moved onto the queue lane.
struct Job {
    decoder: Arc<dyn ThumbDecoder>,
    store: Option<Arc<dyn ThumbStore>>,
    options: FileThumbOptions,
    key: FileThumbKey,
    side: u32,
    stage: Stage,
    generation: u64,
    source: FileThumbSource,
}

impl Job {
    fn run(self, cancel: &Cancel) -> Result<LoadedPicture, LoadFailure> {
        match self.pictures(cancel) {
            Ok((thumb, stage)) => {
                let picture = to_image(thumb);
                // The stage moves with the picture, after the buffer check: a picture that fails
                // leaves the key where it was.
                if picture.is_ok() {
                    self.set_stage(stage);
                }
                picture
            }
            Err(ThumbError::Unsupported) => Ok(LoadedPicture::Blank),
            Err(ThumbError::Missing) => Err(LoadFailure::Missing),
            Err(ThumbError::Failed(message)) => Err(LoadFailure::Message(message)),
            // The queue discards a cancelled job's result; one that arrives is the decoder's own cancel.
            Err(ThumbError::Cancelled) => Err(LoadFailure::Unreadable),
        }
    }

    /// The picture and the stage the key reaches once it lands.
    fn pictures(&self, cancel: &Cancel) -> Result<(DecodedThumb, Stage), ThumbError> {
        let cancelled = || if cancel.is_cancelled() { Err(ThumbError::Cancelled) } else { Ok(()) };
        cancelled()?;
        let stat = self.decoder.stat(&self.key.path)?;
        let request = DecodeRequest { key: self.key.clone(), side: self.side, len: stat.len, modified: stat.modified };
        let store_key = StoreKey { key: self.key.clone(), len: request.len, modified: request.modified, side: self.side };
        if self.stage == Stage::Fresh {
            if let Some(thumb) = self.store.as_ref().and_then(|store| store.load(&store_key)) {
                return Ok((thumb, Stage::Done));
            }
            cancelled()?;
            if self.options.quick_previews
                && let Some(thumb) = self.decoder.quick(&request, cancel)
            {
                cancelled()?;
                return Ok((thumb, Stage::NeedsFull));
            }
        }
        cancelled()?;
        let thumb = self.decoder.decode(&request, cancel)?;
        cancelled()?;
        if self.options.save_to_store
            && let Some(store) = &self.store
        {
            store.save(&store_key, &thumb);
        }
        // After a quick preview the stage stays `NeedsFull`: the full picture is that revision's.
        Ok((thumb, if self.stage == Stage::Fresh { Stage::Done } else { self.stage }))
    }

    /// Records the key's stage unless `invalidate` or `clear` moved the generation meanwhile.
    fn set_stage(&self, stage: Stage) {
        let mut stages = lock(&self.source.stages);
        if self.source.state_of_generation(&self.key) == self.generation {
            stages.insert((self.key.clone(), self.side), stage);
        }
    }
}

impl FileThumbSource {
    fn state_of_generation(&self, key: &FileThumbKey) -> u64 {
        lock(&self.generations).get(&key.path).copied().unwrap_or(0).wrapping_add(*lock(&self.epoch) << 32)
    }
}

/// Turns a finished [`DecodedThumb`] into the picture the cache uploads. A zero-sized thumb is
/// `Blank` (nothing to show); a buffer that is not `width × height × 4` bytes is
/// [`LoadFailure::Unreadable`].
pub(super) fn to_image(thumb: DecodedThumb) -> Result<LoadedPicture, LoadFailure> {
    let (width, height) = (thumb.width as usize, thumb.height as usize);
    if width == 0 || height == 0 {
        return Ok(LoadedPicture::Blank);
    }
    if width.checked_mul(height).and_then(|pixels| pixels.checked_mul(4)) != Some(thumb.rgba.len()) {
        return Err(LoadFailure::Unreadable);
    }
    Ok(LoadedPicture::Image(ColorImage::from_rgba_unmultiplied([width, height], &thumb.rgba)))
}

impl ThumbSource<FileThumbKey> for FileThumbSource {
    fn revision(&self, key: &FileThumbKey, request: &ThumbRequest) -> Revision {
        let (generation, stage) = self.state_of(key, request.side);
        Revision::of(&(generation, u8::from(stage == Stage::NeedsFull)))
    }

    fn render(&self, key: &FileThumbKey, request: &ThumbRequest) -> ThumbRender {
        let (generation, stage) = self.state_of(key, request.side);
        let job = Job {
            decoder: Arc::clone(&self.decoder),
            store: self.store.clone(),
            options: self.options,
            key: key.clone(),
            side: request.side,
            stage,
            generation,
            source: self.clone(),
        };
        ThumbRender::Later(Box::new(move |cancel| job.run(cancel)))
    }
}

/// [`FileThumbSource::keyed`]: a [`ThumbSource`] over the widget's key.
pub struct KeyedFileThumbs<'a, F> {
    source: &'a FileThumbSource,
    locate: F,
}

impl<K, F: Fn(&K) -> Option<FileThumbKey>> ThumbSource<K> for KeyedFileThumbs<'_, F> {
    /// The located key's revision; `Revision::new(0)` when `locate` gives `None`.
    fn revision(&self, key: &K, request: &ThumbRequest) -> Revision {
        match (self.locate)(key) {
            Some(file) => self.source.revision(&file, request),
            None => Revision::new(0),
        }
    }

    /// The located key's render; `ThumbRender::Blank` when `locate` gives `None`.
    fn render(&self, key: &K, request: &ThumbRequest) -> ThumbRender {
        match (self.locate)(key) {
            Some(file) => self.source.render(&file, request),
            None => ThumbRender::Blank,
        }
    }
}
