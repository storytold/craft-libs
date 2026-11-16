//! `InspectOverlay`: the debug inspector's drawing over an inspected region ([`inspect::record`]): each element's
//! padding, the hit areas and the focus stops numbered in Tab order, as the record of the region says.

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Role};
use crate::primitives::{Draw, PaintShape, StrokeWidth, TextRun};
use crate::runtime::inspect::Recorded;
use crate::style::{ComponentStyle, StyleColor, StyleCx, TypeRole};
use craft_behavior::{Metrics, Sense};
use craft_theme::Opacity;
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// Which drawings an [`InspectOverlay`] shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InspectLayers {
    /// Each element's box, its padding filled.
    pub padding: bool,
    /// The hit areas; those under `size.control.sm` across are flagged.
    pub hits: bool,
    /// The focus stops, numbered in Tab order.
    pub focus: bool,
}

impl Default for InspectLayers {
    fn default() -> Self {
        InspectLayers { padding: true, hits: true, focus: true }
    }
}

/// The sizes and colours of an [`InspectOverlay`].
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct InspectOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    guide: StyleColor,
    band: StyleColor,
    stroke: StyleColor,
    flagged: StyleColor,
    halo: StyleColor,
    marquee: StyleColor,
    marquee_fill: StyleColor,
    badge: StyleColor,
    badge_text: StyleColor,
    /// Hit areas under this size across are flagged.
    pub min_target: f32,
}

/// The inspector's drawing over an inspected region, shown at the region's rect with `show_at`. It draws the
/// region's record and takes no input.
///
/// ```ignore
/// let (_, recorded) = inspect::record(ui, key, |ui| story(ui));
/// if let Some(recorded) = recorded {
///     let overlay = InspectOverlay::new(recorded);
///     match inspect::pointed(ui.ctx(), key) {
///         Some(index) => overlay.pointed(index),
///         None => overlay,
///     }
///     .show_at(ui, rect);
/// }
/// ```
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct InspectOverlay {
    #[common]
    common: Common,
    recorded: Arc<Recorded>,
    /// The layers shown.
    #[prop]
    layers: InspectLayers,
    /// Marks the element at this index of the record, the one the inspector's list points at.
    #[prop]
    pointed: Option<usize>,
}

impl InspectOverlay {
    /// The drawing of `recorded`, every layer on.
    pub fn new(recorded: Arc<Recorded>) -> Self {
        InspectOverlay { common: Common::new(), recorded, layers: InspectLayers::default(), pointed: None }
    }
}

/// The band between `outer` and `content` as up to four rects.
fn band_parts(outer: Rect, content: Rect) -> impl Iterator<Item = Rect> {
    let inner = content.intersect(outer);
    [
        Rect::from_min_max(outer.min, Pos2::new(outer.max.x, inner.min.y)),
        Rect::from_min_max(Pos2::new(outer.min.x, inner.max.y), outer.max),
        Rect::from_min_max(Pos2::new(outer.min.x, inner.min.y), Pos2::new(inner.min.x, inner.max.y)),
        Rect::from_min_max(Pos2::new(inner.max.x, inner.min.y), Pos2::new(outer.max.x, inner.max.y)),
    ]
    .into_iter()
    .filter(|part| part.is_positive())
}

impl Component for InspectOverlay {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = InspectOverlayStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: InspectOverlayStyleKey) -> InspectOverlayStyle {
        let color = &cx.tokens.color;
        InspectOverlayStyle {
            metrics: Metrics::new(),
            guide: cx.color(color.overlay.guide),
            band: cx.color(color.overlay.guide.faded(Opacity::TINT)),
            stroke: cx.color(color.overlay.stroke),
            flagged: cx.color(color.status.bad.solid),
            halo: cx.color(color.overlay.halo),
            marquee: cx.color(color.overlay.marquee),
            marquee_fill: cx.color(color.overlay.marquee.faded(Opacity::TINT)),
            badge: cx.color(color.status.info.solid),
            badge_text: cx.color(color.text.on_status),
            min_target: cx.tokens.size.control.sm,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// Paints cut to the rect it is shown at, as the region it draws over.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.draw().clipped(cx.content_rect, |draw| self.draw(draw, cx));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl InspectOverlay {
    fn draw(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>) {
        let (style, recorded) = (cx.style, &*self.recorded);
        let rect = |rect: Rect| PaintShape::Rect(rect, CornerRadius::ZERO);
        if self.layers.padding {
            for element in recorded.elements.iter().filter(|element| element.part.is_none() && element.rect.is_positive()) {
                for part in band_parts(element.rect, element.content) {
                    draw.fill(&rect(part), style.band);
                }
                draw.stroke(&rect(element.content), StrokeWidth::Hairline, style.guide);
            }
        }
        if self.layers.hits {
            for hit in &recorded.hits {
                let small = hit.rect.width() < style.min_target || hit.rect.height() < style.min_target;
                let (width, halo, color) = if small {
                    (StrokeWidth::Medium, StrokeWidth::Heavy, style.flagged)
                } else {
                    (StrokeWidth::Thin, StrokeWidth::Thick, style.stroke)
                };
                draw.stroke(&rect(hit.rect), halo, style.halo);
                draw.stroke(&rect(hit.rect), width, color);
            }
        }
        if let Some(bounds) = self.pointed.and_then(|index| recorded.elements.get(index)).map(|element| element.rect) {
            draw.fill(&rect(bounds), style.marquee_fill);
            draw.stroke(&rect(bounds), StrokeWidth::Medium, style.marquee);
        }
        if self.layers.focus {
            let rtl = cx.env.direction().is_rtl();
            let mut stops: Vec<(usize, usize)> =
                recorded.elements.iter().enumerate().filter_map(|(index, element)| Some((element.stop?, index))).collect();
            stops.sort_unstable();
            for (number, index) in stops {
                let Some(bounds) = recorded.stop_rect(index) else { continue };
                let centre = if rtl { bounds.right_top() } else { bounds.left_top() };
                let run = TextRun::new(number.to_string()).role(TypeRole::Caption).color(style.badge_text);
                let layout = draw.layout(&run, f32::INFINITY);
                let radius = (layout.size().max_elem() / 2.0 + 2.0).max(7.0);
                draw.fill(&PaintShape::Circle(centre, radius), style.badge);
                draw.stroke(&PaintShape::Circle(centre, radius), StrokeWidth::Thin, style.halo);
                draw.galley(centre - layout.size() / 2.0, &layout);
            }
        }
    }
}
