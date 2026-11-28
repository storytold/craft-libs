//! The body's scroll region: its offset and jumps, the scroll bars, their input and AccessKit scroll actions.

use super::BodyHooks;
use super::inherit::own_surface;
use super::placement::overlay_of;
use super::regions::rect_of;
use crate::contract::cx::CxCore;
use crate::contract::{Container, ContainerProps, Cx, Scroll};
use crate::primitives::{HiddenContent, ScrollBar};
use crate::runtime::egui::pass;
use crate::runtime::egui::{repaint, sizing};
use crate::runtime::env::Env;
use crate::runtime::lazy;
use crate::runtime::pass_state;
use crate::style::{SurfaceKind, UiTokens, scroll_bar_look, surface_style};
use craft_behavior::{Axis, PartKey, ScrollPos, ScrollSpan};
use egui::{Context, Id, Pos2, Rect, Ui, Vec2};

const SCROLL_Y: PartKey = PartKey::new("body_scroll_y");

const SCROLL_X: PartKey = PartKey::new("body_scroll_x");

/// The scroll strip above the view of a region with `ScrollIndicator::Strips`.
#[doc(hidden)]
pub const STRIP_BACK: PartKey = PartKey::new("body_scroll_strip_back");

/// The scroll strip below the view.
#[doc(hidden)]
pub const STRIP_FORWARD: PartKey = PartKey::new("body_scroll_strip_forward");

/// Rows per second a region scrolls at while the pointer rests on one of its strips, in rows a control high.
const STRIP_ROWS_PER_SECOND: f32 = 24.0;

/// The child the scroll position of the scrolling body `body` is named by (`lazy::Anchor`), for trials.
#[cfg(feature = "testing")]
pub(crate) fn anchor_of(ctx: &Context, body: Id) -> Option<Id> {
    crate::runtime::memory::with_session::<ScrollState, _>(ctx, crate::runtime::memory::sub(body, "scroll"), |state| {
        state.anchor.map(|anchor| anchor.child)
    })
}

/// The height of a scroll strip.
pub(super) fn scroll_strip_height(tokens: &UiTokens) -> f32 {
    tokens.size.control.sm
}

/// The scroll offset a container with no `Component::scroll` binding keeps for the session, and the content extent
/// every scrolling container keeps with it.
#[derive(Clone, Default)]
pub(super) struct ScrollState {
    pub(super) offset: (f64, f64),
    /// A bottom-anchored region follows the end while nothing moved it away.
    pub(super) follow_end: Option<bool>,
    /// The same position named by content: the offset follows it while extents before its child change.
    pub(super) anchor: Option<crate::runtime::lazy::Anchor>,
    /// The glide to a `Lazy` child a reveal asked for, until the offset reaches it or something else moves it.
    pub(super) glide: Option<Glide>,
    /// The content's extent from the last run that laid it out: it bounds the offset, so it is kept as long, a hidden
    /// body's included.
    pub(super) extent: (f32, f32),
}

/// A glide of the offset to the `Lazy` child `child` (`Lazy::reveal`). `to` is the offset that puts the child at the
/// view's start, from the last real run that placed it; it moves while extents before the child resolve, so the
/// offset follows it (`spec::FOLLOW`). `running` once the glide's tween started from the offset shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Glide {
    pub(super) child: Id,
    pub(super) to: Option<Vec2>,
    pub(super) running: bool,
}

/// The offsets the update step moved scroll bodies to (`scroll_input`, a reveal), with the frame that queued each, for
/// step 4 of each body's next pass. An entry no body took in that frame or the next belongs to a body that is no longer
/// shown: it is dropped.
#[derive(Default)]
pub(crate) struct Jumps(pub(super) Vec<(Id, ScrollPos, u64)>);

/// The frame a jump is queued and taken in: the root's, since one store serves the bodies of every viewport.
pub(super) fn jump_frame(ctx: &Context) -> u64 {
    pass::root_frame_nr(ctx)
}

/// The offset the update step of this pass queued for the body `id`, if it queued one.
pub(super) fn queued_jump(ctx: &Context, id: Id) -> Option<ScrollPos> {
    pass_state::with::<Jumps, _>(ctx, |jumps| jumps.0.iter().find(|(owner, ..)| *owner == id).map(|(_, offset, _)| *offset))
}

/// Queues `offset` for step 4 of the body `id`'s next pass, in place of one queued before, and repeats the pass to show
/// it.
pub(super) fn queue_jump(ctx: &Context, id: Id, offset: ScrollPos) {
    let frame = jump_frame(ctx);
    pass_state::with::<Jumps, _>(ctx, |jumps| {
        jumps.0.retain(|(owner, _, queued)| *owner != id && queued + 1 >= frame);
        jumps.0.push((id, offset, frame));
    });
    // A bound offset reaches the app in the repeated pass.
    log::debug!(target: sizing::RERUN_LOG, "{id:?} jumps to {offset:?}");
    sizing::request_discard(ctx, sizing::Rerun::HeldUpdate);
    if !ctx.will_discard() {
        repaint::request(ctx);
    }
}

