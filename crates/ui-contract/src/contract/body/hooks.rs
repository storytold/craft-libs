//! The body step's `ShapeHooks`: the pipeline's calls into a container's body.

use super::inherit::{BodyScope, register_instances};
use super::open::open_now;
use super::placement::{Placement, measure_layer, overlay_of, shown_placement};
use super::regions::{
    LabelColumns, LabelsFound, RegionFrame, RegionSizes, Regions, RuleBands, end_full_height, footer_in_body, footer_in_end, footer_rule_under,
    gap_of, header_gap_of, label_extra_of, least_of, pad_of, paint_bracket, paint_edge_rule, paint_rules, rect_of, shared_labels, shared_line,
};
use super::reorder::REORDER_BAND;
use super::scroll::{
    Glide, Jumps, STRIP_BACK, STRIP_FORWARD, ScrollState, aim_glide, follow_anchor, jump_frame, overflow_inset, scroll_of, scroll_strip_height,
    strips_of, view_within,
};
use super::size::MeasureKey;
use super::window::{WindowMode, os_window, top_band, window_instance};
use super::{BodyHooks, BodyView};
use crate::contract::cx::CxCore;
use crate::contract::cx::{BodyCx, BodyOwner, RegionShare};
use crate::contract::label::{self, LabelIn};
use crate::contract::run::{Measuring, ShapeHooks, ShowAt};
use crate::contract::{Container, ContainerProps, Cx, DragBand, Gap, Internal, Mount, PartsCx, Resolved, Scroll, SetPosition, Sizing, ValueOf};
use crate::layout::{self};
use crate::primitives::SurfaceSlot;
use crate::runtime::access::scope;
use crate::runtime::access::{self, Politeness, Role};
use crate::runtime::commands;
use crate::runtime::egui::{alloc, areas, child, repaint, response, style};
use crate::runtime::env::{AppValues, ArenaId, ColumnSweep, Env, FooterSlotOf, Installed, Message, RowLine, ThemeScope};
use crate::runtime::focus::FocusPolicy;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::motion::{Driver, Presence, spec};
use crate::runtime::overlay::CloseReason;
use crate::runtime::pass_state;
use crate::runtime::{keys, layout_memory, pointer, wheel, windows};
use crate::style::ComponentStyle as _;
use crate::style::{StyleCx, SurfaceKind, band_color, scroll_bar_look, surface_style};
use craft_behavior::layout_memory::measure_tolerance;
use craft_behavior::{CancelRule, CursorIcon, EditEvent, KeyRole, Mixed, ReorderKeys, Roving, ScrollPos, ScrollSpan, WheelRoom, WheelTaker};
use craft_theme::variant::Tone;
use egui::emath::GuiRounding as _;
use egui::layers::ShapeIdx;
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
        let props = comp.props(Internal::new());
        let moving = overlay_of(props).filter(|overlay| overlay.movable).and_then(|_| self.mode.running_move(at.ui.ctx(), at.id));
        let mounted = crate::contract::mount::host(props, &at, self.open, at.label, moving);
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

    fn theme_scope(&self, comp: &C) -> Option<ThemeScope> {
        comp.props(Internal::new()).theme.clone()
    }

    fn fit_rank(&self, comp: &C) -> Option<u8> {
        comp.props(Internal::new()).fit_rank
    }

    fn memory_owner(&self, comp: &C) -> Option<Id> {
        comp.props(Internal::new()).owner
    }

    fn dismissed(&mut self, comp: &mut C, cx: &mut Cx<'_, C>, why: CloseReason) {
        self.close = true;
        self.open = false;
        comp.closed(why);
        crate::contract::overlay::note_layer_closed(&cx.core.ctx, cx.core.id, why);
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
        KeyRole::new()
            .reorderable(props.header.is_some_and(|header| header.band == DragBand::Reorder).then_some(ReorderKeys::Stacking))
            .document(props.scroll_owner)
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
        let start = layout::flow_start(&ctx, env);
        self.after_rule = start == Some(true);
        self.rules = RuleBands::of(props, env, crate::runtime::egui::scale::pixels_per_point(&ctx), start);
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
            // A viewport whose fallback covers the window is fixed in both modes, as a full-window mount is: its body
            // never sizes as a layer's content.
            Mount::Viewport { fallback, .. } if fallback.is_full_window() => true,
            Mount::Viewport { .. } if windows::mount_window(&ctx, id) => true,
            Mount::Layer(_) | Mount::Viewport { .. } => {
                let placement = overlay_of(props).map_or_else(Placement::default, |overlay| {
                    let screen = crate::contract::overlay::screen_for(&ctx, env, overlay);
                    shown_placement(&ctx, id, save.as_ref(), overlay, None, screen, env.tokens.space.inset.screen)
                });
                // A layer the user sized takes that size, at its saved position or where its anchor places it.
                placement.size.is_some()
            }
            _ => false,
        };
        self.last = stage.recall::<BodyView>(sub(id, "view"));
        self.strip = scroll_strip_height(&env.tokens);
        self.row = env.tokens.size.control.md;
        let Some(scroll) = scroll else { return };
        // A scrolled body of a layer, or in one, starts at the top at every opening of the layer, as at its first.
        let opening = match props.mount {
            Mount::Layer(_) => crate::runtime::overlay::appears(&ctx, id, false),
            _ => crate::runtime::overlay::drawing_parent(&ctx).is_some_and(|layer| crate::runtime::overlay::opening(&ctx, layer)),
        };
        if opening && comp.scroll().is_none() {
            *stage.session::<ScrollState>(sub(id, "scroll")) = ScrollState::default();
        }
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
        let view = self.last.view.map_or(Vec2::ZERO, |view| self.shown_view(scroll, rect_of(view)).size());
        // Step 4 applies, in order, the scroll bar, AccessKit and reveal moves the update step of the last pass queued
        // (`Jumps`), this pass's wheel share and the scroll moves queued for the body.
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
        self.room = self.mount_room(props, ui, id, at, scx.tokens);
        let env = Env::of(ui);
        self.given = self.given_axes(props, &env, at);
        self.fixed_height = self.fixed_height(props, at);
        self.passes = self.passes_width(props, &env, at);
        self.held = self.held(props, &env);
        self.across = self.held && matches!(env.column, Some(ColumnSweep::Column(..)));
        self.in_column = label::in_column(ui);
        self.egui_cell = env.egui_cell;
        self.given.1 |= self.given_room(props, &env, at, ui.ctx(), id);
        let measured = self.measured_body.unwrap_or(Vec2::ZERO);
        let body = if self.presence.visible { Vec2::new(measured.x, measured.y * self.presence.t) } else { Vec2::ZERO };
        let room = self.room.x - 2.0 * self.pads(props, scx.tokens).x;
        let width = if self.measured_body.is_some() { self.shared_width(room, props) } else { 0.0 };
        self.overflow = self.wanted_width(props).map_or(0.0, |wanted| (wanted - room.max(0.0)).max(0.0));
        let (header, end) = (self.measured_header.unwrap_or(Vec2::ZERO), self.measured_end.unwrap_or(Vec2::ZERO));
        let height = self.regions_size(props, body, header, self.measured_footer, end).y;
        Vec2::new(width, height) + self.frame_size(props, scx.tokens, top_band(props, ui.ctx(), id, scx.tokens))
    }

    /// The regions run once in an invisible sizing pass over the room, as `content` runs them, and the container takes
    /// what they took: nothing an earlier pass recorded counts. Along an axis the body scrolls on it takes at most
    /// the room. What the regions took is recorded for the rest of the pass; a record this pass already holds serves
    /// instead when the room still allows it (`reusable`).
    fn measured_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>) -> Vec2 {
        let scrolled_share = match self.measure_room(comp, at, cx) {
            Ok(scrolled_share) => scrolled_share,
            Err(extent) => return extent,
        };
        let plan = self.measure_plan(at, cx.core.id, scrolled_share);
        let (vertical, pad) = (plan.vertical, plan.pad);
        let took = match plan.reused {
            Some(reused) => self.take_reused(&plan, reused),
            // A share of a scrolling view is laid out as given while the content fits it, so the children that fill
            // share it; taller, the body takes its content height (the children that fill at their least) and the view
            // scrolls.
            None if plan.scrolled_share => self.measure_share(comp, at, cx, &plan),
            // A capped body takes its content height while that fits its room. Taller, the run is taken back and the body
            // is laid out as given the room, at the footer and header this run measured, so the children that fill share
            // it; its record of the content height lets later runs of the pass and kept transcripts lay it out the same.
            None if self.capped(&self.props, at.env, at.at) => self.measure_capped(comp, at, cx, &plan),
            None if self.footed_layer() => self.measure_footed_layer(comp, at, cx, &plan),
            None => self.measure_run(comp, at, cx, plan.key, None),
        };
        self.measured_took(took, pad, vertical)
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
            let mut fresh = self.measuring_hooks(fading);
            crate::contract::run::measured_extent(comp, &mut *fresh, at, &mut stage, style, shown);
        }
        let took = |key: Id| layout_memory::measured(&ctx, key).map(|measured| measured.took);
        // The real run decides the scroll from the content this pass measured: whether it overflows (the wheel, the
        // bars, the strips) and how far it scrolls.
        if let Some(extent) = took(sub(id, "extent")).filter(|_| scroll_of(&props).is_some()) {
            self.last_extent = (extent.x, extent.y);
        }
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
                use craft_behavior::Axis;
                // The available rect, not at least it: a fill child that took the whole room in an earlier pass (a
                // content-sized layer offers its whole room before the footer is known) must not keep its remembered
                // size once the room is less.
                let room = crate::layout::flow_room(ui);
                let (x, y) = (
                    crate::layout::fill_size(ui, id, Axis::X, size.x, room.x, self.given.0),
                    crate::layout::fill_size(ui, id, Axis::Y, size.y, room.y, self.given.1),
                );
                Vec2::new(x, y)
            }
            Sizing::FillWidth => {
                let room = crate::layout::flow_fill_room(ui).x;
                // A fill-width child held in a content column is not given the width (it never sets the column's
                // width) but shows across the column. It widens to the column it is held in, never past it, and only
                // where it is shown: its measure never sets the column's width.
                let held = !self.given.0 && self.held(props, &Env::of(ui));
                let room = if held { room.min(crate::layout::flow_room(ui).x) } else { room };
                // Given the height of the row line it sits on, it fills that line (`given_axes`).
                let height = if self.given.1 {
                    crate::layout::fill_size(ui, id, craft_behavior::Axis::Y, size.y, crate::layout::flow_room(ui).y, true)
                } else {
                    size.y
                };
                Vec2::new(crate::layout::fill_size(ui, id, craft_behavior::Axis::X, size.x, room, self.given.0 || held), height)
            }
            // Its content's width, and the height as `Fill` takes it.
            Sizing::FillHeight => {
                let room = crate::layout::flow_room(ui);
                Vec2::new(size.x, crate::layout::fill_size(ui, id, craft_behavior::Axis::Y, size.y, room.y, self.given.1))
            }
            Sizing::Content => size,
        }
    }

    fn fills_width(&self, comp: &C) -> bool {
        !self.sizing(comp.props(Internal::new())).content_width()
    }

    fn fills_width_only(&self, comp: &C) -> bool {
        self.sizing(comp.props(Internal::new())) == Sizing::FillWidth
    }

    /// The first line this pass's measuring run found: its header's first child's, else its body's
    /// (`layout::FirstLines`).
    fn first_line(&self, _comp: &C, ctx: &Context, id: Id, _height: f32) -> Option<f32> {
        layout_memory::measured(ctx, sub(id, "first")).map(|found| found.took.y)
    }

    /// A container with no header whose body is an unwrapped row reports that row's line as its first line, and one
    /// labelled above its body the line of its body's first control: it sits on a row's line by that control, as a
    /// control labelled above does.
    fn first_is_row_line(&self, comp: &C) -> bool {
        let props = comp.props(Internal::new());
        let above = props.label.as_ref().is_some_and(|side| {
            matches!(side.layout, label::FieldLayout::Above | label::FieldLayout::Caption(_)) && side.kind != label::LabelKind::Hidden
        });
        props.header.is_none() && (above || super::size::unwrapped_row(props))
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
        let pad = self.pads(&props, tokens);
        let band = top_band(&props, cx.ui.ctx(), cx.id, tokens);
        let inner = self.inner_rect(&props, tokens, content, band);
        let regions = self
            .regions(&props, inner, pad, self.room_size(rect, inner), rtl, self.measured_sizes(), !cx.ui.is_sizing_pass())
            .below(content.min.y + band);
        let (_, slots) = self.header_line(regions.header, pad_of(tokens, header.inset), parts, rtl);
        Some(std::rc::Rc::new(slots))
    }

    fn parts(&mut self, _comp: &mut C, rect: Rect, pcx: &mut PartsCx<'_, C>) {
        let props = Arc::clone(&self.props);
        let ctx = pcx.ui.ctx().clone();
        let rtl = pcx.flow.is_rtl();
        let tokens = pcx.tokens;
        let pad = self.pads(&props, tokens);
        let band = top_band(&props, &ctx, pcx.id, tokens);
        let inner = self.inner_rect(&props, tokens, pcx.content_rect, band);
        let room = self.room_size(rect, inner);
        let regions =
            self.regions(&props, inner, pad, room, rtl, self.measured_sizes(), !pcx.ui.is_sizing_pass()).below(pcx.content_rect.min.y + band);
        let scroll = scroll_of(&props);
        if self.open && scroll.is_some() {
            let at = regions.body;
            self.view = Some(at);
            let view = scroll.map_or(at, |scroll| self.shown_view(scroll, at));
            if let Some(scroll) = scroll {
                self.clamp_offset(scroll, view.size());
            }
            for (bar, span) in self.scroll_geometry(&props, at, rtl, tokens) {
                pcx.scroll(bar.key, bar.track, bar.axis, span);
            }
            if let Some((back, forward)) = scroll.and_then(|scroll| self.strips(scroll, at)) {
                for (strip, rect) in [(STRIP_BACK, back), (STRIP_FORWARD, forward)] {
                    pcx.part(strip, rect).pointer_only("The arrow keys, Page Up and Page Down and the wheel scroll the rows");
                }
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
                        WheelTaker::new(pcx.id, at)
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
        Self::move_parts(&props, &regions, rect, self.mode.held(), pcx);
        self.pane_grip = Self::pane_grip(&props, rect, pcx);
    }

    fn sensed(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        self.reorder_keys(comp.props(Internal::new()), cx.core);
    }

    fn updated(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        use crate::contract::Pointer as _;
        let props = comp.props(Internal::new());
        let (scroll, layer) = (scroll_of(props), matches!(props.mount, Mount::Layer(_) | Mount::Viewport { .. }));
        // The OS asked a viewport mount's window to close: the container closes at step 15.
        if matches!(props.mount, Mount::Viewport { .. }) && windows::take_close_request(cx.core.memory.ctx(), cx.core.id) {
            self.close = true;
            self.reason = self.reason.or(Some(CloseReason::Requested));
        }
        if scroll.is_some() {
            self.scroll_input(props, cx);
        }
        // A reveal queued for the body moves its offset against the view this pass shows, on top of a move
        // `scroll_input` queued, within the offsets the content allows; the next pass takes the offset at step 4, as it
        // takes the scroll bar's, and places by it. A reveal that moves nothing queues no jump, so it costs no pass. The
        // part of the rect the moved view shows goes on to the scroll owner around.
        if let Some((scroll, view)) = scroll.zip(self.view)
            && let Some(rect) = crate::contract::take_reveal(&cx.core.ctx, cx.core.id)
        {
            let view = self.shown_view(scroll, view);
            let (by, _) = crate::contract::steps::reveal_move(rect, view);
            let rtl = cx.core.env.direction().is_rtl();
            let id = cx.core.id;
            let from = super::scroll::queued_jump(&cx.core.ctx, id).unwrap_or(self.offset);
            let mut offset = from;
            offset.y += f64::from(by.y);
            offset.x += f64::from(if rtl { -by.x } else { by.x });
            let offset = self.clamped(scroll, view.size(), offset);
            // A reveal of what the view shows moves nothing: no jump, so no repeated pass.
            if offset != from {
                super::scroll::queue_jump(&cx.core.ctx, id, offset);
            }
            let moved = Vec2::new((offset.x - from.x) as f32, (offset.y - from.y) as f32);
            let moved = Vec2::new(if rtl { -moved.x } else { moved.x }, moved.y);
            crate::contract::Focus::reveal(cx, rect.translate(-moved).intersect(view));
        }
        // AccessKit Expand, Collapse and Click reach a collapsible container as an activation: it folds or unfolds.
        if props.collapse.is_some() && cx.activation().is_some_and(|activation| activation.by_key) {
            self.open = !self.open;
        }
        self.band_double(cx.core);
        self.surface_press(comp.sense().contains(craft_behavior::Sense::SURFACE), cx.core);
        let band = props.header.is_some_and(|header| header.band == DragBand::Move);
        if (band || matches!(props.mount, Mount::Viewport { .. } | Mount::Window { .. }))
            && let Some(viewport) = os_window(props, cx.core.memory.ctx(), cx.core.id, &cx.core.env)
        {
            self.window_os(viewport, props, cx);
            self.window_movable(viewport, cx.core);
        }
        self.pane_drag(props, cx.core);
        if layer && !(matches!(props.mount, Mount::Viewport { .. }) && windows::mount_window(cx.core.memory.ctx(), cx.core.id)) {
            self.window_keys(props, cx);
            self.window_drag(props, cx);
        }
        self.window_placed(props, cx.core);
        // The body moves and resizes the layer in screen space; the caller places it in its parent's layer space.
        let from_screen = crate::runtime::overlay::parent_space(&cx.core.ctx, cx.core.id).inverse();
        for mut change in std::mem::take(&mut self.reports) {
            change.rect = from_screen * change.rect;
            comp.placement_changed(&change);
        }
    }

    /// The body step. The children of the header, the body, the end region and the footer run inside their own step
    /// functions; between them this frame holds only the `BodyPlan` the regions share.
    fn content(&mut self, comp: &mut C, ui: &mut Ui, env: &Env, cx: &mut Cx<'_, C>, values: &AppValues) {
        let props = Arc::clone(&self.props);
        let mut plan = self.plan_content(&props, ui, env, cx, values);
        // B1 header
        if props.header.is_some() {
            self.header_step(comp, ui, env, cx, &mut plan);
        }
        self.line_step(&mut plan);
        // The body is measured in the line the real run gives it: what the header and the end region leave. The
        // measuring run places it once both have measured, so its end region runs first (its content never depends
        // on the body); the real run places it from what they took and keeps reading order.
        if !plan.real && plan.is_open && props.end.is_some() {
            self.end_step(comp, ui, cx, &mut plan);
        }
        // A footer the body places is measured before the body, as the end region is.
        if footer_in_body(&props) {
            self.footer_slot_step(comp, ui, cx, &mut plan);
        }
        // B2 label: the control gets the rect the label leaves; the label column adds to the container's extent.
        self.label_step(ui, env, cx.core, &mut plan);
        // B3 body, if open, under the body env; the gate is read after the header changed it.
        if plan.is_open {
            let fits_end = !plan.real && self.end_fit.is_none() && props.end.is_some_and(|end| end.fit);
            if fits_end {
                layout::ask_shortfall(&plan.ctx, plan.layout_id);
            }
            self.body_step(comp, ui, env, cx.core, &mut plan);
            // The end region's ranked units give up the body's shortfall.
            if fits_end && let Some(short) = layout::take_shortfall(&plan.ctx, plan.layout_id) {
                self.end_unfit = Some(short);
            }
            self.after_body(ui, env, &mut plan);
            if plan.real && props.end.is_some() {
                self.end_step(comp, ui, cx, &mut plan);
            }
        } else {
            // A collapsed body keeps its last full size in layout memory, so opening it again places by that size
            // and asks for no discard pass.
            self.body_size = if props.collapse.is_some() { None } else { Some(Vec2::ZERO) };
        }
        self.close_label(&mut plan);
        // B4 footer: under the control column of a label-scoped container, which starts where the label step put
        // the control.
        if props.footer.is_some() {
            self.footer_step(comp, ui, env, cx, &mut plan);
        }
        self.finish_content(ui, env, cx, &plan);
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
            let why = self.reason.take().unwrap_or(CloseReason::Requested);
            comp.closed(why);
            crate::contract::overlay::note_layer_closed(&cx.core.ctx, cx.core.id, why);
        }
        self.write_open(comp, cx);
        // A drag held at the body's edge scrolls it, unless a scroller inside it took the step.
        if let Some(scroll) = scroll_of(comp.props(Internal::new()))
            && let Some(to) = self.edge_step_body(scroll, &cx.core.ctx, self.offset)
        {
            self.offset = to;
        }
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
            crate::contract::steps::record_body_size(ctx, &self.props.mount, id, size);
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

/// How far below the container's top `top` the centre of its first line lies, from what the first children placed in
/// its header and body reported (`targets`): the header's first line, else the body's, moved by `moved` with the body
/// after its run. `None` when the first child there has no line of its own, or nothing was placed.
fn first_line_found(ctx: &Context, (header, body): (Id, Id), top: f32, moved: f32) -> Option<f32> {
    let (header, body) = (layout::close_first_line(ctx, header), layout::close_first_line(ctx, body));
    match header {
        Some(found) => found.map(|y| y - top),
        None => body.flatten().map(|y| y + moved - top),
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
        let (layer, rect) = self.begin_measure_run(at, cx.core, cx.style.metrics().padding);
        let mut values = AppValues::default();
        comp.content_env(cx, &mut values);
        // A scrolling body's view (a bar that takes room, the strips) follows from its content's overflow. The content is
        // laid out as overflowing along every axis it scrolls, beside the bars; when its overflow leaves another view
        // (content that fits), the sweep is taken back and the content runs once more at that view. Content that
        // overflows, whose children are the costly ones to run twice, runs once.
        let overflows = crate::contract::MAX_EXTENT;
        (self.last_extent, self.view_moves) = ((overflows, overflows), None);
        if scroll_of(&self.props).is_some() {
            let ctx = at.ui.ctx().clone();
            let (overflow, _) = crate::contract::draft(
                &ctx,
                || {
                    self.sweeps(comp, at, cx, (layer, rect), &values);
                    self.view_moves.take()
                },
                Option::is_none,
            );
            if let Some(extent) = overflow {
                self.last_extent = extent;
                self.sweeps(comp, at, cx, (layer, rect), &values);
                self.view_moves = None;
            }
        } else {
            self.sweeps(comp, at, cx, (layer, rect), &values);
        }
        self.end_measure_run(at, cx.core.id, key, content)
    }

    /// The measuring run's sweeps of the regions at the view it lays the content out for.
    fn sweeps<C: Container>(
        &mut self,
        comp: &mut C,
        at: &Measuring<'_, '_>,
        cx: &mut Cx<'_, C>,
        (layer, rect): (areas::LayerKey, Rect),
        values: &AppValues,
    ) {
        if self.props.end.is_some_and(|end| end.fit) {
            self.fitting_sweeps(comp, at, cx, (layer, rect), values);
        } else if self.props.label_column {
            self.settling_sweeps(comp, at, cx, (layer, rect), values);
        } else {
            self.sweep(comp, at, cx, (layer, rect), values);
        }
    }

    /// An end region that joins the body's fit (`EndRegion::fit`) gives way before the body: when the body's units ask
    /// for more than the line the region leaves, the sweep is taken back and the content runs once more with the
    /// region's first row giving up the body's shortfall by its ranked units, so they leave before a body unit narrows
    /// or leaves.
    #[inline(never)]
    fn fitting_sweeps<C: Container>(
        &mut self,
        comp: &mut C,
        at: &Measuring<'_, '_>,
        cx: &mut Cx<'_, C>,
        (layer, rect): (areas::LayerKey, Rect),
        values: &AppValues,
    ) {
        let ctx = at.ui.ctx().clone();
        let (unfit, _) = crate::contract::draft(
            &ctx,
            || {
                self.sweep(comp, at, cx, (layer, rect), values);
                self.end_unfit.take()
            },
            Option::is_none,
        );
        if let Some(short) = unfit {
            self.end_fit = Some(short);
            self.sweep(comp, at, cx, (layer, rect), values);
            (self.end_fit, self.end_unfit) = (None, None);
        }
    }

    /// A label group settles its columns before a measure inside stands: when a row of a body that shows at its
    /// measured width read a column narrower than the group's widest, the sweep is taken back and the content runs
    /// once more with the group opened at the widths the first sweep found.
    // The draft's locals, which opt-level 2 would merge into `sweeps`, on the stack of every measuring run below.
    #[inline(never)]
    fn settling_sweeps<C: Container>(
        &mut self,
        comp: &mut C,
        at: &Measuring<'_, '_>,
        cx: &mut Cx<'_, C>,
        (layer, rect): (areas::LayerKey, Rect),
        values: &AppValues,
    ) {
        let ctx = at.ui.ctx().clone();
        let (unsettled, _) = crate::contract::draft(
            &ctx,
            || {
                self.sweep(comp, at, cx, (layer, rect), values);
                self.unsettled.take()
            },
            Option::is_none,
        );
        if let Some(widths) = unsettled {
            (self.settle, self.reached) = (Some(widths), None);
            self.sweep(comp, at, cx, (layer, rect), values);
            (self.settle, self.unsettled) = (None, None);
        }
    }

    /// Opens the measuring run of `cx`'s container: its rect at the room's start edge, which `cx` takes, and the layer
    /// the run lays the regions out on.
    fn begin_measure_run(&mut self, at: &Measuring<'_, '_>, core: &mut CxCore<'_>, padding: craft_behavior::Insets) -> (areas::LayerKey, Rect) {
        let (id, ctx) = (core.id, at.ui.ctx());
        #[cfg(feature = "testing")]
        crate::runtime::measure::count_body(ctx, id);
        layout_memory::begin_measure(ctx, id);
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
        core.rect = rect;
        core.flow = craft_behavior::Flow::new(rect, at.env.direction.is_rtl());
        core.content_rect = core.flow.inset(padding);
        (measure_layer(&self.props, id, at.ui), rect)
    }

    /// One sweep of the measuring run: the regions in an invisible sizing pass at `rect` on `layer`, as `content` runs
    /// them. The regions take explicit scope ids under the container's id, so every component in them gets the id the
    /// real run gives it and the records it leaves match.
    fn sweep<C: Container>(
        &mut self,
        comp: &mut C,
        at: &Measuring<'_, '_>,
        cx: &mut Cx<'_, C>,
        (layer, rect): (areas::LayerKey, Rect),
        values: &AppValues,
    ) {
        let ctx = at.ui.ctx();
        let mut measure_ui = measure_ui(ctx, cx.core.id, layer, rect);
        let installed = Env::install_for(&measure_ui, at.env.clone());
        <Self as ShapeHooks<C>>::content(self, comp, &mut measure_ui, at.env, cx, values);
        Env::restore(ctx, installed);
    }

    /// Closes the measuring run: the records of what the regions took, the reach of a content-sized body's rows, and
    /// the regions' size with the padding.
    fn end_measure_run(&mut self, at: &Measuring<'_, '_>, id: Id, key: MeasureKey, content: Option<Vec2>) -> Vec2 {
        let props = Arc::clone(&self.props);
        let ctx = at.ui.ctx();
        let pad = self.frame_size(&props, at.scx.tokens, top_band(&props, ctx, id, at.scx.tokens));
        let record = |key: Id, took: Vec2| layout_memory::emit_record(ctx, key, layout_memory::Measured::new(self.room, took));
        record(id, self.body_size.unwrap_or(Vec2::ZERO));
        if let Some(first) = self.first_line {
            record(sub(id, "first"), Vec2::new(0.0, first));
        }
        for (key, size) in [("header", self.header_size), ("footer", self.footer_size), ("end", self.end_size), ("controls", self.controls_size)] {
            if let Some(size) = size {
                record(sub(id, key), size);
            }
        }
        if let Some(content) = content {
            record(sub(id, "content"), content);
        }
        // What a scrolling body's content took, for the real run's scroll (`pass_extent`).
        if let Some(extent) = self.extent.filter(|_| scroll_of(&props).is_some()) {
            record(sub(id, "extent"), extent);
        }
        // The rows of a content-sized body reach past the columns of the group around by as much more as the
        // container does: a box's padding, or a row's members after its labelled one. The group measures its extent
        // from these, at its widest columns.
        if let Some((reached, row)) = self.reached.take().filter(|_| self.open) {
            let (body, [header, footer, end]) =
                (self.body_size.unwrap_or(Vec2::ZERO), [self.header_size, self.footer_size, self.end_size].map(Option::unwrap_or_default));
            let outer = (self.regions_size(&props, body, header, footer, end) + pad).x;
            reached.forward(ctx, |columns, past| if row { outer - columns } else { past + outer - body.x });
        }
        layout_memory::end_measure(ctx, id, key);
        let body = if self.open { self.body_size.unwrap_or(Vec2::ZERO) } else { Vec2::ZERO };
        let (header, footer, end) =
            (self.header_size.unwrap_or(Vec2::ZERO), self.footer_size.unwrap_or(Vec2::ZERO), self.end_size.unwrap_or(Vec2::ZERO));
        self.regions_size(&props, body, header, footer, end) + pad
    }
}

/// What the regions of one body step share: where they lie, the envs they run under and the label step's placement.
/// `content` holds it, on the heap, while the regions' children run.
struct BodyPlan<'p> {
    props: &'p ContainerProps,
    resolved: &'p Resolved<'p>,
    owner: BodyOwner<'p>,
    ctx: Context,
    id: Id,
    body_id: Id,
    layout_id: Id,
    rect: Rect,
    rtl: bool,
    node: bool,
    /// Not a measuring run.
    real: bool,
    scroll: Option<Scroll>,
    pad: Vec2,
    inner: Rect,
    room: Vec2,
    gap: Vec2,
    regions: Regions,
    surface_slot: SurfaceSlot,
    band_slot: ShapeIdx,
    region_env: Env,
    /// What the body and the end region share: gate, roving group and commands.
    line_env: Env,
    footer_env: Env,
    body_env: Option<Env>,
    open: bool,
    gate: Option<bool>,
    gate_mixed: bool,
    close: Option<CloseReason>,
    field_message: Option<(Message, Tone)>,
    /// The ids the measuring run's first lines of the header and the body report under.
    first: (Id, Id),
    animated: bool,
    is_open: bool,
    content_column: bool,
    /// The body's shared line.
    line: Option<f32>,
    body_rect: Rect,
    label_rect: Rect,
    label_input: Option<LabelIn<'p>>,
    caption: f32,
    /// Where the body runs; a measuring run may move it once the control reported its minimum width.
    ran_top: f32,
    label_extra: Vec2,
    /// The room a scrolling body's bars take under or beside its content this run: no part of the line.
    bars_inset: Vec2,
}

impl<'p> BodyPlan<'p> {
    /// What a region's `BodyCx` takes of the container; `gate` for the header, whose gate check box shows the mixed state.
    fn share(&self, gate: bool) -> RegionShare<'p> {
        RegionShare { gate_mixed: gate && self.gate_mixed, field_message: self.field_message.clone(), owner: self.owner }
    }

    fn frame(&self) -> RegionFrame<'_> {
        RegionFrame { id: self.id, rtl: self.rtl, node: self.node, env: &self.region_env, gap: self.gap }
    }
}

