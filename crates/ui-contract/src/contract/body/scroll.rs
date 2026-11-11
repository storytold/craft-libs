//! The body's scroll region: its offset and jumps, the scroll bars, their input and AccessKit scroll actions.

use super::BodyHooks;
use super::inherit::own_surface;
use super::placement::overlay_of;
use super::regions::rect_of;
use crate::contract::{Container, ContainerProps, Cx, Scroll};
use crate::primitives::{HiddenContent, ScrollBar};
use crate::runtime::egui::pass;
use crate::runtime::egui::{memory as store, repaint, sizing};
use crate::runtime::env::Env;
use crate::style::{SurfaceKind, UiTokens, scroll_bar_look, surface_style};
use craft_behavior::{Axis, PartKey, ScrollPos, ScrollSpan};
use egui::{Context, Id, Pos2, Rect, Ui, Vec2};

const SCROLL_Y: PartKey = PartKey::new("body_scroll_y");

const SCROLL_X: PartKey = PartKey::new("body_scroll_x");

/// The scroll offset a container with no `Component::scroll` binding keeps for the session.
#[derive(Clone, Default)]
pub(super) struct ScrollState {
    pub(super) offset: (f64, f64),
    /// A bottom-anchored region follows the end while nothing moved it away.
    pub(super) follow_end: Option<bool>,
}

/// The offsets `scroll_input` moved scroll bodies to, with the frame that queued each, for step 4 of each body's next
/// pass. An entry no body took in that frame or the next belongs to a body that is no longer shown: it is dropped.
#[derive(Default)]
pub(super) struct Jumps(pub(super) Vec<(Id, ScrollPos, u64)>);

/// The frame a jump is queued and taken in: the root's, since one store serves the bodies of every viewport.
pub(super) fn jump_frame(ctx: &Context) -> u64 {
    pass::root_frame_nr(ctx)
}

/// The room the scroll bars of `scroll` take from a view of `view` over content of `extent`: `bar` across the view
/// for each axis whose content overflows it.
pub(super) fn overflow_inset(scroll: Scroll, extent: Vec2, view: Vec2, bar: f32) -> Vec2 {
    Vec2::new(
        if scroll.vertical_on() && f64::from(extent.y) > f64::from(view.y) + 0.5 { bar } else { 0.0 },
        if scroll.horizontal_on() && f64::from(extent.x) > f64::from(view.x) + 0.5 { bar } else { 0.0 },
    )
}

/// The scroll region of a container: its own, or for a layer the vertical one that takes the overflow of any fit
/// (none when the body is one collection, which owns the scroll, or the overlay turns its body scroll off). A
/// container whose body is one collection has no scroll of its own, whatever its constructor set.
pub(super) fn scroll_of(props: &ContainerProps) -> Option<Scroll> {
    if props.collection_body {
        return None;
    }
    props.scroll.or_else(|| overlay_of(props).filter(|overlay| overlay.body_scroll).map(|_| Scroll::vertical()))
}

/// One axis of a scroll region: its track and its span.
pub(super) struct Bar {
    pub(super) key: PartKey,
    pub(super) axis: Axis,
    pub(super) track: Rect,
}

