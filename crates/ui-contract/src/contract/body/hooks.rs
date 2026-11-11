//! The body step's `ShapeHooks`: the pipeline's calls into a container's body.

use super::inherit::{BodyScope, register_instances};
use super::open::open_now;
use super::placement::{Placement, measure_layer, overlay_of, shown_placement};
use super::regions::{
    RegionFrame, RegionSizes, footer_in_end, gap_of, header_gap_of, label_extra_of, least_of, pad_of, paint_rules, rect_of, shared_labels,
    shared_line,
};
use super::reorder::REORDER_BAND;
use super::scroll::{Jumps, ScrollState, jump_frame, overflow_inset, scroll_of};
use super::window::{WindowMode, os_window, window_instance};
use super::{BodyHooks, BodyView};
use crate::contract::cx::{BodyCx, BodyOwner};
use crate::contract::label::{self, LabelIn};
use crate::contract::run::{Measuring, ShapeHooks, ShowAt};
use crate::contract::{Container, Cx, DragBand, Gap, Internal, Mount, Notch, PartsCx, SetPosition, Sizing, ValueOf};
use crate::layout::{self};
use crate::primitives::Surface;
use crate::runtime::access::scope;
use crate::runtime::access::{self, Politeness, Role};
use crate::runtime::commands;
use crate::runtime::egui::{alloc, areas, child, memory as store, response, style, viewports};
use crate::runtime::env::{AppValues, ArenaId, Env};
use crate::runtime::focus::FocusPolicy;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::motion::{Driver, Presence, spec};
use crate::runtime::overlay::CloseReason;
use crate::runtime::{keys, layout_memory, pointer, wheel};
use crate::style::ComponentStyle as _;
use crate::style::{Corners, StyleCx, SurfaceKind, scroll_bar_look, surface_style};
use craft_behavior::layout_memory::measure_tolerance;
use craft_behavior::{CancelRule, CursorIcon, EditEvent, KeyRole, Mixed, Roving, ScrollPos, ScrollSpan, WheelNotch, WheelRoom, WheelTaker};
use egui::{Context, CornerRadius, Id, Pos2, Rect, Shape, Ui, UiBuilder, Vec2};
use std::any::Any;
use std::sync::Arc;

