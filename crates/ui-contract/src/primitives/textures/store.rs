//! `TextureStore<K>`: keyed textures with revisions, budgets and eviction, kept in runtime memory.

use super::limits::{fit_image, reduce_image, texels, texture_factor, texture_side};
use super::loader::LoadFailure;
use crate::runtime::egui::memory;
use crate::runtime::egui::pass;
use crate::runtime::egui::textures;
use craft_behavior::PixelSize;
use egui::load::SizedTexture;
use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions, Vec2};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::Arc;

/// What can key a stored texture: a layer id, a path, a `(document, page)` pair. Every type with these bounds is
/// one.
pub trait TextureKey: Clone + Eq + Hash + Send + Sync + 'static {}

impl<T: Clone + Eq + Hash + Send + Sync + 'static> TextureKey for T {}

/// The version of a key's picture, computed by the app: a stored fingerprint, an `Arc` address with a `Weak` guard,
/// a frame key. The library never reads pixels or compares images; a new revision means "this key's picture
/// changed", the same revision means "keep it".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Revision(u64);

impl Revision {
    /// The revision `value` (a document's own change counter).
    pub const fn new(value: u64) -> Revision {
        Revision(value)
    }

    /// The number.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The revision of anything hashable: `Revision::of(&(fingerprint, doc_width))`. Stable within one run of the
    /// app (std's `DefaultHasher` with its fixed keys), not across builds, so it is never stored.
    pub fn of(value: &impl Hash) -> Revision {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        Revision(hasher.finish())
    }
}

/// How much a store may hold and upload. Entries past `max_bytes` or `max_entries` are evicted; `uploads_per_pass`
/// and `upload_bytes_per_pass` are the per-frame allowance a `TextureLoader` keeps to (`TextureStore::allow_upload`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextureBudget {
    /// The texture bytes held, 4 per texel.
    pub max_bytes: usize,
    /// The entries held, pictures, empty and failed ones alike.
    pub max_entries: usize,
    /// Uploads a loader makes per frame.
    pub uploads_per_pass: usize,
    /// Bytes a loader uploads per frame.
    pub upload_bytes_per_pass: usize,
}

const MIB: usize = 1024 * 1024;
const UPLOADS_PER_PASS: usize = 16;
const UPLOAD_BYTES_PER_PASS: usize = 16 * MIB;

impl TextureBudget {
    /// Holds `max_bytes` and `max_entries`; a loader uploads 16 pictures and 16 MiB per frame.
    pub const fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self { max_bytes, max_entries, uploads_per_pass: UPLOADS_PER_PASS, upload_bytes_per_pass: UPLOAD_BYTES_PER_PASS }
    }

    /// Other upload limits per frame.
    #[must_use]
    pub const fn uploads(mut self, count: usize, bytes: usize) -> Self {
        self.uploads_per_pass = count;
        self.upload_bytes_per_pass = bytes;
        self
    }
}

/// A store's settings, read when the store is created (`TextureStore::in_context`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextureStoreOptions {
    /// The store's size limits.
    pub budget: TextureBudget,
    /// The sampling of every texture: `TextureOptions::LINEAR` by default; the clipping overlay and pixel-exact
    /// previews use `TextureOptions::NEAREST`.
    pub filter: TextureOptions,
}

impl TextureStoreOptions {
    /// Row and grid thumbnails: 32 MiB and 512 entries.
    pub const fn thumbnails() -> Self {
        Self::with_budget(32 * MIB, 512)
    }

    /// Document pages: 128 MiB and 64 entries.
    pub const fn pages() -> Self {
        Self::with_budget(128 * MIB, 64)
    }

    /// Video frames and other named images: 64 MiB and 512 entries.
    pub const fn frames() -> Self {
        Self::with_budget(64 * MIB, 512)
    }

    const fn with_budget(max_bytes: usize, max_entries: usize) -> Self {
        Self { budget: TextureBudget::new(max_bytes, max_entries), filter: TextureOptions::LINEAR }
    }

