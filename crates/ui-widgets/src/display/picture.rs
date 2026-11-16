//! `Picture`: an image from pixels the caller makes, at the icon side or a thumbnail step.

use super::display_sense;
use crate::contract::{Access, Common, Component, Cx, DisabledLook, MeasureCx, PaintUi, Props as _, Role};
use crate::list::{ThumbAspect, ThumbSize};
use crate::primitives::{ContentPainterCx, Revision, TextureStore, TextureStoreOptions, fit_rect};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, Sense};
use egui::{ColorImage, Id, Pos2, Rect, Response, Vec2};

#[cfg(feature = "testing")]
mod stories;

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
            PictureSize::Icon => cx.control().icon,
            PictureSize::Thumb(step) => step.pick([thumbs.sm, thumbs.md, thumbs.lg]),
            PictureSize::Sample(step) => step.pick([thumbs.sample.sm, thumbs.sample.md, thumbs.sample.lg]),
        };
        PictureStyle { metrics: Metrics::new(), height }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
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
                let store = TextureStore::<(Id, u64)>::in_context(&cx.ctx, "picture", TextureStoreOptions::thumbnails());
                let texture = store.get_or_put(&cx.ctx, (cx.resolved.id, *key), self.revision, *image);
                let target = fit_rect(cx.content_rect, texture.size_vec2());
                let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                ui.content_painter(cx).picture(texture.id, target, whole, 0.0);
            }
            Source::Painter(paint) => paint(&ui.content_painter(cx), cx.content_rect),
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
