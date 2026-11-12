//! The body step's `ShapeHooks`: the pipeline's calls into a container's body.

use super::inherit::{BodyScope, register_instances};
use super::open::open_now;
use super::placement::{Placement, measure_layer, overlay_of, shown_placement};
use super::regions::{
    LabelColumns, LabelsFound, RegionFrame, RegionSizes, end_full_height, footer_in_end, gap_of, header_gap_of, label_extra_of, least_of, pad_of,
    paint_edge_rule, paint_rules, rect_of, shared_labels, shared_line,
};
use super::reorder::REORDER_BAND;
use super::scroll::{Glide, Jumps, ScrollState, aim_glide, follow_anchor, jump_frame, overflow_inset, scroll_of};
use super::size::MeasureKey;
use super::window::{WindowMode, os_window, window_instance};
use super::{BodyHooks, BodyView};
use crate::contract::cx::{BodyCx, BodyOwner};
use crate::contract::label::{self, LabelIn};
use crate::contract::run::{Measuring, ShapeHooks, ShowAt};
use crate::contract::{Container, Cx, DragBand, Gap, Internal, Mount, PartsCx, SetPosition, Sizing, ValueOf};
use crate::layout::{self};
use crate::primitives::SurfaceSlot;
use crate::runtime::access::scope;
use crate::runtime::access::{self, Politeness, Role};
use crate::runtime::commands;
use crate::runtime::egui::{alloc, areas, child, repaint, response, style, viewports};
use crate::runtime::env::{AppValues, ArenaId, Env};
use crate::runtime::focus::FocusPolicy;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::motion::{Driver, Presence, spec};
use crate::runtime::overlay::CloseReason;
use crate::runtime::pass_state;
use crate::runtime::{keys, layout_memory, pointer, wheel};
use crate::style::ComponentStyle as _;
use crate::style::{ScrollBarKind, StyleCx, SurfaceKind, scroll_bar_look, surface_style};
use craft_behavior::layout_memory::measure_tolerance;
use craft_behavior::{CancelRule, CursorIcon, EditEvent, KeyRole, Mixed, Roving, ScrollPos, ScrollSpan, WheelRoom, WheelTaker};
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
        let props = comp.props(Internal::new());
        KeyRole::new().reorderable(props.header.is_some_and(|header| header.band == DragBand::Reorder)).document(props.scroll_owner)
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
        let props = Arc::clone(&self.props);
        let props = &*props;
        let scroll = scroll_of(props);
        let collapse = props.collapse;
        let save = props.save.clone();
        self.save = save.clone();
        // The overlay host decides a layer's Escape; so does a full-window mount's.
        self.layer = matches!(props.mount, Mount::Layer(_) | Mount::Viewport { .. } | Mount::FullWindow);
        self.region_gap = gap_of(&env.tokens, Gap::Tight);
        self.end_gap = gap_of(&env.tokens, props.end.map_or(Gap::Tight, |end| end.gap));
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
        self.last_extent = stage.session::<ScrollState>(sub(id, "scroll")).extent;
        self.scroll_bound = comp.scroll().is_some();
        let held = match comp.scroll() {
            Some(bind) => match bind.get() {
                Some(Mixed::Same(pos)) => pos,
                _ => ScrollPos::default(),
            },
            None => {
                let state = stage.session::<ScrollState>(sub(id, "scroll"));
                self.anchor = state.anchor;
                ScrollPos { x: state.offset.0, y: state.offset.1 }
            }
        };
        self.anchor_at_read = self.anchor;
        self.offset = held;
        self.offset_at_read = held;
        let rtl = env.direction.is_rtl();
        let view = self.last.view.map_or(Vec2::ZERO, |view| rect_of(view).size());
        // Step 4 applies, in order, the scroll bar and AccessKit moves of the last pass (`scroll_input`), this pass's
        // wheel share and the reveal request queued last pass.
        let frame = jump_frame(&ctx);
        let jump = pass_state::with::<Jumps, _>(&ctx, |jumps| {
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
            let (by, shown) = crate::contract::steps::reveal_move(rect, view_rect);
            self.offset.y += f64::from(by.y);
            self.offset.x += f64::from(if rtl { -by.x } else { by.x });
            self.moved = true;
            self.pass_on = (!quiet).then_some(shown);
        }
        for (mv, _) in crate::contract::steps::scroll_moves(&ctx, id, quiet) {
            let (visible, extent, offset, vertical) = if scroll.vertical_on() {
                (view.y, self.last_extent.1, self.offset.y, true)
            } else {
                (view.x, self.last_extent.0, self.offset.x, false)
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
        // A move of the area's own ends a glide; a child reveal starts one. A running glide moves the offset toward
        // its aim.
        self.glide = if self.moved { None } else { stage.session::<ScrollState>(sub(id, "scroll")).glide };
        if let Some(child) = crate::contract::child_reveal(&ctx, id, quiet) {
            self.glide = Some(Glide { child, to: None, running: false });
        }
        self.glide_at_read = self.glide;
        if let Some(glide) = self.glide.as_mut()
            && let Some(to) = glide.to
            && (glide.running || !quiet)
        {
            let driver = Driver { stage: &*stage, base: id, policy: env.motion };
            if !glide.running {
                driver.jump("glide", Vec2::new(self.offset.x as f32, self.offset.y as f32));
                glide.running = true;
            }
            let at = driver.anim("glide", to, spec::FOLLOW);
            self.offset = ScrollPos { x: f64::from(at.x), y: f64::from(at.y) };
            self.moved = true;
            if at == to {
                self.glide = None;
            }
        }
        let follows = stage.session::<ScrollState>(sub(id, "scroll")).follow_end.unwrap_or(true);
        self.follows_end = scroll.anchor == crate::contract::ScrollAnchor::End && follows;
        if self.follows_end && !self.moved {
            self.offset.y = f64::from((self.last_extent.1 - view.y).max(0.0));
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
        let env = Env::of(ui);
        self.given = self.given_axes(props, &env, at);
        self.passes = self.passes_width(props, &env, at);
        self.given.1 |= self.over_room(props, &env, at, ui.ctx(), id);
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
        let props = Arc::clone(&self.props);
        // A mount outside the flow takes no room in a parent's sizing pass, so its regions run only when it is
        // measured itself. A pane as tall as its content takes its height in the flow.
        let flows = match &props.mount {
            Mount::Inline => true,
            Mount::Pane(pane) => pane.content_height,
            _ => false,
        };
        if !flows && at.ui.is_sizing_pass() {
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
        // the bars the kept content extent asks for, so the records it leaves serve that run.
        self.last = BodyView::default();
        self.offset = ScrollPos::default();
        self.region_gap = gap_of(&at.env.tokens, Gap::Tight);
        self.end_gap = gap_of(&at.env.tokens, props.end.map_or(Gap::Tight, |end| end.gap));
        self.header_gap = header_gap_of(&props, &at.env.tokens);
        self.least_width = least_of(&at.env.tokens, props.least_width);
        self.room = match at.width {
            Some(width) => Vec2::new(width, props.room_height.unwrap_or(f32::INFINITY)),
            None => self.mount_room(&props, at.ui, id, at.at),
        };
        self.given = self.given_axes(&props, at.env, at.at);
        self.passes = self.passes_width(&props, at.env, at.at);
        // A body that fills the main axis of the body it sits in is measured and shown at its share of it. The first
        // sweep of that body's measuring run only counts it: its contents run in the second sweep, at the share.
        let (x, y) = (craft_behavior::Axis::X, craft_behavior::Axis::Y);
        // A container placed at an explicit rect, or mounted on a layer or in a window, is no flow child of that body:
        // it takes its own room, not a share.
        let flows = matches!(at.at, ShowAt::Flow) && self.in_flow(comp);
        // Along an axis the flow does not bound it fills nothing (`sized`): it measures its content there.
        let bounded = crate::layout::flow_room(at.ui);
        let (fills_x, fills_y) = (self.given.0 && bounded.x.is_finite(), self.given.1 && bounded.y.is_finite());
        if flows
            && at.ui.is_sizing_pass()
            && ((fills_x && crate::layout::counting_fills(at.ui, x)) || (fills_y && crate::layout::counting_fills(at.ui, y)))
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
        let key = self.measure_key(at.env, at.width);
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
                // A capped body whose content outgrew its room was laid out as given it: the record of its content says so.
                if self.over_room(&props, at.env, at.at, &ctx, id) {
                    self.given.1 = true;
                }
                took
            }
            // A capped body takes its content height while that fits its room. Taller, the run is taken back and the body
            // is laid out as given the room, at the footer and header this run measured, so the children that fill share
            // it; its record of the content height lets later runs of the pass and kept transcripts lay it out the same.
            None if self.capped(&props, at.env, at.at) => {
                let fits = self.room.y + tolerance;
                let (content, kept) = crate::contract::draft(&ctx, || self.measure_run(comp, at, cx, key.clone(), None), |took| took.y <= fits);
                if kept {
                    content
                } else {
                    self.given.1 = true;
                    self.measured_footer = self.footer_size.unwrap_or(Vec2::ZERO);
                    self.measure_run(comp, at, cx, key, Some(content))
                }
            }
            None => self.measure_run(comp, at, cx, key, None),
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
        let props = Arc::clone(&self.props);
        let (ctx, id) = (at.ui.ctx().clone(), at.resolved.id);
        let layer = matches!(props.mount, Mount::Layer(_) | Mount::FullWindow | Mount::Viewport { .. });
        let fading = layer && !self.open && crate::runtime::motion::layer_fading(&ctx, sub(id, "presence"));
        if !layer || self.open || fading {
            // The measuring run starts from what this run's `read` found: the measuring run of a sizing parent reads
            // the same state.
            let mut stage = MemoryStage::open_peek(&ctx, id);
            let mut fresh = BodyHooks {
                save: self.save.clone(),
                layer: self.layer,
                fixed: self.fixed,
                open_bound: self.open_bound,
                scroll_bound: self.scroll_bound,
                last_extent: self.last_extent,
                anchor: self.anchor,
                follows_end: self.follows_end,
                view_offset: self.view_offset,
                open: self.open,
                open_at_read: self.open,
                presence: self.presence,
                open_given: true,
                shown_closed: fading,
                reuse: true,
                ..BodyHooks::new(&mut *self.body, Arc::clone(&self.props))
            };
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

    fn member(&self, comp: &C) -> bool {
        comp.props(Internal::new()).label.is_some()
    }

    /// The slots of the container's own header parts this pass, for `PartsCx::header_slot`.
    fn geometry(
        &mut self,
        comp: &C,
        cx: &crate::contract::MeasureCx<'_, C>,
        (rect, content): (Rect, Rect),
        rtl: bool,
    ) -> Option<std::rc::Rc<dyn Any>> {
        let props = Arc::clone(&self.props);
        let header = props.header?;
        let parts = layout_memory::boundary(cx.ui.ctx(), true, || comp.header_parts(cx));
        if parts == crate::contract::HeaderParts::NONE {
            return None;
        }
        let tokens = cx.tokens;
        let inner = content.shrink(pad_of(tokens, props.pad));
        let regions = self.regions(&props, inner, pad_of(tokens, props.pad), self.room_size(rect, inner), rtl, self.measured_sizes());
        let (_, slots) = self.header_line(regions.header, pad_of(tokens, header.inset), parts, rtl);
        Some(std::rc::Rc::new(slots))
    }

    fn parts(&mut self, _comp: &mut C, rect: Rect, pcx: &mut PartsCx<'_, C>) {
        let props = Arc::clone(&self.props);
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
                let notch = scroll.notch.wheel(tokens.tokens().typography.body.line_height);
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
                            .gated(crate::runtime::env::Env::of(pcx.ui).wheel == crate::runtime::env::WheelScope::Gated)
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
                .pointer_only("Cmd+] and Cmd+[ move the section, with Shift to the ends");
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
        if let Some(shown) = self.pass_on.take() {
            crate::contract::Focus::reveal(cx, shown);
        }
        // AccessKit Expand, Collapse and Click reach a collapsible container as an activation: it folds or unfolds.
        if props.collapse.is_some() && cx.activation().is_some_and(|activation| activation.by_key) {
            self.open = !self.open;
        }
        self.band_double(cx);
        self.surface_press(comp.sense().contains(craft_behavior::Sense::SURFACE), cx);
        let band = props.header.is_some_and(|header| header.band == DragBand::Move);
        if (band || matches!(props.mount, Mount::Viewport { .. } | Mount::Window { .. }))
            && let Some(viewport) = os_window(props, cx.core.memory.ctx(), cx.core.id, &cx.core.env)
        {
            self.window_os(viewport, cx);
            self.window_movable(viewport, cx);
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
        let props = Arc::clone(&self.props);
        let (id, ctx, rect) = (cx.core.id, cx.core.ctx.clone(), cx.core.rect);
        let rtl = env.direction.is_rtl();
        let node = matches!(C::ROLE, Role::Node(_));
        let scroll = scroll_of(&props);
        let pad = pad_of(&env.tokens, props.pad);
        let inner = cx.core.content_rect.shrink(pad);
        let room = self.room_size(rect, inner);
        let mut regions = self.regions(&props, inner, pad, room, rtl, self.measured_sizes());
        let surface_slot = SurfaceSlot::reserve(ui, matches!(props.mount, Mount::Layer(_)));
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
        let gap = Vec2::splat(gap_of(&env.tokens, props.body_gap));
        let owner = BodyOwner::new(cx.style, gap.y);
        let frame = RegionFrame { id, rtl, node, env: &region_env, gap };
        let (resolved, env_view) = (cx.core.resolved, region_env.view());
        let mut open = self.open;
        let mut gate = self.gate;
        let mut close: Option<CloseReason> = None;
        let field_message = if props.label.is_some() { scope::last_message(&ctx, id) } else { None };
        // B1 header
        if props.header.is_some() {
            let header_parts = Self::header_parts(comp, ui, env, cx);
            // The stops of the parts at the header's end edge come after the header's children.
            let after_header = std::mem::take(&mut cx.core.header_stops);
            let mut bcx = BodyCx::new(body_id, env_view, resolved, regions.header, &mut *cx.core.memory, &mut open, gate.as_mut())
                .with_gate_mixed(gate_mixed)
                .with_field_message(field_message.clone())
                .with_owner(owner);
            let trailing = props.header.is_some_and(|header| header.trailing);
            let inset = props.header.map_or(0.0, |header| pad_of(&env.tokens, header.inset));
            close = close.or(self.header_region(comp, ui, &mut bcx, &frame, regions.header, (trailing, inset, header_parts)));
            for stop in after_header {
                crate::runtime::focus::register_stop(&ctx, stop);
            }
        }
        let animated = props.collapse.is_some_and(|collapse| collapse.animated);
        let is_open = open || (animated && self.presence.visible);
        let enabled = gate.unwrap_or(true);
        // What the body and the end region share: gate, size, roving group and commands. The body adds its arena,
        // transform and scroll owner; the end region lies outside them.
        let line_env = if enabled && props.child_size.is_none() && props.child_glyphs.is_none() && props.roving.is_none() {
            region_env.clone()
        } else {
            region_env.derive_local(|scoped| {
                scoped.enabled &= enabled;
                if let Some(size) = props.child_size {
                    scoped.size = size;
                }
                if let Some(role) = props.child_glyphs {
                    scoped.glyphs = role;
                }
                if props.roving.is_some() {
                    scoped.roving = Some(crate::contract::steps::body_group(id));
                }
            })
        };
        let end_frame = RegionFrame { id, rtl, node, env: &line_env, gap };
        // A footer in the end region is part of a column sized from its content: a fill-width child takes its
        // measured width there and shows across the column, so it never sets the column's width.
        let footer_env = if footer_in_end(&props) { region_env.derive_local(|scoped| scoped.content_column = true) } else { region_env.clone() };
        if real {
            crate::runtime::focus::begin_body(&ctx);
        }
        // A stack is a content column where its width comes from its content: a content-sized stack in flow, or a
        // held fill-width container. A stack given its width (a rect it is shown at, or a column's width it passes
        // on) is a given column, so the width reaches down through nested stacks.
        let sizing = self.sizing(&props);
        let content_column = !self.wide() && matches!(props.layout, layout::BodyLayout::Stack);
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
            let placed = self.regions(&props, inner, pad, room, rtl, beside);
            (regions.body, regions.beside) = (placed.body, placed.beside);
        }
        let line = shared_line(&props, beside);
        // B2 label: the control gets the rect the label leaves; the label column adds to the container's extent.
        let mut body_rect = regions.body;
        let mut label_rect = Rect::NOTHING;
        let mut label_input = None;
        let mut caption = 0.0;
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
                claims: false,
                status: props.footer.is_some_and(|footer| footer.status),
                trailing: Vec2::ZERO,
            };
            let placed = label::run(ui, env, &input, None);
            (body_rect, label_rect, caption) = (placed.control, placed.label, placed.caption);
            label_input = Some(input);
        }
        let label_extra = label_extra_of(regions.body, body_rect, label_rect, rtl);
        // The body's control shows an inside label in its slot, in the measuring run and the real run alike.
        let inside = props.label.as_ref().and_then(|side| label::inside(env.view(), side, cx.core.resolved.enabled));
        // In an inspector the control beside its label fills the width the label leaves.
        let cell_fills = props.label.as_ref().is_some_and(|side| side.layout == label::FieldLayout::Beside)
            && env.values.get::<label::LabelScheme>() == label::LabelScheme::Inspector;
        let body_env = line_env.derive_local(|scoped| {
            let label_width = label_extra.x;
            self.scope_body(scoped, &props, id, BodyScope { room, real, sizing, content_column, scroll, label_width, beside: regions.beside });
            scoped.inside_label = inside;
            scoped.cell_fills = cell_fills;
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
            let wide = self.wide();
            let columns = LabelColumns::of(&props, !wide && !self.held(&props, env));
            let (layout, body) = (&props.layout, &mut *self.body);
            // The room a scrolling body's bars take under or beside its content this run: no part of the line.
            let mut bars_inset = Vec2::ZERO;
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
                let mut labels = LabelsFound::default();
                // The size the body took: a scrolling body takes the visible part of its content, since its region
                // is a child that allocates nothing in `body_ui`.
                let took = match scroll {
                    Some(scroll) => {
                        let off = self.offset;
                        let big = crate::contract::MAX_EXTENT;
                        let bar = scroll_bar_look(&body_env.tokens, ScrollBarKind::Overlay).thickness;
                        let inset = overflow_inset(scroll, Vec2::new(self.last_extent.0, self.last_extent.1), at.size(), bar);
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
                        let view = crate::runtime::lazy::View {
                            owner: id,
                            rect: seen.translate(shift),
                            content: max,
                            axes: (scroll.horizontal_on(), scroll.vertical_on()),
                            rtl,
                            anchor: self.anchor.map(|anchor| anchor.child),
                            reveal: self.glide.map(|glide| glide.child),
                        };
                        // Only the run that lays the content out for the view places its children; a measure lays it
                        // out for its own question.
                        let lays_out = real && !crate::runtime::measure::active(&ctx);
                        if lays_out {
                            crate::runtime::lazy::begin(&ctx, id);
                        }
                        let view_env = body_env.derive_local(|scoped| scoped.lazy = Some(view));
                        let region_env = Env::install_for(&region_ui, view_env.clone());
                        layout::run_body_measured(&mut region_ui, &view_env, layout_id, layout, gap, line, |ui| {
                            shared_labels(ui, id, columns, room.x, self.settle.clone(), &mut labels, |ui| body(ui, bcx))
                        });
                        Env::restore(&ctx, region_env);
                        let extent = region_ui.min_rect().size();
                        if lays_out {
                            let placed = crate::runtime::lazy::placed(&ctx, id);
                            let keeps = !self.scroll_bound && !self.follows_end;
                            follow_anchor(
                                &ctx,
                                (&placed, scroll, keeps),
                                (extent, seen.size()),
                                (&mut self.offset, self.offset_at_read),
                                &mut self.anchor,
                            );
                            aim_glide(&mut self.glide, placed.reveal, shown, (extent, seen.size()), scroll);
                            if self.glide.is_some_and(|glide| glide.to != Some(Vec2::new(shown.x as f32, shown.y as f32))) {
                                repaint::request(&ctx);
                            }
                        }
                        if real {
                            crate::runtime::egui::widgets::note_scroll_region(&ctx, region_ui.clip_rect(), region_ui.min_rect().intersect(max));
                        }
                        self.extent = Some(extent);
                        // Along an axis it scrolls the body took what the view shows; along the other it took its content.
                        bars_inset = overflow_inset(scroll, extent, at.size(), bar);
                        let took = extent + bars_inset;
                        Some(Vec2::new(
                            if scroll.horizontal_on() { took.x.min(at.width()) } else { took.x },
                            if scroll.vertical_on() { took.y.min(at.height()) } else { took.y },
                        ))
                    }
                    // A row that places its block itself asks for its children's width, not the room it spans.
                    None => {
                        let (_, natural) = layout::run_body_measured(&mut body_ui, &body_env, layout_id, layout, gap, line, |ui| {
                            shared_labels(ui, id, columns, room.x, self.settle.clone(), &mut labels, |ui| body(ui, bcx))
                        });
                        natural.map(|width| Vec2::new(width, body_ui.min_rect().height()))
                    }
                };
                Env::restore(&ctx, installed);
                let took = took.unwrap_or_else(|| body_ui.min_rect().size());
                // A content-sized column of rows takes, in a measuring run, the width they take at its group's widest
                // label: the rows measured before the widest one measured against a narrower column.
                let stacked = matches!(layout, layout::BodyLayout::Stack) && !wide;
                let took = match labels.extent.filter(|_| stacked && !real) {
                    Some(width) => Vec2::new(took.x.max(width), took.y),
                    None => took,
                };
                // A content-sized body hands its rows' reaches on to the group around once its measure is known.
                if !real && !wide {
                    self.reached = labels.reached.map(|reached| (reached, !matches!(layout, layout::BodyLayout::Stack)));
                }
                if !real {
                    self.unsettled = labels.unsettled;
                }
                Some(took)
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
                let placed = label::control_after(ui, env, input);
                (body_rect, label_rect, caption) = (placed.control, placed.label, placed.caption);
                label_extra = label_extra_of(regions.body, body_rect, label_rect, rtl);
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
                // A caption above the control counts whole toward the container's width, so a row never narrows the
                // field below it.
                size.x = size.x.max(caption);
            }
            self.view = Some(body_rect);
            if real && props.end.is_some() {
                let mut end_cx = BodyCx::new(body_id, line_env.view(), resolved, regions.end, &mut *cx.core.memory, &mut open, gate.as_mut())
                    .with_field_message(field_message.clone())
                    .with_owner(owner);
                // The end region is centred on the line it shares with the body, above a scroll bar under the body.
                let mut end = regions.end;
                if !end_full_height(&props) {
                    end.max.y = (end.max.y - bars_inset.y).max(end.min.y);
                }
                close = close.or(self.end_region(comp, ui, &mut end_cx, &end_frame, &props, end));
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
            let mut footer_ui = frame.region(ui, "footer", footer_rect);
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
            surface_slot.paint(&env.tokens, kind, rect, env.direction);
        }
        if let Some(kind) = props.header.and_then(|header| header.fill) {
            let style = surface_style(&env.tokens, props.surface.unwrap_or(SurfaceKind::Panel));
            let radius = CornerRadius { sw: 0, se: 0, ..style.radius };
            ui.painter().set(band_slot, Shape::rect_filled(regions.header, radius, surface_style(&env.tokens, kind).fill));
        }
        paint_rules(ui, &props, env, inner, &regions);
        paint_edge_rule(ui, &props, env, rect);
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
        if self.offset != self.offset_at_read || self.anchor != self.anchor_at_read {
            match comp.scroll() {
                Some(mut bind) => {
                    let (label, doc_key) = (cx.core.label_arc(), cx.core.resolved.doc_key);
                    bind.apply(&EditEvent::set(Mixed::Same(self.offset_at_read), self.offset, label, doc_key));
                }
                None => {
                    let state = cx.core.memory.session::<ScrollState>(sub(cx.core.id, "scroll"));
                    state.offset = (self.offset.x, self.offset.y);
                    state.anchor = self.anchor;
                }
            }
            self.offset_at_read = self.offset;
            self.anchor_at_read = self.anchor;
        }
        if self.glide != self.glide_at_read {
            cx.core.memory.session::<ScrollState>(sub(cx.core.id, "scroll")).glide = self.glide;
            self.glide_at_read = self.glide;
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
        if let Some(extent) = self.extent {
            let state = stage.session::<ScrollState>(sub(id, "scroll"));
            // A bottom-anchored region that follows the end shows the rows that arrived this pass in this frame: the
            // repeated pass starts from the new extent.
            if self.follows_end && !self.moved && (extent.y - state.extent.1).abs() > 0.5 {
                layout_memory::ask_discard(ctx);
            }
            state.extent = (extent.x, extent.y);
        }
        if self.extent.is_some() || self.view.is_some() {
            stage.shown::<BodyView>(sub(id, "view")).view = self.view.map(|view| [view.min.x, view.min.y, view.max.x, view.max.y]);
        }
        if *stage.session::<WindowMode>(sub(id, "window_mode")) != self.mode {
            *stage.session::<WindowMode>(sub(id, "window_mode")) = self.mode.clone();
        }
        if self.moved {
            let at_end = self.last.view.map(rect_of).is_some_and(|view| self.offset.y >= f64::from(self.last_extent.1 - view.height()) - 0.5);
            stage.session::<ScrollState>(sub(id, "scroll")).follow_end = Some(at_end);
        }
    }
}

impl BodyHooks<'_> {
    /// The measuring run of the regions in `self.room`, under the inputs `key`: what they took together, padding
    /// included. `content`: the size the body's content took in the run taken back before this one, recorded with it.
    fn measure_run<C: Container>(
        &mut self,
        comp: &mut C,
        at: &Measuring<'_, '_>,
        cx: &mut Cx<'_, C>,
        key: MeasureKey,
        content: Option<Vec2>,
    ) -> Vec2 {
        let props = Arc::clone(&self.props);
        let (id, ctx) = (cx.core.id, at.ui.ctx().clone());
        let pad = Vec2::splat(2.0 * pad_of(at.scx.tokens, props.pad));
        #[cfg(feature = "testing")]
        crate::runtime::measure::count_body(&ctx, id);
        layout_memory::begin_measure(&ctx, id);
        self.reached = None;
        let available = match at.at {
            ShowAt::Rect(rect) => rect,
            // Not the cursor: in a right-to-left layout it extends to -inf along the line.
            ShowAt::Flow => at.ui.available_rect_before_wrap(),
        };
        let origin = available.min;
        // The measuring rect starts at the room's start edge, where the real run places the container: in a
        // right-to-left layout that is the right edge, and the rect ends there. Along an axis with no room limit it
        // is as wide as a scrolling region, since egui lays nothing out leftwards from -inf.
        let rect = if at.env.direction.is_rtl() && available.max.x.is_finite() {
            let width = if self.room.x.is_finite() { self.room.x } else { crate::contract::MAX_EXTENT };
            Rect::from_min_max(Pos2::new(available.max.x - width, origin.y), Pos2::new(available.max.x, origin.y + self.room.y))
        } else {
            Rect::from_min_size(origin, self.room)
        };
        cx.core.rect = rect;
        cx.core.flow = craft_behavior::Flow::new(rect, at.env.direction.is_rtl());
        cx.core.content_rect = cx.core.flow.inset(cx.style.metrics().padding);
        let layer = measure_layer(&props, id, at.ui);
        let mut values = AppValues::default();
        comp.content_env(cx, &mut values);
        let sweep = |hooks: &mut Self, comp: &mut C, cx: &mut Cx<'_, C>| {
            let builder = UiBuilder::new().layer_id(layer).max_rect(rect).sizing_pass().invisible();
            // The regions take explicit scope ids under `id`, so every component in them gets the id the real run
            // gives it and the records it leaves match.
            let mut measure_ui = Ui::new(ctx.clone(), sub(id, "measure"), builder);
            let installed = Env::install_for(&measure_ui, at.env.clone());
            <Self as ShapeHooks<C>>::content(hooks, comp, &mut measure_ui, at.env, cx, &values);
            Env::restore(&ctx, installed);
        };
        // A label group settles its columns before a measure inside stands: when a row of a body that shows at its
        // measured width read a column narrower than the group's widest, the sweep is taken back and the content runs
        // once more with the group opened at the widths the first sweep found.
        if props.label_column {
            let (unsettled, _) = layout_memory::draft(
                &ctx,
                || {
                    sweep(self, comp, cx);
                    self.unsettled.take()
                },
                Option::is_none,
            );
            if let Some(widths) = unsettled {
                (self.settle, self.reached) = (Some(widths), None);
                sweep(self, comp, cx);
                (self.settle, self.unsettled) = (None, None);
            }
        } else {
            sweep(self, comp, cx);
        }
        let record = |key: Id, took: Vec2| layout_memory::emit_record(&ctx, key, layout_memory::Measured::new(self.room, took));
        record(id, self.body_size.unwrap_or(Vec2::ZERO));
        for (key, size) in [("header", self.header_size), ("footer", self.footer_size), ("end", self.end_size), ("controls", self.controls_size)] {
            if let Some(size) = size {
                record(sub(id, key), size);
            }
        }
        if let Some(content) = content {
            record(sub(id, "content"), content);
        }
        // The rows of a content-sized body reach past the columns of the group around by as much more as the
        // container does: a box's padding, or a row's members after its labelled one. The group measures its extent
        // from these, at its widest columns.
        if let Some((reached, row)) = self.reached.take().filter(|_| self.open) {
            let (body, [header, footer, end]) =
                (self.body_size.unwrap_or(Vec2::ZERO), [self.header_size, self.footer_size, self.end_size].map(Option::unwrap_or_default));
            let outer = (self.regions_size(&props, body, header, footer, end) + pad).x;
            reached.forward(&ctx, |columns, past| if row { outer - columns } else { past + outer - body.x });
        }
        layout_memory::end_measure(&ctx, id, key);
        let body = if self.open { self.body_size.unwrap_or(Vec2::ZERO) } else { Vec2::ZERO };
        let (header, footer, end) =
            (self.header_size.unwrap_or(Vec2::ZERO), self.footer_size.unwrap_or(Vec2::ZERO), self.end_size.unwrap_or(Vec2::ZERO));
        self.regions_size(&props, body, header, footer, end) + pad
    }
}
