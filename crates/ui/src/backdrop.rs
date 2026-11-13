//! Bounded CPU frosted backdrops from caller-supplied images, independent of document rendering.
//!
//! Applications own capture, layout, invalidation, tint/contrast, reduced-transparency preference
//! and texture lifetime. These helpers neither read a framebuffer nor install a renderer callback.

use egui::{Color32, ColorImage, Context, Rect, TextureHandle, TextureOptions, pos2};

const MAX_SOURCE_PIXELS: usize = 16_777_216;
const MAX_TEXELS: usize = 262_144;
const MAX_SIDE: usize = 1024;

/// An averaged, opaque thumbnail of a captured surface. Pixels are stored in sRGB byte space,
/// matching the original UI effect rather than a color-managed photographic Gaussian blur.
#[derive(Clone, Debug)]
pub struct Thumbnail {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 3]>,
}

impl Thumbnail {
    /// Average `block`-pixel squares, compositing premultiplied input over `paper`. The paper's
    /// unmultiplied RGB is treated as opaque. Zero block means one. Empty/malformed images,
    /// source images over 16,777,216 pixels or thumbnails over 262,144 texels return `None`.
    pub fn from_image(image: &ColorImage, block: usize, paper: Color32) -> Option<Self> {
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
        Some(Self { width, height, pixels })
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

/// One captured surface in caller-chosen screen coordinates. Later layers cover earlier ones.
/// Missing thumbnails use opaque `fallback` RGB, allowing first-frame/capture-off fallback.
pub struct BackdropLayer<'a> {
    pub rect: Rect,
    pub thumbnail: Option<&'a Thumbnail>,
    pub fallback: Color32,
}

/// Sample the layers into a small opaque image and soften it with three box passes per axis
/// (five cells sideways, two vertically). Cells are about four logical points, at most 1024
/// per side and 262,144 in total. Linear texture filtering supplies the final upsampling.
///
/// Invalid/empty/excessive areas return `None`; invalid layer rectangles are skipped. The
/// background and fallback colors use opaque unmultiplied RGB. Layer count and capture cadence
/// are caller policy: cache thumbnails and compose only when contributing content/layout changes.
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

/// An explicitly owned texture cache. Keep one per backdrop and egui context. Identical images
/// do not upload again; changed images update the same texture. Dropping this releases it.
#[derive(Default)]
pub struct BackdropTexture {
    texture: Option<TextureHandle>,
    image: Option<ColorImage>,
}

impl BackdropTexture {
    /// Upload a bounded valid image using linear filtering. Invalid images return `None` without
    /// replacing the cache. Use a distinct cache per context; texture IDs belong to that context.
    pub fn update(&mut self, ctx: &Context, name: &str, image: ColorImage) -> Option<egui::TextureId> {
        if !valid_image(&image, MAX_TEXELS) || image.size.iter().any(|&side| side > MAX_SIDE) {
            return None;
        }
        if self.image.as_ref() != Some(&image) {
            if let Some(texture) = self.texture.as_mut() {
                texture.set(image.clone(), TextureOptions::LINEAR);
            } else {
                self.texture = Some(ctx.load_texture(name, image.clone(), TextureOptions::LINEAR));
            }
            self.image = Some(image);
        }
        self.texture.as_ref().map(TextureHandle::id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area() -> Rect {
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(40.0, 20.0))
    }
    #[test]
    fn transparent_capture_composites_over_paper_and_missing_capture_uses_fallback() {
        let image = ColorImage::new([2, 2], vec![Color32::TRANSPARENT; 4]);
        let thumbnail = Thumbnail::from_image(&image, 2, Color32::RED);
        assert!(thumbnail.is_some());
        let layers = [BackdropLayer { rect: area(), thumbnail: thumbnail.as_ref(), fallback: Color32::BLUE }];
        let image = compose(area(), &layers, Color32::GREEN);
        assert!(image.as_ref().is_some_and(|image| image.pixels.iter().all(|p| *p == Color32::RED)));
        let layers = [BackdropLayer { rect: area(), thumbnail: None, fallback: Color32::BLUE }];
        let image = compose(area(), &layers, Color32::GREEN);
        assert!(image.as_ref().is_some_and(|image| image.pixels.iter().all(|p| *p == Color32::BLUE)));
    }
    #[test]
    fn downsampling_and_softening_mix_neighbours_without_changing_opaque_output() {
        let image = ColorImage::new([2, 1], vec![Color32::BLACK, Color32::WHITE]);
        let thumbnail = Thumbnail::from_image(&image, 1, Color32::WHITE);
        let layers = [BackdropLayer { rect: area(), thumbnail: thumbnail.as_ref(), fallback: Color32::RED }];
        let image = compose(area(), &layers, Color32::GREEN);
        assert!(image.as_ref().is_some_and(|image| image.pixels.iter().all(|p| p.a() == 255 && p.r() > 0 && p.r() < 255)));
    }
    #[test]
    fn malformed_dimensions_empty_images_and_large_allocations_are_bounded() {
        let malformed = ColorImage { size: [usize::MAX, 2], pixels: vec![], source_size: egui::Vec2::ZERO };
        assert!(Thumbnail::from_image(&malformed, usize::MAX, Color32::WHITE).is_none());
        assert!(Thumbnail::from_image(&ColorImage::new([0, 0], vec![]), 0, Color32::WHITE).is_none());
        assert!(compose(Rect::NAN, &[], Color32::WHITE).is_none());
        assert!(compose(Rect::NOTHING, &[], Color32::WHITE).is_none());
        let large = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_000_000.0, 1_000_000.0));
        let image = compose(large, &[], Color32::WHITE);
        assert!(image.as_ref().is_some_and(|image| image.pixels.len() <= MAX_TEXELS && image.size.iter().all(|&side| side <= MAX_SIDE)));
    }
    #[test]
    fn texture_cache_updates_changed_content_but_skips_identical_uploads() {
        let ctx = Context::default();
        let mut cache = BackdropTexture::default();
        let image = ColorImage::new([1, 1], vec![Color32::RED]);
        let mut first_id = None;
        let out = ctx.run_ui(egui::RawInput::default(), |_| {
            first_id = cache.update(&ctx, "backdrop", image.clone());
        });
        assert!(first_id.is_some());
        assert!(out.textures_delta.set.iter().any(|(id, _)| Some(*id) == first_id));
        out.drop_without_applying_deltas();
        let out = ctx.run_ui(egui::RawInput::default(), |_| {
            assert_eq!(cache.update(&ctx, "backdrop", image.clone()), first_id);
        });
        assert!(out.textures_delta.set.iter().all(|(id, _)| Some(*id) != first_id));
        out.drop_without_applying_deltas();
        let out = ctx.run_ui(egui::RawInput::default(), |_| {
            assert_eq!(cache.update(&ctx, "backdrop", ColorImage::new([1, 1], vec![Color32::BLUE])), first_id);
        });
        assert_eq!(out.textures_delta.set.iter().filter(|(id, _)| Some(**id) == first_id).count(), 1);
        out.drop_without_applying_deltas();
    }

    #[test]
    fn texture_upload_rejects_long_thin_images_without_replacing_the_cache() {
        let ctx = Context::default();
        let mut cache = BackdropTexture::default();
        let image = ColorImage::new([1, 1], vec![Color32::RED]);
        let mut kept = None;
        ctx.run_ui(egui::RawInput::default(), |_| {
            kept = cache.update(&ctx, "bounded", image.clone());
        })
        .drop_without_applying_deltas();
        assert!(kept.is_some());
        for size in [[MAX_SIDE + 1, 1], [1, MAX_SIDE + 1]] {
            let out = ctx.run_ui(egui::RawInput::default(), |_| {
                assert!(cache.update(&ctx, "bounded", ColorImage::new(size, vec![Color32::BLUE; size[0] * size[1]])).is_none());
                assert_eq!(cache.update(&ctx, "bounded", image.clone()), kept);
            });
            assert!(out.textures_delta.set.iter().all(|(id, _)| Some(*id) != kept));
            out.drop_without_applying_deltas();
        }
    }
}
