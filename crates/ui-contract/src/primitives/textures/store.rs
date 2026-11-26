//! `TextureStore<K>`: keyed textures with revisions, budgets and eviction, kept in runtime memory.

use super::limits::{fit_image, reduce_image, texels, texture_factor, texture_side};
use super::loader::LoadFailure;
use crate::runtime::egui::memory;
use crate::runtime::egui::pass;
use crate::runtime::egui::textures;
use craft_behavior::PixelSize;
use egui::load::SizedTexture;
use egui::{ColorImage, Context, Rect, TextureHandle, TextureId, TextureOptions, Vec2};
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
    /// The largest side of a picture the store puts on shared pages; `None` gives every picture a texture of its own.
    pub packed: Option<u32>,
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
        Self { budget: TextureBudget::new(max_bytes, max_entries), filter: TextureOptions::LINEAR, packed: None }
    }

    /// Puts pictures up to `max_side` texels per side on a few shared page textures written in place, so browsing
    /// many small pictures creates and frees no textures. When every page is full the store starts over the page
    /// drawn from longest ago, never one drawn from in this frame (its pictures are on screen); the pictures on it
    /// become `Missing` and are put again. When every page was drawn from in this frame, or a picture is larger than
    /// `max_side` or a page, the picture takes a texture of its own. A packed texture is drawn by
    /// [`CachedTexture::uv`].
    #[must_use]
    pub const fn packed(mut self, max_side: u32) -> Self {
        self.packed = Some(max_side);
        self
    }

    /// [`packed`](Self::packed) for the small pictures of an interface (a style tile, a font sample row): those up to a
    /// quarter of a page side share pages, so a few large ones never fill a page alone; a larger picture takes a
    /// texture of its own.
    #[must_use]
    pub const fn packed_small(self) -> Self {
        self.packed(PAGE_SIDE / 4)
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
    /// The picture's top-left texel on its page; `[0, 0]` for an unpacked texture.
    pub origin: [u32; 2],
    /// The size of the page texture the picture sits on; `size` for an unpacked texture.
    pub page: PixelSize,
}

impl CachedTexture {
    /// The picture's part of its page texture, in uv coordinates, to paint with `id`: the full rect for an unpacked
    /// texture.
    pub fn uv(&self) -> Rect {
        let (width, height) = (self.page.width.max(1) as f32, self.page.height.max(1) as f32);
        let [x, y] = self.origin;
        let (x, y) = (x as f32, y as f32);
        Rect::from_min_max(
            egui::pos2(x / width, y / height),
            egui::pos2((x + self.size.width as f32) / width, (y + self.size.height as f32) / height),
        )
    }

    /// `size` as a vector, for fitting: only the aspect matters to a cell. It is the picture's own size, also for a
    /// packed texture.
    pub fn size_vec2(&self) -> Vec2 {
        Vec2::new(self.size.width as f32, self.size.height as f32)
    }

    /// The texture with its size, the form `PreviewPane` and egui's image widgets take. Those draw the whole
    /// texture, so a texture of a packed store is drawn by [`uv`](Self::uv) instead.
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
        /// Seconds since the key's job was queued by the pass clock; `None` when the state does not know of a queued
        /// job (`Thumbnails::state` fills it).
        waited: Option<f64>,
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

impl std::ops::Add for TextureStatistics {
    type Output = TextureStatistics;

    /// The field-wise sum, for the statistics of several stores taken together.
    fn add(self, other: TextureStatistics) -> TextureStatistics {
        TextureStatistics {
            entries: self.entries + other.entries,
            bytes: self.bytes + other.bytes,
            uploads: self.uploads + other.uploads,
            upload_bytes: self.upload_bytes + other.upload_bytes,
            evicted: self.evicted + other.evicted,
        }
    }
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
    /// A picture on the shared page `page` of a packed store, its top-left texel at `origin`.
    Packed {
        page: usize,
        origin: [u32; 2],
        size: PixelSize,
    },
    Empty,
    Failed(LoadFailure),
}

/// The side of a packed store's square pages in texels, at most the device limit.
const PAGE_SIDE: u32 = 2048;
/// At most this many pages: enough for a frame's pictures at a high display scale, still a handful of textures.
const PAGE_COUNT: usize = 4;
/// Transparent texels between pictures, so linear filtering never bleeds one into the next.
const PAGE_GAP: u32 = 2;

/// A shared texture of a packed store, filled row by row.
struct Page {
    handle: TextureHandle,
    side: u32,
    /// Where the next picture goes: the current row's left edge and top, and the row's height.
    x: u32,
    y: u32,
    row_height: u32,
    /// The last frame a picture on the page was seen.
    seen: u64,
    /// The page started over and still holds the old pictures' texels; it is cleared before its next picture.
    stale: bool,
}

impl Page {
    /// A free spot for a picture of `size`, taken; `None` when the page has no room for it.
    fn place(&mut self, size: PixelSize) -> Option<[u32; 2]> {
        let (w, h) = (size.width, size.height);
        if w == 0 || h == 0 || w > self.side || h > self.side {
            return None;
        }
        let (mut x, mut y, mut row_height) = (self.x, self.y, self.row_height);
        if x.saturating_add(w) > self.side {
            x = 0;
            y = y.saturating_add(row_height).saturating_add(PAGE_GAP);
            row_height = 0;
        }
        if y.saturating_add(h) > self.side {
            return None;
        }
        self.x = x.saturating_add(w).saturating_add(PAGE_GAP);
        self.y = y;
        self.row_height = row_height.max(h);
        Some([x, y])
    }