/// The room the scroll bars of `scroll` take from a view of `view` over content of `extent`: `bar` across the view
/// for each axis whose content overflows it. Scroll strips and overlay bars take none across the view.
pub(super) fn overflow_inset(scroll: Scroll, extent: Vec2, view: Vec2, bar: f32) -> Vec2 {
    let takes = !scroll.has_overlay();
    Vec2::new(
        if takes && scroll.vertical_on() && !scroll.has_strips() && f64::from(extent.y) > f64::from(view.y) + 0.5 { bar } else { 0.0 },
        if takes && scroll.horizontal_on() && f64::from(extent.x) > f64::from(view.x) + 0.5 { bar } else { 0.0 },
    )
}

/// The scroll strips of a region with `ScrollIndicator::Strips` over the body rect `at`, whose content of `extent`
/// overflows it along y: the strip above the view and the strip below it, each `height` tall and at most a third of
/// `at`. `None` while the content fits, and for a region with a bar.
pub(super) fn strips_of(scroll: Scroll, extent: Vec2, at: Rect, height: f32) -> Option<(Rect, Rect)> {
    let over = scroll.has_strips() && f64::from(extent.y) > f64::from(at.height()) + 0.5;
    over.then(|| {
        let height = height.min(at.height() / 3.0).max(0.0);
        (Rect::from_min_max(at.min, Pos2::new(at.max.x, at.min.y + height)), Rect::from_min_max(Pos2::new(at.min.x, at.max.y - height), at.max))
    })
}

