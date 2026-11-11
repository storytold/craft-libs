//! `TextureCache`, the icon texture path, and beside it the keyed `TextureStore` with its background
//! `TextureLoader`.

mod limits;
mod loader;
mod store;
mod words;

pub use limits::{MAX_TEXTURE, fit_image, reduce_image, texture_factor, texture_side};
pub use loader::{LoadFailure, LoadResult, LoadedPicture, LoaderOptions, Priority, TextureLoader};
pub use store::{CachedTexture, Revision, TextureBudget, TextureKey, TextureState, TextureStatistics, TextureStore, TextureStoreOptions};
pub use words::TextureWords;

/// The tint that leaves a texture's own colours as they are: every texel is multiplied by white.
#[doc(hidden)]
pub const UNTINTED: Color32 = Color32::from_gray(u8::MAX);

mod thumbnails;
#[cfg(feature = "testing")]
pub use thumbnails::FakeThumbSource;
#[cfg(feature = "testing")]
pub(crate) use thumbnails::fake_thumb_image;
pub use thumbnails::{ThumbFit, ThumbJob, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails, fit_rect, thumb_image};

use crate::runtime::egui::{memory, pass, textures};
use egui::{Color32, ColorImage, Context, TextureHandle, TextureId, TextureOptions};
use std::any::TypeId;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// One cached texture and the frame that last asked for it.
struct Entry {
    handle: TextureHandle,
    used: u64,
}

/// The textures of one context by key hash. A handle that is dropped frees its texture, so the cache owns them.
#[derive(Default)]
struct Loaded {
    handles: HashMap<u64, Entry>,
}

impl Loaded {
    /// Drops the entries no frame asked for lately, oldest first, until the cache is within `CAPACITY`. An entry
    /// that `frame` asked for stays: its texture is in this frame's shapes.
    fn evict(&mut self, frame: u64) {
        let excess = self.handles.len().saturating_sub(TextureCache::CAPACITY);
        if excess == 0 {
            return;
        }
        let mut stale: Vec<(u64, u64)> =
            self.handles.iter().filter(|(_, entry)| entry.used < frame).map(|(hash, entry)| (entry.used, *hash)).collect();
        stale.sort_unstable();
        for (_, hash) in stale.into_iter().take(excess) {
            self.handles.remove(&hash);
        }
    }
}

/// The texture cache of a context: components keep a cache key and ask for the texture when they paint.
pub struct TextureCache;

impl TextureCache {
    /// The number of textures above which the cache drops those no frame asked for lately.
    pub const CAPACITY: usize = 256;

    /// The texture for `key`: loads `image()` on the first request and returns the same texture afterwards. A key
    /// names everything the image depends on (icon, pixel size, stroke, mirroring), so a changed input is a
    /// different key. Keys of different types never meet. A component asks again in every frame it paints the
    /// texture; a texture nobody asks for while the cache is over `CAPACITY` is freed.
    pub fn texture<K: Hash + 'static>(ctx: &Context, key: K, image: impl FnOnce() -> ColorImage) -> TextureId {
        let mut hasher = DefaultHasher::new();
        TypeId::of::<K>().hash(&mut hasher);
        key.hash(&mut hasher);
        let hash = hasher.finish();
        let frame = pass::root_frame_nr(ctx);
        let known = memory::with(ctx, |loaded: &mut Loaded| {
            loaded.handles.get_mut(&hash).map(|entry| {
                entry.used = frame;
                entry.handle.id()
            })
        });
        if let Some(id) = known {
            return id;
        }
        // The image is built and uploaded outside the service lock: neither calls back into the cache.
        let handle = textures::load(ctx, format!("craft_ui/{hash:016x}"), image(), TextureOptions::LINEAR);
        memory::with(ctx, |loaded: &mut Loaded| {
            let id = loaded.handles.entry(hash).or_insert(Entry { handle, used: frame }).handle.id();
            loaded.evict(frame);
            id
        })
    }

    /// The number of textures the context holds.
    pub fn len(ctx: &Context) -> usize {
        memory::with(ctx, |loaded: &mut Loaded| loaded.handles.len())
    }

    /// Whether the context holds no texture.
    pub fn is_empty(ctx: &Context) -> bool {
        Self::len(ctx) == 0
    }

    /// Drops every texture of the context; the next request loads again.
    pub fn clear(ctx: &Context) {
        memory::with(ctx, |loaded: &mut Loaded| loaded.handles.clear());
    }
}
