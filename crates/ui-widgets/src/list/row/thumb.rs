//! A row's thumbnails: their size settings, targets, painters and the RowThumb component.

#[cfg(doc)]
use super::ListRow;
use crate::contract::{Common, Component, Cx, PaintUi, Role};
use crate::list::words::ListWords;
use crate::primitives::ContentPainterCx;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, Sense};
use egui::{Rect, Response};
use std::any::Any;
use std::sync::Arc;

/// The width of a thumbnail as a multiple of its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThumbAspect(f32);

impl ThumbAspect {
    /// As wide as high.
    pub const SQUARE: Self = Self(1.0);

    /// `wide` parts of width to `high` parts of height; a zero part counts as a vanishing one.
    pub fn new(wide: u16, high: u16) -> Self {
        Self((f32::from(wide) / f32::from(high.max(1))).max(f32::EPSILON))
    }

    pub(crate) fn of(self, height: f32) -> f32 {
        self.0 * height
    }
}

/// The thumbnail setting of a list, which the host stores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ThumbSize {
    /// No thumbnail: rows are one control high.
    #[default]
    None,
    /// `size.thumb.sm`.
    Small,
    /// `size.thumb.md`.
    Medium,
    /// `size.thumb.lg`.
    Large,
}

impl ThumbSize {
    /// Every setting a user picks from.
    pub const ALL: [ThumbSize; 4] = [ThumbSize::None, ThumbSize::Small, ThumbSize::Medium, ThumbSize::Large];

    /// The side the setting takes from the small, medium and large sides; 0 for `None`.
    pub(crate) const fn pick(self, sides: [f32; 3]) -> f32 {
        match self {
            ThumbSize::None => 0.0,
            ThumbSize::Small => sides[0],
            ThumbSize::Medium => sides[1],
            ThumbSize::Large => sides[2],
        }
    }

    /// The setting's name in the language of `words`.
    pub fn label(self, words: &ListWords) -> Label {
        match self {
            ThumbSize::None => words.thumb_none.clone(),
            ThumbSize::Small => words.thumb_small.clone(),
            ThumbSize::Medium => words.thumb_medium.clone(),
            ThumbSize::Large => words.thumb_large.clone(),
        }
    }
}

/// One of a row's thumbnails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ThumbTarget {
    /// The first thumbnail ([`ListRow::thumb`]).
    First,
    /// A thumbnail after the first ([`ListRow::extra_thumb`]), counted from 0 in the order added.
    Extra(usize),
}

/// Paints into a thumbnail's rect through the content painter, which is clipped to it.
pub type ThumbPaint<'a> = Box<dyn FnOnce(&ContentPainterCx, Rect) + 'a>;

/// A thumbnail after the first: its painter and the link glyph before it.
pub(in crate::list) struct Thumb<'a> {
    pub paint: Option<ThumbPaint<'a>>,
    /// The state of the link glyph before an extra thumbnail and its name, a whole pattern with `{row}`.
    pub link: Option<(bool, Label)>,
}

/// The payload a thumbnail's drag session carries.
pub(super) type ThumbPayload = Arc<dyn Any + Send + Sync>;

/// The thumbnail at position `at` of the row's thumbnails: the first, then the extra ones.
pub(super) fn target_at(at: usize) -> ThumbTarget {
    if at == 0 { ThumbTarget::First } else { ThumbTarget::Extra(at - 1) }
}

/// The style of a thumbnail: it has no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
pub(crate) struct RowThumbStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The picture of a row's thumbnail, painted by the host through the content painter, which is clipped to the
/// thumbnail's rect.
#[derive(crate::contract::Component)]
#[component(content_painter, sample = RowThumb::sample)]
pub(crate) struct RowThumb<'a> {
    #[common]
    common: Common,
    paint: Option<ThumbPaint<'a>>,
}

impl RowThumb<'static> {
    #[cfg(feature = "testing")]
    fn sample() -> Self {
        RowThumb::new(Box::new(|_, _| {}))
    }
}

impl<'a> RowThumb<'a> {
    pub(crate) fn new(paint: ThumbPaint<'a>) -> Self {
        RowThumb { common: Common::new(), paint: Some(paint) }
    }
}

impl Component for RowThumb<'_> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = RowThumbStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: RowThumbStyleKey) -> RowThumbStyle {
        RowThumbStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        if let Some(paint) = self.paint.take() {
            paint(&painter, cx.rect);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