impl BodyHooks<'_> {
    /// The edge fades and the scroll bars over the body. The fades blend the content into the surface it sits on.
    pub(super) fn paint_scroll_bars<C: Container>(&self, ui: &mut Ui, cx: &Cx<'_, C>, props: &ContainerProps, rtl: bool, env: &Env) {
        let tokens = &*env.tokens;
        let look = scroll_bar_look(tokens);
        if let Some(view) = self.view
            && let Some(scroll) = scroll_of(props)
        {
            let (ex, ey) = self.last.extent;
            let content = Vec2::new(if scroll.horizontal_on() { ex } else { view.width() }, if scroll.vertical_on() { ey } else { view.height() });
            let offset = Vec2::new(self.offset.x as f32, self.offset.y as f32);
            let background = surface_style(tokens, SurfaceKind::under(own_surface(props, cx), env.surface)).fill;
            ui.painter().extend(ScrollBar::fades(&look, view, HiddenContent::new(content, view.size(), offset, rtl), background));
        }
        if let Some(view) = self.view {
            for (bar, span) in self.scroll_geometry(props, view, rtl, tokens) {
                let overflow = span.content.end - span.content.start > span.visible.end - span.visible.start;
                if !overflow {
                    continue;
                }
                let length = match bar.axis {
                    Axis::X => bar.track.width(),
                    _ => bar.track.height(),
                };
                let (at, len) = craft_behavior::scroll::thumb(&span, length, look.min_thumb);
                let thumb = crate::contract::cx::thumb_rect(bar.track, bar.axis, rtl, at, len);
                let enabled = cx.core.resolved.enabled;
                let state = crate::contract::steps::part_state(
                    cx.core.results.get(&(bar.key, Some(crate::contract::cx::THUMB))).unwrap_or(&crate::primitives::PartResult::IDLE),
                    enabled,
                    None,
                );
                ui.painter().extend(ScrollBar::shapes(&look, bar.track, thumb, &state));
            }
        }
    }

    /// Thumb drags, track presses and AccessKit scroll actions. They move the view at step 4 of the next pass, as the
    /// wheel does there, and that pass repeats this frame: the pass that paints an offset is the one whose step 4 set
    /// it, so the content laid out for the view (out-of-view children skip their run, `runtime::lazy`) is the content
    /// shown. A pass that repeats a frame carries no input, and a press held since the frame's first pass adds nothing
    /// there.
    pub(super) fn scroll_input<C: Container>(&self, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        use crate::contract::Pointer as _;
        let Some(scroll) = scroll_of(props) else { return };
        let ctx = cx.core.ctx.clone();
        if ctx.current_pass_index() > 0 {
            return;
        }
        let mut offset = self.offset;
        if let Some(range) = cx.scroll_moved(SCROLL_Y) {
            offset.y = range.start;
        }
        if let Some(range) = cx.scroll_moved(SCROLL_X) {
            offset.x = range.start;
        }
        self.access_scroll(scroll, &mut offset, cx);
        let view = self.last.view.map_or(Vec2::ZERO, |view| rect_of(view).size());
        let offset = self.clamped(scroll, view, offset);
        if offset != self.offset {
            let id = cx.core.id;
            let frame = jump_frame(&ctx);
            store::Service::<Jumps>::run(&ctx, |jumps| {
                jumps.0.retain(|(owner, _, queued)| *owner != id && queued + 1 >= frame);
                jumps.0.push((id, offset, frame));
            });
            // A bound offset reaches the app in the repeated pass.
            sizing::request_discard(&ctx, sizing::Rerun::HeldUpdate);
            if !ctx.will_discard() {
                repaint::request(&ctx);
            }
        }
    }

    /// AccessKit scroll actions aimed at the container's node, applied to `offset`: a page along an axis, or an offset.
    fn access_scroll<C: Container>(&self, scroll: Scroll, offset: &mut ScrollPos, cx: &mut Cx<'_, C>) {
        use craft_behavior::{AccessAction as Action, AccessData as ActionData};
        let rtl = cx.core.env.direction().is_rtl();
        let view = self.last.view.map_or(Vec2::ZERO, |view| rect_of(view).size());
        while let Some(request) = cx.core.intents.take_scroll_access(false) {
            let (vertical, toward_end) = match request.action {
                Action::ScrollUp => (true, false),
                Action::ScrollDown => (true, true),
                Action::ScrollLeft => (false, rtl),
                Action::ScrollRight => (false, !rtl),
                _ => {
                    if let Some(ActionData::SetScrollOffset(point)) = request.data {
                        *offset = ScrollPos { x: point.x, y: point.y };
                    }
                    continue;
                }
            };
            let (visible, extent, at) = if vertical { (view.y, self.last.extent.1, offset.y) } else { (view.x, self.last.extent.0, offset.x) };
            let span = ScrollSpan::new(0.0..f64::from(extent).max(f64::from(visible)), at..at + f64::from(visible));
            let start = craft_behavior::scroll::page(&span, toward_end).start;
            if vertical {
                offset.y = start;
            } else {
                offset.x = start;
            }
            *offset = self.clamped(scroll, view, *offset);
        }
    }
}

impl<'b> BodyHooks<'b> {
    pub(super) fn scroll_geometry(&self, props: &ContainerProps, view: Rect, rtl: bool, tokens: &UiTokens) -> Vec<(Bar, ScrollSpan)> {
        let Some(scroll) = scroll_of(props) else { return Vec::new() };
        let thickness = scroll_bar_look(tokens).thickness;
        let (v_on, h_on) = (scroll.vertical_on(), scroll.horizontal_on());
        let (ex, ey) = self.last.extent;
        let mut bars = Vec::new();
        if v_on {
            let track = if rtl {
                Rect::from_min_max(view.min, Pos2::new(view.min.x + thickness, view.max.y))
            } else {
                Rect::from_min_max(Pos2::new(view.max.x - thickness, view.min.y), view.max)
            };
            let content = f64::from(ey).max(f64::from(view.height()));
            let span = ScrollSpan::new(0.0..content, self.offset.y..self.offset.y + f64::from(view.height()));
            bars.push((Bar { key: SCROLL_Y, axis: Axis::Y, track }, span));
        }
        if h_on {
            let track = Rect::from_min_max(
                Pos2::new(view.min.x, view.max.y - thickness),
                Pos2::new(view.max.x - if v_on { thickness } else { 0.0 }, view.max.y),
            );
            let content = f64::from(ex).max(f64::from(view.width()));
            let span = ScrollSpan::new(0.0..content, self.offset.x..self.offset.x + f64::from(view.width()));
            bars.push((Bar { key: SCROLL_X, axis: Axis::X, track }, span));
        }
        bars
    }

    pub(super) fn max_offset(&self, view: Vec2) -> ScrollPos {
        ScrollPos { x: f64::from((self.last.extent.0 - view.x).max(0.0)), y: f64::from((self.last.extent.1 - view.y).max(0.0)) }
    }

    pub(super) fn clamp_offset(&mut self, scroll: Scroll, view: Vec2) {
        self.offset = self.clamped(scroll, view, self.offset);
    }

    fn clamped(&self, scroll: Scroll, view: Vec2, offset: ScrollPos) -> ScrollPos {
        let max = self.max_offset(view);
        ScrollPos {
            x: if scroll.horizontal_on() { offset.x.clamp(0.0, max.x) } else { 0.0 },
            y: if scroll.vertical_on() { offset.y.clamp(0.0, max.y) } else { 0.0 },
        }
    }
}