    /// The sampling of every texture.
    #[must_use]
    pub const fn filter(mut self, filter: TextureOptions) -> Self {
        self.filter = filter;
        self
    }

    /// Other limits.
    #[must_use]
    pub const fn budget(mut self, budget: TextureBudget) -> Self {
        self.budget = budget;
        self
    }
}

impl Default for TextureStoreOptions {
    /// `thumbnails`.
    fn default() -> Self {
        Self::thumbnails()
    }
}

/// A stored picture as a painter draws it: `painter.image(texture.id, rect, uv, tint)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CachedTexture {
    /// The egui texture to paint.
    pub id: TextureId,
    /// The texture's size in texels (reduced when the image was past the device limit).
    pub size: PixelSize,
}

impl CachedTexture {
    /// `size` as a vector, for fitting: only the aspect matters to a cell.
    pub fn size_vec2(&self) -> Vec2 {
        Vec2::new(self.size.width as f32, self.size.height as f32)
    }

    /// The texture with its size, the form `PreviewPane` and egui's image widgets take.
    pub fn sized(&self) -> SizedTexture {
        SizedTexture::new(self.id, self.size_vec2())
    }
}

/// What a key shows for a revision, from `TextureStore::state`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TextureState {
    /// Nothing is stored for this key, or only an empty or failed entry of another revision.
    Missing,
    /// The picture for this revision is on its way.
    #[non_exhaustive]
    Loading {
        /// The key's picture of another revision, shown until the new one lands.
        stand_in: Option<CachedTexture>,
    },
    /// The picture for this revision.
    Ready(CachedTexture),
    /// This revision has no picture (a group row): nothing is drawn and nothing is asked again.
    Empty,
    /// Making this revision's picture failed; it is not asked again until the revision changes.
    #[non_exhaustive]
    Failed {
        /// Why; the painter words it with `LoadFailure::text` at run time.
        failure: LoadFailure,
    },
}

impl TextureState {
    /// The picture to draw now: the ready one, or the stand-in while loading.
    pub fn shown(&self) -> Option<CachedTexture> {
        match self {
            TextureState::Ready(texture) | TextureState::Loading { stand_in: Some(texture), .. } => Some(*texture),
            _ => None,
        }
    }
}

/// What a store holds and did, for automation, stories and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextureStatistics {
    /// Entries held: pictures, empty and failed ones.
    pub entries: usize,
    /// Texture bytes held.
    pub bytes: usize,
    /// Uploads this frame.
    pub uploads: usize,
    /// Bytes uploaded this frame.
    pub upload_bytes: usize,
    /// Entries evicted since the store was made.
    pub evicted: usize,
}

/// One key's picture.
enum Picture {
    Texture {
        handle: TextureHandle,
        /// The size of the image put, before any reduction.
        source: PixelSize,
        /// The whole factor `fit_image` reduced it by: 1 when it fitted.
        factor: u32,
    },
    Empty,
    Failed(LoadFailure),
}

struct Entry {
    revision: Revision,
    picture: Picture,
    /// The last frame `state`, `put` or a paint saw it.
    seen: u64,
}

struct Inner<K> {
    options: TextureStoreOptions,
    entries: HashMap<K, Entry>,
    bytes: usize,
    /// The frame the upload counters are for, and the counters.
    upload_frame: u64,
    uploads: usize,
    upload_bytes: usize,
    evicted: usize,
    /// Debug names of the egui textures (`"<salt>/<n>"`).
    name: Arc<str>,
    next_name: u64,
    /// Pictures removed while the store was locked; their textures are freed after the lock is released.
    doomed: Vec<Picture>,
}

/// The stores of one context and key type, by salt.
struct Stores<K> {
    by_salt: HashMap<u64, Inner<K>>,
}

impl<K> Default for Stores<K> {
    fn default() -> Self {
        Self { by_salt: HashMap::new() }
    }
}