/// What the body region's run reads of the plan, apart from the gate and open state its `BodyCx` holds.
struct BodyRun<'r> {
    ctx: &'r Context,
    id: Id,
    body_id: Id,
    layout_id: Id,
    rtl: bool,
    node: bool,
    real: bool,
    gap: Vec2,
    room_x: f32,
    scroll: Option<Scroll>,
    layout: &'r layout::BodyLayout,
    line: layout::BodyLine,
    columns: LabelColumns,
    env: &'r Env,
    /// While the body opens or closes, it shows only the part of its last height that the presence reaches.
    reveal_clip: Option<Rect>,
    /// The body shows through a view of its own (`ContainerProps::scroll_owner`): its region is its clip.
    view_clip: bool,
}

/// The region a scrolling body's content runs in, and what its run reads after the content.
struct ScrollRegion {
    ui: Ui,
    /// The env the content runs under, with the view, and the one it replaced in the region.
    env: Env,
    installed: Installed,
    /// The part of the body the view shows, beside the bars.
    seen: Rect,
    /// The region's rect, as large as the scrolling axes allow.
    max: Rect,
    /// The view this pass shows.
    shown: ScrollPos,
    /// The bars' thickness.
    bar: f32,
    /// The run lays the content out for the view.
    lays_out: bool,
    /// The bars' inset and the part of the body the content shows in, as the extent the run starts from gives them.
    laid_for: (Vec2, Rect),
}

