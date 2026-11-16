//! Frosted backdrops: a bounded blurred composition of the layers under a surface, and its upload to the texture
//! store. The helpers read no framebuffer and install no renderer callback.
//!
//! # Use
//!
//! 1. The host captures the contributing content. It builds a [`Thumbnail`] with [`Thumbnail::from_image`] when the
//!    pixels change, at a block size that suits the capture resolution (about four logical points per cell).
//! 2. The host passes [`BackdropLayer`]s in screen coordinates to [`compose`] for the covered area. A layer without a
//!    thumbnail uses its fallback colour, so the first frame is defined.
//! 3. The host uploads the composed image with [`upload`] into a [`TextureStore`] built with
//!    `TextureStoreOptions::default().filter(TextureOptions::LINEAR)`, then paints the texture behind the controls
//!    with a tint it chooses.
//!
//! # What it does
//!
//! The output uses bilinear thumbnail samples and three separable box-blur passes (five cells sideways, two
//! vertically): soft interface glass, not a photographic filter. Captures are premultiplied and composite over the
//! paper's opaque RGB. The output is opaque; the host's tint supplies translucency. No brightness threshold or alpha
//! is fixed here, so the host picks the tint for text contrast and an opaque fallback for reduced transparency.
//!
//! # Limits
//!
//! Inputs are validated before any multiplication or allocation. Source images are capped at [`MAX_SOURCE_PIXELS`],
//! thumbnails and composed images at [`MAX_TEXELS`], and composed and uploaded sides at [`MAX_SIDE`]. A long, thin
//! upload is refused even when its pixel count fits, and the stored texture stays. Invalid captures and areas give
//! `None`; invalid layer rectangles are skipped.
//!
//! # Host duties
//!
//! The host owns capture and invalidation, including changes while transparency is off: when the effect returns it
//! refreshes missing or stale captures first. It keeps captures out of the composition being captured, so the
//! effect does not feed back.

use super::{CachedTexture, Revision, TextureKey, TextureStore};
use egui::{Color32, ColorImage, Context, Rect, pos2};

/// The most pixels a source image may have for [`Thumbnail::from_image`].
pub const MAX_SOURCE_PIXELS: usize = 16_777_216;

/// The most texels a composed image may have.
pub const MAX_TEXELS: usize = 262_144;

/// The longest side a composed image may have.
pub const MAX_SIDE: usize = 1024;

/// An averaged, opaque thumbnail of a captured surface. Pixels are in sRGB byte space, matching the original
/// interface effect rather than a colour-managed Gaussian blur.
#[derive(Clone, Debug)]
pub struct Thumbnail {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 3]>,
}

impl Thumbnail {
    /// Averages `block`-pixel squares of `image`, compositing its premultiplied pixels over `paper` (taken as
    /// opaque). A zero block means one. `None` for a malformed or empty image, a source over [`MAX_SOURCE_PIXELS`]
    /// or a thumbnail over [`MAX_TEXELS`].
    pub fn from_image(image: &ColorImage, block: usize, paper: Color32) -> Option<Thumbnail> {
        if !valid_image(image, MAX_SOURCE_PIXELS) {
            return None;
        }
        let block = block.max(1);
        let [iw, ih] = image.size;
        let (width, height) = (iw.div_ceil(block), ih.div_ceil(block));
        let count = width.checked_mul(height).filter(|&n| n <= MAX_TEXELS)?;
        let paper = opaque_rgb(paper);
        let mut pixels = Vec::with_capacity(count);
        for ty in 0..height {
            for tx in 0..width {
                let mut sum = [0.0; 3];
                let mut n = 0.0_f32;
                for y in ty * block..(ty * block).saturating_add(block).min(ih) {
                    for x in tx * block..(tx * block).saturating_add(block).min(iw) {
                        let color = image.pixels.get(y * iw + x).copied().unwrap_or(Color32::TRANSPARENT);
                        let missing = 1.0 - f32::from(color.a()) / 255.0;
                        for (channel, value) in sum.iter_mut().enumerate() {
                            *value += f32::from(color.to_array()[channel]) + paper[channel] * missing;
                        }
                        n += 1.0;
                    }
                }
                pixels.push(sum.map(|value| value / n.max(1.0)));
            }
        }
        Some(Thumbnail { width, height, pixels })
    }

    fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let x = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let at = |x, y| self.pixels.get(y * self.width + x).copied().unwrap_or([0.0; 3]);
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        std::array::from_fn(|i| (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty)
    }
}

/// One layer of a backdrop: where it lies, its reduced picture and the colour that stands in for a missing one.
/// Later layers cover earlier ones.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct BackdropLayer<'a> {
    /// Where the layer lies.
    pub rect: Rect,
    /// The layer's reduced picture.
    pub thumbnail: Option<&'a Thumbnail>,
    /// The colour of the layer without a thumbnail (opaque RGB).
    pub fallback: Color32,
}