/// Textures by key and revision, with byte and entry budgets: the one place an app's pictures become egui
/// textures. The app never holds a `TextureHandle`. The state lives in runtime memory of the context under the
/// salt; the value is a cheap handle that clones share.
///
/// - **Revisions.** `state` answers for a key and the revision the app computes now. A new revision keeps the old
///   picture as the stand-in until the new one is put.
/// - **Upload.** `put` replaces an existing texture in place (same `TextureId`, so painters holding the id last
///   frame keep working) and uploads a new one otherwise. An image past the device limit (`texture_side`) is
///   reduced first (`fit_image`).
/// - **Eviction**, after every put: least recently seen first, until both the bytes and the entry count are within
///   the budget, but never an entry seen in this frame or the previous one; what is in view stays even past the
///   budget.
/// - **One texture per entry**, no atlas.
///
/// The icon path is `TextureCache`; the store keeps pictures that change and that the app names.
pub struct TextureStore<K: TextureKey> {
    salt: u64,
    name: Arc<str>,
    options: TextureStoreOptions,
    key: PhantomData<fn() -> K>,
}

impl<K: TextureKey> Clone for TextureStore<K> {
    fn clone(&self) -> Self {
        Self { salt: self.salt, name: Arc::clone(&self.name), options: self.options, key: PhantomData }
    }
}

impl<K: TextureKey> TextureStore<K> {
    /// The store of `ctx` under `salt`, made with `options` the first time. Later calls with the same salt get the
    /// same store and ignore `options`.
    pub fn in_context(ctx: &Context, salt: impl Hash + Debug, options: TextureStoreOptions) -> Self {
        let mut hasher = DefaultHasher::new();
        salt.hash(&mut hasher);
        let probe = Self { salt: hasher.finish(), name: format!("{salt:?}").into(), options, key: PhantomData };
        let options = probe.with(ctx, |inner| inner.options);
        Self { options, ..probe }
    }

    /// The hash of the salt: the key of this store's state in runtime memory, shared by the loader beside it.
    pub(super) fn salt(&self) -> u64 {
        self.salt
    }

    /// Runs `f` on the store's state under the service lock, then frees the textures `f` removed. `f` makes no
    /// call that reaches the context.
    fn with<R>(&self, ctx: &Context, f: impl FnOnce(&mut Inner<K>) -> R) -> R {
        let (out, doomed) = memory::with(ctx, |stores: &mut Stores<K>| {
            let inner = stores.by_salt.entry(self.salt).or_insert_with(|| Inner::new(Arc::clone(&self.name), self.options));
            let out = f(inner);
            (out, std::mem::take(&mut inner.doomed))
        });
        drop(doomed);
        out
    }

    /// What `key` shows for `revision` (see `TextureState`), and marks the entry seen this frame.
    /// - The entry has `revision`: `Ready`, `Empty` or `Failed`.
    /// - The entry has a picture of another revision: `Loading { stand_in: Some(..) }`.
    /// - No entry, or an empty or failed one of another revision: `Missing`.
    pub fn state(&self, ctx: &Context, key: &K, revision: Revision) -> TextureState {
        let frame = pass::root_frame_nr(ctx);
        self.with(ctx, |inner| inner.state(key, revision, frame))
    }