/// The `Ui` a measuring run lays a transformed body out in: on the body layer the real run shows it on.
fn measure_layer_ui(ui: &mut Ui, id: Id, body_rect: Rect) -> Box<Ui> {
    Box::new(alloc::child(ui, UiBuilder::new().layer_id(areas::body_layer(ui.layer_id(), id)).max_rect(body_rect)))
}

/// The body region's `Ui` at `at` in `parent`, with the gap between its children and the clip of an opening body. It
/// lives on the heap: the body's children run in it.
fn body_ui(run: &BodyRun<'_>, parent: &mut Ui, at: Rect) -> Box<Ui> {
    let builder = UiBuilder::new().max_rect(at).layout(child::column_layout(run.rtl)).scope_id(run.body_id);
    let mut body_ui = Box::new(alloc::child(parent, if run.node { access::with_parent(builder, run.id) } else { builder }));
    style::set_item_spacing(&mut body_ui, run.gap);
    if let Some(clip) = run.reveal_clip {
        body_ui.set_clip_rect(clip.intersect(body_ui.clip_rect()));
    }
    if run.view_clip {
        body_ui.set_clip_rect(at.intersect(body_ui.clip_rect()));
    }
    body_ui
}

struct BodyFacts<'p> {
    node: bool,
    owner: BodyOwner<'p>,
    surface: Option<SurfaceKind>,
    gap: Vec2,
    gate_mixed: bool,
}

