//! [`PreviewPixels`] and [`PreviewTextures`]: the moving pixels of a transform, read once and
//! uploaded as a mip chain built on demand, off the UI thread where a finer level can stand in.

use std::sync::{Arc, Mutex, PoisonError};

use craft_host::queue::Cancel;
use craft_ui::primitives::{
    LoadFailure, LoadResult, LoadedPicture, LoaderOptions, Priority, Revision, Serials, TextureLoader, TextureState, TextureStatistics, TextureStore,
    TextureStoreOptions,
};
use egui::{Color32, ColorImage, Context, TextureId, TextureOptions};

use super::PreviewQuality;

/// The salt of the previews' texture stores and loader.
const STORE: &str = "craft_imaging.transform_preview";

/// The app's pixels of the moving layer: premultiplied texels of which `base_uv` covers the
/// source frame (the rest is padding up to the GPU's texture limit). The app reads them from its
/// own surface once per session (PhotoCraft `read_surface`).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PreviewPixels {
    /// The texels, premultiplied.
    pub image: ColorImage,
    /// The uv extent of `image` that covers the source frame, each 0 to 1.
    pub base_uv: [f32; 2],
}

impl PreviewMask {
    /// A mask of `values`, one per texel of the image, row by row, at `density` (0 to 1).
    pub fn new(values: Vec<u8>, density: f32) -> Self {
        Self { values, density }
    }
}

impl PreviewDisplay {
    /// A conversion `convert` of premultiplied document texels to the monitor's, identified by `key`.
    pub fn new(key: u64, convert: impl Fn(&ColorImage) -> ColorImage + Send + Sync + 'static) -> Self {
        Self { key, convert: Arc::new(convert) }
    }
}

impl TextureChoice {
    /// Level `level`, with nearest filtering when `nearest`.
    pub fn new(level: u32, nearest: bool) -> Self {
        Self { level, nearest }
    }
}

impl PreviewPixels {
    /// `image` covering the source frame with `base_uv` of its extent (clamped to 0 to 1).
    pub fn new(image: ColorImage, base_uv: [f32; 2]) -> Self {
        let base_uv = [base_uv[0].clamp(0.0, 1.0), base_uv[1].clamp(0.0, 1.0)];
        Self { image, base_uv }
    }

    /// The pixels with `mask` applied: each texel's alpha (and its premultiplied colour) scaled
    /// by `1 − density × (1 − value)`, as the compositor applies a pixel mask.
    pub fn masked(mut self, mask: &PreviewMask) -> Self {
        let density = mask.density.clamp(0.0, 1.0);
        for (texel, &value) in self.image.pixels.iter_mut().zip(&mask.values) {
            let keep = (1.0 - density * (1.0 - f32::from(value) / 255.0)).clamp(0.0, 1.0);
            let [r, g, b, a] = texel.to_array();
            let scale = |c: u8| (f32::from(c) * keep).round() as u8;
            *texel = Color32::from_rgba_premultiplied(scale(r), scale(g), scale(b), scale(a));
        }
        self
    }
}

/// A layer mask over the preview's pixels: one value per texel of [`PreviewPixels::image`], row by
/// row, 0 hiding and 255 showing, and the mask's density (0 to 1).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PreviewMask {
    /// One value per texel of the image, row by row.
    pub values: Vec<u8>,
    /// The mask's density, 0 to 1.
    pub density: f32,
}

/// The monitor conversion of the displayed preview (LC27): `key` changes whenever `convert` does;
/// `convert` takes premultiplied document texels to the monitor's. The app builds it from
/// `craft-color-ui`'s display transform (PhotoCraft `CanvasDisplay`). Effects keep reading the raw
/// levels.
#[derive(Clone)]
#[non_exhaustive]
pub struct PreviewDisplay {
    /// Changes whenever `convert` does, so caches keyed on it refresh.
    pub key: u64,
    /// Converts premultiplied document texels to the monitor's.
    pub convert: Arc<dyn Fn(&ColorImage) -> ColorImage + Send + Sync>,
}