    /// Stores `image` as `key`'s picture at `revision` and returns it: in place when the key has a texture (same
    /// id), else a new texture with the store's filter. Reduced first when past `texture_side`. Counts as an upload
    /// this frame, marks the entry seen, then evicts. The image is sRGBA as egui's `ColorImage` holds it (built
    /// from unmultiplied values with `ColorImage::from_rgba_unmultiplied`).
    pub fn put(&self, ctx: &Context, key: K, revision: Revision, image: ColorImage) -> CachedTexture {
        let frame = pass::root_frame_nr(ctx);
        let side = texture_side(ctx);
        let source = texels(image.size);
        let factor = texture_factor(source, side);
        let image = fit_image(image, side);
        let size = texels(image.size);
        let (existing, filter, name) = self.with(ctx, |inner| {
            let existing = inner.handle_of(&key);
            let name = existing.is_none().then(|| inner.next_texture_name());
            (existing, inner.options.filter, name)
        });
        // The upload runs outside the service lock: it does not call back into the store.
        let handle = match (existing, name) {
            (Some(mut handle), _) => {
                textures::replace(&mut handle, image, filter);
                handle
            }
            (None, name) => textures::load(ctx, name.unwrap_or_default(), image, filter),
        };
        let texture = CachedTexture { id: handle.id(), size };
        let bytes = byte_size(size);
        self.with(ctx, |inner| {
            inner.roll_uploads(frame);
            inner.uploads += 1;
            inner.upload_bytes += bytes;
            inner.store(key, Entry { revision, picture: Picture::Texture { handle, source, factor }, seen: frame });
            inner.evict(frame);
        });
        texture
    }

    /// Updates part of `key`'s picture in place and moves the entry to `revision`: `image` goes at `origin`, both
    /// in the pixels of the image last put (the navigator's damage updates). The id stays. Returns `None` and
    /// changes nothing when the key has no picture, or when the region does not fit the stored image.
    ///
    /// An image that was reduced by the whole factor `f` on put is updated reduced the same way: the region's
    /// origin must be a multiple of `f`, and its far edges multiples of `f` or the image's own edges; it is
    /// box-averaged by `f` and written at `origin / f`. A region off that grid returns `None`, and the app puts the
    /// whole image.
    pub fn put_region(&self, ctx: &Context, key: &K, revision: Revision, origin: [u32; 2], image: ColorImage) -> Option<CachedTexture> {
        let frame = pass::root_frame_nr(ctx);
        let extent = texels(image.size);
        let (mut handle, source, factor, filter) = self.with(ctx, |inner| {
            inner.roll_uploads(frame);
            inner.region_target(key, origin, extent)
        })?;
        let sent = if factor > 1 { reduce_image(&image, factor) } else { image };
        let uploaded = byte_size(texels(sent.size));
        if uploaded > 0 {
            textures::replace_region(&mut handle, [origin[0] / factor, origin[1] / factor], sent, filter);
        }
        let texture = cached(&handle, source, factor);
        self.with(ctx, |inner| {
            let entry = inner.entries.get_mut(key)?;
            entry.revision = revision;
            entry.seen = frame;
            inner.uploads += 1;
            inner.upload_bytes += uploaded;
            Some(())
        })?;
        Some(texture)
    }

    /// `key`'s picture at `revision`, calling `make` and putting its image only when the entry has another revision
    /// or none. An `Empty` or `Failed` entry at `revision` is made again.
    pub fn get_or_put(&self, ctx: &Context, key: K, revision: Revision, make: impl FnOnce() -> ColorImage) -> CachedTexture {
        let frame = pass::root_frame_nr(ctx);
        let found = self.with(ctx, |inner| {
            let entry = inner.entries.get_mut(&key)?;
            if entry.revision != revision {
                return None;
            }
            let Picture::Texture { handle, source, factor } = &entry.picture else {
                return None;
            };
            entry.seen = frame;
            Some(cached(handle, *source, *factor))
        });
        match found {
            Some(texture) => texture,
            None => self.put(ctx, key, revision, make()),
        }
    }

    /// Records that `key` has no picture at `revision` (state `Empty`); frees its texture.
    pub fn put_empty(&self, ctx: &Context, key: K, revision: Revision) {
        self.put_marker(ctx, key, revision, Picture::Empty);
    }

    /// Records that making `key`'s picture at `revision` failed (state `Failed`), so nothing asks for it again
    /// until the revision changes; frees its texture.
    pub fn put_failure(&self, ctx: &Context, key: K, revision: Revision, failure: LoadFailure) {
        self.put_marker(ctx, key, revision, Picture::Failed(failure));
    }