impl BodyHooks<'_> {
    /// The regions and the envs the body step runs them under; registers the container's command instances.
    // Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
    #[inline(never)]
    fn plan_content<'p, 'a: 'p, C: Container>(
        &mut self,
        props: &'p ContainerProps,
        ui: &mut Ui,
        env: &Env,
        cx: &mut Cx<'a, C>,
        values: &AppValues,
    ) -> Box<BodyPlan<'p>> {
        // Only a container that declares its value a gate (a group box) gates its content; a radio group whose value
        // is a bool does not.
        // A mixed gate reads as off, and its check box shows the mixed face.
        self.gate = cx.shown.as_deref().filter(|_| props.gates).and_then(|value| (value as &dyn Any).downcast_ref::<bool>()).copied();
        let gate_mixed = props.gates && cx.mixed;
        self.gate = self.gate.or(gate_mixed.then_some(false));
        self.gate_at_read = self.gate;
        let gap = Vec2::splat(gap_of(&env.tokens, props.body_gap));
        let facts = BodyFacts {
            node: matches!(C::ROLE, Role::Node(_)),
            owner: BodyOwner::new(cx.style, gap.y),
            surface: super::inherit::own_surface(props, cx),
            gap,
            gate_mixed,
        };
        self.plan_regions(props, ui, env, cx.core, values, facts)
    }

    #[inline(never)]
    fn plan_regions<'p, 'a: 'p>(
        &mut self,
        props: &'p ContainerProps,
        ui: &mut Ui,
        env: &Env,
        core: &CxCore<'a>,
        values: &AppValues,
        facts: BodyFacts<'p>,
    ) -> Box<BodyPlan<'p>> {
        let BodyFacts { node, owner, surface, gap, gate_mixed } = facts;
        let (id, ctx, rect) = (core.id, core.ctx.clone(), core.rect);
        let rtl = env.direction.is_rtl();
        let scroll = scroll_of(props);
        let pad = self.pads(props, &env.tokens);
        let band = top_band(props, &ctx, id, &env.tokens);
        let inner = self.inner_rect(props, &env.tokens, core.content_rect, band);
        let room = self.room_size(rect, inner);
        // A measuring run lays out and measures: it registers no instance, focus body, scroll region or body layer,
        // and claims no room in a parent flow.
        let real = !ui.is_sizing_pass();
        let regions = self.regions(props, inner, pad, room, rtl, self.measured_sizes(), real).below(core.content_rect.min.y + band);
        let surface_slot = SurfaceSlot::reserve(ui, matches!(props.mount, Mount::Layer(_)));
        let band_slot = ui.painter().add(Shape::Noop);
        let window = window_instance(props, id).filter(|window| *window != id);
        if real {
            register_instances(&ctx, props, id, window);
        }
        let region_env = env.derive(|scoped| {
            self.scope_regions(scoped, props, core, window, values, real);
            scoped.surface = SurfaceKind::under(surface, scoped.surface);
        });
        // The size and glyph role the container gives its children reach every region: header, body, end region and
        // footer.
        let region_env = if props.child_size.is_none() && props.child_glyphs.is_none() {
            region_env
        } else {
            region_env.derive_local(|scoped| {
                if let Some(size) = props.child_size {
                    scoped.size = size;
                }
                if let Some(role) = props.child_glyphs {
                    scoped.glyphs = role;
                }
            })
        };
        let field_message = if props.label.is_some() { scope::last_message(&ctx, id) } else { None };
        // A measuring run finds the container's first line: the first child its header or body places in flow reports
        // it (`layout::FirstLines`).
        let first = (sub(id, "first_header"), sub(id, "first_body"));
        if !real {
            layout::open_first_lines(&ctx, &[first.0, first.1]);
        }
        Box::new(BodyPlan {
            props,
            resolved: core.resolved,
            owner,
            id,
            body_id: sub(id, "body"),
            layout_id: sub(id, "layout"),
            rect,
            rtl,
            node,
            real,
            scroll,
            pad,
            inner,
            room,
            gap,
            regions,
            surface_slot,
            band_slot,
            line_env: region_env.clone(),
            footer_env: region_env.clone(),
            region_env,
            body_env: None,
            open: self.open,
            gate: self.gate,
            gate_mixed,
            close: None,
            field_message,
            first,
            animated: false,
            is_open: false,
            content_column: false,
            line: None,
            body_rect: Rect::NOTHING,
            label_rect: Rect::NOTHING,
            label_input: None,
            caption: 0.0,
            ran_top: 0.0,
            label_extra: Vec2::ZERO,
            bars_inset: Vec2::ZERO,
            ctx,
        })
    }

    /// B1: the header region, with the container's own header parts beside it.
    fn header_step<C: Container>(&mut self, comp: &mut C, ui: &mut Ui, env: &Env, cx: &mut Cx<'_, C>, plan: &mut BodyPlan<'_>) {
        let props = plan.props;
        let header_parts = Self::header_parts(comp, ui, env, cx);
        // The stops of the parts at the header's end edge come after the header's children.
        let after_header = std::mem::take(&mut cx.core.header_stops);
        let share = plan.share(true);
        let mut bcx = BodyCx::new(
            plan.body_id,
            plan.region_env.view(),
            plan.resolved,
            plan.regions.header,
            &mut *cx.core.memory,
            &mut plan.open,
            plan.gate.as_mut(),
            Some(share),
        );
        let trailing = props.header.is_some_and(|header| header.trailing);
        let inset = props.header.map_or(0.0, |header| pad_of(&env.tokens, header.inset));
        let header_env = (!plan.real).then(|| plan.region_env.derive_local(|scoped| scoped.first_line = Some(plan.first.0)));
        let frame = RegionFrame { id: plan.id, rtl: plan.rtl, node: plan.node, env: header_env.as_ref().unwrap_or(&plan.region_env), gap: plan.gap };
        let closed = self.header_region(comp, ui, &mut bcx, &frame, plan.regions.header, (trailing, inset, header_parts));
        plan.close = plan.close.or(closed);
        for stop in after_header {
            crate::runtime::focus::register_stop(&plan.ctx, stop);
        }
    }

    /// The env the body and the end region share, the footer's, and whether the body shows.
    fn line_step(&mut self, plan: &mut BodyPlan<'_>) {
        let props = plan.props;
        plan.animated = props.collapse.is_some_and(|collapse| collapse.animated);
        plan.is_open = plan.open || (plan.animated && self.presence.visible);
        let enabled = plan.gate.unwrap_or(true);
        // What the body and the end region share: gate, roving group and commands. The body adds its arena,
        // transform and scroll owner; the end region lies outside them.
        if !enabled || props.roving.is_some() {
            let id = plan.id;
            plan.line_env = plan.region_env.derive_local(|scoped| {
                scoped.enabled &= enabled;
                if props.roving.is_some() {
                    scoped.roving = Some(crate::contract::steps::body_group(id));
                }
            });
        }
        // A footer in the end region is part of a column sized from its content: a fill-width child takes its
        // measured width there and shows across the column, so it never sets the column's width. A footer the body
        // places shows across its slot the same way.
        if footer_in_end(props) || footer_in_body(props) {
            plan.footer_env = plan.region_env.derive_local(|scoped| scoped.content_column = true);
        }
        if plan.real {
            crate::runtime::focus::begin_body(&plan.ctx);
        }
        // A stack is a content column where its width comes from its content: a content-sized stack in flow, or a
        // held fill-width container. A stack given its width (a rect it is shown at, or a column's width it passes
        // on) is a given column, so the width reaches down through nested stacks.
        plan.content_column = !self.wide() && matches!(props.layout, layout::BodyLayout::Stack);
    }

    /// The end region: in the measuring run before the body, at the region's place; in the real run after it, on the
    /// body's line.
    fn end_step<C: Container>(&mut self, comp: &mut C, ui: &mut Ui, cx: &mut Cx<'_, C>, plan: &mut BodyPlan<'_>) {
        let props = plan.props;
        let share = plan.share(false);
        let mut end_cx = BodyCx::new(
            plan.body_id,
            plan.line_env.view(),
            plan.resolved,
            plan.regions.end,
            &mut *cx.core.memory,
            &mut plan.open,
            plan.gate.as_mut(),
            Some(share),
        )
        .with_exiting(self.exiting);
        // The end region shares the body's line, above a scroll bar under the body: its children sit on the
        // members' line, from the body's top (a label may stand above the body).
        let mut end = plan.regions.end;
        let mut line = None;
        if plan.real && !end_full_height(props) {
            end.max.y = (end.max.y - plan.bars_inset.y).max(end.min.y);
            let down = plan.body_rect.min.y - end.min.y;
            line = layout::members_line(&plan.ctx, plan.layout_id).map(|line| RowLine {
                centre: line.centre.map(|centre| centre + down),
                height: line.height.map(|height| height + down),
                ..line
            });
        }
        let fitted = self.end_fit.filter(|_| !plan.real).map(|short| plan.line_env.derive_local(|scoped| scoped.rank_short = Some(short)));
        let frame = RegionFrame { id: plan.id, rtl: plan.rtl, node: plan.node, env: fitted.as_ref().unwrap_or(&plan.line_env), gap: plan.gap };
        let closed = self.end_region(comp, ui, &mut end_cx, &frame, props, end, line);
        plan.close = plan.close.or(closed);
    }

    /// B2: the body's place beside the regions measured so far, the label step and the body's env.
    // Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
    #[inline(never)]
    fn label_step(&mut self, ui: &mut Ui, env: &Env, core: &CxCore<'_>, plan: &mut BodyPlan<'_>) {
        let props = plan.props;
        let (ctx, id, real) = (&plan.ctx, plan.id, plan.real);
        let beside =
            if real { self.measured_sizes() } else { RegionSizes { header: self.header_size, footer: self.measured_footer, end: self.end_size } };
        if !real {
            let placed = self.regions(props, plan.inner, plan.pad, plan.room, plan.rtl, beside, real);
            (plan.regions.body, plan.regions.beside) = (placed.body, placed.beside);
        }
        let line = shared_line(props, beside);
        // A row given its height sets its line at the middle of its band: the line is as high as the band.
        plan.line = if self.centres_line(props) { Some(line.unwrap_or(0.0).max(plan.regions.body.height())) } else { line };
        plan.body_rect = plan.regions.body;
        if let Some(side) = props.label.as_ref() {
            let resolved = plan.resolved;
            let input = LabelIn {
                scope: id,
                owner: id,
                parent: plan.node.then_some(id),
                row: plan.regions.body,
                room: if self.sizing(props).content_width() {
                    layout::ambient_room(ctx).map_or(plan.room.x, |offered| plan.room.x.max(offered))
                } else {
                    plan.regions.body.width()
                },
                content_row: false,
                egui_cell: self.egui_cell,
                text: resolved.label_value,
                side,
                help: props.help.as_ref(),
                enabled: resolved.enabled,
                size: resolved.size,
                control_min: scope::measured_control_min(ctx, id),
                leaf: false,
                in_column: self.in_column,
                claims: false,
                status: props.footer.is_some_and(|footer| footer.status),
                trailing: Vec2::ZERO,
            };
            let placed = label::run(ui, env, &input, None);
            (plan.body_rect, plan.label_rect, plan.caption) = (placed.control, placed.label, placed.caption);
            plan.label_input = Some(input);
        }
        plan.ran_top = plan.body_rect.min.y;
        plan.label_extra = label_extra_of(plan.regions.body, plan.body_rect, plan.label_rect, plan.rtl);
        // The body's control shows an inside label in its slot, in the measuring run and the real run alike.
        let inside = props.label.as_ref().and_then(|side| label::inside(env.view(), side, core.resolved.enabled));
        // In an inspector the control beside its label fills the width the label leaves.
        let cell_fills = props.label.as_ref().is_some_and(|side| side.layout == label::FieldLayout::Beside)
            && env.values.get::<label::LabelScheme>() == label::LabelScheme::Inspector;
        let (room, content_column, scroll, beside, label_width) =
            (plan.room, plan.content_column, plan.scroll, plan.regions.beside, plan.label_extra.x);
        let (first_body, layout_id) = (plan.first.1, plan.layout_id);
        // The slot of a footer the body places takes the size the measuring run's footer took.
        let footer_slot = footer_in_body(props)
            .then(|| FooterSlotOf { owner: id, size: if real { self.measured_footer } else { self.footer_size.unwrap_or(Vec2::ZERO) } });
        plan.body_env = Some(plan.line_env.derive_local(|scoped| {
            self.scope_body(scoped, props, id, BodyScope { room, real, content_column, scroll, label_width, beside, layout: layout_id });
            scoped.inside_label = inside.map(|inside| inside as crate::runtime::env::Held);
            scoped.cell_fills = cell_fills;
            if footer_slot.is_some() {
                scoped.footer_slot = footer_slot;
            }
            // A scrolling body shows its first child wherever the view is: the container has no first line there.
            scoped.first_line = (!real && scroll.is_none()).then_some(first_body);
        }));
    }

    /// B3: the body region, on the body layer when the container transforms its body.
    #[expect(clippy::expect_used, reason = "the label step sets the body env before the body step")]
    fn body_step<C: Container>(&mut self, comp: &mut C, ui: &mut Ui, env: &Env, core: &mut CxCore<'_>, plan: &mut BodyPlan<'_>) {
        let props = plan.props;
        let body_env = plan.body_env.as_ref().expect("the label step ran");
        let reveal_clip = (plan.animated && self.presence.t < 1.0).then(|| {
            let body_rect = plan.body_rect;
            Rect::from_min_max(
                body_rect.min,
                Pos2::new(body_rect.max.x, body_rect.min.y + self.measured_body.map_or(0.0, |size| size.y) * self.presence.t),
            )
        });
        let run = BodyRun {
            ctx: &plan.ctx,
            id: plan.id,
            body_id: plan.body_id,
            layout_id: plan.layout_id,
            rtl: plan.rtl,
            node: plan.node,
            real: plan.real,
            gap: plan.gap,
            room_x: plan.room.x,
            scroll: plan.scroll,
            layout: &props.layout,
            line: layout::BodyLine { shared: plan.line, least: props.least_height.unwrap_or(0.0) },
            columns: LabelColumns::of(props, !self.wide() && !self.held(props, env)),
            env: body_env,
            reveal_clip,
            view_clip: props.scroll_owner && props.transform.is_none(),
        };
        let body_rect = plan.body_rect;
        let share = plan.share(false);
        let mut bcx =
            BodyCx::new(plan.body_id, body_env.view(), plan.resolved, body_rect, &mut *core.memory, &mut plan.open, plan.gate.as_mut(), Some(share));
        let mut bars = Vec2::ZERO;
        let mut end = |ui: &mut Ui, bcx: &mut BodyCx<'_>| comp.body_end(ui, bcx);
        self.body_size = match props.transform {
            Some(transform) => self.run_transformed(&run, ui, (transform, core.root_sense, plan.rect, body_rect), &mut bcx, &mut bars, &mut end),
            None => self.run_body(&run, ui, body_rect, &mut bcx, &mut bars, &mut end),
        };
        plan.close = plan.close.or(bcx.close_requested());
        plan.bars_inset = bars;
    }

    /// The body region on its own layer, under the container's transform. The measuring run lays the body out on the
    /// layer the real run shows it on.
    fn run_transformed(
        &mut self,
        run: &BodyRun<'_>,
        ui: &mut Ui,
        (transform, root_sense, rect, body_rect): (craft_behavior::CanvasTransform, craft_behavior::Sense, Rect, Rect),
        bcx: &mut BodyCx<'_>,
        bars: &mut Vec2,
        end: &mut dyn FnMut(&mut Ui, &mut BodyCx<'_>),
    ) -> Option<Vec2> {
        if !run.real {
            return self.run_body(run, &mut measure_layer_ui(ui, run.id, body_rect), body_rect, bcx, bars, end);
        }
        let parent = ui.layer_id();
        let layer = areas::body_layer(parent, run.id);
        let tf = areas::body_transform(body_rect.min, transform.scale, transform.offset);
        if self.props.arena {
            crate::primitives::sensing::set_layer(run.ctx, ArenaId::new(run.id), layer);
        }
        let forward = areas::Forward { root: run.id, sense: response::to_egui(root_sense), rect: rect.intersect(ui.clip_rect()) };
        let body_layer =
            areas::BodyLayer { parent, layer, transform: tf, at: body_rect.min, clip: body_rect.intersect(ui.clip_rect()), forward: Some(forward) };
        areas::show_body_layer(run.ctx, body_layer, |area_ui| self.run_body(run, area_ui, body_rect, bcx, bars, end))
    }

    /// The body region's run at `at` in `parent`: its `Ui`, its layout and the label columns its rows share, then the
    /// body-end hook `end`. Returns the size the body took.
    fn run_body(
        &mut self,
        run: &BodyRun<'_>,
        parent: &mut Ui,
        at: Rect,
        bcx: &mut BodyCx<'_>,
        bars: &mut Vec2,
        end: &mut dyn FnMut(&mut Ui, &mut BodyCx<'_>),
    ) -> Option<Vec2> {
        let mut body_ui = body_ui(run, parent, at);
        let installed = Env::install_for(&body_ui, run.env.clone());
        let mut labels = LabelsFound::default();
        // The size the body took: a scrolling body takes the visible part of its content, since its region is a
        // child that allocates nothing in `body_ui`.
        let took = match run.scroll {
            Some(scroll) => self.run_scrolled(run, scroll, &mut body_ui, at, bcx, &mut labels, bars),
            // A row that places its block itself asks for its children's width, not the room it spans.
            None => {
                let (body, settle) = (&mut *self.body, &self.settle);
                let (_, natural) = layout::run_body_measured(&mut body_ui, run.env, run.layout_id, run.layout, run.gap, run.line, |ui| {
                    shared_labels(ui, run.id, run.columns, run.room_x, settle.clone(), &mut labels, |ui| body(ui, bcx))
                });
                natural.map(|width| Vec2::new(width, body_ui.min_rect().height()))
            }
        };
        // The body-end hook runs in the body's own column below its layout and scroll region, outside the shared label
        // columns, so it lies under the body and above the footer in every footer place: beside an `EndTop` footer
        // column too. A scrolling body's region allocates nothing in `body_ui`, so the hook starts below the part it
        // shows.
        if run.scroll.is_some() {
            crate::runtime::egui::alloc::space(&mut body_ui, Vec2::new(0.0, took.map_or(0.0, |size| size.y)));
        }
        let before = body_ui.min_rect().height();
        end(&mut body_ui, bcx);
        let ended = body_ui.min_rect().height() - before;
        let took = took.map(|size| Vec2::new(size.x, size.y + ended));
        Env::restore(run.ctx, installed);
        let took = took.unwrap_or_else(|| body_ui.min_rect().size());
        self.body_took(run, took, labels)
    }

    /// What the body took, as its container counts it, and what its rows found of the label columns.
    fn body_took(&mut self, run: &BodyRun<'_>, took: Vec2, labels: LabelsFound) -> Option<Vec2> {
        // A content-sized column of rows takes, in a measuring run, the width they take at its group's widest
        // label: the rows measured before the widest one measured against a narrower column.
        let wide = self.wide();
        let stacked = matches!(run.layout, layout::BodyLayout::Stack) && !wide;
        let took = match labels.extent.filter(|_| stacked && !run.real) {
            Some(width) => Vec2::new(took.x.max(width), took.y),
            None => took,
        };
        // A content-sized body hands its rows' reaches on to the group around once its measure is known.
        if !run.real && !wide {
            self.reached = labels.reached.map(|reached| (reached, !matches!(run.layout, layout::BodyLayout::Stack)));
        }
        if !run.real {
            self.unsettled = labels.unsettled;
        }
        Some(took)
    }

    /// A scrolling body's run: its content lies in a region of its own, offset by the view, and only the run that
    /// lays the content out for the view places its children. Returns the visible part of what the content took.
    #[allow(clippy::too_many_arguments, reason = "the body region's run, its scroll, its `Ui` and the three results it writes")]
    // A scrolling body's second `Ui` and view; opt-level 2 inlines it into `run_body`, where every body would hold them.
    #[inline(never)]
    fn run_scrolled(
        &mut self,
        run: &BodyRun<'_>,
        scroll: Scroll,
        body_ui: &mut Ui,
        at: Rect,
        bcx: &mut BodyCx<'_>,
        labels: &mut LabelsFound,
        bars: &mut Vec2,
    ) -> Option<Vec2> {
        let mut region = self.scroll_region(run, scroll, body_ui, at);
        let (body, settle) = (&mut *self.body, &self.settle);
        layout::run_body_measured(&mut region.ui, &region.env, run.layout_id, run.layout, run.gap, run.line, |ui| {
            shared_labels(ui, run.id, run.columns, run.room_x, settle.clone(), labels, |ui| body(ui, bcx))
        });
        self.scrolled(run, scroll, at, region, bars)
    }

    /// The region a scrolling body's content lies in this run, and the view it shows.
    fn scroll_region(&mut self, run: &BodyRun<'_>, scroll: Scroll, body_ui: &mut Ui, at: Rect) -> ScrollRegion {
        let (ctx, id, rtl) = (run.ctx, run.id, run.rtl);
        let off = self.offset;
        let big = crate::contract::MAX_EXTENT;
        let bar = scroll_bar_look(&run.env.tokens).thickness;
        let last = Vec2::new(self.last_extent.0, self.last_extent.1);
        let inset = overflow_inset(scroll, last, at.size(), bar);
        let shows = view_within(scroll, last, at, self.strip);
        let seen = Rect::from_min_max(
            Pos2::new(if rtl { shows.min.x + inset.x } else { shows.min.x }, shows.min.y),
            Pos2::new(if rtl { shows.max.x } else { shows.max.x - inset.x }, shows.max.y - inset.y),
        );
        let (width, height) = (if scroll.horizontal_on() { big } else { seen.width() }, if scroll.vertical_on() { big } else { seen.height() });
        // The content moves by whole physical pixels on steps of egui's UI rounding, the grids widget rects and paint
        // are rounded to: a settled run moved with the content then replays where a run would be
        // (`lazy::keeps_placement`).
        let ppp = ctx.pixels_per_point();
        let (x, y) = ((off.x as f32).round_to_pixels(ppp).round_ui(), (off.y as f32).round_to_pixels(ppp).round_ui());
        let x = if scroll.horizontal_on() { x } else { 0.0 };
        let min_x = if rtl { seen.max.x - width + x } else { seen.min.x - x };
        let max = Rect::from_min_size(Pos2::new(min_x, seen.min.y - y), Vec2::new(width, height));
        let layout_of = *body_ui.layout();
        let mut region_ui = alloc::child(body_ui, UiBuilder::new().scope_id(sub(id, "scroll_region")).layout(layout_of).max_rect(max));
        region_ui.set_clip_rect(shows.intersect(body_ui.clip_rect()));
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
        };
        // Only the run that lays the content out for the view places its children; a measure lays it out for its
        // own question.
        let lays_out = run.real && !crate::runtime::measure::active(ctx);
        if lays_out {
            crate::runtime::lazy::begin(ctx, view, self.anchor.map(|anchor| anchor.child), self.glide.map(|glide| glide.child));
        }
        let env = run.env.derive_local(|scoped| scoped.lazy = Some(view));
        let installed = Env::install_for(&region_ui, env.clone());
        ScrollRegion { ui: region_ui, env, installed, seen, max, shown, bar, lays_out, laid_for: (inset, shows) }
    }

    /// After a scrolling body's content ran: the anchor and glide follow what it placed, and the body takes what the
    /// view shows of it.
    fn scrolled(&mut self, run: &BodyRun<'_>, scroll: Scroll, at: Rect, region: ScrollRegion, bars: &mut Vec2) -> Option<Vec2> {
        let (ctx, id) = (run.ctx, run.id);
        let ScrollRegion { ui: region_ui, installed, seen, max, shown, bar, lays_out, laid_for, .. } = region;
        Env::restore(ctx, installed);
        if lays_out {
            crate::contract::run::close_bar_line(ctx, id);
        }
        let extent = region_ui.min_rect().size();
        if lays_out {
            let placed = crate::runtime::lazy::placed(ctx, id);
            let keeps = !self.scroll_bound && !self.follows_end;
            follow_anchor(ctx, (&placed, scroll, keeps), (extent, seen.size()), (&mut self.offset, self.offset_at_read), &mut self.anchor);
            aim_glide(&mut self.glide, placed.reveal, shown, (extent, seen.size()), scroll);
            if self.glide.is_some_and(|glide| glide.to != Some(Vec2::new(shown.x as f32, shown.y as f32))) {
                repaint::request(ctx);
            }
        }
        if run.real {
            crate::runtime::egui::widgets::note_scroll_region(ctx, region_ui.clip_rect(), region_ui.min_rect().intersect(max));
        }
        self.extent = Some(extent);
        // A measuring run whose content leaves another view than the one it laid the content out for runs again there.
        let leaves = (overflow_inset(scroll, extent, at.size(), bar), view_within(scroll, extent, at, self.strip));
        if !run.real && leaves != laid_for {
            self.view_moves = Some((extent.x, extent.y));
        }
        // Along an axis it scrolls the body took what the view shows; along the other it took its content.
        *bars = overflow_inset(scroll, extent, at.size(), bar);
        // Strips take their height from the view: a body that overflows takes all of its room.
        let strips = strips_of(scroll, extent, at, self.strip).map_or(0.0, |(back, forward)| back.height() + forward.height());
        let took = extent + *bars + Vec2::new(0.0, strips);
        Some(Vec2::new(
            if scroll.horizontal_on() { took.x.min(at.width()) } else { took.x },
            if scroll.vertical_on() { took.y.min(at.height()) } else { took.y },
        ))
    }

    /// After the body: a measuring run decides the label again from the control's reported minimum, and the body's
    /// size counts the label column.
    fn after_body(&mut self, ui: &mut Ui, env: &Env, plan: &mut BodyPlan<'_>) {
        let (props, ctx, real) = (plan.props, &plan.ctx, plan.real);
        if let Some(bracket) = props.bracket {
            let took = layout::take_bracket(ctx, plan.layout_id);
            if let (true, crate::contract::Bracket::Linked, Some(took)) = (real, bracket, took) {
                paint_bracket(ui, env, &took, plan.rtl);
            }
        }
        // A measuring run decided the label before the control reported its minimum width: it decides again
        // from the minimum this run recorded, so the extent it measures is the one the real run lays out.
        if let (false, Some(input)) = (real, plan.label_input.as_mut()) {
            input.control_min = scope::open_control_min(ctx, plan.id);
            // The footer starts under the control column this decision gives, not the one before the control
            // reported its minimum.
            let placed = label::control_after(ui, env, input);
            (plan.body_rect, plan.label_rect, plan.caption) = (placed.control, placed.label, placed.caption);
            plan.label_extra = label_extra_of(plan.regions.body, plan.body_rect, plan.label_rect, plan.rtl);
        }
        if let Some(size) = &mut self.body_size {
            // A labelled row of a bracketed row reports where its control lies: the bracket runs from it.
            if let (Some(row), Some(_)) = (env.bracket, props.label.as_ref()) {
                let body = plan.body_rect;
                let left = if plan.rtl { body.max.x - size.x } else { body.min.x };
                layout::report_bracket_control(ctx, row, Rect::from_min_size(egui::pos2(left, body.min.y), *size), plan.rtl);
            }
            let label_extra = plan.label_extra;
            // A body beside its label in a content-sized column of rows reports how far it reaches past the label
            // column, so that column's container measures its rows at the widest label.
            let beside = |side: &&label::SideLabel| side.layout != label::FieldLayout::After && label_extra.x > 0.0 && label_extra.y <= 0.0;
            if let (false, true, Some(side)) = (real, env.content.0, props.label.as_ref().filter(beside)) {
                let column = if matches!(side.kind, label::LabelKind::Glyph(_)) { layout::GLYPHS } else { layout::LABELS };
                layout::report_beyond(ctx, std::slice::from_ref(&column), label::label_gap(env) + size.x);
            }
            *size += label_extra;
            // A caption above the control counts whole toward the container's width, so a row never narrows the
            // field below it.
            size.x = size.x.max(plan.caption);
            if let (false, true, Some(side)) = (real, env.content.0, props.label.as_ref()) {
                label::report_after_reach(ctx, env, side, (plan.regions.body, plan.body_rect), size.x);
            }
        }
        self.view = Some(plan.body_rect);
    }

    /// The label scope covers the body only: footer controls are not the field's controls. A measuring run registers
    /// no control, so it leaves the facts the real run records alone; it finds the container's first line.
    fn close_label(&mut self, plan: &mut BodyPlan<'_>) {
        let (ctx, id) = (&plan.ctx, plan.id);
        if plan.props.label.is_some() {
            if plan.real {
                scope::finish(ctx, id, None);
            } else {
                scope::discard(ctx, id);
            }
        }
        if !plan.real {
            self.first_line = first_line_found(ctx, plan.first, plan.rect.min.y, plan.body_rect.min.y - plan.ran_top);
        }
    }

    /// A footer the body places: a measuring run of an open body measures it before the body, in a content column of
    /// unbounded room, so it takes its content's size and the slot takes that; the real run takes the size from the
    /// measuring run's record and draws the footer in the slot after the body, in the footer step.
    fn footer_slot_step<C: Container>(&mut self, comp: &mut C, ui: &mut Ui, cx: &mut Cx<'_, C>, plan: &mut BodyPlan<'_>) {
        if plan.real {
            crate::contract::footer_slot::clear(&plan.ctx, plan.id);
            return;
        }
        if !plan.is_open {
            return;
        }
        let at = Rect::from_min_size(plan.regions.body.min, Vec2::INFINITY);
        let mut footer_ui = plan.frame().region(ui, "footer", at);
        let share = plan.share(false);
        let mut bcx = BodyCx::new(
            plan.body_id,
            plan.region_env.view(),
            plan.resolved,
            at,
            &mut *cx.core.memory,
            &mut plan.open,
            plan.gate.as_mut(),
            Some(share),
        );
        let installed = Env::install_for(&footer_ui, plan.footer_env.clone());
        comp.footer(&mut footer_ui, &mut bcx);
        Env::restore(&plan.ctx, installed);
        plan.close = plan.close.or(bcx.close_requested());
        self.footer_size = Some(self.region_size(&footer_ui));
    }

    /// B4: the footer region. A footer the body places is drawn at the slot the body showed, keeping the size it
    /// measured before the body: it stretched to its cell, not to the footer.
    fn footer_step<C: Container>(&mut self, comp: &mut C, ui: &mut Ui, env: &Env, cx: &mut Cx<'_, C>, plan: &mut BodyPlan<'_>) {
        let props = plan.props;
        let in_body = footer_in_body(props);
        let slot = (in_body && plan.real).then(|| crate::contract::footer_slot::take(&plan.ctx, plan.id)).flatten();
        if in_body {
            debug_assert!(
                !(plan.real && plan.is_open) || slot.is_some(),
                "a container whose footer stands in its body (FooterPlace::Body) shows one FooterSlot in it"
            );
            if plan.real {
                self.footer_size = Some(self.measured_footer);
            }
            if slot.is_none() {
                return;
            }
        }
        let (mut footer_rect, offset) = match slot {
            Some(slot) => (slot, 0.0),
            None => self.place_footer(props, &mut plan.regions, plan.inner, plan.body_rect, plan.rtl),
        };
        // The region's size counts the rule's band, the gap and the inset (they are part of what the footer took), so
        // the content starts below the band and the gap and stays the inset away from the sides and the foot.
        let lead = props.footer.and_then(|footer| footer.gap).map_or(0.0, |gap| gap_of(&env.tokens, gap));
        let inset = props.footer.map_or(0.0, |footer| pad_of(&env.tokens, footer.inset));
        let (rule_over, rule_under) = if footer_rule_under(props) { (0.0, self.rules.footer) } else { (self.rules.footer, 0.0) };
        footer_rect.min.y += rule_over + lead;
        footer_rect.max.y = (footer_rect.max.y - inset - rule_under).max(footer_rect.min.y);
        footer_rect.min.x += inset;
        footer_rect.max.x = (footer_rect.max.x - inset).max(footer_rect.min.x);
        let mut footer_ui = plan.frame().region(ui, "footer", footer_rect);
        // A footer in the body shows only where the body's view does.
        if let (Some(_), Some(view)) = (slot, self.view) {
            footer_ui.set_clip_rect(view.intersect(footer_ui.clip_rect()));
        }
        let share = plan.share(false);
        let mut bcx = BodyCx::new(
            plan.body_id,
            plan.region_env.view(),
            plan.resolved,
            footer_rect,
            &mut *cx.core.memory,
            &mut plan.open,
            plan.gate.as_mut(),
            Some(share),
        );
        let installed = Env::install_for(&footer_ui, plan.footer_env.clone());
        comp.footer(&mut footer_ui, &mut bcx);
        Env::restore(&plan.ctx, installed);
        plan.close = plan.close.or(bcx.close_requested());
        let mut took = self.region_size(&footer_ui);
        // The footer's width counts from the container's edge, past the column it starts after.
        if took.x > 0.0 {
            took.x += offset + 2.0 * inset;
        }
        if took.y > 0.0 {
            took.y += lead + inset;
        }
        took.y += self.rules.footer;
        if slot.is_none() {
            self.footer_size = Some(took);
        }
    }

    /// The open and gate state the regions left, the scroll bars, the surface at the final rect and the rules.
    // Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
    #[inline(never)]
    fn finish_content<C: Container>(&mut self, ui: &mut Ui, env: &Env, cx: &Cx<'_, C>, plan: &BodyPlan<'_>) {
        let props = plan.props;
        self.open = plan.open;
        self.gate = plan.gate;
        if plan.close.is_some() {
            self.close = true;
            self.reason = self.reason.or(plan.close);
        }
        if plan.is_open && plan.scroll.is_some() {
            self.paint_scroll_bars(ui, cx, props, plan.rtl, env);
        }
        // The surface at the final rect, then the rules.
        if let Some(kind) = props.surface {
            plan.surface_slot.paint(&env.tokens, kind, plan.rect, env.direction);
        }
        if let Some(fill) = props.header.and_then(|header| header.fill) {
            let style = surface_style(&env.tokens, props.surface.unwrap_or(SurfaceKind::Panel));
            let radius = CornerRadius { sw: 0, se: 0, ..style.radius };
            ui.painter().set(plan.band_slot, Shape::rect_filled(plan.regions.header, radius, band_color(&env.tokens, fill)));
        }
        paint_rules(ui, env, &plan.regions);
        paint_edge_rule(ui, props, env, plan.rect);
    }
}