impl<'a> BackdropLayer<'a> {
    /// The layer over `rect` showing `thumbnail`, or `fallback` where it has none.
    pub const fn new(rect: Rect, thumbnail: Option<&'a Thumbnail>, fallback: Color32) -> Self {
        Self { rect, thumbnail, fallback }
    }
}

/// The blurred composition of `layers` over `background` for `area`: an opaque image of cells about four points
/// wide, at most [`MAX_SIDE`] per side and [`MAX_TEXELS`] in all, softened by three box passes per axis. Linear
/// texture filtering supplies the final upsampling. `None` for an invalid, empty or excessive area; invalid layer
/// rectangles are skipped.
pub fn compose(area: Rect, layers: &[BackdropLayer<'_>], background: Color32) -> Option<ColorImage> {
    if !valid_rect(area) {
        return None;
    }
    let width = (area.width() / 4.0).ceil().clamp(1.0, MAX_SIDE as f32) as usize;
    let height = (area.height() / 4.0).ceil().clamp(1.0, (MAX_TEXELS / width).min(MAX_SIDE) as f32) as usize;
    let mut pixels = vec![opaque_rgb(background); width * height];
    for layer in layers.iter().filter(|layer| valid_rect(layer.rect) && layer.rect.intersects(area)) {
        let fallback = opaque_rgb(layer.fallback);
        for y in 0..height {
            for x in 0..width {
                let at =
                    pos2(area.left() + (x as f32 + 0.5) * area.width() / width as f32, area.top() + (y as f32 + 0.5) * area.height() / height as f32);
                if layer.rect.contains(at) {
                    let (u, v) = ((at.x - layer.rect.left()) / layer.rect.width(), (at.y - layer.rect.top()) / layer.rect.height());
                    if let Some(pixel) = pixels.get_mut(y * width + x) {
                        *pixel = layer.thumbnail.map_or(fallback, |thumbnail| thumbnail.sample(u, v));
                    }
                }
            }
        }
    }
    for _ in 0..3 {
        box_pass(&mut pixels, width, height, true);
        box_pass(&mut pixels, width, height, false);
    }
    let byte = |value: f32| value.round().clamp(0.0, 255.0) as u8;
    Some(ColorImage::new([width, height], pixels.into_iter().map(|p| Color32::from_rgb(byte(p[0]), byte(p[1]), byte(p[2]))).collect()))
}

/// Puts `image` in `store` under `key`: an equal revision keeps the cached texture, a new one replaces it in place
/// (the id stays), and the store's budget and eviction apply. The store should filter linearly
/// (`TextureStoreOptions::default().filter(TextureOptions::LINEAR)`). `None` for an image over [`MAX_TEXELS`] or with
/// a side over [`MAX_SIDE`], without touching the store.
pub fn upload<K: TextureKey>(store: &TextureStore<K>, ctx: &Context, key: K, image: ColorImage) -> Option<CachedTexture> {
    if !valid_image(&image, MAX_TEXELS) || image.size.iter().any(|&side| side > MAX_SIDE) {
        return None;
    }
    let revision = Revision::of(&(image.size, &image.pixels));
    Some(store.get_or_put(ctx, key, revision, || image))
}

fn box_pass(pixels: &mut [[f32; 3]], width: usize, height: usize, horizontal: bool) {
    let (lines, length, radius) = if horizontal { (height, width, 5) } else { (width, height, 2) };
    let at = |line: usize, cell: usize| if horizontal { line * width + cell } else { cell * width + line };
    let divisor = (2 * radius + 1) as f32;
    for line in 0..lines {
        let source: Vec<_> = (0..length).map(|cell| pixels.get(at(line, cell)).copied().unwrap_or([0.0; 3])).collect();
        for cell in 0..length {
            let mut sum = [0.0; 3];
            for offset in 0..=2 * radius {
                let value = source.get((cell + offset).saturating_sub(radius).min(length - 1)).copied().unwrap_or([0.0; 3]);
                for channel in 0..3 {
                    sum[channel] += value[channel];
                }
            }
            if let Some(pixel) = pixels.get_mut(at(line, cell)) {
                *pixel = sum.map(|v| v / divisor);
            }
        }
    }
}

fn opaque_rgb(color: Color32) -> [f32; 3] {
    let [r, g, b, _] = color.to_srgba_unmultiplied();
    [f32::from(r), f32::from(g), f32::from(b)]
}

fn valid_rect(rect: Rect) -> bool {
    rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0 && rect.size().max_elem() <= 1_000_000.0
}

fn valid_image(image: &ColorImage, limit: usize) -> bool {
    let [w, h] = image.size;
    w > 0 && h > 0 && w.checked_mul(h).is_some_and(|n| n <= limit && n == image.pixels.len())
}
