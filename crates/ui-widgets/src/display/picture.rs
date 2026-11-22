//! `Picture`: an image from pixels the caller makes, at the icon side or a thumbnail step.

use super::display_sense;
use crate::contract::{Access, Common, Component, Cx, DisabledLook, MeasureCx, PaintUi, Props as _, Role};
use crate::list::{ThumbAspect, ThumbSize};
use crate::primitives::{
    ContentPainterCx, Priority, Revision, TextureStore, TextureStoreOptions, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails, fit_rect,
};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, Sense};
use egui::{ColorImage, Id, Rect, Response, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// How large a picture is: its height, from a token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PictureSize {
    /// The icon side of the control size.
    #[default]
    Icon,
    /// A thumbnail step (`size.thumb.*`); `ThumbSize::None` shows no picture.
    Thumb(ThumbSize),
    /// The sample of a thumbnail step in a panel's options dialog (`size.thumb.sample.*`); `ThumbSize::None` shows no
    /// picture.
    Sample(ThumbSize),
    /// The image at its own size in device pixels, one texel to a pixel (a bitmap drawn for the screen's density).
    /// A painter or a picture made off the frame has no own size and takes the icon side.
    DevicePixels,
}

/// The style of a picture.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PictureStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The picture's height.
    height: f32,
}

/// A content painter that paints into the picture's rect.
type Paint<'a> = Box<dyn Fn(&ContentPainterCx, Rect) + 'a>;

/// Where a picture's content comes from.
enum Source<'a> {
    /// Pixels the caller makes, stored as a texture under `key`.
    Image { key: u64, image: &'a dyn Fn() -> ColorImage },
    /// A content painter that paints into the picture's rect.
    Painter(Paint<'a>),
    /// A picture rendered off the frame by a thumbnail source, shown pending until it arrives.
    Rendered { key: u64, thumbnails: &'a Thumbnails<u64>, source: &'a dyn ThumbSource<u64> },
}

/// A picture of an image the caller makes (`new`) or of what a content painter paints (`painter`); both sit in a rect
/// of the same token sizes. `image` runs only when the store holds no texture for `key` at the picture's revision,
/// so a picture that does not change decodes once. The texture fits inside the picture's rect at its own aspect; a
/// painter gets the whole rect. It is decoration unless labelled: only a labelled picture is in the AccessKit tree, as an
/// image, and its label shows as its tooltip.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(content_painter)]
pub struct Picture<'a> {
    #[common]
    common: Common,
    /// The version of the key's picture: a new revision makes `image` run again.
    #[prop]
    revision: Revision,
    source: Source<'a>,
    /// The width of the picture's rect as a multiple of its height.
    #[prop]
    aspect: ThumbAspect,
    /// The picture's height.
    #[style_key]
    #[prop]
    extent: PictureSize,
}

impl<'a> Picture<'a> {
    /// A picture of the image `image` makes, stored under `key`. `label` names it for a screen reader; an empty
    /// one leaves it decorative.
    pub fn new(label: impl Into<Label>, key: u64, image: &'a dyn Fn() -> ColorImage) -> Self {
        Picture::from_source(label, Source::Image { key, image })
    }

    /// A picture of what `paint` paints into the picture's rect, whose width is `aspect` times its height. `label`
    /// names it for a screen reader; an empty one leaves it decorative.
    pub fn painter(label: impl Into<Label>, aspect: ThumbAspect, paint: impl Fn(&ContentPainterCx, Rect) + 'a) -> Self {
        Picture::from_source(label, Source::Painter(Box::new(paint))).aspect(aspect)
    }

    /// A picture made off the frame by `source` for `key`: pending, with the framework's placeholder, until the
    /// source's picture arrives, then fitted as `new`'s texture is. `label` names it for a screen reader.
    /// The source's picture is asked through [`Thumbnails::state`] at the picture's rect and the screen's density,
    /// so a `ThumbRender::Later` render runs on the loader's lanes and no pass waits for it; its revision is the
    /// source's, not the `revision` prop.
    pub fn rendered(label: impl Into<Label>, key: u64, thumbnails: &'a Thumbnails<u64>, source: &'a dyn ThumbSource<u64>) -> Self {
        Picture::from_source(label, Source::Rendered { key, thumbnails, source })
    }

    fn from_source(label: impl Into<Label>, source: Source<'a>) -> Self {
        Picture { common: Common::new(), revision: Revision::default(), source, aspect: ThumbAspect::SQUARE, extent: PictureSize::Icon }.label(label)
    }
}

impl Component for Picture<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = PictureStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: PictureStyleKey) -> PictureStyle {
        let thumbs = &cx.tokens.size.thumb;
        let height = match key.extent {
            PictureSize::Icon | PictureSize::DevicePixels => cx.control().icon,
            PictureSize::Thumb(step) => step.pick([thumbs.sm, thumbs.md, thumbs.lg]),
            PictureSize::Sample(step) => step.pick([thumbs.sample.sm, thumbs.sample.md, thumbs.sample.lg]),
        };
        PictureStyle { metrics: Metrics::new(), height }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if let (PictureSize::DevicePixels, Source::Image { key, image }) = (self.extent, &self.source) {
            // The texture is the measured content: its size is known only once the image is made, so it is made here
            // and `paint` finds it at this revision and makes nothing.
            let ctx = cx.ui.ctx();
            let texture = store(ctx).get_or_put(ctx, (cx.resolved.id, *key), self.revision, *image);
            return texture.size_vec2() / ctx.pixels_per_point();
        }
        let height = cx.style.height;
        Vec2::new(self.aspect.of(height), height)
    }

    fn sense(&self) -> Sense {
        display_sense(&self.common, true)
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.role(Role::None);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if cx.style.height <= 0.0 {
            return;
        }
        match &self.source {
            Source::Image { key, image } => {
                let texture = store(&cx.ctx).get_or_put(&cx.ctx, (cx.resolved.id, *key), self.revision, *image);
                let target = fit_rect(cx.content_rect, texture.size_vec2());
                // The store packs, so the texture is the picture's part of a shared page.
                ui.content_painter(cx).picture(texture.id, target, texture.uv(), 0.0);
            }
            Source::Painter(paint) => paint(&ui.content_painter(cx), cx.content_rect),
            Source::Rendered { key, thumbnails, source } => {
                let rect = cx.content_rect;
                let request = ThumbRequest::for_cell(rect.width().max(rect.height()), cx.ctx.pixels_per_point());
                let state = thumbnails.state(&cx.ctx, *source, key, &request, Priority::VISIBLE);
                thumbnails.paint(&ui.content_painter(cx), rect, &state, &ThumbPaint::default());
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The pictures' store of `ctx`: packed, so many small pictures (style tiles, font samples) share a few page
/// textures; a larger picture takes a texture of its own.
fn store(ctx: &egui::Context) -> TextureStore<(Id, u64)> {
    TextureStore::in_context(ctx, "picture", TextureStoreOptions::thumbnails().packed_small())
}
