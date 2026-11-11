//! `CanvasScrollbars`: the scroll bars of a canvas, as Photoshop's document window has them (20.6): a horizontal bar
//! along the bottom of the view and a vertical one along the right, always shown. The thumb is the visible fraction of
//! the scrollable extent (the document, plus wherever the view has been scrolled past it); drag it to scroll, or press
//! the track to page by one screenful. With overscroll on, a document smaller than the window can also be scrolled from
//! one edge of the window to the other. With overscroll off the app clamps the view before it draws
//! ([`CanvasScrollbars::clamp`]).
//!
//! The bars measure the view along the screen axes, so on a rotated view they show the extent of the unrotated
//! document.

#[cfg(feature = "testing")]
mod stories;

use super::words::CanvasWords;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _, Props as _,
    Role, Styled as _, THUMB, WordSetCx as _,
};
use crate::primitives::PaintShape;
use crate::runtime::access::Numeric;
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, RuleKind, ScrollBarLook, StyleColor, StyleCx};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::scroll::{self, ScrollSpan};
use craft_behavior::{Axis, CanvasView, Metrics, PartKey, PartRef, PressPhase};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};
use std::ops::{Deref, Range};

const HORIZONTAL: PartKey = PartKey::new("horizontal");
const VERTICAL: PartKey = PartKey::new("vertical");

/// The thumb part that `PartsCx::scroll` declares for the bar `bar`.
fn thumb_of(bar: PartKey) -> PartRef {
    bar.at(THUMB)
}

/// The style of the scroll bars.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ScrollbarsStyle {
    #[style(metrics)]
    metrics: Metrics,
    look: ScrollBarLook,
    /// Between a bar's edge and its thumb.
    inset: f32,
    /// The tracks and the corner.
    track: StyleColor,
}

/// One axis of the view, in document units counted from the document's start on that axis.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    /// Document length on this axis.
    len: f64,
    /// First and last visible coordinate.
    v0: f64,
    v1: f64,
}

impl Span {
    /// The axis of a view: `size` screen points at `zoom` around `center`.
    fn of(len: f64, center: f64, size: f64, zoom: f64) -> Option<Span> {
        let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { return None };
        let half = size / zoom / 2.0;
        let span = Span { len, v0: center - half, v1: center + half };
        (span.len.is_finite() && span.len > 0.0 && span.v0.is_finite() && span.v1.is_finite() && span.v1 > span.v0).then_some(span)
    }

    /// The scrollable extent: the document and the visible span. With `overscroll`, a document shorter than the span
    /// may slide edge to edge within it.
    fn content(&self, overscroll: bool) -> Range<f64> {
        let (lo, hi) = (self.v0.min(0.0), self.v1.max(self.len));
        if !overscroll {
            return lo..hi;
        }
        let visible = self.v1 - self.v0;
        lo.min(self.len - visible)..hi.max(visible)
    }

    fn scroll(&self, overscroll: bool) -> ScrollSpan {
        ScrollSpan::new(self.content(overscroll), self.v0..self.v1)
    }
}

/// One bar: where it lies and what it scrolls.
struct Bar {
    rect: Rect,
    span: ScrollSpan,
}

/// The bars of a view over a document, in the order horizontal, vertical.
struct Bars {
    horizontal: Option<Bar>,
    vertical: Option<Bar>,
}

/// How the document's x axis runs on screen: with the view flipped, backwards.
fn along_x(view: &CanvasView, width: f64, c: f64) -> f64 {
    if view.flip_x { width - c } else { c }
}

fn aspect(view: &CanvasView) -> f64 {
    if view.aspect.is_finite() && view.aspect > 0.0 { view.aspect } else { 1.0 }
}

impl Bars {
    /// The bars inside `rect` (bottom and right edges), `width` thick, for `view` over the document `doc`.
    fn of(view: &CanvasView, doc: DocRect, overscroll: bool, horizontal: bool, rect: Rect, width: f32) -> Bars {
        let x = Span::of(doc.width(), along_x(view, doc.width(), view.center.x - doc.x0), f64::from(view.rect.width()) / aspect(view), view.zoom)
            .filter(|_| horizontal);
        let y = Span::of(doc.height(), view.center.y - doc.y0, f64::from(view.rect.height()), view.zoom);
        let right = if y.is_some() { rect.right() - width } else { rect.right() };
        let bottom = if x.is_some() { rect.bottom() - width } else { rect.bottom() };
        let bar = |span: Option<Span>, rect: Rect| span.map(|span| Bar { rect, span: span.scroll(overscroll) });
        Bars {
            horizontal: bar(x, Rect::from_min_max(Pos2::new(rect.left(), bottom), Pos2::new(right, rect.bottom()))),
            vertical: bar(y, Rect::from_min_max(Pos2::new(right, rect.top()), Pos2::new(rect.right(), bottom))),
        }
    }

    fn each(&self) -> impl Iterator<Item = (PartKey, Axis, &Bar)> {
        [(HORIZONTAL, Axis::X, &self.horizontal), (VERTICAL, Axis::Y, &self.vertical)]
            .into_iter()
            .filter_map(|(key, axis, bar)| bar.as_ref().map(|bar| (key, axis, bar)))
    }
}