    fn put_marker(&self, ctx: &Context, key: K, revision: Revision, picture: Picture) {
        let frame = pass::root_frame_nr(ctx);
        self.with(ctx, |inner| {
            inner.store(key, Entry { revision, picture, seen: frame });
            inner.evict(frame);
        });
    }

    /// Drops `key`'s entry and frees its texture.
    pub fn forget(&self, ctx: &Context, key: &K) {
        self.with(ctx, |inner| inner.remove(key));
    }

    /// Keeps only the entries whose key `keep` accepts (PhotoCraft's `prune_thumbs` when the open documents
    /// change). `keep` runs outside the store's lock.
    pub fn retain(&self, ctx: &Context, mut keep: impl FnMut(&K) -> bool) {
        let keys: Vec<K> = self.with(ctx, |inner| inner.entries.keys().cloned().collect());
        let dropped: Vec<K> = keys.into_iter().filter(|key| !keep(key)).collect();
        self.with(ctx, |inner| {
            for key in &dropped {
                inner.remove(key);
            }
        });
    }

    /// Drops every entry (another library opened).
    pub fn clear(&self, ctx: &Context) {
        self.with(ctx, |inner| {
            let all: Vec<K> = inner.entries.keys().cloned().collect();
            for key in &all {
                inner.remove(key);
            }
        });
    }

    /// Whether this frame's upload allowance has room for one more upload of `bytes`: fewer than `uploads_per_pass`
    /// uploads and at most `upload_bytes_per_pass` bytes so far, counting `bytes`. The first upload of a frame is
    /// always allowed, so an image larger than the byte allowance still lands. A loader asks before each put and
    /// leaves the rest for later frames; `put` itself always uploads.
    pub fn allow_upload(&self, ctx: &Context, bytes: usize) -> bool {
        let frame = pass::root_frame_nr(ctx);
        self.with(ctx, |inner| {
            inner.roll_uploads(frame);
            let budget = inner.options.budget;
            inner.uploads == 0
                || (inner.uploads < budget.uploads_per_pass && inner.upload_bytes.saturating_add(bytes) <= budget.upload_bytes_per_pass)
        })
    }

    /// What the store holds and uploaded this frame.
    pub fn statistics(&self, ctx: &Context) -> TextureStatistics {
        let frame = pass::root_frame_nr(ctx);
        self.with(ctx, |inner| {
            inner.roll_uploads(frame);
            TextureStatistics {
                entries: inner.entries.len(),
                bytes: inner.bytes,
                uploads: inner.uploads,
                upload_bytes: inner.upload_bytes,
                evicted: inner.evicted,
            }
        })
    }

    /// The options the store was made with.
    pub fn options(&self) -> TextureStoreOptions {
        self.options
    }
}

fn byte_size(size: PixelSize) -> usize {
    [size.width, size.height].into_iter().fold(4usize, |bytes, side| bytes.saturating_mul(side as usize))
}

/// The texel size of a texture made from an image of size `source` reduced by `factor`. The texture manager's own
/// size lags an in-place `set` until the frame ends.
fn texture_size(source: PixelSize, factor: u32) -> PixelSize {
    if factor <= 1 { source } else { PixelSize::new(source.width.div_ceil(factor).max(1), source.height.div_ceil(factor).max(1)) }
}

fn cached(handle: &TextureHandle, source: PixelSize, factor: u32) -> CachedTexture {
    CachedTexture { id: handle.id(), size: texture_size(source, factor) }
}

fn picture_bytes(picture: &Picture) -> usize {
    match picture {
        Picture::Texture { source, factor, .. } => byte_size(texture_size(*source, *factor)),
        Picture::Empty | Picture::Failed(_) => 0,
    }
}

impl<K: TextureKey> Inner<K> {
    fn new(name: Arc<str>, options: TextureStoreOptions) -> Self {
        Self {
            options,
            entries: HashMap::new(),
            bytes: 0,
            upload_frame: 0,
            uploads: 0,
            upload_bytes: 0,
            evicted: 0,
            name,
            next_name: 0,
            doomed: Vec::new(),
        }
    }