impl<C: Container> ShapeHooks<C> for BodyHooks<'_> {
    fn shape(&self) -> crate::contract::steps::ShapeKind {
        crate::contract::steps::ShapeKind::Body
    }

    fn regions(&self, id: Id) -> Vec<(Id, Vec2)> {
        [
            ("body", self.body_size),
            ("header", self.header_size),
            ("footer", self.footer_size),
            ("end", self.end_size),
            ("controls", self.controls_size),
            ("extent", self.extent),
        ]
        .into_iter()
        .filter_map(|(key, took)| took.map(|took| (sub(id, key), took)))
        .collect()
    }

    fn in_flow(&self, comp: &C) -> bool {
        !matches!(comp.props(Internal::new()).mount, Mount::Layer(_) | Mount::FullWindow | Mount::Viewport { .. })
    }

    fn host(&mut self, comp: &C, at: crate::contract::run::HostCx<'_>) -> Box<dyn crate::contract::run::Host> {
        let mounted = crate::contract::mount::host(comp.props(Internal::new()), &at, self.open, at.label);
        if mounted.exiting {
            self.exiting = true;
            self.open = true;
        }
        mounted.host
    }

    /// A container that is an instance of its command scope sees itself in its own context's chain, so a focused
    /// container (a canvas) is the instance its chords and the commands it takes reach.
    fn instance_scope(&self, comp: &C) -> Option<commands::CommandScope> {
        comp.props(Internal::new()).command_scope
    }

    fn dismissed(&mut self, comp: &mut C, cx: &mut Cx<'_, C>, why: CloseReason) {
        self.close = true;
        self.open = false;
        comp.closed(why);
        self.write_open(comp, cx);
    }

    /// The scope of a layer, full-window or viewport mount is the host's (one scope per layer, from `Overlay.focus`;
    /// a full-window mount's overlay carries `props.focus`); `Inline` and `Pane` containers push `props.focus`.
    fn focus(&self, comp: &C) -> Option<FocusPolicy> {
        let props = comp.props(Internal::new());
        if matches!(props.mount, Mount::Layer(_) | Mount::Viewport { .. }) {
            debug_assert!(
                props.focus == FocusPolicy::NONE,
                "a layer or viewport mount takes its focus scope from `Overlay.focus`, not `ContainerProps.focus`"
            );
            return None;
        }
        if matches!(props.mount, Mount::FullWindow) {
            return None;
        }
        (props.focus != FocusPolicy::NONE).then_some(props.focus)
    }

    fn arena(&self, comp: &C) -> bool {
        comp.props(Internal::new()).arena
    }

    fn expanded(&self, comp: &C) -> Option<bool> {
        comp.props(Internal::new()).collapse.map(|_| self.open)
    }

    fn roving(&self, comp: &C) -> Option<Roving> {
        comp.props(Internal::new()).roving
    }

    fn key_role(&self, comp: &C) -> KeyRole {
        KeyRole::new().reorderable(comp.props(Internal::new()).header.is_some_and(|header| header.band == DragBand::Reorder))
    }

    fn position(&self, comp: &C) -> Option<SetPosition> {
        comp.props(Internal::new()).position
    }

    fn live(&self, comp: &C) -> Option<Politeness> {
        comp.props(Internal::new()).live
    }

    fn scrolls(&self, comp: &C) -> (bool, bool) {
        scroll_of(comp.props(Internal::new())).map_or((false, false), |scroll| (scroll.vertical_on(), scroll.horizontal_on()))
    }

    fn read(&mut self, comp: &mut C, stage: &mut MemoryStage, id: Id, env: &Env) {
        let ctx = stage.ctx().clone();
        let props = comp.props(Internal::new()).clone();
        let props = &props;
        let scroll = scroll_of(props);
        let collapse = props.collapse;
        let save = props.save.clone();
        self.save = save.clone();
        // The overlay host decides a layer's Escape; so does a full-window mount's.
        self.layer = matches!(props.mount, Mount::Layer(_) | Mount::Viewport { .. } | Mount::FullWindow);
        self.region_gap = gap_of(&env.tokens, Gap::Tight);
        self.header_gap = header_gap_of(props, &env.tokens);
        self.least_width = least_of(&env.tokens, props.least_width);
        let quiet = stage.is_peek() || crate::runtime::measure::active(&ctx);
        if !matches!(props.mount, Mount::Inline | Mount::Pane(_))
            && let Some(why) = if quiet { crate::runtime::overlay::peek_close(&ctx, id) } else { crate::runtime::overlay::take_close(&ctx, id) }
        {
            self.close = true;
            self.reason = Some(why);
        }
        self.open_bound = comp.open().is_some();
        self.open = open_now(comp, stage, id, props);
        self.open_at_read = self.open;
        self.presence = match collapse {
            Some(collapse) if collapse.animated => {
                Driver { stage: &*stage, base: id, policy: env.motion }.presence("collapse", self.open, spec::OPEN_RISE)
            }
            _ => Presence { t: f32::from(u8::from(self.open)), visible: self.open, exiting: false },
        };
        self.mode = stage.session::<WindowMode>(sub(id, "window_mode")).clone();
        self.fixed = match &props.mount {
            Mount::FullWindow => true,
            Mount::Viewport { .. } if viewports::available(&ctx) => true,
            Mount::Layer(_) | Mount::Viewport { .. } => {
                let placement = overlay_of(props).map_or_else(Placement::default, |overlay| {
                    let screen = crate::contract::overlay::screen_for(&ctx, &env, overlay);
                    shown_placement(&ctx, id, save.as_ref(), overlay, screen, env.tokens.space.inset.screen)
                });
                placement.pos.is_some() && placement.size.is_some()
            }
            _ => false,
        };
        self.last = stage.recall::<BodyView>(sub(id, "view"));
        let Some(scroll) = scroll else { return };
        self.scroll_bound = comp.scroll().is_some();
        let held = match comp.scroll() {
            Some(bind) => match bind.get() {
                Some(Mixed::Same(pos)) => pos,
                _ => ScrollPos::default(),
            },
            None => {
                let state = stage.session::<ScrollState>(sub(id, "scroll"));
                ScrollPos { x: state.offset.0, y: state.offset.1 }
            }
        };
        self.offset = held;
        self.offset_at_read = held;
        let rtl = env.direction.is_rtl();
        let view = self.last.view.map_or(Vec2::ZERO, |view| rect_of(view).size());
        // Step 4 applies, in order, the scroll bar and AccessKit moves of the last pass (`scroll_input`), this pass's
        // wheel share and the reveal request queued last pass.
        let frame = jump_frame(&ctx);
        let jump = store::Service::<Jumps>::run(&ctx, |jumps| {
            jumps.0.retain(|(.., queued)| queued + 1 >= frame);
            let at = jumps.0.iter().position(|(owner, ..)| *owner == id)?;
            Some(if quiet { jumps.0[at].1 } else { jumps.0.remove(at).1 })
        });
        if let Some(jump) = jump {
            self.offset = jump;
            self.moved = true;
        }
        if let Some(share) = if quiet { wheel::peek(&ctx, id) } else { wheel::take(&ctx, id) } {
            let sign = if rtl { 1.0 } else { -1.0 };
            self.offset.x += f64::from(sign * share.scroll.x);
            self.offset.y -= f64::from(share.scroll.y);
            self.moved = true;
        }
        if let (Some(rect), Some(view_rect)) =
            (if quiet { crate::contract::peek_reveal(&ctx, id) } else { crate::contract::take_reveal(&ctx, id) }, self.last.view.map(rect_of))
        {
            // A view with no extent on an axis shows nothing there: no offset reveals into it.
            let into = |low: f32, high: f32, view_low: f32, view_high: f32| {
                if view_high <= view_low {
                    0.0
                } else if low < view_low {
                    low - view_low
                } else if high > view_high {
                    high - view_high
                } else {
                    0.0
                }
            };
            self.offset.y += f64::from(into(rect.min.y, rect.max.y, view_rect.min.y, view_rect.max.y));
            let dx = into(rect.min.x, rect.max.x, view_rect.min.x, view_rect.max.x);
            self.offset.x += f64::from(if rtl { -dx } else { dx });
            self.moved = true;
        }
        let moves = if quiet { crate::contract::steps::peek_scroll_moves(&ctx, id) } else { crate::contract::steps::take_scroll_moves(&ctx, id) };
        for mv in moves {
            let (visible, extent, offset, vertical) = if scroll.vertical_on() {
                (view.y, self.last.extent.1, self.offset.y, true)
            } else {
                (view.x, self.last.extent.0, self.offset.x, false)
            };
            let span = ScrollSpan::new(0.0..f64::from(extent).max(f64::from(visible)), offset..offset + f64::from(visible));
            if let Some(range) = craft_behavior::scroll::jump(&span, mv) {
                if vertical {
                    self.offset.y = range.start;
                } else {
                    self.offset.x = range.start;
                }
                self.moved = true;
            }
        }
        let follows = stage.session::<ScrollState>(sub(id, "scroll")).follow_end.unwrap_or(true);
        self.follows_end = scroll.anchor == crate::contract::ScrollAnchor::End && follows;
        if self.follows_end && !self.moved {
            self.offset.y = f64::from((self.last.extent.1 - view.y).max(0.0));
        }
        self.clamp_offset(scroll, view);
        self.view_offset = self.offset;
    }

    fn overflow(&self) -> f32 {
        self.overflow
    }

    fn extent(&mut self, comp: &C, ui: &Ui, at: ShowAt, id: Id, scx: &StyleCx<'_>) -> Vec2 {
        let props = comp.props(Internal::new());
        self.room = self.mount_room(props, ui, id, at);
        self.given = self.given_axes(props, &Env::of(ui), at);
        let measured = self.measured_body.unwrap_or(Vec2::ZERO);
        let body = if self.presence.visible { Vec2::new(measured.x, measured.y * self.presence.t) } else { Vec2::ZERO };
        let room = self.room.x - 2.0 * pad_of(scx.tokens, props.pad);
        let width = if self.measured_body.is_some() { self.shared_width(room, props) } else { 0.0 };
        self.overflow = self.wanted_width(props).map_or(0.0, |wanted| (wanted - room.max(0.0)).max(0.0));
        let (header, end) = (self.measured_header.unwrap_or(Vec2::ZERO), self.measured_end.unwrap_or(Vec2::ZERO));
        let height = self.regions_size(props, body, header, self.measured_footer, end).y;
        Vec2::new(width, height) + Vec2::splat(2.0 * pad_of(scx.tokens, props.pad))
    }

    /// The regions run once in an invisible sizing pass over the room, as `content` runs them, and the container takes
    /// what they took: nothing an earlier pass recorded counts. Along an axis the body scrolls on it takes at most
    /// the room. What the regions took is recorded for the rest of the pass; a record this pass already holds serves
    /// instead when the room still allows it (`reusable`).
    fn measured_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>) -> Vec2 {
        let props = comp.props(Internal::new()).clone();
        // A mount outside the flow takes no room in a parent's sizing pass, so its regions run only when it is
        // measured itself.
        if !matches!(props.mount, Mount::Inline) && at.ui.is_sizing_pass() {
            return self.extent(comp, at.ui, at.at, cx.core.id, at.scx);
        }
        let (id, ctx) = (cx.core.id, at.ui.ctx().clone());
        let open = if self.open_given { self.open } else { open_now(comp, &mut *cx.core.memory, id, &props) };
        self.open = open || self.shown_closed;
        self.open_at_read = self.open;
        // An animated collapse shows a share of the full body height, so the body is measured while any of it shows.
        let shows = self.open || (props.collapse.is_some_and(|collapse| collapse.animated) && self.presence.visible);
        self.presence = Presence { t: f32::from(u8::from(shows)), visible: shows, exiting: false };
        self.measured_body = None;
        self.measured_header = None;
        self.measured_footer = Vec2::ZERO;
        self.measured_end = None;
        self.measured_controls = None;
        // The measuring run lays a scrolling body's content out at the width the real run of this pass gives it: beside
        // the bars the content extent the view keeps asks for, so the records it leaves serve that run.
        self.last = BodyView { extent: self.last.extent, ..BodyView::default() };
        self.offset = ScrollPos::default();
        self.region_gap = gap_of(&at.env.tokens, Gap::Tight);
        self.header_gap = header_gap_of(&props, &at.env.tokens);
        self.least_width = least_of(&at.env.tokens, props.least_width);
        self.room = match at.width {
            Some(width) => Vec2::new(width, props.room_height.unwrap_or(f32::INFINITY)),
            None => self.mount_room(&props, at.ui, id, at.at),
        };
        self.given = self.given_axes(&props, at.env, at.at);
        // A body that fills the main axis of the body it sits in is measured and shown at its share of it. The first
        // sweep of that body's measuring run only counts it: its contents run in the second sweep, at the share.
        let (x, y) = (craft_behavior::Axis::X, craft_behavior::Axis::Y);
        // A container placed at an explicit rect is no flow child of that body: it takes its rect, not a share.
        let flows = matches!(at.at, ShowAt::Flow);
        if flows
            && at.ui.is_sizing_pass()
            && ((self.given.0 && crate::layout::counting_fills(at.ui, x)) || (self.given.1 && crate::layout::counting_fills(at.ui, y)))
        {
            return Vec2::ZERO;
        }
        if let Some(share) = crate::layout::fill_share(at.ui, x).filter(|_| flows && self.given.0) {
            self.room.x = share;
        }
        if let Some(share) = crate::layout::fill_share(at.ui, y).filter(|_| flows && self.given.1) {
            self.room.y = share;
        }
        // A body that fills both axes its parent gives it takes the room, whatever its contents take: a parent's
        // measuring run does not run them. It measures them in its own real run.
        if at.ui.is_sizing_pass() && self.given == (true, true) && self.room.is_finite() {
            return self.room;
        }
        let pad = Vec2::splat(2.0 * pad_of(at.scx.tokens, props.pad));
        let vertical = scroll_of(&props).is_some_and(|scroll| scroll.vertical_on());
        let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(&ctx));
        // A record a run of this pass left serves as before; a kept one serves only while the inputs it was measured
        // under hold.
        let key = self.measure_key(&props, at.env, at.width);
        let reused = (self.reuse || at.ui.is_sizing_pass())
            .then(|| {
                let found = layout_memory::run_found(&ctx, id);
                let serves = found || layout_memory::kept_serves(&ctx, id, &key);
                serves.then(|| self.reusable(&props, &ctx, id, pad, tolerance)).flatten().map(|took| (took, found))
            })
            .flatten();
        let took = match reused {
            Some((took, found)) => {
                if found {
                    layout_memory::kept_also_under(&ctx, id, key);
                }
                took
            }
            None => {
                #[cfg(feature = "testing")]
                crate::runtime::measure::count_body(&ctx, id);
                layout_memory::begin_measure(&ctx, id);
                let available = match at.at {
                    ShowAt::Rect(rect) => rect,
                    // Not the cursor: in a right-to-left layout it extends to -inf along the line.
                    ShowAt::Flow => at.ui.available_rect_before_wrap(),
                };
                let origin = available.min;
                // Along an axis with no room limit the content grows away from its start edge: in a right-to-left
                // layout that edge is the right one, so the measuring rect is as wide as a scrolling region and
                // ends where the room does, as the real run's region does.
                let rect = if self.room.x.is_infinite() && at.env.direction.is_rtl() && available.max.x.is_finite() {
                    Rect::from_min_max(
                        Pos2::new(available.max.x - crate::contract::MAX_EXTENT, origin.y),
                        Pos2::new(available.max.x, origin.y + self.room.y),
                    )
                } else {
                    Rect::from_min_size(origin, self.room)
                };
                cx.core.rect = rect;
                cx.core.flow = craft_behavior::Flow::new(rect, at.env.direction.is_rtl());
                cx.core.content_rect = cx.core.flow.inset(cx.style.metrics().padding);
                let builder = UiBuilder::new().layer_id(measure_layer(&props, id, at.ui)).max_rect(rect).sizing_pass().invisible();
                // The regions take explicit scope ids under `id`, so every component in them gets the id the real run
                // gives it and the records it leaves match.
                let mut measure_ui = Ui::new(ctx.clone(), sub(id, "measure"), builder);
                let installed = Env::install_for(&measure_ui, at.env.clone());
                let mut values = AppValues::default();
                comp.content_env(cx, &mut values);
                self.content(comp, &mut measure_ui, at.env, cx, &values);
                Env::restore(&ctx, installed);
                let record = |key: Id, took: Vec2| layout_memory::emit_record(&ctx, key, layout_memory::Measured::new(self.room, took));
                record(id, self.body_size.unwrap_or(Vec2::ZERO));
                for (key, size) in
                    [("header", self.header_size), ("footer", self.footer_size), ("end", self.end_size), ("controls", self.controls_size)]
                {
                    if let Some(size) = size {
                        record(sub(id, key), size);
                    }
                }
                layout_memory::end_measure(&ctx, id, key);
                let body = if self.open { self.body_size.unwrap_or(Vec2::ZERO) } else { Vec2::ZERO };
                let (header, footer, end) =
                    (self.header_size.unwrap_or(Vec2::ZERO), self.footer_size.unwrap_or(Vec2::ZERO), self.end_size.unwrap_or(Vec2::ZERO));
                self.regions_size(&props, body, header, footer, end) + pad
            }
        };
        // A layer hung from an opener is at least as wide as the opener its overlay asks for, in the measuring run too,
        // so the rows of a content-sized list inside it are placed at that width.
        let took = Vec2::new(overlay_of(&props).map_or(took.x, |overlay| took.x.max(crate::contract::overlay::opener_floor(overlay))), took.y);
        self.overflow = (took.x - self.room.x).max(0.0);
        // A body wider than the room overflows it, or scrolls along it; either way the container keeps to the room.
        Vec2::new(took.x.min(self.room.x), if vertical { took.y.min(self.room.y) } else { took.y })
    }

    /// A real pass places the container by what its measuring run in this pass found: the record a measuring run of
    /// an enclosing container left when the room still allows it, else a measuring run of its own now. A closed
    /// layer that no longer shows runs none.
    fn pass_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, style: &C::Style, shown: Option<Arc<ValueOf<C>>>) -> Vec2 {
        let props = comp.props(Internal::new()).clone();
        let (ctx, id) = (at.ui.ctx().clone(), at.resolved.id);
        let layer = matches!(props.mount, Mount::Layer(_) | Mount::FullWindow | Mount::Viewport { .. });
        let fading = layer && !self.open && crate::runtime::motion::layer_fading(&ctx, sub(id, "presence"));
        if !layer || self.open || fading {
            let mut stage = MemoryStage::open_peek(&ctx, id);
            let mut fresh = BodyHooks::new(&mut *self.body);
            fresh.read(comp, &mut stage, id, at.env);
            // A peek stage holds no session scroll state: the measuring run decides by the offset this pass shows.
            fresh.view_offset = self.view_offset;
            fresh.open = self.open;
            fresh.open_at_read = self.open;
            fresh.presence = self.presence;
            fresh.open_given = true;
            fresh.shown_closed = fading;
            fresh.reuse = true;
            crate::contract::run::measured_extent(comp, &mut fresh, at, &mut stage, style, shown);
        }
        let took = |key: Id| layout_memory::measured(&ctx, key).map(|measured| measured.took);
        self.measured_body = took(id);
        self.measured_header = props.header.and_then(|_| took(sub(id, "header")));
        self.measured_footer = props.footer.as_ref().and_then(|_| took(sub(id, "footer"))).unwrap_or(Vec2::ZERO);
        self.measured_end = props.end.as_ref().and_then(|_| took(sub(id, "end")));
        self.measured_controls = took(sub(id, "controls"));
        self.extent(comp, at.ui, at.at, id, at.scx)
    }

    fn sized(&self, comp: &C, ui: &Ui, id: Id, size: Vec2) -> Vec2 {
        let props = comp.props(Internal::new());
        match self.sizing(props) {
            Sizing::Fill => {
                let room = crate::layout::flow_room(ui);
                // The available rect, not at least it: a fill child that took the whole room in an earlier pass (a
                // content-sized layer offers its whole room before the footer is known) must not keep its remembered
                // size once the room is less. Along an axis it is not given it takes its measured size. Along the main
                // axis of the body it sits in it takes its share of what its siblings leave.
                let axis = |room: f32, given: bool, along: craft_behavior::Axis, measured: f32| {
                    if !(room.is_finite() && given) {
                        return measured;
                    }
                    match crate::layout::main_fill(ui, id, along) {
                        crate::layout::MainFill::Measuring => 0.0,
                        crate::layout::MainFill::Share(share) => share,
                        crate::layout::MainFill::Room => room,
                    }
                };
                Vec2::new(axis(room.x, self.given.0, craft_behavior::Axis::X, size.x), axis(room.y, self.given.1, craft_behavior::Axis::Y, size.y))
            }
            Sizing::FillWidth => {
                let room = crate::layout::flow_fill_room(ui).x;
                // A fill-width child held in a content column is not given the width (it never sets the column's
                // width) but shows across the column.
                // It widens to the column it is held in, never past it, and only where it is shown: its measure never
                // sets the column's width.
                let held = !self.given.0 && self.held(props, &Env::of(ui));
                let room = if held { room.min(crate::layout::flow_room(ui).x) } else { room };
                let across = self.given.0 || held;
                let width = if room.is_finite() && across {
                    match crate::layout::main_fill(ui, id, craft_behavior::Axis::X) {
                        crate::layout::MainFill::Measuring => 0.0,
                        crate::layout::MainFill::Share(share) => share,
                        crate::layout::MainFill::Room => size.x.max(room),
                    }
                } else {
                    size.x
                };
                Vec2::new(width, size.y)
            }
            Sizing::Content => size,
        }
    }

    fn fills_width(&self, comp: &C) -> bool {
        self.sizing(comp.props(Internal::new())) != Sizing::Content
    }

    fn fills_width_only(&self, comp: &C) -> bool {
        self.sizing(comp.props(Internal::new())) == Sizing::FillWidth
    }

    fn on_line(&self, comp: &C) -> bool {
        matches!(comp.props(Internal::new()).layout, layout::BodyLayout::Row { wrap: false, .. })
    }

    fn parts(&mut self, comp: &mut C, rect: Rect, pcx: &mut PartsCx<'_, C>) {
        let props = comp.props(Internal::new()).clone();
        let ctx = pcx.ui.ctx().clone();
        let rtl = pcx.flow.is_rtl();
        let tokens = pcx.tokens;
        let inner = pcx.content_rect.shrink(pad_of(tokens, props.pad));
        let room = self.room_size(rect, inner);
        let regions = self.regions(&props, inner, pad_of(tokens, props.pad), room, rtl, self.measured_sizes());
        let scroll = scroll_of(&props);
        if self.open && scroll.is_some() {
            let view = regions.body;
            self.view = Some(view);
            if let Some(scroll) = scroll {
                self.clamp_offset(scroll, view.size());
            }
            for (bar, span) in self.scroll_geometry(&props, view, rtl, tokens) {
                pcx.scroll(bar.key, bar.track, bar.axis, span);
            }
            if let Some(scroll) = scroll {
                let notch = match scroll.notch {
                    Notch::Lines(lines) => WheelNotch::Points(lines * tokens.tokens().typography.body.line_height),
                    Notch::ViewShare(share) => WheelNotch::ViewShare(share),
                };
                // A region takes an axis only while its content overflows there, and hands the wheel on at its ends.
                let max = self.max_offset(view.size());
                let (vertical, horizontal) = (scroll.vertical_on() && max.y > 0.0, scroll.horizontal_on() && max.x > 0.0);
                let (before, after) = (
                    Vec2::new(self.offset.x as f32, self.offset.y as f32),
                    Vec2::new((max.x - self.offset.x) as f32, (max.y - self.offset.y) as f32),
                );
                // A positive wheel moves the offset back on y, and on x in left-to-right flow.
                let (positive, negative) = if rtl { (Vec2::new(after.x, before.y), Vec2::new(before.x, after.y)) } else { (before, after) };
                if vertical || horizontal {
                    wheel::register(
                        &ctx,
                        WheelTaker::new(pcx.id, view)
                            .vertical(vertical)
                            .horizontal(horizontal)
                            .scrolls(true)
                            .gated(crate::runtime::env::Env::of(pcx.ui).wheel_gated)
                            .notch(notch)
                            .room(WheelRoom::new(positive, negative)),
                    );
                }
            }
        }
        self.nudge = gap_of(tokens, Gap::Row);
        // The reorder band lies under the header's own parts and children: its default priority loses to theirs, so
        // their buttons keep their clicks.
        if props.header.is_some_and(|header| header.band == DragBand::Reorder) && regions.header.height() > 0.0 {
            pcx.part(REORDER_BAND, regions.header)
                .sense(craft_behavior::Sense::DRAG)
                .cursor(CursorIcon::Grab)
                .drag_cursor(CursorIcon::Grabbing)
                .pointer_only("Alt+Up and Alt+Down move the section");
        }
        Self::move_parts(&props, &regions, rect, pcx);
        self.pane_grip = Self::pane_grip(&props, rect, pcx);
    }

    fn sensed(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        self.reorder_keys(comp.props(Internal::new()), cx);
    }

    fn updated(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        use crate::contract::Pointer as _;
        let props = comp.props(Internal::new());
        let (scroll, layer) = (scroll_of(props), matches!(props.mount, Mount::Layer(_) | Mount::Viewport { .. }));
        // The OS asked a viewport mount's window to close: the container closes at step 15.
        if matches!(props.mount, Mount::Viewport { .. }) && crate::runtime::windows::take_close_request(cx.core.memory.ctx(), cx.core.id) {
            self.close = true;
            self.reason = self.reason.or(Some(CloseReason::Requested));
        }
        if scroll.is_some() {
            self.scroll_input(props, cx);
        }
        // AccessKit Expand, Collapse and Click reach a collapsible container as an activation: it folds or unfolds.
        if props.collapse.is_some() && cx.activation().is_some_and(|activation| activation.by_key) {
            self.open = !self.open;
        }
        self.band_double(cx);
        self.surface_press(comp.sense().contains(craft_behavior::Sense::SURFACE), cx);
        let band = props.header.is_some_and(|header| header.band == DragBand::Move);
        if (band || matches!(props.mount, Mount::Viewport { .. } | Mount::Window { .. }))
            && let Some(viewport) = os_window(props, cx.core.memory.ctx(), cx.core.id)
        {
            self.window_os(viewport, cx);
            self.window_movable(cx);
        }
        self.pane_drag(props, cx);
        if layer && !(matches!(props.mount, Mount::Viewport { .. }) && viewports::available(cx.core.memory.ctx())) {
            self.window_keys(props, cx);
            self.window_drag(props, cx);
        }
        self.window_placed(props, cx);
        for change in std::mem::take(&mut self.reports) {
            comp.placement_changed(&change);
        }
    }

    fn content(&mut self, comp: &mut C, ui: &mut Ui, env: &Env, cx: &mut Cx<'_, C>, values: &AppValues) {
        let props = comp.props(Internal::new()).clone();
        let (id, ctx, rect) = (cx.core.id, cx.core.ctx.clone(), cx.core.rect);
        let rtl = env.direction.is_rtl();
        let node = matches!(C::ROLE, Role::Node(_));
        let scroll = scroll_of(&props);
        let pad = pad_of(&env.tokens, props.pad);
        let inner = cx.core.content_rect.shrink(pad);
        let room = self.room_size(rect, inner);
        let mut regions = self.regions(&props, inner, pad, room, rtl, self.measured_sizes());
        // A layer's surface casts its shadow past the layer's own clip: it paints under the whole window. So does a popup
        // surface at the top of a layer a control owns (`Menu::inline`).
        let mut surface_painter = ui.painter().clone();
        let popup_in_layer = matches!(props.surface, Some(SurfaceKind::Popup)) && crate::runtime::overlay::drawing_parent(ui.ctx()).is_some();
        if matches!(props.mount, Mount::Layer(_)) || popup_in_layer {
            surface_painter.set_clip_rect(ui.ctx().content_rect());
        }
        let surface_slot = surface_painter.add(Shape::Noop);
        let band_slot = ui.painter().add(Shape::Noop);
        // A measuring run lays out and measures: it registers no instance, focus body, scroll region or body layer,
        // and claims no room in a parent flow.
        let real = !ui.is_sizing_pass();
        // Only a container that declares its value a gate (a group box) gates its content; a radio group whose value
        // is a bool does not.
        // A mixed gate reads as off, and its check box shows the mixed face.
        self.gate = cx.shown.as_deref().filter(|_| props.gates).and_then(|value| (value as &dyn Any).downcast_ref::<bool>()).copied();
        let gate_mixed = props.gates && cx.mixed;
        self.gate = self.gate.or(gate_mixed.then_some(false));
        self.gate_at_read = self.gate;
        let window = window_instance(&props, id).filter(|window| *window != id);
        if real {
            register_instances(&ctx, &props, id, window);
        }
        let region_env = env.derive(|scoped| self.scope_regions(scoped, &props, cx, window, values, real));
        let body_id = sub(id, "body");
        let owner = BodyOwner::new(cx.style, gap_of(&env.tokens, props.body_gap));
        let frame = RegionFrame { id, rtl, node, env: &region_env };
        let (resolved, env_view) = (cx.core.resolved, region_env.view());
        let mut open = self.open;
        let mut gate = self.gate;
        let mut close: Option<CloseReason> = None;
        let field_message = if props.label.is_some() { scope::last_message(&ctx, id) } else { None };
        // B1 header
        if props.header.is_some() {
            let mut bcx = BodyCx::new(body_id, env_view, resolved, regions.header, &mut *cx.core.memory, &mut open, gate.as_mut())
                .with_gate_mixed(gate_mixed)
                .with_field_message(field_message.clone())
                .with_owner(owner);
            let trailing = props.header.is_some_and(|header| header.trailing);
            let inset = props.header.map_or(0.0, |header| pad_of(&env.tokens, header.inset));
            close = close.or(self.header_region(comp, ui, &mut bcx, &frame, regions.header, trailing, inset));
        }
        let animated = props.collapse.is_some_and(|collapse| collapse.animated);
        let is_open = open || (animated && self.presence.visible);
        let enabled = gate.unwrap_or(true);
        // What the body and the end region share: gate, size, roving group and commands. The body adds its arena,
        // transform and scroll owner; the end region lies outside them.
        let line_env = region_env.derive_local(|scoped| {
            scoped.enabled &= enabled;
            if let Some(size) = props.child_size {
                scoped.size = size;
            }
            if props.roving.is_some() {
                scoped.roving = Some(crate::contract::steps::body_group(id));
            }
        });
        let end_frame = RegionFrame { id, rtl, node, env: &line_env };
        // A footer in the end region is part of a column sized from its content: a fill-width child takes its
        // measured width there and shows across the column, so it never sets the column's width.
        let footer_env = if footer_in_end(&props) { region_env.derive_local(|scoped| scoped.content_column = true) } else { region_env.clone() };
        if real {
            crate::runtime::focus::begin_body(&ctx);
        }
        // A held fill-width container is part of the content column it sits in: its own stack is one too.
        let (sizing, held) = (self.sizing(&props), self.held(&props, env));
        let content_column = (sizing == Sizing::Content || held || !self.given.0) && matches!(props.layout, layout::BodyLayout::Stack);
        let gap = Vec2::splat(gap_of(&env.tokens, props.body_gap));
        let layout_id = sub(id, "layout");
        // The body is measured in the line the real run gives it: what the header and the end region leave. The
        // measuring run places it once both have measured, so its end region runs first (its content never depends
        // on the body); the real run places it from what they took and keeps reading order.
        if !real && is_open && props.end.is_some() {
            let mut end_cx = BodyCx::new(body_id, line_env.view(), resolved, regions.end, &mut *cx.core.memory, &mut open, gate.as_mut())
                .with_field_message(field_message.clone())
                .with_owner(owner);
            close = close.or(self.end_region(comp, ui, &mut end_cx, &end_frame, &props, regions.end));
        }
        let beside =
            if real { self.measured_sizes() } else { RegionSizes { header: self.header_size, footer: self.measured_footer, end: self.end_size } };
        if !real {
            regions.body = self.regions(&props, inner, pad, room, rtl, beside).body;
        }
        let line = shared_line(&props, beside);
        // B2 label: the control gets the rect the label leaves; the label column adds to the container's extent.
        let mut body_rect = regions.body;
        let mut label_input = None;
        if let Some(side) = props.label.as_ref() {
            let resolved = cx.core.resolved;
            let input = LabelIn {
                scope: id,
                owner: id,
                parent: node.then_some(id),
                row: regions.body,
                room: if self.sizing(&props) == Sizing::Content {
                    layout::ambient_room(&ctx).map_or(room.x, |offered| room.x.max(offered))
                } else {
                    regions.body.width()
                },
                content_row: false,
                text: resolved.label_value,
                side,
                help: props.help.as_ref(),
                enabled: resolved.enabled,
                size: resolved.size,
                control_min: scope::measured_control_min(&ctx, id),
                leaf: false,
            };
            body_rect = label::run(ui, env, &input).control;
            label_input = Some(input);
        }
        let label_extra = label_extra_of(regions.body, body_rect);
        let body_env = line_env.derive_local(|scoped| {
            let label_width = label_extra.x;
            self.scope_body(scoped, &props, id, BodyScope { room, real, sizing, content_column, scroll, label_width });
        });
        // B3 body, if open, under the body env; the gate is read after the header changed it.
        if is_open {
            let mut bcx = BodyCx::new(body_id, body_env.view(), resolved, body_rect, &mut *cx.core.memory, &mut open, gate.as_mut())
                .with_field_message(field_message.clone())
                .with_owner(owner);
            // While the body opens or closes, it shows only the part of its last height that the presence reaches.
            let reveal_clip = (animated && self.presence.t < 1.0).then(|| {
                Rect::from_min_max(
                    body_rect.min,
                    Pos2::new(body_rect.max.x, body_rect.min.y + self.measured_body.map_or(0.0, |size| size.y) * self.presence.t),
                )
            });
            let (layout, body) = (&props.layout, &mut *self.body);
            let label_column = props.label_column;
            let mut run = |parent: &mut Ui, at: Rect, bcx: &mut BodyCx<'_>| -> Option<Vec2> {
                let mut body_ui = alloc::child(parent, {
                    let builder = UiBuilder::new().max_rect(at).layout(child::column_layout(rtl)).scope_id(body_id);
                    if node { access::with_parent(builder, id) } else { builder }
                });
                style::set_item_spacing(&mut body_ui, gap);
                if let Some(clip) = reveal_clip {
                    body_ui.set_clip_rect(clip.intersect(body_ui.clip_rect()));
                }
                let installed = Env::install_for(&body_ui, body_env.clone());
                let mut group_extent = None;
                // The size the body took: a scrolling body takes the visible part of its content, since its region
                // is a child that allocates nothing in `body_ui`.
                let took = match scroll {
                    Some(scroll) => {
                        let off = self.offset;
                        let big = crate::contract::MAX_EXTENT;
                        let bar = scroll_bar_look(&body_env.tokens).thickness;
                        let inset = overflow_inset(scroll, Vec2::new(self.last.extent.0, self.last.extent.1), at.size(), bar);
                        let seen = Rect::from_min_max(
                            Pos2::new(if rtl { at.min.x + inset.x } else { at.min.x }, at.min.y),
                            Pos2::new(if rtl { at.max.x } else { at.max.x - inset.x }, at.max.y - inset.y),
                        );
                        let (width, height) =
                            (if scroll.horizontal_on() { big } else { seen.width() }, if scroll.vertical_on() { big } else { seen.height() });
                        let x = if scroll.horizontal_on() { off.x as f32 } else { 0.0 };
                        let min_x = if rtl { seen.max.x - width + x } else { seen.min.x - x };
                        let max = Rect::from_min_size(Pos2::new(min_x, seen.min.y - off.y as f32), Vec2::new(width, height));
                        let layout_of = *body_ui.layout();
                        let mut region_ui =
                            alloc::child(&mut body_ui, UiBuilder::new().scope_id(sub(id, "scroll_region")).layout(layout_of).max_rect(max));
                        region_ui.set_clip_rect(at.intersect(body_ui.clip_rect()));
                        // The view this pass shows, where this run lays the content out.
                        let shown = self.view_offset;
                        let shift = Vec2::new(
                            if scroll.horizontal_on() { (shown.x - off.x) as f32 * if rtl { -1.0 } else { 1.0 } } else { 0.0 },
                            if scroll.vertical_on() { (shown.y - off.y) as f32 } else { 0.0 },
                        );
                        let view = crate::runtime::lazy::View { owner: id, rect: seen.translate(shift) };
                        let region_env = Env::install_for(&region_ui, body_env.derive_local(|scoped| scoped.lazy = Some(view)));
                        layout::run_body(&mut region_ui, layout_id, layout, gap, |ui| {
                            shared_labels(ui, id, label_column, room.x, &mut group_extent, |ui| body(ui, bcx))
                        });
                        Env::restore(&ctx, region_env);
                        let extent = region_ui.min_rect().size();
                        if real {
                            crate::runtime::egui::widgets::note_scroll_region(&ctx, region_ui.clip_rect(), region_ui.min_rect().intersect(max));
                        }
                        self.extent = Some(extent);
                        // Along an axis it scrolls the body took what the view shows; along the other it took its content.
                        let took = extent + overflow_inset(scroll, extent, at.size(), bar);
                        Some(Vec2::new(
                            if scroll.horizontal_on() { took.x.min(at.width()) } else { took.x },
                            if scroll.vertical_on() { took.y.min(at.height()) } else { took.y },
                        ))
                    }
                    // A row that places its block itself asks for its children's width, not the room it spans.
                    None => {
                        let (_, natural) = layout::run_body_measured(&mut body_ui, layout_id, layout, gap, line, |ui| {
                            shared_labels(ui, id, label_column, room.x, &mut group_extent, |ui| body(ui, bcx))
                        });
                        natural.map(|width| Vec2::new(width, body_ui.min_rect().height()))
                    }
                };
                Env::restore(&ctx, installed);
                let took = took.unwrap_or_else(|| body_ui.min_rect().size());
                // A content-sized column of rows takes, in a measuring run, the width they take at the group's widest
                // label: the rows measured before the widest one measured against a narrower column.
                let stacked = matches!(layout, layout::BodyLayout::Stack) && !self.given.0;
                Some(match group_extent.filter(|_| stacked && !real) {
                    Some(width) => Vec2::new(took.x.max(width), took.y),
                    None => took,
                })
            };
            self.body_size = match props.transform {
                // The measuring run lays the body out on the layer the real run shows it on.
                Some(_) if !real => {
                    let builder = UiBuilder::new().layer_id(areas::body_layer(ui.layer_id(), id)).max_rect(body_rect);
                    run(&mut alloc::child(ui, builder), body_rect, &mut bcx)
                }
                Some(transform) => {
                    let parent = ui.layer_id();
                    let layer = areas::body_layer(parent, id);
                    let tf = areas::body_transform(body_rect.min, transform.scale, transform.offset);
                    if props.arena {
                        crate::primitives::sensing::set_layer(&ctx, ArenaId::new(id), layer);
                    }
                    let forward = areas::Forward {
                        id: sub(id, "transform"),
                        sense: response::to_egui(cx.core.root_sense),
                        rect: rect.intersect(ui.clip_rect()),
                    };
                    let body_layer = areas::BodyLayer {
                        parent,
                        layer,
                        transform: tf,
                        at: body_rect.min,
                        clip: body_rect.intersect(ui.clip_rect()),
                        forward: Some(forward),
                    };
                    areas::show_body_layer(&ctx, body_layer, |area_ui| run(area_ui, body_rect, &mut bcx))
                }
                None => run(ui, body_rect, &mut bcx),
            };
            close = close.or(bcx.close_requested());
            // A measuring run decided the label before the control reported its minimum width: it decides again
            // from the minimum this run recorded, so the extent it measures is the one the real run lays out.
            let mut label_extra = label_extra;
            if let (false, Some(input)) = (real, label_input.as_mut()) {
                input.control_min = scope::open_control_min(&ctx, id);
                // The footer starts under the control column this decision gives, not the one before the control
                // reported its minimum.
                body_rect = label::control_after(ui, env, input);
                label_extra = label_extra_of(regions.body, body_rect);
            }
            if let Some(size) = &mut self.body_size {
                // A body beside its label in a content-sized column of rows reports how far it reaches past the label
                // column, so that column's container measures its rows at the widest label.
                if let (false, true, Some(side)) = (real, env.content.0, props.label.as_ref().filter(|_| label_extra.x > 0.0 && label_extra.y <= 0.0))
                {
                    let column = if matches!(side.kind, label::LabelKind::Glyph(_)) { layout::GLYPHS } else { layout::LABELS };
                    layout::report_beyond(&ctx, std::slice::from_ref(&column), label::label_gap(env) + size.x);
                }
                *size += label_extra;
            }
            self.view = Some(body_rect);
            if real && props.end.is_some() {
                let mut end_cx = BodyCx::new(body_id, line_env.view(), resolved, regions.end, &mut *cx.core.memory, &mut open, gate.as_mut())
                    .with_field_message(field_message.clone())
                    .with_owner(owner);
                close = close.or(self.end_region(comp, ui, &mut end_cx, &end_frame, &props, regions.end));
            }
        } else {
            // A collapsed body keeps its last full size in layout memory, so opening it again places by that size
            // and asks for no discard pass.
            self.body_size = if props.collapse.is_some() { None } else { Some(Vec2::ZERO) };
        }
        // The label scope covers the body only: footer controls are not the field's controls.
        // A measuring run registers no control, so it leaves the facts the real run records alone.
        if props.label.is_some() {
            if real {
                scope::finish(&ctx, id, None);
            } else {
                scope::discard(&ctx, id);
            }
        }
        // B4 footer: under the control column of a label-scoped container, which starts where the label step put
        // the control.
        if props.footer.is_some() {
            let (mut footer_rect, offset) = self.place_footer(&props, &mut regions, inner, body_rect, rtl);
            // The region's height counts the gap (it is part of what the footer took), so the content starts below it.
            let lead = props.footer.and_then(|footer| footer.gap).map_or(0.0, |gap| gap_of(&env.tokens, gap));
            footer_rect.min.y += lead;
            footer_rect.max.y = footer_rect.max.y.max(footer_rect.min.y);
            let mut bcx = BodyCx::new(body_id, env_view, resolved, footer_rect, &mut *cx.core.memory, &mut open, gate.as_mut())
                .with_field_message(field_message)
                .with_owner(owner);
            let mut footer_ui = alloc::child(ui, frame.region("footer", footer_rect));
            let installed = Env::install_for(&footer_ui, footer_env);
            comp.footer(&mut footer_ui, &mut bcx);
            Env::restore(&ctx, installed);
            close = close.or(bcx.close_requested());
            let mut took = self.region_size(&footer_ui);
            // The footer's width counts from the container's edge, past the column it starts after.
            if took.x > 0.0 {
                took.x += offset;
            }
            if took.y > 0.0 {
                took.y += lead;
            }
            self.footer_size = Some(took);
        }
        self.open = open;
        self.gate = gate;
        if close.is_some() {
            self.close = true;
            self.reason = self.reason.or(close);
        }
        if is_open && scroll.is_some() {
            self.paint_scroll_bars(ui, cx, &props, rtl, env);
        }
        // The surface at the final rect, then the rules.
        if let Some(kind) = props.surface {
            let shapes = Surface::shapes(&surface_style(&env.tokens, kind), None, rect, Corners::All, env.direction, ctx.pixels_per_point());
            surface_painter.set(surface_slot, Shape::Vec(shapes));
        }
        if let Some(kind) = props.header.and_then(|header| header.fill) {
            let style = surface_style(&env.tokens, props.surface.unwrap_or(SurfaceKind::Panel));
            let radius = CornerRadius { sw: 0, se: 0, ..style.radius };
            ui.painter().set(band_slot, Shape::rect_filled(regions.header, radius, surface_style(&env.tokens, kind).fill));
        }
        paint_rules(ui, &props, env, inner, &regions);
    }

    fn settle(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        use crate::contract::Edit as _;
        if self.gate != self.gate_at_read
            && let Some(gate) = self.gate
            && let Ok(value) = (Box::new(gate) as Box<dyn Any>).downcast::<crate::contract::ValueOf<C>>()
        {
            cx.set(*value);
            // A viewport mount settles once per child pass: the baseline moves so a change applies once.
            self.gate_at_read = self.gate;
        }
        let id = cx.core.id;
        let (collapsible, policy) = {
            let props = comp.props(Internal::new());
            (props.collapse.is_some(), props.focus)
        };
        // A collapse is the user's own toggle: Escape never folds an inline container, only closes one with an open state.
        let closable = !collapsible && comp.open().is_some();
        // The focus inside the scope runs the scope's Escape step; with nothing focused, the window's untaken Escape
        // closes a scope whose rule is `Close`.
        if !self.layer
            && closable
            && policy.cancel != CancelRule::Ignore
            && match crate::runtime::focus::focus_in_scope(&cx.core.ctx, id) {
                Some(true) => matches!(
                    keys::scope_escape(&cx.core.ctx, policy.cancel, pointer::press_captured(&cx.core.ctx), false),
                    Some(craft_behavior::focus::EscapeStep::CloseScope)
                ),
                Some(false) => false,
                None => policy.cancel == CancelRule::Close && keys::take_window_cancel(&cx.core.ctx),
            }
        {
            self.close = true;
            self.reason = self.reason.or(Some(CloseReason::Escape));
        }
        if self.exiting {
            // The container closed already; the fade only keeps its body on screen.
            self.open = self.open_at_read;
            self.close = false;
        }
        if self.close {
            self.open = false;
            comp.closed(self.reason.take().unwrap_or(CloseReason::Requested));
        }
        self.write_open(comp, cx);
        if self.offset != self.offset_at_read {
            match comp.scroll() {
                Some(mut bind) => {
                    let (label, doc_key) = (cx.core.label_arc(), cx.core.resolved.doc_key);
                    bind.apply(&EditEvent::set(Mixed::Same(self.offset_at_read), self.offset, label, doc_key));
                }
                None => {
                    let state = cx.core.memory.session::<ScrollState>(sub(cx.core.id, "scroll"));
                    state.offset = (self.offset.x, self.offset.y);
                }
            }
            self.offset_at_read = self.offset;
        }
    }

    fn record(&mut self, stage: &mut MemoryStage, ctx: &Context, id: Id) {
        if let Some(size) = self.body_size {
            crate::contract::steps::record_size(ctx, id, size);
        }
        if let Some(size) = self.header_size {
            crate::contract::steps::record_size(ctx, sub(id, "header"), size);
        }
        if let Some(size) = self.footer_size {
            crate::contract::steps::record_size(ctx, sub(id, "footer"), size);
        }
        if let Some(size) = self.end_size {
            crate::contract::steps::record_size(ctx, sub(id, "end"), size);
        }
        if let Some(size) = self.controls_size {
            crate::contract::steps::record_size(ctx, sub(id, "controls"), size);
        }
        if self.extent.is_some() || self.view.is_some() {
            let view = stage.shown::<BodyView>(sub(id, "view"));
            if let Some(extent) = self.extent {
                // A bottom-anchored region that follows the end shows the rows that arrived this pass in this
                // frame: the repeated pass starts from the new extent.
                if self.follows_end && !self.moved && (extent.y - view.extent.1).abs() > 0.5 {
                    layout_memory::ask_discard(ctx);
                }
                view.extent = (extent.x, extent.y);
            }
            view.view = self.view.map(|view| [view.min.x, view.min.y, view.max.x, view.max.y]);
        }
        if *stage.session::<WindowMode>(sub(id, "window_mode")) != self.mode {
            *stage.session::<WindowMode>(sub(id, "window_mode")) = self.mode.clone();
        }
        if self.moved {
            let at_end = self.last.view.map(rect_of).is_some_and(|view| self.offset.y >= f64::from(self.last.extent.1 - view.height()) - 0.5);
            stage.session::<ScrollState>(sub(id, "scroll")).follow_end = Some(at_end);
        }
    }
}
