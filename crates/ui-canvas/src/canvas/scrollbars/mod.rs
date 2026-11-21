//! `CanvasScrollbars`: the scroll bars of a canvas (20.6): a horizontal bar along the bottom of the view and a vertical
//! one along the right, always shown, in the docked look (`ScrollBarKind::Docked`): the thumb has the track's whole
//! thickness at rest and under the pointer. Document chrome uses inset thumbs, step buttons and a corner.
//! The thumb is the visible fraction of
//! the scrollable extent (the document, plus wherever the view has been scrolled past it); drag it to scroll, or press
//! the track to page by one screenful. With overscroll on, a document smaller than the window can also be scrolled from
//! one edge of the window to the other. With overscroll off the app clamps the view before it draws
//! ([`CanvasScrollbars::clamp`]).
//!
//! The bars measure the view along the screen axes, so on a rotated view they show the extent of the unrotated
//! document.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::words::CanvasWords;
use crate::contract::overlay::{Anchor, Place};
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Name, Outcome, Overlay, OverlayUi, Overlays as _, PaintUi,
    PartsCx, Pointer as _, Props as _, Remember as _, Role, Show as _, THUMB, WordSetCx as _,
};
use crate::display::Text;
use crate::primitives::{Draw, PaintShape, ScrollBar, StrokeWidth};
use crate::runtime::access::Numeric;
use crate::runtime::commands::FireSource;
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::runtime::frame;
use crate::style::{ComponentStyle, ScrollBarKind, ScrollBarLook, Stateful, StyleColor, StyleCx};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::overlay::{Align, Side};
use craft_behavior::scroll::ScrollSpan;
use craft_behavior::{Axis, CanvasView, Metrics, PartKey, PartRef, PressPhase, Sense};
use egui::{CornerRadius, Rect, Response, Vec2, pos2, vec2};
use std::ops::{Deref, Range};

const HORIZONTAL: PartKey = PartKey::new("horizontal");
const CORNER: PartKey = PartKey::new("corner");
const VERTICAL: PartKey = PartKey::new("vertical");
const START_ARROW: u64 = 1;
const END_ARROW: u64 = 2;
/// Distance of a line step in device pixels, like the canvas's scroll wheel.
const ARROW_STEP_PIXELS: f64 = 16.0;
const ARROW_HOLD_DELAY: f64 = 0.4;
const ARROW_HOLD_INTERVAL: f64 = 0.06;

/// The popup of the tip beside a dragged vertical thumb.
const TIP: &str = "tip";

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
    document: Option<DocumentLook>,
}

#[derive(Clone, Debug, PartialEq)]
struct DocumentLook {
    corner: StyleColor,
    arrow: Stateful<StyleColor>,
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
    track: Rect,
    span: ScrollSpan,
}

impl Bar {
    fn new(rect: Rect, axis: Axis, span: ScrollSpan, document_chrome: bool) -> Self {
        let mut track = rect;
        if document_chrome {
            let cap = if axis == Axis::X { rect.height() } else { rect.width() };
            let length = if axis == Axis::X { rect.width() } else { rect.height() };
            if length > 2.0 * cap && cap > 0.0 {
                let inset = if axis == Axis::X { vec2(cap, 0.0) } else { vec2(0.0, cap) };
                track = rect.shrink2(inset);
            }
        }
        Self { rect, track, span }
    }

    fn arrows(&self, key: PartKey, axis: Axis) -> impl Iterator<Item = (PartRef, Rect, bool)> {
        (self.track != self.rect)
            .then(|| {
                let (before, after) = if axis == Axis::X {
                    (pos2(self.track.left(), self.rect.bottom()), pos2(self.track.right(), self.rect.top()))
                } else {
                    (pos2(self.rect.right(), self.track.top()), pos2(self.rect.left(), self.track.bottom()))
                };
                [
                    (key.at(START_ARROW), Rect::from_min_max(self.rect.min, before), false),
                    (key.at(END_ARROW), Rect::from_min_max(after, self.rect.max), true),
                ]
            })
            .into_iter()
            .flatten()
    }
}

/// The bars of a view over a document, in the order horizontal, vertical.
struct Bars {
    horizontal: Option<Bar>,
    vertical: Option<Bar>,
    corner: Option<Rect>,
}

#[derive(Clone, Default)]
struct ArrowRepeat {
    part: Option<PartRef>,
    next: f64,
}

