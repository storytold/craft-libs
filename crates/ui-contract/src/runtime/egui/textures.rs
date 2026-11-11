//! Texture uploads and painting: the one place egui's texture calls are made.

use egui::{Color32, ColorImage, Context, CornerRadius, Painter, Rect, TextureHandle, TextureId, TextureOptions};

/// Uploads `image` as a texture called `name`, sampled with `filter`. The texture lives as long as the handle.
pub(crate) fn load(ctx: &Context, name: String, image: ColorImage, filter: TextureOptions) -> TextureHandle {
    ctx.load_texture(name, image, filter)
}

/// Replaces the whole picture of `handle` with `image`; the texture id stays.
pub(crate) fn replace(handle: &mut TextureHandle, image: ColorImage, filter: TextureOptions) {
    handle.set(image, filter);
}

/// Replaces the part of `handle`'s picture whose top-left texel is `origin` with `image`; the texture id stays.
pub(crate) fn replace_region(handle: &mut TextureHandle, origin: [u32; 2], image: ColorImage, filter: TextureOptions) {
    handle.set_partial(origin.map(|n| n as usize), image, filter);
}

/// Paints `texture` in `rect`, the part `uv` of it, tinted by `tint`, with corners rounded by `radius`.
#[doc(hidden)]
pub fn paint(painter: &Painter, texture: TextureId, rect: Rect, uv: Rect, tint: Color32, radius: CornerRadius) {
    if radius == CornerRadius::ZERO {
        painter.image(texture, rect, uv, tint);
    } else {
        painter.add(egui::epaint::RectShape::filled(rect, radius, tint).with_texture(texture, uv));
    }
}