/// The part of the body rect `at` the content shows in: all of it, less the scroll strips while they show.
pub(super) fn view_within(scroll: Scroll, extent: Vec2, at: Rect, height: f32) -> Rect {
    strips_of(scroll, extent, at, height)
        .map_or(at, |(back, forward)| Rect::from_min_max(Pos2::new(at.min.x, back.max.y), Pos2::new(at.max.x, forward.min.y)))
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

/// After the real run of a scroll body laid out content of `extent` behind a view of `view`, from what its children
/// reported (`placed`): the anchor rule (`lazy::follow`) moves the offset, and the pass runs again at it. A body that
/// `keeps` no anchor (its offset is bound to the app, or it follows its end) has none.
pub(super) fn follow_anchor(
    ctx: &Context,
    (placed, scroll, keeps): (&lazy::Placed, Scroll, bool),
    (extent, view): (Vec2, Vec2),
    (offset, read): (&mut ScrollPos, ScrollPos),
    anchor: &mut Option<lazy::Anchor>,
) {
    if !keeps {
        *anchor = None;
        return;
    }
    let short = |x: f64| x as f32;
    let moved = Vec2::new(short(offset.x - read.x), short(offset.y - read.y));
    let back = Vec2::new(short(offset.x), short(offset.y));
    let ahead = (extent - view) - back;
    if let Some(shift) = lazy::follow(anchor, placed, moved, view, (back, ahead), (scroll.horizontal_on(), scroll.vertical_on())) {
        offset.x += f64::from(shift.x);
        offset.y += f64::from(shift.y);
        log::debug!(target: sizing::RERUN_LOG, "a scroll body keeps {:?} in place by {shift:?}", anchor.map(|anchor| anchor.child));
        sizing::request_discard(ctx, sizing::Rerun::Layout);
    }
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
        if let Some(at) = self.view
            && let Some(scroll) = scroll_of(props)
            && let Some(strips) = self.strips(scroll, at)
        {
            let max = self.max_offset(self.shown_view(scroll, at).size()).y;
            let (live, dead) = (crate::style::rgba_color(tokens.color.text.normal), crate::style::rgba_color(tokens.color.text.disabled));
            for (strip, back, moves) in [(strips.0, true, self.offset.y > 0.5), (strips.1, false, self.offset.y < max - 0.5)] {
                ui.painter().add(ScrollBar::strip_arrow(strip, back, if moves { live } else { dead }));
            }
            return;
        }
        if let Some(view) = self.view
            && let Some(scroll) = scroll_of(props)
        {
            let (ex, ey) = self.last_extent;
            let content = Vec2::new(if scroll.horizontal_on() { ex } else { view.width() }, if scroll.vertical_on() { ey } else { view.height() });
            let offset = Vec2::new(self.offset.x as f32, self.offset.y as f32);
            let background = surface_style(tokens, SurfaceKind::under(own_surface(props, cx), env.surface)).fill;
            ui.painter().extend(ScrollBar::fades(view, HiddenContent::new(content, view.size(), offset, rtl), background));
        }
        if let Some(view) = self.view {
            use crate::contract::Pointer as _;
            let overlay = scroll_of(props).is_some_and(|scroll| scroll.overlay_hides_at_rest());
            for (bar, span) in self.scroll_geometry(props, view, rtl, tokens) {
                let overflow = span.content.end - span.content.start > span.visible.end - span.visible.start;
                // An overlay bar shows while the pointer is over the container, or keeps the press that drags it, unless
                // it shows at rest.
                let hidden = overlay && !cx.response.contains_pointer() && !cx.part_if_declared(bar.key).is_some_and(|track| track.pressed());
                if !overflow || hidden {
                    continue;
                }
                let state = cx.core.scroll_bar_state(bar.key);
                ui.painter().extend(ScrollBar::shapes(&look, bar.track, bar.axis, rtl, &span, &state));
            }
        }
    }

    /// Thumb drags, track presses and AccessKit scroll actions. They move the view at step 4 of the next pass, as the
    /// wheel does there, and that pass repeats this frame: the pass that paints an offset is the one whose step 4 set
    /// it, so the content laid out for the view (out-of-view children skip their run, `runtime::lazy`) is the content
    /// shown. Every input read here is the pass's own, and the strips step by the pass's elapsed time, so a pass that
    /// repeats a frame moves nothing.
    pub(super) fn scroll_input<C: Container>(&self, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        use crate::contract::Pointer as _;
        let Some(scroll) = scroll_of(props) else { return };
        let ctx = cx.core.ctx.clone();
        let mut offset = self.offset;
        if let Some(range) = cx.scroll_moved(SCROLL_Y) {
            offset.y = range.start;
        }
        // Resting the pointer on a strip scrolls toward it at a steady speed.
        if self.last.view.is_some_and(|at| self.strips(scroll, rect_of(at)).is_some()) {
            let elapsed = crate::runtime::frame::facts(&ctx).elapsed.clamp(0.0, 0.1);
            let step = f64::from(STRIP_ROWS_PER_SECOND * self.row) * elapsed;
            for (strip, toward) in [(STRIP_BACK, -1.0), (STRIP_FORWARD, 1.0)] {
                if cx.core.results.get(&(strip, None)).is_some_and(|result| result.hovered) {
                    offset.y += toward * step;
                }
            }
        }
        if let Some(range) = cx.scroll_moved(SCROLL_X) {
            offset.x = range.start;
        }
        self.access_scroll(scroll, &mut offset, cx.core);
        let view = self.last.view.map_or(Vec2::ZERO, |view| self.shown_view(scroll, rect_of(view)).size());
        let offset = self.clamped(scroll, view, offset);
        if offset != self.offset {
            queue_jump(&ctx, cx.core.id, offset);
        }
    }

    /// AccessKit scroll actions aimed at the container's node, applied to `offset`: a page along an axis, or an offset.
    fn access_scroll(&self, scroll: Scroll, offset: &mut ScrollPos, core: &mut CxCore<'_>) {
        use craft_behavior::{AccessAction as Action, AccessData as ActionData};
        let rtl = core.env.direction().is_rtl();
        let view = self.last.view.map_or(Vec2::ZERO, |view| self.shown_view(scroll, rect_of(view)).size());
        while let Some(request) = core.intents.take_scroll_access(false) {
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
            let (visible, extent, at) = if vertical { (view.y, self.last_extent.1, offset.y) } else { (view.x, self.last_extent.0, offset.x) };
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
    /// The offset a drag held in an edge band of the view scrolls the body to from `from`, by the frame's time. The
    /// body takes the step only while no scroller inside it took one this pass (`reorder::edge_step_taken`), and marks
    /// it taken so a body around it does not step too. `None` when nothing moves, a rerun pass of a frame included: it
    /// has no elapsed time.
    pub(super) fn edge_step_body(&self, scroll: Scroll, ctx: &Context, from: ScrollPos) -> Option<ScrollPos> {
        use crate::contract::reorder;
        if !crate::runtime::drag::running(ctx) || reorder::edge_step_taken(ctx) {
            return None;
        }
        let pos = crate::runtime::pointer::frame(ctx).pos?;
        let view = self.shown_view(scroll, rect_of(self.last.view?));
        if !view.contains(pos) {
            return None;
        }
        let max = self.max_offset(view.size());
        let unit = |on: bool, at: f32, len: f32, room: f64| if on && room > 0.0 { reorder::edge_step(len, self.row, at, 1.0) } else { 0.0 };
        let x = unit(scroll.horizontal_on(), pos.x - view.min.x, view.width(), max.x);
        let y = unit(scroll.vertical_on(), pos.y - view.min.y, view.height(), max.y);
        if x == 0.0 && y == 0.0 {
            return None;
        }
        reorder::mark_edge_step(ctx);
        let dt = crate::runtime::frame::facts(ctx).elapsed.clamp(0.0, 0.1);
        let to = self.clamped(scroll, view.size(), ScrollPos { x: from.x + f64::from(x) * dt, y: from.y + f64::from(y) * dt });
        (to != from).then_some(to)
    }

    /// The scroll strips over the body rect `at`, from the content's last extent.
    pub(super) fn strips(&self, scroll: Scroll, at: Rect) -> Option<(Rect, Rect)> {
        strips_of(scroll, Vec2::new(self.last_extent.0, self.last_extent.1), at, self.strip)
    }

    /// The part of the body rect `at` the content shows in, from the content's last extent.
    pub(super) fn shown_view(&self, scroll: Scroll, at: Rect) -> Rect {
        view_within(scroll, Vec2::new(self.last_extent.0, self.last_extent.1), at, self.strip)
    }

    pub(super) fn scroll_geometry(&self, props: &ContainerProps, view: Rect, rtl: bool, tokens: &UiTokens) -> Vec<(Bar, ScrollSpan)> {
        let Some(scroll) = scroll_of(props) else { return Vec::new() };
        let thickness = scroll_bar_look(tokens).thickness;
        let (v_on, h_on) = (scroll.vertical_on(), scroll.horizontal_on());
        let (ex, ey) = self.last_extent;
        // An overlay bar's outer edge is the container's: the padding past the view's end edge holds it.
        let past = if scroll.has_overlay() { self.pads(props, tokens) } else { Vec2::ZERO };
        let mut bars = Vec::new();
        if v_on && !scroll.has_strips() {
            let track = if rtl {
                let outer = view.min.x - past.x;
                Rect::from_min_max(Pos2::new(outer, view.min.y), Pos2::new(outer + thickness, view.max.y))
            } else {
                let outer = view.max.x + past.x;
                Rect::from_min_max(Pos2::new(outer - thickness, view.min.y), Pos2::new(outer, view.max.y))
            };
            let content = f64::from(ey).max(f64::from(view.height()));
            let span = ScrollSpan::new(0.0..content, self.offset.y..self.offset.y + f64::from(view.height()));
            bars.push((Bar { key: SCROLL_Y, axis: Axis::Y, track }, span));
        }
        if h_on {
            let outer = view.max.y + past.y;
            let track = Rect::from_min_max(
                Pos2::new(view.min.x, outer - thickness),
                Pos2::new(view.max.x - if v_on && !scroll.has_overlay() { thickness } else { 0.0 }, outer),
            );
            let content = f64::from(ex).max(f64::from(view.width()));
            let span = ScrollSpan::new(0.0..content, self.offset.x..self.offset.x + f64::from(view.width()));
            bars.push((Bar { key: SCROLL_X, axis: Axis::X, track }, span));
        }
        bars
    }

    pub(super) fn max_offset(&self, view: Vec2) -> ScrollPos {
        ScrollPos { x: f64::from((self.last_extent.0 - view.x).max(0.0)), y: f64::from((self.last_extent.1 - view.y).max(0.0)) }
    }

    pub(super) fn clamp_offset(&mut self, scroll: Scroll, view: Vec2) {
        self.offset = self.clamped(scroll, view, self.offset);
    }

    pub(super) fn clamped(&self, scroll: Scroll, view: Vec2, offset: ScrollPos) -> ScrollPos {
        let max = self.max_offset(view);
        ScrollPos {
            x: if scroll.horizontal_on() { offset.x.clamp(0.0, max.x) } else { 0.0 },
            y: if scroll.vertical_on() { offset.y.clamp(0.0, max.y) } else { 0.0 },
        }
    }
}

/// After the real run of a scroll body that glides: the offset that puts the glide's child at the view's start, from
/// its place in this run (`at`, from the view shown at `shown`), within the offsets the content allows. A run that did
/// not reach the child ends the glide.
pub(super) fn aim_glide(glide: &mut Option<Glide>, at: Option<Vec2>, shown: ScrollPos, (extent, view): (Vec2, Vec2), scroll: Scroll) {
    let Some(aim) = glide.as_mut() else { return };
    let Some(at) = at else {
        *glide = None;
        return;
    };
    let most = (extent - view).max(Vec2::ZERO);
    let to = Vec2::new(
        if scroll.horizontal_on() { (shown.x as f32 + at.x).clamp(0.0, most.x) } else { shown.x as f32 },
        if scroll.vertical_on() { (shown.y as f32 + at.y).clamp(0.0, most.y) } else { shown.y as f32 },
    );
    aim.to = Some(to);
}