    fn cached(&self, origin: [u32; 2], size: PixelSize) -> CachedTexture {
        CachedTexture { id: self.handle.id(), size, origin, page: PixelSize::new(self.side, self.side) }
    }
}

/// Where a packed picture goes, decided under the lock; the texture work runs after it.
enum Spot {
    /// At `origin` on the page `page`; `clear` when the page started over and its texture holds old pictures.
    On { page: usize, origin: [u32; 2], handle: TextureHandle, side: u32, clear: bool },
    /// On a new page, whose texture is named `name`.
    New { name: String },
}

/// A transparent page of `side` texels.
fn blank_page(side: u32) -> ColorImage {
    ColorImage::filled([side as usize; 2], egui::Color32::TRANSPARENT)
}

/// The picture `picture` shows, on its page when packed; `None` for an empty or failed entry.
fn shown(pages: &[Page], picture: &Picture) -> Option<CachedTexture> {
    match picture {
        Picture::Texture { handle, source, factor } => Some(cached(handle, *source, *factor)),
        Picture::Packed { page, origin, size } => pages.get(*page).map(|page| page.cached(*origin, *size)),
        Picture::Empty | Picture::Failed(_) => None,
    }
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
    /// The shared pages of a packed store; none for an unpacked one.
    pages: Vec<Page>,
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

/// The numbers that keep apart the textures one owner makes: a mask or preview made anew gets a number no earlier one
/// of the same `Serials` had, so an entry kept under its key is never taken for the new one's. The owner of what the
/// textures show (a tool's state, a model) holds the `Serials`. The numbers are unique per owner, not per context: a
/// texture store that several owners share keys each entry by the owner's component id as well as the serial. Plain
/// data, so it can sit in tracked or model state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Serials(u64);

impl Serials {
    /// A new number: never one this `Serials` gave before.
    pub fn mint(&mut self) -> u64 {
        self.0 += 1;
        self.0
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
/// - **One texture per entry**, unless the store is built with [`TextureStoreOptions::packed`]: then small pictures
///   share a few pages, each page's bytes counted once, and a page, not an entry, is what starts over.
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
    /// from unmultiplied values with `ColorImage::from_rgba_unmultiplied`). A packed store writes a small picture
    /// onto a shared page instead (see [`TextureStoreOptions::packed`]).
    pub fn put(&self, ctx: &Context, key: K, revision: Revision, image: ColorImage) -> CachedTexture {
        let frame = pass::root_frame_nr(ctx);
        let side = texture_side(ctx);
        let source = texels(image.size);
        let factor = texture_factor(source, side);
        let image = fit_image(image, side);
        let image = match self.put_packed(ctx, &key, revision, image, frame, side) {
            Ok(texture) => return texture,
            Err(image) => image,
        };
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
        let texture = CachedTexture { id: handle.id(), size, origin: [0, 0], page: size };
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

    /// Writes `image` onto a page of a packed store as `key`'s picture at `revision`; gives the image back when the
    /// store is unpacked, the picture is larger than `max_side` or a page, or no page has room and every page was
    /// drawn from in `frame` (the picture then takes a texture of its own, `put`'s ordinary path). Two passes of
    /// one frame share its number, so a page drawn from in the first pass is kept in the second.
    fn put_packed(&self, ctx: &Context, key: &K, revision: Revision, image: ColorImage, frame: u64, side: u32) -> Result<CachedTexture, ColorImage> {
        let size = texels(image.size);
        let page_side = PAGE_SIDE.min(side);
        let Some(max_side) = self.options.packed else {
            return Err(image);
        };
        if size.width == 0 || size.height == 0 || size.width.max(size.height) > max_side.min(page_side) {
            return Err(image);
        }
        let Some(spot) = self.with(ctx, |inner| inner.spot(size, frame, page_side)) else {
            return Err(image);
        };
        // Texture work runs outside the service lock, as `put`'s upload does.
        let (page, origin, mut handle, page_side) = match spot {
            Spot::On { page, origin, mut handle, side, clear } => {
                if clear {
                    textures::replace(&mut handle, blank_page(side), self.options.filter);
                }
                (page, origin, handle, side)
            }
            Spot::New { name } => {
                let handle = textures::load(ctx, name, blank_page(page_side), self.options.filter);
                let Some((page, origin)) = self.with(ctx, |inner| inner.add_page(handle.clone(), page_side, size)) else {
                    return Err(image);
                };
                (page, origin, handle, page_side)
            }
        };
        textures::replace_region(&mut handle, origin, image, self.options.filter);
        let bytes = byte_size(size);
        let texture = CachedTexture { id: handle.id(), size, origin, page: PixelSize::new(page_side, page_side) };
        self.with(ctx, |inner| {
            inner.roll_uploads(frame);
            inner.uploads += 1;
            inner.upload_bytes += bytes;
            if let Some(shared) = inner.pages.get_mut(page) {
                shared.seen = frame;
            }
            inner.store(key.clone(), Entry { revision, picture: Picture::Packed { page, origin, size }, seen: frame });
            inner.evict(frame);
        });
        Ok(texture)
    }

    /// Updates part of `key`'s picture in place and moves the entry to `revision`: `image` goes at `origin`, both
    /// in the pixels of the image last put (the navigator's damage updates). The id stays. Returns `None` and
    /// changes nothing when the key has no picture, or when the region does not fit the stored image.
    ///
    /// An image that was reduced by the whole factor `f` on put is updated reduced the same way: the region's
    /// origin must be a multiple of `f`, and its far edges multiples of `f` or the image's own edges; it is
    /// box-averaged by `f` and written at `origin / f`. A region off that grid returns `None`, and the app puts the
    /// whole image. A picture on a packed store's page also returns `None`, as for a key with no texture, so the
    /// caller puts the whole picture: packed pictures are small.
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
            let texture = shown(&inner.pages, &entry.picture)?;
            entry.seen = frame;
            if let Picture::Packed { page, .. } = entry.picture
                && let Some(shared) = inner.pages.get_mut(page)
            {
                shared.seen = frame;
            }
            Some(texture)
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

    /// Drops every entry (another library opened); a packed store's pages start over and keep their textures.
    pub fn clear(&self, ctx: &Context) {
        self.with(ctx, |inner| {
            let all: Vec<K> = inner.entries.keys().cloned().collect();
            for key in &all {
                inner.remove(key);
            }
            for page in 0..inner.pages.len() {
                inner.restart(page);
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
    let size = texture_size(source, factor);
    CachedTexture { id: handle.id(), size, origin: [0, 0], page: size }
}

/// The texture bytes `picture` holds on its own: none for a packed picture, whose page is counted once.
fn picture_bytes(picture: &Picture) -> usize {
    match picture {
        Picture::Texture { source, factor, .. } => byte_size(texture_size(*source, *factor)),
        Picture::Packed { .. } | Picture::Empty | Picture::Failed(_) => 0,
    }
}

impl<K: TextureKey> Inner<K> {
    fn new(name: Arc<str>, options: TextureStoreOptions) -> Self {
        Self {
            options,
            entries: HashMap::new(),
            pages: Vec::new(),
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
        if let Picture::Packed { page, .. } = entry.picture
            && let Some(shared) = self.pages.get_mut(page)
        {
            shared.seen = frame;
        }
        let same = entry.revision == revision;
        match (&entry.picture, shown(&self.pages, &entry.picture)) {
            (_, Some(texture)) if same => TextureState::Ready(texture),
            (_, Some(texture)) => TextureState::Loading { stand_in: Some(texture), waited: None },
            (Picture::Empty, _) if same => TextureState::Empty,
            (Picture::Failed(failure), _) if same => TextureState::Failed { failure: failure.clone() },
            _ => TextureState::Missing,
        }
    }

    /// How many pages a packed store keeps: `PAGE_COUNT`, fewer when the byte budget holds fewer pages of `side`,
    /// and at least one.
    fn page_limit(&self, side: u32) -> usize {
        let page = byte_size(PixelSize::new(side, side)).max(1);
        (self.options.budget.max_bytes / page).clamp(1, PAGE_COUNT)
    }

    /// Where a packed picture of `size` goes: the first page with room, else a new page while there are fewer than
    /// the limit, else the page seen longest ago started over. `None` when every page was seen in `frame`: none
    /// starts over while its pictures are on screen.
    fn spot(&mut self, size: PixelSize, frame: u64, side: u32) -> Option<Spot> {
        let on = |index: usize, page: &mut Page| {
            page.place(size).map(|origin| Spot::On {
                page: index,
                origin,
                handle: page.handle.clone(),
                side: page.side,
                clear: std::mem::take(&mut page.stale),
            })
        };
        if let Some(spot) = self.pages.iter_mut().enumerate().find_map(|(index, page)| on(index, page)) {
            return Some(spot);
        }
        if self.pages.len() < self.page_limit(side) {
            return Some(Spot::New { name: self.next_texture_name() });
        }
        let index = self.pages.iter().enumerate().filter(|(_, page)| page.seen < frame).min_by_key(|(_, page)| page.seen).map(|(index, _)| index)?;
        self.restart(index);
        on(index, self.pages.get_mut(index)?)
    }

    /// Adds a page with `handle`, `side` texels square, and takes a spot for a picture of `size` on it.
    fn add_page(&mut self, handle: TextureHandle, side: u32, size: PixelSize) -> Option<(usize, [u32; 2])> {
        let mut page = Page { handle, side, x: 0, y: 0, row_height: 0, seen: 0, stale: false };
        let origin = page.place(size)?;
        self.bytes += byte_size(PixelSize::new(side, side));
        self.pages.push(page);
        Some((self.pages.len() - 1, origin))
    }

    /// Starts the page `index` over: its layout is forgotten, its pictures become `Missing`, and its texture is
    /// cleared before the next picture goes on it.
    fn restart(&mut self, index: usize) {
        let Some(page) = self.pages.get_mut(index) else {
            return;
        };
        (page.x, page.y, page.row_height, page.stale) = (0, 0, 0, true);
        let before = self.entries.len();
        self.entries.retain(|_, entry| !matches!(entry.picture, Picture::Packed { page: on, .. } if on == index));
        self.evicted += before - self.entries.len();
    }

    /// The texture handle of `key`, if it has a texture of its own; a packed picture's page is shared, so it has none.
    fn handle_of(&self, key: &K) -> Option<TextureHandle> {
        match &self.entries.get(key)?.picture {
            Picture::Texture { handle, .. } => Some(handle.clone()),
            Picture::Packed { .. } | Picture::Empty | Picture::Failed(_) => None,
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

    /// Drops the least recently seen units until the budget holds, never one seen in `frame` or the frame before. A
    /// unit is an entry with a texture of its own, an empty or failed entry, or a whole page of a packed store with
    /// its pictures: a packed picture holds no bytes, so it goes alone only for the entry count, while the bytes are
    /// within budget.
    fn evict(&mut self, frame: u64) {
        if self.within_budget() {
            return;
        }
        let protected = frame.saturating_sub(1);
        let mut units: Vec<(u64, Unit<K>)> =
            self.entries.iter().filter(|(_, entry)| entry.seen < protected).map(|(key, entry)| (entry.seen, Unit::Entry(key.clone()))).collect();
        units.extend(self.pages.iter().filter(|page| page.seen < protected).map(|page| (page.seen, Unit::Page(page.handle.id()))));
        units.sort_by_key(|(seen, _)| *seen);
        for (_, unit) in units {
            if self.within_budget() {
                break;
            }
            match unit {
                Unit::Entry(key) => {
                    let packed = matches!(self.entries.get(&key).map(|entry| &entry.picture), Some(Picture::Packed { .. }));
                    if packed && self.bytes > self.options.budget.max_bytes {
                        continue;
                    }
                    if self.remove(&key) {
                        self.evicted += 1;
                    }
                }
                Unit::Page(id) => {
                    if let Some(index) = self.pages.iter().position(|page| page.handle.id() == id) {
                        self.drop_page(index);
                    }
                }
            }
        }
    }

    /// Removes the page `index` with its pictures and frees its texture. The last page takes its place in the list,
    /// so the pictures on it follow.
    fn drop_page(&mut self, index: usize) {
        self.restart(index);
        let page = self.pages.swap_remove(index);
        let side = PixelSize::new(page.side, page.side);
        self.bytes -= byte_size(side);
        let moved = self.pages.len();
        if index != moved {
            for entry in self.entries.values_mut() {
                if let Picture::Packed { page: on, .. } = &mut entry.picture
                    && *on == moved
                {
                    *on = index;
                }
            }
        }
        // The page is a texture like any other; its handle is dropped after the lock is released.
        self.doomed.push(Picture::Texture { handle: page.handle, source: side, factor: 1 });
    }
}

/// What the budget loop drops: an entry, or a page of a packed store by its texture.
enum Unit<K> {
    Entry(K),
    Page(TextureId),
}