impl std::fmt::Debug for PreviewDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewDisplay").field("key", &self.key).finish_non_exhaustive()
    }
}

/// Which level of the mip chain to draw with, and whether with nearest filtering
/// ([`PreviewTextures::choose`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TextureChoice {
    /// 0 is full resolution; level `n` is `2ⁿ` times smaller.
    pub level: u32,
    /// Whether to draw with nearest filtering instead of linear.
    pub nearest: bool,
}

/// The key of a preview's level in the shared stores: the preview's name and the level.
type LevelKey = (Arc<str>, u32);

/// The moving pixels uploaded for the preview: level 0 at once, the other mip levels (2 × 2 box filtered) and the
/// nearest-filtered level 0 on first use. The CPU levels are kept for the session.
///
/// - **Stores.** Every preview of a context shares one store of linear levels and one of nearest ones, keyed by the
///   preview's name and the level, so a closed preview's uploads are evicted under the stores' budget. Two previews
///   painted at once must not share a name. The revision is the preview's own (new pixels under an old name replace
///   the old textures in place) with the display key (a profile change replaces them in place too).
/// - **Off the UI thread.** A level first needed while a finer level is uploaded for this preview and display is
///   built (halved, converted, uploaded) by a job of the context's `TextureLoader`; the closest finer uploaded level
///   shows until it lands. A level with nothing finer uploaded is built in the pass. A preview no longer painted for
///   two frames drops its jobs and their results, as every loader key does.
pub struct PreviewTextures {
    name: Arc<str>,
    serial: u64,
    /// The CPU levels built so far, premultiplied; level 0 first. Shared with the jobs that build levels.
    images: Arc<Mutex<Vec<Arc<ColorImage>>>>,
    base_uv: [f32; 2],
}

impl PreviewTextures {
    /// The preview of `pixels`, named `name`: the store entries are keyed by it. `serials` tells this preview from an
    /// earlier one of the same name (new pixels under an old name replace its textures); the previews of one owner
    /// share it. Nothing is uploaded until the preview is painted.
    pub fn new(serials: &mut Serials, name: impl Into<String>, pixels: PreviewPixels) -> Self {
        let base_uv = pixels.base_uv;
        Self::over(serials, name, vec![Arc::new(pixels.image)], base_uv)
    }

    /// The preview over `levels` already built, level 0 first, covering the source with `base_uv`.
    #[cfg(feature = "screen-testing")]
    pub(crate) fn from_levels(serials: &mut Serials, name: impl Into<String>, levels: Vec<Arc<ColorImage>>, base_uv: [f32; 2]) -> Self {
        Self::over(serials, name, levels, base_uv)
    }

    fn over(serials: &mut Serials, name: impl Into<String>, levels: Vec<Arc<ColorImage>>, base_uv: [f32; 2]) -> Self {
        let serial = serials.mint();
        Self { name: Arc::from(name.into()), serial, images: Arc::new(Mutex::new(levels)), base_uv }
    }