/// The invisible sizing-pass `Ui` a measuring sweep of container `id` lays its regions out in.
fn measure_ui(ctx: &Context, id: Id, layer: areas::LayerKey, rect: Rect) -> Box<Ui> {
    Box::new(Ui::new(ctx.clone(), sub(id, "measure"), UiBuilder::new().layer_id(layer).max_rect(rect).sizing_pass().invisible()))
}

/// What the room and reuse decision of a measuring run leaves for the run itself. It lives on the heap: the measuring
/// run's frame holds it while the regions' children run.
struct MeasurePlan {
    ctx: Context,
    id: Id,
    /// The inputs the run's record is kept under.
    key: MeasureKey,
    tolerance: f32,
    /// The container's padding on both sides, along each axis.
    pad: Vec2,
    /// The body scrolls vertically: it takes at most the room's height.
    vertical: bool,
    /// The body shares a scrolling view along y.
    scrolled_share: bool,
    /// A record this pass or a kept transcript holds, and whether this pass's run left it.
    reused: Option<(Vec2, bool)>,
}

impl BodyHooks<'_> {
    /// The room of a measuring run, and whether the body shares a scrolling view; `Err` with the extent when no run is
    /// needed.
    fn measure_room<C: Container>(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>) -> Result<bool, Vec2> {
        let props = &self.props;
        // A mount outside the flow takes no room in a parent's sizing pass, so its regions run only when it is
        // measured itself. A pane as large as its content takes its extent in the flow.
        let flows = match &props.mount {
            Mount::Inline => true,
            Mount::Pane(pane) => pane.content_extent,
            _ => false,
        };
        if !flows && at.ui.is_sizing_pass() {
            return Err(self.extent(comp, at.ui, at.at, cx.core.id, at.scx));
        }
        let id = cx.core.id;
        let open = if self.open_given { self.open } else { open_now(comp, &mut *cx.core.memory, id, props) };
        let flows = matches!(at.at, ShowAt::Flow) && self.in_flow(comp);
        self.measure_open_room(at, id, open, flows)
    }

    fn measure_open_room(&mut self, at: &Measuring<'_, '_>, id: Id, open: bool, flows: bool) -> Result<bool, Vec2> {
        let props = Arc::clone(&self.props);
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
        let start = layout::flow_start(at.ui.ctx(), at.env);
        self.after_rule = start == Some(true);
        self.rules = RuleBands::of(&props, at.env, crate::runtime::egui::scale::pixels_per_point(at.ui.ctx()), start);
        self.least_width = least_of(&at.env.tokens, props.least_width);
        self.room = match at.width {
            Some(width) => Vec2::new(width, props.room_height.unwrap_or(f32::INFINITY)),
            None => self.mount_room(&props, at.ui, id, at.at, at.scx.tokens),
        };
        self.given = self.given_axes(&props, at.env, at.at);
        self.given.1 |= self.given_room(&props, at.env, at.at, at.ui.ctx(), id);
        self.fixed_height = self.fixed_height(&props, at.at);
        self.passes = self.passes_width(&props, at.env, at.at);
        self.held = self.held(&props, at.env);
        self.across = self.held && matches!(at.env.column, Some(ColumnSweep::Column(..)));
        // A held container across its column measures at the column's width.
        if let Some(ColumnSweep::Column(_, width)) = at.env.column.filter(|_| self.across && at.width.is_none()) {
            self.room.x = width;
        }
        self.in_column = label::in_column(at.ui);
        self.egui_cell = at.env.egui_cell;
        // A body that fills the main axis of the body it sits in is measured and shown at its share of it. The first
        // sweep of that body's measuring run only counts it: its contents run in the second sweep, at the share.
        let (x, y) = (craft_behavior::Axis::X, craft_behavior::Axis::Y);
        // Along an axis the flow does not bound it fills nothing (`sized`): it measures its content there. In a `Stack`
        // body that scrolls along y it shares what the view shows.
        let bounded = crate::layout::flow_room(at.ui);
        let scrolled_y = !bounded.y.is_finite() && crate::layout::shares_y(at.ui) && crate::layout::view_room(at.ui, y).is_some();
        let (fills_x, fills_y) = (self.given.0 && bounded.x.is_finite(), self.given.1 && (bounded.y.is_finite() || scrolled_y));
        if flows
            && at.ui.is_sizing_pass()
            && ((fills_x && crate::layout::counting_fills(at.ui, x)) || (fills_y && crate::layout::counting_fills(at.ui, y)))
        {
            return Err(Vec2::ZERO);
        }
        if let Some(share) = crate::layout::fill_share(at.ui, x).filter(|_| flows && self.given.0) {
            self.room.x = share;
        }
        let mut scrolled_share = false;
        if let Some(share) = crate::layout::fill_share(at.ui, y).filter(|_| flows && self.given.1) {
            self.room.y = share;
            scrolled_share = scrolled_y;
        }
        // A body that fills both axes its parent gives it takes the room, whatever its contents take: a parent's
        // measuring run does not run them. It measures them in its own real run. A share of a scrolling view is the
        // least of what it takes: its content may be taller.
        if at.ui.is_sizing_pass() && self.given == (true, true) && self.room.is_finite() && !scrolled_share {
            return Err(self.room);
        }
        Ok(scrolled_share)
    }

    /// Whether a record serves the measuring run of container `id` in the room `measure_room` set.
    fn measure_plan(&mut self, at: &Measuring<'_, '_>, id: Id, scrolled_share: bool) -> Box<MeasurePlan> {
        let props = Arc::clone(&self.props);
        let ctx = at.ui.ctx().clone();
        let pad = self.frame_size(&props, at.scx.tokens, top_band(&props, &ctx, id, at.scx.tokens));
        let vertical = scroll_of(&props).is_some_and(|scroll| scroll.vertical_on());
        let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(&ctx));
        // A record a run of this pass left serves as before; a kept one serves only while the inputs it was measured
        // under hold.
        let key = self.measure_key(at.env, at.width);
        // In its column's second sweep a held container measures across the column: its first-sweep record is
        // content-sized.
        let reused = ((self.reuse || at.ui.is_sizing_pass()) && !(self.across && at.ui.is_sizing_pass()))
            .then(|| {
                let found = layout_memory::run_found(&ctx, id);
                let serves = found || layout_memory::kept_serves(&ctx, id, &key);
                serves.then(|| self.reusable(&props, &ctx, id, pad, tolerance)).flatten().map(|took| (took, found))
            })
            .flatten();
        Box::new(MeasurePlan { ctx, id, key, tolerance, pad, vertical, scrolled_share, reused })
    }

    /// A record that serves: it stands for the run, and a kept one is noted under this run's inputs too.
    fn take_reused(&mut self, plan: &MeasurePlan, (took, found): (Vec2, bool)) -> Vec2 {
        if found {
            layout_memory::kept_also_under(&plan.ctx, plan.id, plan.key.clone());
        }
        took
    }

    /// The measuring run of a share of a scrolling view: as given while the content fits it, else at its content.
    fn measure_share<C: Container>(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>, plan: &MeasurePlan) -> Vec2 {
        let fits = self.room.y + plan.tolerance;
        self.given.1 = false;
        let (content, kept) = crate::contract::draft(&plan.ctx, || self.measure_run(comp, at, cx, plan.key.clone(), None), |took| took.y > fits);
        self.given.1 = true;
        if kept { content } else { self.measure_run(comp, at, cx, plan.key.clone(), None) }
    }

    /// The measuring run of a capped body: its content height while that fits the room, else laid out as given the room.
    /// A body already given its room (`given_room`: on its row's settled line, which holds the content the row measured)
    /// fits it as laid out.
    fn measure_capped<C: Container>(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>, plan: &MeasurePlan) -> Vec2 {
        let fits = self.room.y + plan.tolerance;
        let (content, kept) = crate::contract::draft(&plan.ctx, || self.measure_run(comp, at, cx, plan.key.clone(), None), |took| took.y <= fits);
        if kept {
            content
        } else {
            self.given.1 = true;
            self.measured_footer = self.footer_size.unwrap_or(Vec2::ZERO);
            self.measure_run(comp, at, cx, plan.key.clone(), Some(content))
        }
    }

    /// Whether the container is a layer with a footer: its body is laid out before the footer runs, so a body that
    /// fills the layer's room takes the footer's height too, unless the run knows the footer from an earlier one.
    fn footed_layer(&self) -> bool {
        overlay_of(&self.props).is_some() && self.props.footer.is_some() && self.room.y.is_finite()
    }

    /// The measuring run of a layer with a footer (photocraft#16326): while what the regions took fits the room it
    /// stands; taller (a body that fills, such as a scrolling list, took the room the footer needs), the run is taken
    /// back and the body is laid out again beside the footer this run measured, so the layer keeps to its room with
    /// the footer as its last row.
    fn measure_footed_layer<C: Container>(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>, plan: &MeasurePlan) -> Vec2 {
        let fits = self.room.y + plan.tolerance;
        let (took, kept) = crate::contract::draft(&plan.ctx, || self.measure_run(comp, at, cx, plan.key.clone(), None), |took| took.y <= fits);
        if kept {
            return took;
        }
        self.measured_footer = self.footer_size.unwrap_or(Vec2::ZERO);
        self.measure_run(comp, at, cx, plan.key.clone(), None)
    }

    /// The extent a measuring run reports from what its regions `took`.
    fn measured_took(&mut self, took: Vec2, pad: Vec2, vertical: bool) -> Vec2 {
        // A layer hung from an opener or a rect is at least as wide as its overlay asks for, in the measuring run too,
        // so the rows of a content-sized list inside it are placed at that width.
        let took = Vec2::new(overlay_of(&self.props).map_or(took.x, |overlay| took.x.max(crate::contract::overlay::anchor_floor(overlay))), took.y);
        // The least width holds in the measure a parent places it by, as in its own extent (`wanted_width`): a row
        // measuring a side menu among its children gave it only its content's width, so the menu shrank to its widest
        // row's name.
        let took = Vec2::new(took.x.max(self.least_width + pad.x), took.y);
        self.overflow = (took.x - self.room.x).max(0.0);
        // A body wider than the room overflows it, or scrolls along it; either way the container keeps to the room.
        Vec2::new(took.x.min(self.room.x), if vertical { took.y.min(self.room.y) } else { took.y })
    }
}

impl BodyHooks<'_> {
    /// The hooks of the measuring run a real pass runs: they start from what this run's `read` found, as the
    /// measuring run of a sizing parent does.
    fn measuring_hooks(&mut self, fading: bool) -> Box<BodyHooks<'_>> {
        Box::new(BodyHooks {
            save: self.save.clone(),
            layer: self.layer,
            fixed: self.fixed,
            open_bound: self.open_bound,
            scroll_bound: self.scroll_bound,
            last_extent: self.last_extent,
            strip: self.strip,
            row: self.row,
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
        })
    }
}
