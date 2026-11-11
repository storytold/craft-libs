//! Stories rendered to images offscreen: egui's wgpu test renderer over what [`Frames`](super::Frames) recorded with
//! `record_paint`. The renderer prefers a software adapter, so it runs without a display or a GPU.

use crate::runtime::egui::fixture::FullOutput;
use crate::runtime::overlay;
use egui::{Context, Rect, TextureId, TexturesDelta};
use egui_kittest::TestRenderer as _;
use egui_kittest::wgpu::WgpuTestRenderer;
use std::collections::HashSet;

pub use image::RgbaImage;

/// A wgpu device and egui renderer that renders one context's frames after another's.
pub struct Renderer {
    inner: WgpuTestRenderer,
    /// The textures the current context has set, which [`Renderer::finish`] frees.
    textures: HashSet<TextureId>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// A renderer on the preferred adapter.
    ///
    /// # Panics
    /// When wgpu finds no adapter.
    pub fn new() -> Self {
        Renderer { inner: WgpuTestRenderer::new(), textures: HashSet::new() }
    }

    /// The image of `paint` (from [`Frames::take_paint`](super::Frames::take_paint)) at the context's pixels per point,
    /// over a transparent ground. The texture changes in `paint` are applied first.
    ///
    /// # Errors
    /// When wgpu fails to render or read back the image.
    pub fn render(&mut self, ctx: &Context, mut paint: FullOutput) -> Result<RgbaImage, String> {
        self.textures.extend(paint.textures_delta.set.iter().map(|(id, _)| *id));
        for id in &paint.textures_delta.free {
            self.textures.remove(id);
        }
        self.inner.handle_delta(&mut paint.textures_delta);
        self.inner.render(ctx, &paint)
    }

    /// Frees the textures of the context rendered last, before the renderer takes another context's frames.
    pub fn finish(&mut self) {
        let mut delta = TexturesDelta { set: Default::default(), free: self.textures.drain().collect() };
        self.inner.handle_delta(&mut delta);
    }
}

/// The screen rects the layers shown last frame and not fading out paint in: popups, dialogs, windows shown as layers,
/// each within the region it is contained in. A layer's cover (a scrim) is not among them.
pub fn layer_rects(ctx: &Context) -> Vec<Rect> {
    overlay::open_entries(ctx).into_iter().map(|entry| entry.region.map_or(entry.rect, |region| entry.rect.intersect(region))).collect()
}
