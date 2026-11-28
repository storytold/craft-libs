//! The device texture limit in one place: the largest side a texture may have, how much a large image is reduced
//! to fit it, and `fit_image` for any image past the limit.

use craft_behavior::PixelSize;
use egui::{Color32, ColorImage, Context};

/// The largest texture side the library uploads.
pub const MAX_TEXTURE: u32 = 4096;

/// The texture side on this device: [`MAX_TEXTURE`], or the device's smaller limit (2048 on some older GPUs and
/// WebGL); past it egui asserts and the backend fails.
pub fn texture_side(ctx: &Context) -> u32 {
    u32::try_from(ctx.input(|input| input.max_texture_side)).unwrap_or(MAX_TEXTURE).clamp(1, MAX_TEXTURE)
}

/// How many document pixels per side one texel of the base texture holds for a document of `size` with textures at
/// most `side` per side: 1 when it fits.
pub fn texture_factor(size: PixelSize, side: u32) -> u32 {
    size.width.max(size.height).div_ceil(side.max(1)).max(1)
}

/// `image` reduced to at most `max_side` texels per side, or unchanged when it fits. The image is reduced by the
/// whole factor `texture_factor(size, max_side)`: each texel is the box average of `factor x factor` pixels (of
/// fewer at the right and bottom edges), averaged in egui's premultiplied values, so a transparent edge stays
/// clean. The result is `size.div_ceil(factor)`, at least 1 x 1. A whole factor keeps every reduced texel on whole
/// source pixels, so a later region update maps onto it exactly (`TextureStore::put_region`).
pub fn fit_image(image: ColorImage, max_side: u32) -> ColorImage {
    let factor = texture_factor(texels(image.size), max_side);
    if factor == 1 { image } else { reduce_image(&image, factor) }
}

/// `image` box-averaged by the whole `factor`, in premultiplied values.
pub fn reduce_image(image: &ColorImage, factor: u32) -> ColorImage {
    let f = factor.max(1) as usize;
    let [w, h] = image.size;
    let (rw, rh) = (w.div_ceil(f).max(1), h.div_ceil(f).max(1));
    let mut pixels = Vec::with_capacity(rw * rh);
    for ry in 0..rh {
        for rx in 0..rw {
            let (x0, y0) = (rx * f, ry * f);
            let (x1, y1) = ((x0 + f).min(w), (y0 + f).min(h));
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                for pixel in &image.pixels[y * w + x0..y * w + x1] {
                    for (total, channel) in sum.iter_mut().zip(pixel.to_array()) {
                        *total += u64::from(channel);
                    }
                }
            }
            let count = ((x1 - x0) * (y1 - y0)).max(1) as u64;
            let mean = sum.map(|total| u8::try_from((total + count / 2) / count).unwrap_or(u8::MAX));
            pixels.push(Color32::from_rgba_premultiplied(mean[0], mean[1], mean[2], mean[3]));
        }
    }
    ColorImage::new([rw, rh], pixels)
}

/// An image size in texels, saturating.
pub(super) fn texels(size: [usize; 2]) -> PixelSize {
    let [width, height] = size.map(|n| u32::try_from(n).unwrap_or(u32::MAX));
    PixelSize::new(width, height)
}