fn arrow_pressed(cx: &mut Cx<'_, CanvasScrollbars<'_>>, part: PartRef, rect: Rect) -> bool {
    if matches!(cx.part_activated(part), Some(FireSource::Access | FireSource::Key)) {
        return true;
    }
    let phase = cx.part(part).press.phase;
    if !matches!(phase, PressPhase::Down { .. } | PressPhase::Held { .. }) {
        return false;
    }
    let ctx = cx.response.ctx.clone();
    let now = frame::clock(&ctx);
    match phase {
        PressPhase::Down { .. } => {
            *cx.memory::<ArrowRepeat>() = ArrowRepeat { part: Some(part), next: now + ARROW_HOLD_DELAY };
            frame::request_repaint_after(&ctx, ARROW_HOLD_DELAY);
            true
        }
        PressPhase::Held { pos, .. } if rect.contains(pos) => {
            let repeat = cx.memory::<ArrowRepeat>();
            if repeat.part != Some(part) {
                return false;
            }
            let due = now >= repeat.next;
            if due {
                repeat.next = now + ARROW_HOLD_INTERVAL;
            }
            frame::request_repaint_after(&ctx, repeat.next - now);
            due
        }
        _ => false,
    }
}

fn paint_arrow(draw: &Draw<'_>, rect: Rect, axis: Axis, after: bool, color: StyleColor) {
    let size = rect.width().min(rect.height()) / 4.0;
    let sign = if after { 1.0 } else { -1.0 };
    let point = |along, across| {
        let along = along * sign;
        rect.center() + if axis == Axis::X { vec2(along, across) } else { vec2(across, along) }
    };
    let path = PaintShape::Path(vec![point(-size * 0.5, -size), point(size * 0.5, 0.0), point(-size * 0.5, size)]);
    draw.stroke(&path, StrokeWidth::Thin, color);
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
    fn of(view: &CanvasView, doc: DocRect, overscroll: bool, horizontal: bool, rect: Rect, style: &ScrollbarsStyle) -> Bars {
        let document_chrome = style.document.is_some();
        let mut width = style.look.thickness;
        if document_chrome {
            width = width.min(rect.width().max(0.0)).min(rect.height().max(0.0));
        }
        let x = Span::of(doc.width(), along_x(view, doc.width(), view.center.x - doc.x0), f64::from(view.rect.width()) / aspect(view), view.zoom)
            .filter(|_| horizontal);
        let y = Span::of(doc.height(), view.center.y - doc.y0, f64::from(view.rect.height()), view.zoom);
        let right = if y.is_some() { rect.right() - width } else { rect.right() };
        let bottom = if x.is_some() { rect.bottom() - width } else { rect.bottom() };
        let corner = (document_chrome && x.is_some() && y.is_some()).then(|| Rect::from_min_max(pos2(right, bottom), rect.max));
        let bar = |span: Option<Span>, axis, rect| span.map(|span| Bar::new(rect, axis, span.scroll(overscroll), document_chrome));
        Bars {
            horizontal: bar(x, Axis::X, Rect::from_min_max(pos2(rect.left(), bottom), pos2(right, rect.bottom()))),
            vertical: bar(y, Axis::Y, Rect::from_min_max(pos2(right, rect.top()), pos2(rect.right(), bottom))),
            corner,
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

/// Clamp one axis of the view centre with overscroll off: a document larger than the view can't leave its edges, and
/// one smaller than the view (or exactly its size) is centred in it, so zooming out around the pointer keeps it in the
/// middle. Photoshop 25.4 was measured leaving a smaller document where zooming put it; reports of Photoshop itself
/// say it stays centred with Overscroll off, which is what turning Overscroll off asks for. Overscroll on keeps the
/// free view.
fn clamp_axis(center: f64, len: f64, size: f64, zoom: f64) -> f64 {
    if !(zoom.is_finite() && zoom > 0.0 && len.is_finite() && len > 0.0 && size.is_finite() && size > 0.0) {
        return center;
    }
    if !center.is_finite() {
        return len / 2.0;
    }
    let half = size / zoom / 2.0;
    if 2.0 * half >= len {
        return len / 2.0;
    }
    center.clamp(half, len - half)
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
#[component(canvas_control, overlay)]
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
    /// Wider bars with inset thumbs, step arrows and a corner for a document window.
    #[prop(flag)]
    #[style_key]
    document_chrome: bool,
    /// While the vertical thumb drags, the bars show the tip this builds from the view as the drag moved it, beside the
    /// thumb on the reading-start side.
    #[prop]
    tip: Option<&'a dyn Fn(&CanvasView) -> Label>,
}

impl<'a> CanvasScrollbars<'a> {
    /// Scroll bars named `label` that edit `view`, a view over the document `document` (document units).
    pub fn new(label: impl Into<Label>, view: Bind<'a, CanvasView>, document: DocRect) -> Self {
        CanvasScrollbars { common: Common::new(), view, document, overscroll: false, vertical_only: false, document_chrome: false, tip: None }
            .label(label)
    }

    /// Applies overscroll off to `view` over the document `doc`: the view can't leave the document's edges, and a
    /// document smaller than the view is centred in it. True when the view moved (then repaint).
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

    fn bars(&self, view: &CanvasView, rect: Rect, style: &ScrollbarsStyle) -> Bars {
        Bars::of(view, self.document, self.overscroll, !self.vertical_only, rect, style)
    }
}

impl Component for CanvasScrollbars<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS;
    type Style = ScrollbarsStyle;
    type Output = CanvasScrollbarsOutput;

    fn style(cx: &StyleCx<'_>, key: CanvasScrollbarsStyleKey) -> ScrollbarsStyle {
        let mut look = cx.scroll_bar_of(ScrollBarKind::Docked);
        let document = key.document_chrome.then(|| {
            let color = &cx.tokens.color;
            look.thickness = cx.tokens.size.icon.md;
            look.track = Stateful::new(cx.color(color.surface.sunken));
            DocumentLook { corner: cx.color(color.surface.panel), arrow: cx.quiet_ink() }
        });
        ScrollbarsStyle { metrics: Metrics::new(), look, document }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.look.thickness)
    }

    fn bind(&mut self) -> Option<BindMut<'_, CanvasView>> {
        Some(self.view.reborrow())
    }

    /// The canvas settles a view it is given to whole device pixels and to its own rect, so a thumb drag's proposal
    /// comes back with its centre moved by under a point (photocraft#16006): that is the drag's own value, not the app
    /// changing the view under it, and the drag goes on. A different zoom, turn, mirroring or aspect, or a centre a
    /// point or more away, is a change.
    fn same_value(&self, proposed: &CanvasView, host: &CanvasView) -> bool {
        let reach = 1.0 / proposed.zoom.max(f64::MIN_POSITIVE);
        proposed.zoom == host.zoom
            && proposed.rotation == host.rotation
            && proposed.flip_x == host.flip_x
            && proposed.aspect == host.aspect
            && (proposed.center.x - host.center.x).abs() < reach
            && (proposed.center.y - host.center.y).abs() < reach
    }

    /// The bars: `PartsCx::scroll` runs the thumb drag and the page press in the pipeline.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let bars = self.bars(&view, rect, cx.style);
        for (key, axis, bar) in bars.each() {
            cx.scroll(key, bar.track, axis, bar.span.clone());
            for (part, rect, _) in bar.arrows(key, axis) {
                // As in the reference application the step arrows take no keyboard focus: the canvas scrolls with its own
                // keys (the arrows, Page Up/Down, Home and End), and a focused arrow would take the Escape that ends a
                // tool's edit on the canvas.
                cx.part(part, rect).sense(Sense::CLICK).pointer_only("the canvas scrolls with its own keys");
            }
        }
        if let Some(corner) = bars.corner {
            cx.part(CORNER, corner).sense(Sense::CLICK).pointer_only("the corner only reserves space between the two scroll bars");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // The tip shows in the passes the vertical thumb drags and in no other: its popup opens or closes every pass.
        let tip = cx.popup(TIP);
        if self.tip.is_some() && thumb_dragging(cx) == Some(Axis::Y) {
            tip.open(cx);
        } else {
            tip.close(cx);
        }
        let Some(view) = cx.value().copied() else { return };
        let bars = self.bars(&view, cx.rect, cx.style);
        for (key, axis, bar) in bars.each() {
            for (part, rect, after) in bar.arrows(key, axis) {
                if arrow_pressed(cx, part, rect) {
                    let scale = if axis == Axis::X { aspect(&view) } else { 1.0 };
                    let pixel_zoom = view.zoom * scale * f64::from(cx.response.ctx.pixels_per_point());
                    let toward_end = after != (axis == Axis::X && cx.flow.is_rtl());
                    let sign = if toward_end { 1.0 } else { -1.0 };
                    let range = craft_behavior::scroll::shifted(&bar.span, sign * ARROW_STEP_PIXELS / pixel_zoom);
                    let next = moved(&view, self.document, axis, &range);
                    if next != view {
                        cx.set(next);
                    }
                }
            }
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

    /// The tip beside the dragged vertical thumb, on the reading-start side, in a tooltip layer of the bars' own: the
    /// tooltip pipeline hides a tip while its subject is pressed, and here the press is the point.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup(TIP);
        let (Some(tip), Some(view), Some(thumb)) = (self.tip, cx.value().copied(), cx.part_if_declared(thumb_of(VERTICAL)).map(|result| result.rect))
        else {
            return;
        };
        let label = tip(&view);
        let layer = Overlay::tooltip().place(Place::anchored(Anchor::Rect(thumb), Side::Start, Align::Center));
        let _ = ui.show(popup, layer, |ui, _| Text::new(label.clone()).show(ui));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let style = cx.style;
        let bars = self.bars(&view, cx.rect, style);
        let draw = ui.draw();
        let rtl = cx.flow.is_rtl();
        let fill = |rect, color| draw.fill(&PaintShape::Rect(rect, CornerRadius::ZERO), color);
        for (key, axis, bar) in bars.each() {
            let state = cx.scroll_bar_state(key);
            if let Some(look) = &style.document {
                fill(bar.rect, state.resolve(&style.look.track));
                let inset = if axis == Axis::X { vec2(0.0, bar.rect.height() / 8.0) } else { vec2(bar.rect.width() / 8.0, 0.0) };
                let thumb = ScrollBar::thumb(style.look.min_thumb, bar.track, axis, rtl, &bar.span).shrink2(inset);
                fill(thumb, state.resolve(&style.look.thumb));
                for (part, rect, after) in bar.arrows(key, axis) {
                    let color = cx.part_state(part).resolve(&look.arrow);
                    paint_arrow(&draw, rect, axis, after, color);
                }
            } else {
                draw.scroll_bar(&style.look, bar.track, axis, rtl, &bar.span, &state);
            }
        }
        if let (Some(corner), Some(look)) = (bars.corner, &style.document) {
            fill(corner, look.corner);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let words = cx.words::<CanvasWords>();
        let bars = self.bars(&view, cx.rect, cx.style);
        for (key, axis, bar) in bars.each() {
            let name = if axis == Axis::X { &words.scroll_horizontal } else { &words.scroll_vertical };
            let room = (bar.span.content.end - bar.span.content.start - (bar.span.visible.end - bar.span.visible.start)).max(0.0);
            let position = (bar.span.visible.start - bar.span.content.start).clamp(0.0, room);
            let node = access.node(key, Role::SCROLL_BAR);
            node.name = Name::Text(name.as_str().to_owned());
            node.numeric = Some(Numeric::new(position, 0.0..=room));
            for (part, _, after) in bar.arrows(key, axis) {
                let name = match (axis, after) {
                    (Axis::X, false) => &words.scroll_left,
                    (Axis::X, true) => &words.scroll_right,
                    (_, false) => &words.scroll_up,
                    (_, true) => &words.scroll_down,
                };
                access.node(part, Role::BUTTON).name = Name::Text(name.as_str().to_owned());
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CanvasScrollbarsOutput {
        let is_over = |part| cx.part_if_declared(part).is_some_and(|result| result.hovered || result.press.phase.is_press());
        let over = is_over(PartRef::from(CORNER))
            || [HORIZONTAL, VERTICAL].into_iter().any(|key| {
                let arrows = self.document_chrome.then_some([key.at(START_ARROW), key.at(END_ARROW)]);
                [PartRef::from(key), thumb_of(key)].into_iter().chain(arrows.into_iter().flatten()).any(is_over)
            });
        let dragging = thumb_dragging(&cx);
        CanvasScrollbarsOutput { view: cx.edit_response(), over, dragging }
    }
}

/// The axis whose thumb drags in this pass: its press past the drag threshold and still down.
fn thumb_dragging(cx: &Cx<'_, CanvasScrollbars<'_>>) -> Option<Axis> {
    [(HORIZONTAL, Axis::X), (VERTICAL, Axis::Y)].into_iter().find_map(|(key, axis)| {
        let press = cx.part_if_declared(thumb_of(key))?.press;
        matches!(press.phase, PressPhase::DragStart { .. } | PressPhase::Drag { .. }).then_some(axis)
    })
}

/// What the scroll bars report in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct CanvasScrollbarsOutput {
    /// The view: a thumb drag's gesture, a page press's change.
    pub view: EditResponse<CanvasView>,
    /// The pointer is over a bar or dragging one: the canvas leaves it alone.
    pub over: bool,
    /// The thumb being dragged this pass.
    pub dragging: Option<Axis>,
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