    /// How many mip levels are built so far (level 0 counts): levels are built when asked for.
    pub fn levels_built(&self) -> usize {
        self.images.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// The size of level 0 in texels.
    pub fn size(&self) -> [usize; 2] {
        self.images.lock().unwrap_or_else(PoisonError::into_inner).first().map_or([0, 0], |image| image.size)
    }

    /// What the store of every preview's linear levels in `ctx` holds and uploaded this frame.
    pub fn statistics(ctx: &Context) -> TextureStatistics {
        linear_store(ctx).statistics(ctx)
    }

    /// The level and filter for a source pixel shown at `zoom` screen pixels per document pixel,
    /// where the transform scales it by `scale` (PhotoCraft's rule, LC24): with `nearest`
    /// interpolation always level 0 nearest (point sampling; a mip level would blur pixel art);
    /// below `MIP_BELOW` (½) screen pixels per source pixel level `⌊log2(1 / (zoom × scale))⌋`, at
    /// most `MIP_LEVELS` (12); otherwise level 0, nearest when `zoom ≥ CRISP_ZOOM` (2) and
    /// `scale ≤ CRISP_SCALE` (1.05). A product that is not finite or not positive gives level 0
    /// linear. Constants in [`PreviewQuality`](super::PreviewQuality).
    pub fn choose(zoom: f32, scale: f32, nearest: bool) -> TextureChoice {
        let screen = zoom * scale;
        if !screen.is_finite() || screen <= 0.0 {
            return TextureChoice::default();
        }
        if nearest {
            return TextureChoice { level: 0, nearest: true };
        }
        if screen < PreviewQuality::MIP_BELOW {
            let level = (1.0 / screen).log2().floor().clamp(0.0, PreviewQuality::MIP_LEVELS as f32) as u32;
            return TextureChoice { level, nearest: false };
        }
        TextureChoice { level: 0, nearest: zoom >= PreviewQuality::CRISP_ZOOM && scale <= PreviewQuality::CRISP_SCALE }
    }

    /// The texture and its uv extent for [`choose`](Self::choose)'s answer; with `display`, the converted upload (the
    /// CPU level stays raw). Level 0, its nearest variant and a level with nothing finer uploaded for this display are
    /// uploaded in the pass on first use; any other level is built by a job while the closest finer uploaded level
    /// stands in. Each pass asks again: a store hit costs a lookup.
    pub fn pick(&self, ctx: &Context, choice: TextureChoice, display: Option<&PreviewDisplay>) -> Option<(TextureId, [f32; 2])> {
        let revision = Revision::of(&(self.serial, display.map_or(0, |d| d.key)));
        let key = |level: u32| (Arc::clone(&self.name), level);
        let (level, uv) = self.extent(choice.level)?;
        if choice.nearest || level == 0 {
            let store = if choice.nearest { nearest_store(ctx) } else { linear_store(ctx) };
            let texture = store.get_or_put(ctx, key(level), revision, || self.shown(level, display));
            return Some((texture.id, uv));
        }
        let loader = TextureLoader::<LevelKey>::in_context(ctx, STORE, LoaderOptions::new().store(TextureStoreOptions::pages()).lanes(2));
        let store = loader.cache();
        // The level itself, else the closest finer level uploaded for this display.
        let shown = (0..=level).rev().find_map(|at| match store.state(ctx, &key(at), revision) {
            TextureState::Ready(texture) => Some((at, texture)),
            _ => None,
        });
        let texture = match shown {
            Some((at, texture)) if at == level => texture,
            Some((finer, stand_in)) => {
                let job = || {
                    let (images, convert) = (Arc::clone(&self.images), display.map(|d| Arc::clone(&d.convert)));
                    move |_: &Cancel| -> LoadResult {
                        let image = level_of(&images, level).map(|(image, _)| image).ok_or(LoadFailure::Missing)?;
                        Ok(LoadedPicture::Image(converted(&image, convert.as_deref())))
                    }
                };
                match loader.request(ctx, &key(level), revision, Priority::VISIBLE, job) {
                    TextureState::Ready(texture) => texture,
                    _ => return Some((stand_in.id, self.extent(finer)?.1)),
                }
            }
            None => store.get_or_put(ctx, key(level), revision, || self.shown(level, display)),
        };
        Some((texture.id, uv))
    }

    /// Level `level` built in the pass, converted for `display`.
    fn shown(&self, level: u32, display: Option<&PreviewDisplay>) -> ColorImage {
        match self.level_image(level) {
            Some((image, _)) => converted(&image, display.map(|d| &*d.convert)),
            None => ColorImage::default(),
        }
    }

    /// Level `n` of the mip chain (built on demand, at most down to one texel) and its uv extent
    /// covering the source. The styled preview resamples from it.
    pub fn level_image(&self, n: u32) -> Option<(Arc<ColorImage>, [f32; 2])> {
        let (image, level) = level_of(&self.images, n)?;
        Some((image, self.extent(level)?.1))
    }

    /// The level the chain gives for `n` (the last one when it ends at one texel above `n`) and its uv extent
    /// covering the source, from level 0's size alone.
    fn extent(&self, n: u32) -> Option<(u32, [f32; 2])> {
        let base = self.images.lock().unwrap_or_else(PoisonError::into_inner).first()?.size;
        let (mut size, mut level) = (base, 0);
        while level < n && (size[0] > 1 || size[1] > 1) {
            size = [size[0].div_ceil(2).max(1), size[1].div_ceil(2).max(1)];
            level += 1;
        }
        // Level k covers size × 2^k base texels; the last row or column may be padding.
        let factor = (1u64 << level) as f32;
        let uv = |axis: usize| (self.base_uv[axis] * base[axis] as f32 / (size[axis] as f32 * factor).max(1.0)).min(1.0);
        Some((level, [uv(0), uv(1)]))
    }
}

/// The store of every preview's linear levels in `ctx`: the loader's.
fn linear_store(ctx: &Context) -> TextureStore<LevelKey> {
    TextureStore::in_context(ctx, STORE, TextureStoreOptions::pages())
}

/// The store of every preview's nearest-filtered level 0 in `ctx`.
fn nearest_store(ctx: &Context) -> TextureStore<LevelKey> {
    TextureStore::in_context(ctx, (STORE, "nearest"), TextureStoreOptions::pages().filter(TextureOptions::NEAREST))
}

/// `image` converted by `convert`, or as it is.
fn converted(image: &ColorImage, convert: Option<&(dyn Fn(&ColorImage) -> ColorImage + Send + Sync)>) -> ColorImage {
    match convert {
        Some(convert) => convert(image),
        None => image.clone(),
    }
}

/// Level `n` of the chain in `images` and its index (the last level when the chain ends at one texel above `n`),
/// built on demand. The halving runs with the lock released, so a job building a level never holds up the pass;
/// when two callers build the same level, the first one's stays.
fn level_of(images: &Mutex<Vec<Arc<ColorImage>>>, n: u32) -> Option<(Arc<ColorImage>, u32)> {
    let lock = || images.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        let (previous, built) = {
            let images = lock();
            if let Some(image) = images.get(n as usize) {
                return Some((Arc::clone(image), n));
            }
            (Arc::clone(images.last()?), images.len())
        };
        if previous.size[0] <= 1 && previous.size[1] <= 1 {
            return Some((previous, (built - 1) as u32));
        }
        let next = Arc::new(halve(&previous));
        let mut images = lock();
        if images.len() == built {
            images.push(next);
        }
    }
}

/// 2 × 2 box filter of a premultiplied image; an odd edge repeats its last row or column.
fn halve(image: &ColorImage) -> ColorImage {
    let [w, h] = image.size;
    let (nw, nh) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
    let mut out = vec![Color32::TRANSPARENT; nw * nh];
    let texel = |x: usize, y: usize| image.pixels.get(y.min(h.saturating_sub(1)) * w + x.min(w.saturating_sub(1))).copied().unwrap_or_default();
    for (y, row) in out.chunks_mut(nw).enumerate() {
        for (x, o) in row.iter_mut().enumerate() {
            let quad = [texel(2 * x, 2 * y), texel(2 * x + 1, 2 * y), texel(2 * x, 2 * y + 1), texel(2 * x + 1, 2 * y + 1)];
            let mean = |channel: fn(&Color32) -> u8| ((quad.iter().map(|p| u32::from(channel(p))).sum::<u32>() + 2) / 4) as u8;
            *o = Color32::from_rgba_premultiplied(mean(Color32::r), mean(Color32::g), mean(Color32::b), mean(Color32::a));
        }
    }
    ColorImage::new([nw, nh], out)
}