/// `view` with the visible range on `axis` moved to `range`, in the document units the bars count.
fn moved(view: &CanvasView, doc: DocRect, axis: Axis, range: &Range<f64>) -> CanvasView {
    let c = (range.start + range.end) / 2.0;
    let mut next = *view;
    match axis {
        Axis::X => next.center.x = along_x(view, doc.width(), c) + doc.x0,
        _ => next.center.y = c + doc.y0,
    }
    next
}

/// Clamp one axis of the view centre with overscroll off, as Photoshop 25.4 does: a document larger than the view
/// can't leave its edges, and one smaller than the view stays inside it, where zooming put it.
fn clamp_axis(center: f64, len: f64, size: f64, zoom: f64) -> f64 {
    if !(zoom.is_finite() && zoom > 0.0 && len.is_finite() && len > 0.0 && size.is_finite() && size > 0.0) {
        return center;
    }
    if !center.is_finite() {
        return len / 2.0;
    }
    let half = size / zoom / 2.0;
    let (a, b) = (half, len - half);
    center.clamp(a.min(b), a.max(b))
}

/// The four corners of a document rectangle.
fn doc_corners(doc: DocRect) -> [Point; 4] {
    [Point::new(doc.x0, doc.y0), Point::new(doc.x1, doc.y0), Point::new(doc.x1, doc.y1), Point::new(doc.x0, doc.y1)]
}

/// The scroll bars of a canvas. They sit inside the rect they are shown at, over its bottom and right edges, and take
/// a press before the canvas below does; the app reads [`CanvasScrollbarsOutput::over`] to keep its tool from reacting to the
/// same press. Show them at the view's rect inside the `Canvas` body. The bars are AccessKit scroll bars and no Tab
/// stops: the canvas scrolls from the keyboard as it does without bars.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control)]
pub struct CanvasScrollbars<'a> {
    #[common]
    common: Common,
    view: Bind<'a, CanvasView>,
    document: DocRect,
    /// Whether a document that fits in the window can still be scrolled edge to edge within it (Preferences,
    /// Tools, Overscroll).
    #[prop]
    overscroll: bool,
    /// Shows the vertical bar only, for an app that scrolls one axis: the document's width is then ignored and no
    /// horizontal bar or corner is shown.
    #[prop(flag)]
    vertical_only: bool,
}

impl<'a> CanvasScrollbars<'a> {
    /// Scroll bars named `label` that edit `view`, a view over the document `document` (document units).
    pub fn new(label: impl Into<Label>, view: Bind<'a, CanvasView>, document: DocRect) -> Self {
        CanvasScrollbars { common: Common::new(), view, document, overscroll: false, vertical_only: false }.label(label)
    }

    /// Applies overscroll off to `view` over the document `doc`: the view can't leave the document's edges, and a
    /// document smaller than the view stays inside it. True when the view moved (then repaint).
    pub fn clamp(view: &mut CanvasView, doc: DocRect) -> bool {
        if !(doc.width() > 0.0 && doc.height() > 0.0) || !view.zoom.is_finite() || view.zoom <= 0.0 {
            return false;
        }
        let size = view.rect.size();
        let (w, h) = (doc.width(), doc.height());
        let zoom = view.zoom;
        let local = if view.rotation.abs() < 1e-8 {
            [
                clamp_axis(view.center.x - doc.x0, w, f64::from(size.x) / aspect(view), zoom),
                clamp_axis(view.center.y - doc.y0, h, f64::from(size.y), zoom),
            ]
        } else {
            if !size.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
                return false;
            }
            // The displayed document's bounds against the view's: the offset that puts the document inside the view
            // (or the view inside it) is the same clamp, on screen.
            let level = (*view).flip_x(false);
            let corners = doc_corners(doc).map(|p| level.to_screen(p));
            let image = corners.iter().skip(1).fold(Rect::from_min_max(corners[0], corners[0]), |r, &p| r.union(Rect::from_min_max(p, p)));
            if !image.is_finite() {
                return false;
            }
            let offset = view.rect.center() - image.min;
            let (ox, oy) = (f64::from(offset.x), f64::from(offset.y));
            let wanted = Vec2::new(
                (clamp_axis(ox, f64::from(image.width()), f64::from(size.x), 1.0) - ox) as f32,
                (clamp_axis(oy, f64::from(image.height()), f64::from(size.y), 1.0) - oy) as f32,
            );
            let d = level.unmap_vec(wanted);
            [view.center.x + d.x - doc.x0, view.center.y + d.y - doc.y0]
        };
        let center = Point::new(local[0] + doc.x0, local[1] + doc.y0);
        let changed = !view.center.x.is_finite()
            || !view.center.y.is_finite()
            || (center.x - view.center.x).abs() > 1e-3
            || (center.y - view.center.y).abs() > 1e-3;
        if changed {
            view.center = center;
        }
        changed
    }

    fn bars(&self, view: &CanvasView, rect: Rect, width: f32) -> Bars {
        Bars::of(view, self.document, self.overscroll, !self.vertical_only, rect, width)
    }
}