    fn state(&mut self, key: &K, revision: Revision, frame: u64) -> TextureState {
        let Some(entry) = self.entries.get_mut(key) else {
            return TextureState::Missing;
        };
        entry.seen = frame;
        let same = entry.revision == revision;
        match &entry.picture {
            Picture::Texture { handle, source, factor } if same => TextureState::Ready(cached(handle, *source, *factor)),
            Picture::Texture { handle, source, factor } => TextureState::Loading { stand_in: Some(cached(handle, *source, *factor)) },
            Picture::Empty if same => TextureState::Empty,
            Picture::Failed(failure) if same => TextureState::Failed { failure: failure.clone() },
            Picture::Empty | Picture::Failed(_) => TextureState::Missing,
        }
    }

    /// The texture handle of `key`, if it has a picture.
    fn handle_of(&self, key: &K) -> Option<TextureHandle> {
        match &self.entries.get(key)?.picture {
            Picture::Texture { handle, .. } => Some(handle.clone()),
            Picture::Empty | Picture::Failed(_) => None,
        }
    }

    /// The texture a region update of `extent` texels at `origin` writes into, with the source size, the whole
    /// reduction factor and the filter; `None` when the key has no picture or the region is off the image or off the
    /// reduction grid.
    fn region_target(&self, key: &K, origin: [u32; 2], extent: PixelSize) -> Option<(TextureHandle, PixelSize, u32, TextureOptions)> {
        let Picture::Texture { handle, source, factor } = &self.entries.get(key)?.picture else {
            return None;
        };
        let [x, y] = origin;
        let (w, h) = (extent.width, extent.height);
        if x.checked_add(w)? > source.width || y.checked_add(h)? > source.height {
            return None;
        }
        let f = *factor;
        let on_grid = |start: u32, len: u32, whole: u32| start.is_multiple_of(f) && ((start + len).is_multiple_of(f) || start + len == whole);
        if f > 1 && !(on_grid(x, w, source.width) && on_grid(y, h, source.height)) {
            return None;
        }
        Some((handle.clone(), *source, f, self.options.filter))
    }

    /// Starts the upload counters over when `frame` is a new frame.
    fn roll_uploads(&mut self, frame: u64) {
        if self.upload_frame != frame {
            self.upload_frame = frame;
            self.uploads = 0;
            self.upload_bytes = 0;
        }
    }

    fn next_texture_name(&mut self) -> String {
        self.next_name += 1;
        format!("{}/{}", self.name, self.next_name)
    }

    /// Replaces `key`'s entry, keeping the byte count.
    fn store(&mut self, key: K, entry: Entry) {
        self.bytes += picture_bytes(&entry.picture);
        if let Some(old) = self.entries.insert(key, entry) {
            self.bytes -= picture_bytes(&old.picture);
            self.doomed.push(old.picture);
        }
    }

    fn remove(&mut self, key: &K) -> bool {
        let Some(entry) = self.entries.remove(key) else {
            return false;
        };
        self.bytes -= picture_bytes(&entry.picture);
        self.doomed.push(entry.picture);
        true
    }

    fn within_budget(&self) -> bool {
        self.bytes <= self.options.budget.max_bytes && self.entries.len() <= self.options.budget.max_entries
    }

    /// Drops the least recently seen entries until the budget holds, never one seen in `frame` or the frame before.
    fn evict(&mut self, frame: u64) {
        if self.within_budget() {
            return;
        }
        let protected = frame.saturating_sub(1);
        let mut candidates: Vec<(u64, K)> =
            self.entries.iter().filter(|(_, entry)| entry.seen < protected).map(|(key, entry)| (entry.seen, key.clone())).collect();
        candidates.sort_by_key(|(seen, _)| *seen);
        for (_, key) in candidates {
            if self.within_budget() {
                break;
            }
            if self.remove(&key) {
                self.evicted += 1;
            }
        }
    }
}