impl Component for CanvasScrollbars<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS;
    type Style = ScrollbarsStyle;
    type Output = CanvasScrollbarsOutput;

    fn style(cx: &StyleCx<'_>, _: CanvasScrollbarsStyleKey) -> ScrollbarsStyle {
        ScrollbarsStyle {
            metrics: Metrics::new(),
            look: cx.scroll_bar(),
            inset: cx.tokens.space.gap.tight,
            track: cx.color(cx.tokens.color.surface.chrome),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.look.thickness)
    }

    fn bind(&mut self) -> Option<BindMut<'_, CanvasView>> {
        Some(self.view.reborrow())
    }

    /// The bars: `PartsCx::scroll` runs the thumb drag and the page press in the pipeline.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let bars = self.bars(&view, rect, cx.style.look.thickness);
        for (key, axis, bar) in bars.each() {
            cx.scroll(key, bar.rect, axis, bar.span.clone());
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let bars = self.bars(&view, cx.rect, cx.style.look.thickness);
        for (key, axis, _) in bars.each() {
            let phase = cx.part(thumb_of(key)).press.phase;
            match phase {
                PressPhase::DragStart { .. } => cx.begin(),
                PressPhase::Release { click: false, .. } => cx.commit(),
                _ => {}
            }
            let Some(range) = cx.scroll_moved(key) else { continue };
            let next = moved(&view, self.document, axis, &range);
            if matches!(phase, PressPhase::Drag { .. }) {
                cx.propose(next);
            } else if next != view {
                cx.set(next);
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let style = cx.style;
        let bars = self.bars(&view, cx.rect, style.look.thickness);
        let draw = ui.draw();
        if let (Some(h), Some(v)) = (&bars.horizontal, &bars.vertical) {
            draw.fill(&PaintShape::Rect(Rect::from_min_max(Pos2::new(v.rect.left(), h.rect.top()), cx.rect.max), CornerRadius::ZERO), style.track);
        }
        for (key, axis, bar) in bars.each() {
            let track = bar.rect;
            draw.fill(&PaintShape::Rect(track, CornerRadius::ZERO), style.track);
            let (from, to) = match axis {
                Axis::X => (track.left_top(), track.right_top()),
                _ => (track.left_top(), track.left_bottom()),
            };
            draw.rule(RuleKind::Edge, from, to);
            let length = if axis == Axis::X { track.width() } else { track.height() };
            let (at, len) = scroll::thumb(&bar.span, length, style.look.min_thumb);
            let thumb = match axis {
                Axis::X if cx.flow.is_rtl() => Rect::from_min_size(Pos2::new(track.max.x - at - len, track.min.y), Vec2::new(len, track.height())),
                Axis::X => Rect::from_min_size(Pos2::new(track.min.x + at, track.min.y), Vec2::new(len, track.height())),
                _ => Rect::from_min_size(Pos2::new(track.min.x, track.min.y + at), Vec2::new(track.width(), len)),
            }
            .shrink(style.inset);
            // Fully round: the radius is half the thumb's thickness.
            let radius = (thumb.width().min(thumb.height()) / 2.0).clamp(0.0, f32::from(u8::MAX)) as u8;
            draw.fill(&PaintShape::Rect(thumb, CornerRadius::same(radius)), cx.resolve_part(thumb_of(key), &style.look.thumb));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let words = cx.words::<CanvasWords>();
        let bars = self.bars(&view, cx.rect, cx.style.look.thickness);
        for (key, axis, bar) in bars.each() {
            let name = if axis == Axis::X { &words.scroll_horizontal } else { &words.scroll_vertical };
            let room = (bar.span.content.end - bar.span.content.start - (bar.span.visible.end - bar.span.visible.start)).max(0.0);
            let position = (bar.span.visible.start - bar.span.content.start).clamp(0.0, room);
            let node = access.node(key, Role::SCROLL_BAR);
            node.name = Name::Text(name.as_str().to_owned());
            node.numeric = Some(Numeric::new(position, 0.0..=room));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CanvasScrollbarsOutput {
        let over = [HORIZONTAL, VERTICAL].into_iter().any(|key| {
            [PartRef::from(key), thumb_of(key)]
                .into_iter()
                .any(|part| cx.part_if_declared(part).is_some_and(|result| result.hovered || result.press.phase.is_press()))
        });
        CanvasScrollbarsOutput { view: cx.edit_response(), over }
    }
}

/// What the scroll bars report in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct CanvasScrollbarsOutput {
    /// The view: a thumb drag's gesture, a page press's change.
    pub view: EditResponse<CanvasView>,
    /// The pointer is over a bar or dragging one: the canvas leaves it alone.
    pub over: bool,
}

impl Deref for CanvasScrollbarsOutput {
    type Target = EditResponse<CanvasView>;

    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

impl Outcome for CanvasScrollbarsOutput {
    type Value = CanvasView;

    fn response(&self) -> &Response {
        &self.view.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.view.story_facts()
    }
}
