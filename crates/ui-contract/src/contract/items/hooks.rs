//! The item step's `ShapeHooks`: the pipeline's calls into a collection's rows.

use super::drag::Drags;
use super::geometry::bar_track;
use super::header::HeaderMemory;
use super::members::Member;
use super::{Held, ITEM_NODE, ItemsHooks, LastLayout, Lead, Placement, Renaming, Rows, SCROLL, Scrolled, Source, key_hash, section_part};
use super::{request, rows};
use crate::contract::ValueOf;
use crate::contract::collection::{
    CollectionEvent, CollectionProps, ColumnLayout, DropAt, Gestures, HoverLead, ItemsOut, PendingDrop, Rename, RowState, ShownRow,
};
use crate::contract::cx::{BodyCx, CxCore, Pointer as _};
use crate::contract::run::{Measuring, ShapeHooks, ShowAt};
use crate::contract::steps::ShapeKind;
use crate::contract::{Bind, Collection, Cx, Internal, ItemCx, ItemOutput as _, PartsCx, Props, Show as _, Sizing};
use crate::primitives::{AutoScrollMark, HiddenContent, ScrollBar, SurfaceSlot};
use crate::runtime::access::{self, Role};
use crate::runtime::egui::{alloc, areas, child, response};
use crate::runtime::env::WheelScope;
use crate::runtime::env::{AppValues, ArenaId, Env, ItemFacts, ThemeScope};
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::{focus, keys, layout_memory, lazy, overlay, wheel};
use crate::style::auto_scroll_mark_look;
use crate::style::{ComponentStyle as _, SurfaceKind, UiTokens, scroll_bar_look, surface_style};
use craft_behavior::{
    Axis, Buttons, CancelReason, EditEvent, Intent, ItemLayout, MiddleButton, MiddleScroll, Mixed, MoveMods, NavMove, PointerButton, PressPhase,
    Proposal, ScrollPos, ScrollSpan, SelectionModel, Sense, WheelRoom, WheelTaker,
};
use craft_keymap::Chord;
use egui::{Context, Id, Pos2, Rect, Ui, UiBuilder, Vec2};
use std::rc::Rc;
use std::sync::Arc;

impl<C: Collection> ShapeHooks<C> for ItemsHooks<'_, '_, C> {
    fn shape(&self) -> ShapeKind {
        ShapeKind::Item
    }

    fn host(&mut self, _: &C, _: crate::contract::run::HostCx<'_>) -> Box<dyn crate::contract::run::Host> {
        Box::new(crate::contract::run::Inline)
    }

    fn arena(&self, _: &C) -> bool {
        true
    }

    /// A collection whose rows run along X is one line; one whose rows run along Y has no line of its own.
    fn first_line(&self, comp: &C, _ctx: &Context, _id: Id, height: f32) -> Option<f32> {
        (comp.items().overflow == Axis::X).then_some(height / 2.0)
    }

    /// A collection that keeps focus on press senses the press without taking focus.
    fn sense(&self, comp: &C, own: Sense) -> Sense {
        if comp.items().focus_on_press { own } else { own.difference(Sense::TAKE_FOCUS).union(Sense::KEEP_FOCUS) }
    }

    fn key_role(&self, comp: &C) -> craft_behavior::KeyRole {
        let props = comp.items();
        craft_behavior::KeyRole::new()
            .reorderable(props.reorder.map(|rule| rule.keys))
            .reorder_axis(props.overflow)
            .pages(props.pages)
            .rename(props.rename)
            .type_ahead(props.type_ahead)
            .digits(props.digit_keys)
            .space(true)
            .multi_select(matches!(props.select, crate::contract::collection::SelectMode::Multi | crate::contract::collection::SelectMode::Cells))
            .moves(true)
            .deletes(props.deletable)
            .owner_arrows(props.item_keys == crate::contract::collection::ItemKeys::Owner)
            .stacked(self.stacked(comp))
            .home_row(self.layout.as_ref().map_or(0, |layout| layout.first_navigable()))
    }

    fn instance_scope(&self, comp: &C) -> Option<crate::runtime::commands::CommandScope> {
        comp.props(Internal::new()).command_scope
    }

    fn theme_scope(&self, comp: &C) -> Option<ThemeScope> {
        comp.props(Internal::new()).theme.clone()
    }

    fn read(&mut self, comp: &mut C, stage: &mut MemoryStage, id: Id, env: &Env) {
        let ctx = stage.ctx().clone();
        let natural = comp.items().natural_height;
        let overflow = comp.items().overflow;
        self.capped = comp.items().max_rows.is_some();
        debug_assert!(
            !shares_scroll(comp.items(), env) || natural || self.capped,
            "a collection owns its scroll: show it outside a body that scrolls along its rows, cap its rows, or give it the natural height (16.3)"
        );
        self.scrolls = scrolls_rows(comp.items(), env);
        self.own_wheel = crate::contract::Component::sense(comp).intersects(Sense::WHEELS);
        self.overflow = overflow;
        self.selection = match comp.selection() {
            Some(bind) => match bind.get() {
                Some(Mixed::Same(held)) => held,
                _ => C::Selection::default(),
            },
            None => stage.session::<Held<C::Selection>>(sub(id, "selection")).0.clone(),
        };
        self.selection_at_read = self.selection.clone();
        self.renaming = stage.session::<Renaming<C::Key>>(sub(id, "rename")).clone();
        // The pass walks the collection's view of the source. The view keeps the row whose rename runs, or is asked
        // for, whether or not it matches, so the row is named in place.
        let kept = self.renaming.key.clone().or_else(|| request::peek_rename::<C::Key>(&ctx, id));
        if let Source::Given(given) = self.source
            && let Some(view) = comp.filtered(given, env, kept.as_ref())
        {
            self.source = Source::Filtered(view);
        }
        self.previous = stage.session::<LastLayout<C::ItemLayout>>(sub(id, "item_layout")).0.take();
        let bound = comp.column_layout().map(|bind| match bind.get() {
            Some(Mixed::Same(layout)) => layout,
            _ => ColumnLayout::default(),
        });
        self.column_layout_at_read = bound.clone();
        let hosted: Vec<_> = comp.items().columns.iter().map(|spec| spec.key.clone()).filter(|key| comp.hosts_header(key)).collect();
        self.header.read(stage, id, comp.items(), bound, hosted);
        self.drags = stage.session::<Drags<C::Key>>(sub(id, "drags")).clone();
        self.lead = stage.session::<Lead<C::Key>>(sub(id, "lead")).clone();
        // A collection in a layer starts from `lead_on_open` at every opening of the layer, not only the first.
        if overlay::drawing_parent(&ctx).is_some_and(|layer| overlay::opening(&ctx, layer)) {
            self.lead = Lead::default();
        }
        let stamp = comp.items().rows_stamp;
        if stamp != self.lead.stamp {
            self.lead = Lead::default();
            self.lead.stamp = stamp;
            // The selection follows the lead: the one the collection holds goes with it.
            if comp.selection().is_none() {
                self.selection = C::Selection::default();
            }
        }
        // A selection can hold items this source does not show; a lead that was never a shown row is not a lead that left.
        if self.lead.key.is_none() {
            self.lead.key = self.selection.lead().filter(|key| self.index_of(key).is_some()).cloned();
        }
        (self.lead_at_read, self.renaming_at_read) = (self.lead.key.clone(), self.renaming.key.clone());
        let scrolled = match comp.scroll() {
            Some(bind) => match bind.get() {
                Some(Mixed::Same(pos)) => Scrolled { y: pos.y, x: pos.x, anchor: None },
                _ => Scrolled::default(),
            },
            None => stage.session::<Scrolled>(sub(id, "scroll")).clone(),
        };
        (self.offset, self.offset_x) = if self.scrolls { (scrolled.y, scrolled.x) } else { (0.0, 0.0) };
        self.anchor = scrolled.anchor.filter(|_| self.scrolls);
        self.anchor_at_read = self.anchor;
        self.offset_at_read = self.offset;
        self.offset_x_at_read = self.offset_x;
        if self.scrolls {
            let quiet = stage.is_peek() || crate::runtime::measure::active(&ctx);
            let taken = if self.own_wheel {
                None
            } else if quiet {
                wheel::peek(&ctx, id)
            } else {
                wheel::take(&ctx, id)
            };
            if let Some(share) = taken {
                self.offset -= f64::from(share.scroll.y);
                self.offset_x -= f64::from(share.scroll.x);
                if share.zoom != 1.0
                    && let Some(about) = share.at
                {
                    self.events.push(CollectionEvent::Zoom { factor: share.zoom, about });
                }
            }
        }
    }

    fn measured_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>) -> Vec2 {
        let id = cx.core.id;
        self.record_largest(comp, at, id);
        self.items_extent(comp, at, id)
    }

    fn pass_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, _style: &C::Style, _shown: Option<Arc<ValueOf<C>>>) -> Vec2 {
        let id = at.resolved.id;
        self.record_largest(comp, at, id);
        self.items_extent(comp, at, id)
    }

    fn pad(&self, comp: &C, tokens: &UiTokens) -> f32 {
        Self::props_pad(comp, tokens)
    }

    fn sized(&self, comp: &C, ui: &Ui, id: Id, size: Vec2) -> Vec2 {
        let sizing = comp.props(Internal::new()).sizing;
        // A collection that measured its width from its content takes that width; the column it sits in holds it.
        let width = if self.content_sized(comp) || self.from_content.0 {
            size.x
        } else {
            crate::layout::body::fill_size(ui, id, Axis::X, size.x, self.room.x, true)
        };
        let height = match (self.scrolls && self.room.y.is_finite(), sizing) {
            (true, Sizing::Fill | Sizing::FillHeight) if !self.from_content.1 && !self.capped => self.room.y,
            (true, Sizing::Fill | Sizing::FillHeight) => size.y.min(self.room.y),
            (true, Sizing::Content | Sizing::FillWidth) => size.y.min(self.room.y),
            (false, _) => size.y,
        };
        Vec2::new(width, height)
    }

    fn geometry(&mut self, _: &C, mcx: &crate::contract::MeasureCx<'_, C>, (_, content): (Rect, Rect), rtl: bool) -> Option<Rc<dyn std::any::Any>> {
        let view = self.below_header(content);
        let clear = self.clear_of_bar(view, scroll_bar_look(mcx.tokens).thickness, rtl);
        let offset = ScrollPos { x: self.offset_x, y: self.offset.clamp(0.0, self.max_offset(view.height())) };
        let mut layout = self.layout.take()?;
        Arc::make_mut(&mut layout).set_view(view.size(), offset);
        let (_, order) = self.built(&layout, view, offset);
        let rects = order.into_iter().map(|at| (self.source.key(at), self.screen(&layout, view, at, offset, rtl))).collect();
        self.layout = Some(layout);
        Some(Rc::new(Rows { rects, offset, view: clear }))
    }

    fn parts(&mut self, comp: &mut C, _: Rect, pcx: &mut PartsCx<'_, C>) {
        self.header.parts(pcx, self.offset_x as f32);
        self.edge_parts(comp, pcx);
        self.notch = comp.items().wheel_notch(pcx.tokens.typography.body.line_height);
        if !self.scrolls {
            return;
        }
        let view = self.below_header(pcx.content_rect);
        let track = bar_track(view, scroll_bar_look(pcx.tokens).thickness, pcx.flow.is_rtl());
        let content = f64::from(self.content.y).max(f64::from(view.height()));
        self.offset = self.offset.clamp(0.0, self.max_offset(view.height()));
        let span = ScrollSpan::new(0.0..content, self.offset..self.offset + f64::from(view.height()));
        pcx.scroll(SCROLL, track, Axis::Y, span);
    }

    fn sensed(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        cx.core.collection_selection = Some(Box::new(self.selection.clone()));
        if self.own_wheel {
            // A collection that takes the wheel itself reads it in its own notches, in place of the rows' scroll.
            if cx.core.resolved.enabled
                && let Some(taker) = WheelTaker::for_sense(cx.core.id, cx.core.rect, crate::contract::Component::sense(comp))
            {
                wheel::register(&cx.core.ctx, taker.notch(self.notch).gated(cx.core.env.wheel() == WheelScope::Gated));
            }
        } else if self.scrolls {
            // The taker covers the whole collection, header row and frame included: the wheel over the header scrolls
            // the rows (Shift sideways).
            let view = self.below_header(cx.core.content_rect);
            let (vertical, horizontal) = (self.overflow == Axis::Y, self.content.x > view.width());
            let max = Vec2::new((self.content.x - view.width()).max(0.0), self.max_offset(view.height()) as f32);
            let offset = Vec2::new(self.offset_x as f32, self.offset as f32);
            wheel::register(
                &cx.core.ctx,
                WheelTaker::new(cx.core.id, cx.core.rect)
                    .notch(self.notch)
                    .vertical((vertical && max.y > 0.0) || horizontal)
                    .horizontal(horizontal)
                    .scrolls(true)
                    .gated(cx.core.env.wheel() == WheelScope::Gated)
                    .room(WheelRoom::new(offset, max - offset))
                    .zoom(comp.zooms()),
            );
        }
    }

    fn updated(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        let rect = self.rows_rect(cx);
        let view = rect.height();
        let rtl = cx.core.flow.is_rtl();
        self.xf = comp.props(Internal::new()).transform.map(|zoom| areas::body_map(rect.min, zoom.scale, zoom.offset));
        let (offset, unpaged) = scroll_rows(cx.core, rect, self.scrolls, self.offset, self.max_offset(view));
        self.offset = offset;
        let swallowed = self.scrolls && self.middle_scroll(comp, cx);
        self.offset = self.offset.clamp(0.0, self.max_offset(view));
        let header_held = self.header_step(cx);
        self.offset_x = self.offset_x.clamp(0.0, f64::from((self.content.x - rect.width()).max(0.0)));
        self.open_lead(comp);
        self.follow_lead_source(comp);
        // A lead set in place (from the selection, `open_lead`, a row that left) has not rested yet: a lead that comes
        // back to a row it rested on before reports the rest again.
        if self.lead.rested != self.lead.key {
            self.lead.rested = None;
        }
        let lead_at = self.lead.key.as_ref().and_then(|key| self.index_of(key));
        let props = comp.items().clone();
        if cx.core.resolved.enabled
            && props.deletable
            && props.delete_target == crate::contract::DeleteTarget::FocusOrHover
            && !cx.core.memory.is_peek()
            && !crate::runtime::measure::active(&cx.core.ctx)
        {
            crate::runtime::keys::register_hover_delete(&cx.core.ctx, cx.core.id);
        }
        let Some(layout) = self.layout.take() else { return };
        let offset = ScrollPos { x: self.offset_x, y: self.offset };
        let at = Placement { rect, offset, rtl };
        // A press lands on the rows the user saw: the offset of the last pass, before this pass's wheel share (8.3).
        let seen = Placement { offset: ScrollPos { x: self.offset_x_at_read, y: self.offset_at_read }, ..at };
        if !swallowed {
            self.edge_step(cx);
            self.press_step(comp, cx, &props, &layout, seen);
            if props.select_under_drag {
                self.drag_select(comp, cx, &layout, seen);
            }
            self.drag_step(comp, cx, &layout, (rect, offset, rtl), &props);
            self.marquee_step(comp, cx, &layout, (rect, offset, rtl), &props);
        }
        if props.hover_lead != HoverLead::Off {
            self.hover_lead(comp, cx, &layout, seen, props.hover_lead);
        }
        // The moves a control in the rows left that do not page them (an arrow a region's fallback collection takes)
        // navigate the rows as the collection's own keys do.
        for (mv, mods) in unpaged {
            cx.core.intents.push(craft_behavior::Intent::Move(mv, mods));
        }
        self.navigate(comp, cx, &layout, view, header_held);
        self.switch_page(comp, cx, &props, &layout);
        let lead_at = self.lead.key.as_ref().and_then(|key| self.index_of(key)).or(lead_at);
        if matches!(props.select, crate::contract::SelectMode::Multi | crate::contract::SelectMode::Cells)
            && !header_held
            && self.renaming.key.is_none()
            && cx.core.intents.untaken(&Intent::Cancel)
            && !self.placed().rows().is_empty()
            && cx.core.intents.take(Intent::Cancel)
        {
            self.change(craft_behavior::SelectionChange::Clear);
        }
        self.intent_actions(comp, cx, &props, lead_at, header_held);
        self.context_key(cx, &layout, at, lead_at);
        self.delete_key(cx, &props, lead_at);
        self.toggle_by_access(cx, &layout, at);
        self.node_by_access(comp, cx, &props, &layout, at);
        self.reorder_and_rename(comp, cx, &props, lead_at);
        self.requested_rename(comp, cx, &props, &layout);
        self.requested_reveal(comp, &layout);
        self.type_ahead(comp, cx, &layout, lead_at);
        self.rest_lead(cx, &layout, seen);
        request::record_lead(&cx.core.ctx, cx.core.id, self.lead.key.as_ref().and_then(|key| self.index_of(key)));
        if let Some(at) = self.page_reveal.take().filter(|&at| layout.shown(at)) {
            cx.core.reveal = Some(self.screen(&layout, rect, at, ScrollPos::default(), rtl));
        }
        self.layout = Some(layout);
    }

    fn apply(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        if self.selection == self.selection_at_read {
            return;
        }
        let id = cx.core.id;
        match comp.selection() {
            Some(mut bind) => {
                let (label, doc_key) = (cx.core.label_arc(), cx.core.resolved.doc_key);
                let before = self.selection_at_read.clone();
                bind.apply(&EditEvent::set(Mixed::Same(before), self.selection.clone(), label, doc_key));
            }
            None => cx.core.memory.session::<Held<C::Selection>>(sub(id, "selection")).0 = self.selection.clone(),
        }
    }

    fn content(&mut self, comp: &mut C, ui: &mut Ui, env: &Env, cx: &mut Cx<'_, C>, values: &AppValues) {
        let Some(mut layout) = self.layout.take() else { return };
        let (id, ctx) = (cx.core.id, cx.core.ctx.clone());
        let rect = self.rows_rect(cx);
        // The surface the props name lies under the header and the rows, as a container's lies under its regions.
        let surface = comp.props(Internal::new()).surface;
        if let Some(kind) = surface {
            SurfaceSlot::reserve(ui, false).paint(&env.tokens, kind, cx.core.rect, env.direction);
        }
        let rtl = env.direction.is_rtl();
        let node = matches!(C::ROLE, Role::Node(_));
        // The rows sit on the surface the collection paints, else on the one the collection sits on.
        let own = surface.or_else(|| cx.style.surface().and_then(|look| look.kind_in(&cx.core.state)));
        let base_env = Self::rows_env(comp, env, id, cx.core.resolved.size, own).derive(|scoped| scoped.extend_values(values));
        let known = self.known(comp, false);
        let known_at_read = self.known(comp, true);
        let declared = self.offset;
        let revealed = self.place_reveal(comp, ui, (id, cx.core.env, &base_env), (&mut layout, &known), rect);
        // The parts (the scroll bar's thumb among them) stand at the offset before the reveal moved it: the next pass
        // declares them where the rows now are, and a memoized view around the rows runs it instead of replaying this.
        if self.offset != declared {
            crate::runtime::egui::repaint::request(&ctx);
        }
        let offset = ScrollPos { x: self.offset_x, y: self.offset };
        Arc::make_mut(&mut layout).set_view(rect.size(), offset);
        if self.scrolls {
            note_scroll_regions(ui, rect, self.content, offset, rtl, self.header.view());
        }
        let sheet = Self::selects_cells(comp);
        let columns: Vec<u64> = match layout.grid() {
            Some(grid) => grid.sections.iter().flat_map(|section| section.columns.clone().map(u64::from)).collect(),
            None => (0..layout.column_count() as u64).collect(),
        };
        let grid = layout.grid().map(|grid| self.grid_screen(&layout, grid, rect, rtl));
        let (shown, order) = self.built(&layout, rect, offset);
        self.visible = shown.first().copied().unwrap_or(0)..shown.last().map_or(0, |last| last + 1);
        let command_scope = comp.props(Internal::new()).command_scope;
        if let Some(scope) = command_scope {
            crate::runtime::commands::register_instance(&ctx, scope, id);
        }
        self.header.paint(ui, cx, comp, env, id, rtl);
        let len = self.source.len();
        let tree = matches!(layout.nav(), craft_behavior::NavShape::Tree);
        focus::registration::record_tree(&ctx, id, tree);
        let focused = focus::is_focused(&ctx, id);
        let lead = self.lead.key.clone();
        let mut lead_child = None;
        let striped = comp.items().striped;
        let mut stripe: Option<(usize, usize)> = None;
        let transform = comp.props(Internal::new()).transform;
        let (parent, clip) = (ui.layer_id(), rect.intersect(ui.clip_rect()));
        let forward = areas::Forward { root: id, sense: response::to_egui(cx.core.root_sense), rect: cx.core.rect.intersect(ui.clip_rect()) };
        let placed = known.placed.clone();
        let tolerance = craft_behavior::layout_memory::measure_tolerance(crate::runtime::egui::scale::pixels_per_point(&ctx));
        // What this pass measured of the rows it builds, before any of them is built: (width and height, widths).
        let fresh: Vec<(Option<Vec2>, Option<Vec2>)> = rows::measured_this_pass(&ctx, id, |fresh| {
            order
                .iter()
                .map(|&at| {
                    let key = self.source.key(at);
                    (fresh(rows::measured_id(id, &key)), fresh(rows::measured_width_id(id, &key)))
                })
                .collect()
        });
        let mut row_groups = Vec::with_capacity(order.len());
        let mut shown_rows: Vec<ShownRow<C::Key>> = Vec::with_capacity(order.len());
        // A section row the component declares as its own `section_part` reports its own node; any other one is the
        // collection's: a heading named by the row's text (a row with no text makes an unnamed heading, which the access
        // lint reports).
        for &at in order.iter().filter(|&&at| self.source.section_stop(at) && layout.shown(at)) {
            if cx.part_if_declared(section_part(&self.source.key(at))).is_none() {
                let row = self.screen(&layout, rect, at, offset, rtl);
                self.report_section_node(&ctx, id, at, row, cx.core.resolved.enabled);
            }
        }
        // The pointer counts as over a row only while the sensing pass puts it on this collection, so a popup or another
        // widget above it takes the hover as it does for the rows' parts.
        let outcome = crate::primitives::sensing::read_outcome(&ctx, ArenaId::new(cx.core.id));
        let hover_at = ctx.pointer_latest_pos().filter(|_| outcome.hover.any());
        let over_at = ctx.pointer_latest_pos().filter(|_| outcome.contains);
        self.hovered = None;
        let menu_open = self.drags.menu.is_some() && crate::contract::overlay::container_open::opened_from(&ctx, cx.core.id);
        // A double click on a column's edge fits the column to its widest cell among the rows built here, in view.
        let fit = self.header.fit.as_ref().map(|(column, _)| column.clone());
        let mut fit_widest = 0.0_f32;
        let aligns = self.header.aligns();
        let place_rows = |ui: &mut Ui| {
            // The items of a collection whose items are members of its run share one `Ui` under the collection's env.
            let members_ui = C::MEMBERS.then(|| {
                let builder = UiBuilder::new().max_rect(rect).layout(child::column_layout(rtl)).scope_id(sub(id, "members"));
                let members_ui = alloc::child(ui, builder);
                let installed = Env::install_for(&members_ui, base_env.clone());
                (members_ui, installed)
            });
            let mut members = Vec::new();
            for (place, at) in order.into_iter().enumerate() {
                if self.source.section_stop(at) {
                    continue;
                }
                let key = self.source.key(at);
                let item_id = sub(id, ("key", key_hash(&key)));
                let row_group = rows::row_group(item_id);
                row_groups.push(row_group);
                let mut row = self.screen(&layout, rect, at, offset, rtl);
                let odd = striped && {
                    let run = self.stripe_run(stripe, at);
                    stripe = Some((at, run));
                    run.is_multiple_of(2) && !self.source.is_section(at)
                };
                let (known_state, mut flags) = self.known_row(at, &known, odd);
                // A row step 5 measured whose measured state this pass's input changed since is measured again.
                let restated = !revealed.contains(&at) && self.known_row(at, &known_at_read, odd).0 != known_state;
                let state = RowState {
                    dragging: self.drags.grabbed.as_ref() == Some(&key),
                    drop_target: matches!(&self.drags.pending, Some(PendingDrop::Drop(DropAt::Onto(over))) if *over == key),
                    hovered: hover_at.is_some_and(|pos| row.contains(pos)),
                    pointer_over: over_at.is_some_and(|pos| row.contains(pos)),
                    pressed: self.pressed.as_ref().filter(|(held, _)| *held == at).map(|(_, pressed)| pressed.clone()),
                    menu_open: self.drags.menu.as_ref().is_some_and(|request| request.key == key) && menu_open,
                    activated_by_key: self.keyed_activation.as_ref() == Some(&key),
                    ..known_state
                };
                // A drag's pointer is over the drop place, not a row the app reads as hovered.
                if state.hovered && self.drags.grabbed.is_none() {
                    self.hovered = Some(key.clone());
                }
                let child = if state.lead { rows::child_focus(&ctx, row_group) } else { None };
                lead_child = lead_child.or(child);
                // A list that never takes key focus (a popup list: the host forwards its keys) shows its lead always.
                flags.set(crate::style::Flags::LEAD, state.lead && (focused || child.is_some() || !comp.items().focus_on_press));
                flags.set(crate::style::Flags::DRAGGING, state.dragging);
                flags.set(crate::style::Flags::DROP_TARGET, state.drop_target);
                let position = self.row_position(at, columns.len(), tree);
                let mut cells = Vec::new();
                layout.cells(at, offset, &mut cells);
                for (_, cell) in &mut cells {
                    *cell = self.to_screen(&layout, rect, *cell, rtl);
                }
                if sheet {
                    rows::put_cell_flags(&ctx, item_id, self.cell_flag_list(&placed, at, &columns, focused));
                }
                let column_keys: Vec<craft_behavior::ColumnKey> = cells.iter().map(|(key, _)| key.clone()).collect();
                let shown_cells = cells.clone();
                let toggle_keys: Vec<craft_behavior::ColumnKey> = self.header.toggle_columns().map(|(key, _)| key).collect();
                let mut icx = ItemCx {
                    key: &key,
                    at,
                    rect: row,
                    row: state.clone(),
                    depth: self.source.depth(at),
                    set: position,
                    row_index: at,
                    row_count: len,
                    column_count: columns.len(),
                    env: cx.core.env,
                    id: sub(item_id, "item"),
                    cells,
                    grid: grid.as_ref(),
                    tip: None,
                    detail: None,
                    openers: Vec::new(),
                    item: item_id,
                    memory: &mut *cx.core.memory,
                };
                icx.memory.claim(icx.id);
                #[cfg(feature = "testing")]
                crate::runtime::measure::count_row_build(&ctx, item_id);
                let item = (self.item)(&key, &mut icx);
                let mut item = comp.dress(item, &state);
                let tip = icx.tip.take();
                let detail = icx.take_detail(layout.is_auto(at));
                let openers = std::mem::take(&mut icx.openers);
                drop(icx);
                item = Props::id(item, item_id);
                // A scrolled row slides under the frozen rows: it never paints into their band.
                let below_frozen = Rect::from_min_max(Pos2::new(rect.min.x, rect.min.y + layout.frozen_band(at)), rect.max);
                let clip = below_frozen.intersect(ui.clip_rect());
                let row_ui = match &members_ui {
                    Some(_) => None,
                    None => {
                        let builder = UiBuilder::new().max_rect(row).layout(child::column_layout(rtl)).scope_id(sub(item_id, "row"));
                        let builder = if node { access::with_parent(builder, id) } else { builder };
                        let mut row_ui = alloc::child(ui, builder);
                        row_ui.set_clip_rect(clip);
                        let installed = Env::install_for(
                            &row_ui,
                            base_env.derive_local(|scoped| {
                                scoped.item = Some(item_id);
                                scoped.row = Some(row_group);
                                scoped.item_facts = Some(Arc::new(ItemFacts {
                                    item: item_id,
                                    flags,
                                    position,
                                    columns: column_keys,
                                    toggles: toggle_keys,
                                    toggle_keys: self.header.toggle_keys(),
                                    aligns: aligns.clone(),
                                    pressed: state.pressed.clone(),
                                    menu_open: state.menu_open,
                                }));
                            }),
                        );
                        Some((row_ui, installed))
                    }
                };
                // The height of the leaf of a row with a detail body; the slot is the leaf and the body.
                let mut leaf_height: Option<f32> = None;
                // A row is a run or a member, so one of the two is there.
                if let Some((measure_ui, _)) = row_ui.as_ref().or(members_ui.as_ref()) {
                    // A row this pass measured is placed by that measure. A row it did not measure (one the reveal's
                    // scroll or a kept lead brings in) is measured here. A restated row is measured again with its new
                    // state; a size other than the one the pass placed by asks for the frame's discard pass, so the
                    // rerun places it from step 5. A row with a detail body measures its leaf here every pass: its
                    // height is the leaf's and the body's, recorded once the body is shown.
                    let (measured_size, measured_widths) = fresh[place];
                    if layout.auto_widths() && (measured_widths.is_none() || restated) {
                        let widths = Vec2::new(item.measure_in(measure_ui, Proposal::Ideal).x, item.measure_in(measure_ui, Proposal::Min).x);
                        rows::record_measure(&ctx, id, rows::measured_width_id(id, &key), widths);
                        if measured_widths.is_some_and(|before| (before - widths).abs().max_elem() > tolerance) {
                            layout_memory::ask_discard(&ctx);
                        }
                        Arc::make_mut(&mut layout).set_width(at, widths.x, widths.y);
                        row = self.screen(&layout, rect, at, offset, rtl);
                    }
                    let measured_size = measured_size.filter(|size| (size.x - row.width()).abs() <= tolerance);
                    if layout.is_auto(at) && (detail.is_some() || measured_size.is_none() || restated) {
                        let size = item.measure_in(measure_ui, Proposal::Width(row.width()));
                        let measured = size.y;
                        #[cfg(feature = "testing")]
                        crate::runtime::measure::count_row_measure(&ctx, rows::measured_id(id, &key));
                        if detail.is_some() {
                            leaf_height = Some(measured);
                        } else {
                            rows::record_measure(&ctx, id, rows::measured_id(id, &key), Vec2::new(row.width(), measured));
                            if measured_size.is_some_and(|before| (before.y - measured).abs() > tolerance) {
                                layout_memory::ask_discard(&ctx);
                            }
                            let fit = rows::fits(&item).then_some(size.x);
                            let refit = rows::keep_fit(&ctx, id, key_hash(&key), fit);
                            if measured != row.height() || refit {
                                let held = Arc::make_mut(&mut layout);
                                held.set_height(at, measured);
                                held.set_fit(at, fit);
                                row = self.screen(&layout, rect, at, offset, rtl);
                            }
                        }
                    }
                    if let Some(column) = &fit {
                        fit_widest = fit_widest.max(cell_width(&ctx, &mut item, measure_ui, column).unwrap_or_default());
                    }
                }
                let leaf = leaf_height.map_or(row, |height| Rect::from_min_size(row.min, Vec2::new(row.width(), height)));
                // A member has no node of its own: it reports as a sub-node of the collection's.
                let row_node = if C::MEMBERS { super::members::item_node(id, &key) } else { item_id };
                shown_rows.push(ShownRow::new(key.clone(), row_node, leaf, shown_cells.clone()).with_hover(state.hovered));
                let Some((mut row_ui, installed)) = row_ui else {
                    debug_assert!(tip.is_none() && detail.is_none(), "a member of a collection's run takes no rich tooltip and no detail body");
                    let shown = shown_rows.len() - 1;
                    members.push(Member { at, key, item, row, clip, flags, position, renaming: state.renaming, openers, shown });
                    continue;
                };
                let out = match tip {
                    Some(tip) => item.show_at_with_tip(&mut row_ui, leaf, tip),
                    None => item.show_at(&mut row_ui, leaf),
                };
                if let Some(body) = detail {
                    // The body's stops follow the leaf's: it shows inside the item's scope, under the leaf.
                    let below = Rect::from_min_size(Pos2::new(row.min.x, leaf.max.y), Vec2::new(row.width(), f32::INFINITY));
                    let builder = UiBuilder::new().max_rect(below).layout(child::column_layout(rtl)).scope_id(sub(item_id, "detail"));
                    let mut body_ui = alloc::child(&mut row_ui, builder);
                    body_ui.set_clip_rect(clip);
                    body(&mut body_ui);
                    let total = leaf.height() + body_ui.min_rect().height();
                    rows::record_measure(&ctx, id, rows::measured_id(id, &key), Vec2::new(row.width(), total));
                    if (total - row.height()).abs() > tolerance {
                        Arc::make_mut(&mut layout).set_height(at, total);
                        // The rows after this one were placed by the old height: the frame's discard pass places them by the new one.
                        layout_memory::ask_discard(&ctx);
                    }
                }
                // After the row is shown: its toggle cells have recorded their names by then.
                self.report_toggle_cells(&ctx, (id, item_id), (at, &key), (shown_cells.as_slice(), cx.core.resolved.enabled), &cx.core.env);
                let label = out.label_rect().unwrap_or(leaf);
                self.slow_click_label(&key, label);
                crate::runtime::keys::redirect_node(&ctx, item_id, id, ITEM_NODE.at(key_hash(&key)));
                if let Some(row) = shown_rows.last_mut() {
                    row.openers = self.item_openers(&row_ui, &ctx, id, cx.core.resolved.enabled, openers);
                }
                self.report_item_output(&ctx, id, &key, out);
                if state.renaming {
                    self.rename_row(comp, &mut row_ui, (&ctx, id, rtl), (at, &key), label);
                }
                Env::restore(&ctx, installed);
            }
            if let Some((mut members_ui, installed)) = members_ui {
                let base = (&ctx, id, rtl);
                self.show_members(comp, &mut members_ui, cx, (base, &base_env), members, &mut shown_rows);
                Env::restore(&ctx, installed);
            }
        };
        match transform {
            Some(transform) => {
                let layer = areas::body_layer(parent, id);
                let tf = areas::body_transform(rect.min, transform.scale, transform.offset);
                crate::primitives::sensing::set_layer(&ctx, ArenaId::new(id), layer);
                let body_layer = areas::BodyLayer { parent, layer, transform: tf, at: rect.min, clip, forward: Some(forward) };
                areas::show_body_layer(&ctx, body_layer, place_rows);
            }
            None => place_rows(ui),
        }
        if self.header.fit.is_some() {
            self.header.apply_fit(ui, &cx.core.env, fit_widest);
            self.take_column_events();
        }
        self.paint_marquee(ui, env, &layout, (rect, offset, rtl));
        focus::register_row_groups(&ctx, id, rtl, row_groups);
        self.shown = shown_rows;
        cx.core.grid = crate::contract::GridPlace::grid(len, columns.len());
        let size = layout.content_size();
        self.content = Vec2::new(size.x as f32, size.y as f32);
        if self.scrolls && !ui.is_sizing_pass() && !crate::runtime::measure::active(&ctx) {
            self.follow_anchor(comp, (&ctx, id), &layout, (rect, rtl));
        }
        // The renamed item left the source (removed, renamed or hidden by the app, or folded away): its editor is gone, so the
        // rename ends rather than waiting for the item to return and open the old draft again.
        if let Some(key) = self.renaming.key.clone()
            && !self.ended_rename
            && !ui.is_sizing_pass()
            && !crate::runtime::measure::active(&ctx)
            && self.index_of(&key).is_none()
        {
            let before = Rename { key, name: self.renaming.draft.clone() };
            self.events.push(CollectionEvent::Rename(EditEvent::cancel(self.renaming.gesture, CancelReason::Removed, Some(Mixed::Same(before)))));
            self.ended_rename = true;
        }
        if self.ended_rename {
            self.renaming = Renaming::default();
            if self.refocus {
                focus::request_now(&ctx, focus::FocusTarget::Id(id));
            }
            self.refocus = false;
        }
        if (focused || lead_child.is_some())
            && let Some(at) = lead.as_ref().and_then(|key| self.index_of(key))
        {
            let key = self.source.key(at);
            if self.source.section_stop(at) {
                access::set_focus(&ctx, crate::runtime::access::sub_node_id(id, section_part(&key)));
            } else {
                if C::MEMBERS {
                    access::set_focus(&ctx, super::members::item_node(id, &key));
                } else {
                    let item_id = sub(id, ("key", key_hash(&key)));
                    let cell = (sheet && self.selection.lead_column().is_some()).then_some(self.lead.column);
                    access::set_focus(&ctx, crate::runtime::access::items::focus_node(item_id, lead_child, cell));
                }
            }
        }
        if self.source.is_empty() {
            let mut open = true;
            let builder = UiBuilder::new().max_rect(rect).layout(child::column_layout(rtl)).scope_id(sub(id, "empty"));
            let mut empty_ui = alloc::child(ui, builder);
            let installed = Env::install_for(&empty_ui, env.derive(|scoped| scoped.extend_values(values)));
            let (resolved, view) = (cx.core.resolved, cx.core.env);
            let mut bcx = BodyCx::new(sub(id, "body"), view, resolved, rect, &mut *cx.core.memory, &mut open, None, None);
            comp.empty(&mut empty_ui, &mut bcx);
            Env::restore(&ctx, installed);
        }
        if self.scrolls {
            let look = scroll_bar_look(&env.tokens);
            // The rows blend into the surface they sit on.
            let background = surface_style(&env.tokens, SurfaceKind::under(own, env.surface)).fill;
            let hidden = HiddenContent::new(self.content, rect.size(), Vec2::new(self.offset_x as f32, self.offset as f32), rtl);
            ui.painter().extend(ScrollBar::fades(rect, hidden, background));
            if self.shows_bar(rect) {
                let track = bar_track(rect, look.thickness, rtl);
                let content = f64::from(self.content.y);
                let span = ScrollSpan::new(0.0..content, self.offset..self.offset + f64::from(rect.height()));
                let state = cx.core.scroll_bar_state(SCROLL);
                ui.painter().extend(ScrollBar::shapes(&look, track, Axis::Y, rtl, &span, &state));
            }
            middle_mark(ui, env, cx.core.memory.session::<MiddleScroll>(sub(id, "middle")), rect);
        }
        self.layout = Some(layout);
    }

    fn settle(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        let id = cx.core.id;
        // A collection's focus move reveals through its lead (`page_reveal`), never the collection's whole rect. A
        // header cell's reveal moves the sideways offset the header and the rows share, to show the whole cell.
        if let Some(Some(part)) = focus::take_reveal_of(&cx.core.ctx, id)
            && self.scrolls
            && let Some(by) = self.header.reveal_x(part, cx.core.flow.is_rtl())
        {
            let max = f64::from((self.content.x - self.rows_rect(cx).width()).max(0.0));
            self.offset_x = (self.offset_x + by).clamp(0.0, max);
            crate::runtime::egui::repaint::request(&cx.core.ctx);
        }
        if self.scrolls && (self.offset != self.offset_at_read || self.offset_x != self.offset_x_at_read || self.anchor != self.anchor_at_read) {
            match comp.scroll() {
                Some(mut bind) => {
                    let (label, doc_key) = (cx.core.label_arc(), cx.core.resolved.doc_key);
                    let (before, after) =
                        (ScrollPos { x: self.offset_x_at_read, y: self.offset_at_read }, ScrollPos { x: self.offset_x, y: self.offset });
                    bind.apply(&EditEvent::set(Mixed::Same(before), after, label, doc_key));
                }
                None => {
                    let scrolled = cx.core.memory.session::<Scrolled>(sub(id, "scroll"));
                    (scrolled.y, scrolled.x, scrolled.anchor) = (self.offset, self.offset_x, self.anchor);
                }
            }
        }
        *cx.core.memory.session::<Lead<C::Key>>(sub(id, "lead")) = self.lead.clone();
        cx.core.memory.session::<LastLayout<C::ItemLayout>>(sub(id, "item_layout")).0 = self.layout.clone();
        let requests = std::mem::take(&mut cx.core.column_requests);
        if !requests.is_empty() {
            self.header.apply_requests(&cx.core.ctx, requests, cx.core.flow.is_rtl());
            self.take_column_events();
        }
        if let Some(before) = self.column_layout_at_read.take()
            && *self.header.layout() != before
            && let Some(mut bind) = comp.column_layout()
        {
            let (label, doc_key) = (cx.core.label_arc(), cx.core.resolved.doc_key);
            bind.apply(&EditEvent::set(Mixed::Same(before), self.header.layout().clone(), label, doc_key));
        }
        *cx.core.memory.session::<HeaderMemory>(sub(id, "columns")) = self.header.persist();
        // The menu a request of this pass asks for opens after the collection shows, so it counts as open in this
        // pass; an older request goes once no menu hung from the collection is open.
        let asked = self.events.iter().any(|event| matches!(event, CollectionEvent::ContextMenu(_)));
        if !asked && !crate::contract::overlay::container_open::opened_from(&cx.core.ctx, id) {
            self.drags.menu = None;
        }
        *cx.core.memory.session::<Drags<C::Key>>(sub(id, "drags")) = self.drags.clone();
        self.header.report(&cx.core.ctx, id, cx);
        *cx.core.memory.session::<Renaming<C::Key>>(sub(id, "rename")) = self.renaming.clone();
        cx.core.collection_selection = Some(Box::new(self.selection.clone()));
        cx.core.collection_events = Some(Box::new(ItemsOut {
            events: std::mem::take(&mut self.events),
            shown: std::mem::take(&mut self.shown),
            activation_clicks: self.activation_clicks,
            pending_drop: self.drags.pending.clone(),
            hovered: self.hovered.take(),
            menu: self.drags.menu.clone().filter(|request| self.index_of(&request.key).is_some()),
            auto_scrolling: cx.core.memory.session::<MiddleScroll>(sub(id, "middle")).active(),
        }));
        cx.core.visible_rows = self.visible.clone();
        cx.core.gestures = Gestures::of(comp.items());
    }
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// After the rows of the real run were built in the rows rect `rect`, at the offset the run read: the anchor rule
    /// (`lazy::follow`) moves the offset by how far a change of the heights of rows before the anchor's row moved it,
    /// and the pass runs again. A bound offset is the app's and keeps no anchor.
    ///
    /// A zoom the collection takes in this pass names its own anchor once the anchor an earlier pass named is in place:
    /// the row under `Collection::zoom_anchor`'s point, at its place in this pass's layout, the one the zoom changes
    /// from. `settle` keeps it with the offset and the next pass, laid out at the new zoom, puts that row back where it
    /// is now. While a zoom runs over several passes, the row that pass shifts back keeps its anchor.
    fn follow_anchor(&mut self, comp: &mut C, (ctx, id): (&Context, Id), layout: &C::ItemLayout, (rect, rtl): (Rect, bool)) {
        if comp.scroll().is_some() {
            self.anchor = None;
            return;
        }
        let view = rect.size();
        let offset = ScrollPos { x: self.offset_x, y: self.offset };
        let source = &*self.source;
        let named = self.anchor.map(|anchor| anchor.child);
        let mut placed = lazy::Placed::named(named, None, (false, true));
        // Frozen rows and section rows can stay put while the rows scroll: only a row that scrolls names the position.
        let first = layout.first_navigable();
        let scrolling = |at: usize| at >= first && layout.shown(at) && !source.section_stop(at);
        // The zoom event the read step reported in this pass, the only place the pass keeps its point.
        let wheel = self.events.iter().rev().find_map(|event| match event {
            CollectionEvent::Zoom { about, .. } => Some(*about),
            _ => None,
        });
        let zoom_at = comp.zoom_anchor(wheel);
        for at in self.visible.clone().filter(|&at| scrolling(at)) {
            let row = layout.rect(at, offset);
            let item = sub(id, ("key", key_hash(&self.source.key(at))));
            placed.note(item, Vec2::new(0.0, row.min.y), row.size());
        }
        // A zoom reflows the rows, so the anchor's row may lie outside the rows this pass built: the layout still
        // knows where it is.
        if let (Some(child), None) = (named, placed.anchor) {
            let away = (0..source.len()).filter(|&at| scrolling(at)).find(|&at| sub(id, ("key", key_hash(&source.key(at)))) == child);
            if let Some(at) = away {
                let row = layout.rect(at, offset);
                placed.note(child, Vec2::new(0.0, row.min.y), row.size());
            }
        }
        let moved = Vec2::new(0.0, (self.offset - self.offset_at_read) as f32);
        let room = (Vec2::new(0.0, self.offset as f32), Vec2::new(0.0, (self.max_offset(view.y) - self.offset) as f32));
        // The anchor an earlier pass named is followed first: egui spreads one wheel notch's zoom over the passes of
        // several frames, and each of them lays the rows out at the zoom the pass before it stepped to.
        if let Some(shift) = lazy::follow(&mut self.anchor, &placed, moved, view, room, (false, true)) {
            self.offset += f64::from(shift.y);
            log::debug!(target: crate::runtime::egui::sizing::RERUN_LOG, "collection {id:?} keeps its anchor row in place by {shift:?}");
            crate::runtime::egui::sizing::request_discard(ctx, crate::runtime::egui::sizing::Rerun::Layout);
            return;
        }
        // With the earlier anchor in place, a zoom in this pass names the row under its point, at its place in this
        // pass's layout.
        if let Some(point) = zoom_at {
            let point = self.xf.map_or(point, |map| map.to_layer(point));
            let under = self.visible.clone().filter(|&at| scrolling(at)).find(|&at| self.screen(layout, rect, at, offset, rtl).contains(point));
            if let Some(at) = under {
                let child = sub(id, ("key", key_hash(&source.key(at))));
                self.anchor = Some(lazy::Anchor { child, at: Vec2::new(0.0, layout.rect(at, offset).min.y) });
            }
        }
    }
}

/// The latch's mark over the rows in `rect`, at the place the middle press latched; nothing while no latch is held.
fn middle_mark(ui: &Ui, env: &Env, scroll: &MiddleScroll, rect: Rect) {
    if let Some(anchor) = scroll.anchor() {
        let mark = AutoScrollMark::shapes(&auto_scroll_mark_look(&env.tokens), anchor);
        ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).extend(mark);
    }
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// Middle-button auto-scroll (`MiddleButton::AutoScroll`) on the rows' offset, once per pass of the real run. The
    /// collection's own middle press latches it (the arena gave the press to the collection, so a layer over the rows
    /// keeps its own); while latched, any middle press ends it, as do Escape, the window losing focus and a wheel
    /// notch. The press itself picks no row (`press_step`). A rerun pass of a frame carries none of the frame's
    /// presses, releases, keys or wheel (`runtime/pointer.rs`, `PointerFrame`) and its elapsed time is 0
    /// (`frame::facts`), so it steps no time and counts no held pass: the offset moves once per frame. The latch, the
    /// gesture state of one press, lives in the collection's session memory beside `Drags`.
    fn middle_scroll(&mut self, comp: &C, cx: &mut Cx<'_, C>) -> bool {
        let props = comp.items();
        if props.middle_button != MiddleButton::AutoScroll {
            return false;
        }
        let (ctx, id) = (cx.core.ctx.clone(), cx.core.id);
        if cx.core.memory.is_peek() || crate::runtime::measure::active(&ctx) {
            return false;
        }
        let frame = crate::runtime::pointer::frame(&ctx);
        let own_press = match cx.core.press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Middle => Some(start.origin),
            _ => None,
        };
        // A middle press moves no focus, so a claim of the collection's own would miss Escape: the latch captures it
        // as a mode does (`keys::capture`), whichever component has focus.
        let escape = Chord::plain(craft_keymap::Key::Escape);
        let escaped = keys::take_claimed(&ctx, id, escape);
        let blurred = crate::runtime::pointer::with_pass_pointer(&ctx, |pointer| {
            pointer.focus_lost || pointer.events.iter().any(|event| matches!(event, craft_behavior::PointerEvent::Gone))
        });
        let wheeled = frame.scroll != Vec2::ZERO || frame.zoom != 1.0;
        let scroll = cx.core.memory.session::<MiddleScroll>(sub(id, "middle"));
        let swallowed = scroll.swallows(cx.core.press.phase);
        if scroll.active() && (escaped || blurred || wheeled) {
            scroll.cancel();
        } else if scroll.active() && !swallowed {
            if let Some(press) = frame.press.filter(|press| press.button == PointerButton::Middle) {
                scroll.press(press.origin, own_press.is_some());
            }
        } else if !swallowed && let Some(at) = own_press {
            scroll.press(at, true);
        }
        if scroll.active() && frame.release == Some(PointerButton::Middle) {
            scroll.release();
        }
        if !scroll.active() {
            return swallowed;
        }
        let dt = crate::runtime::frame::facts(&ctx).elapsed.clamp(0.0, 0.1);
        let held = frame.buttons.contains(Buttons::MIDDLE);
        let by = frame.pos.map_or(Vec2::ZERO, |pointer| scroll.step(pointer, held, dt, props.invert_middle));
        // `step` is negative with the pointer below the latch, as the wheel's share is: the rows scroll toward the end.
        self.offset -= f64::from(by.y);
        keys::capture(&ctx, id, escape);
        crate::runtime::egui::repaint::request(&ctx);
        swallowed
    }
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// Goes on renaming after a Shift+Enter commit: the field holds the applied name `name`, selected whole when focus
    /// is requested again, and the next commit is its own undo step.
    fn continue_rename(&mut self, ctx: &Context, name: String) {
        self.renaming.draft = name;
        self.renaming.focused = false;
        self.renaming.proposed = false;
        self.renaming.gesture = crate::runtime::edit::next_gesture_id(ctx);
        self.refocus = false;
    }

    /// The rename editor of the row `(at, key)` being renamed, over its label rect `label`, and the end of the rename
    /// it reports.
    pub(super) fn rename_row(
        &mut self,
        comp: &mut C,
        row_ui: &mut Ui,
        (ctx, id, rtl): (&Context, Id, bool),
        (at, key): (usize, &C::Key),
        label: Rect,
    ) {
        let len = self.source.len();
        let mut draft = std::mem::take(&mut self.renaming.draft);
        let name = self.source.text(at).map(|text| text.into_owned()).unwrap_or_default();
        let editor = comp.rename_editor(row_ui, Bind::bound(&mut draft), label, &name);
        self.renaming.draft = draft;
        if let Some(editor) = editor {
            let gesture = self.renaming.gesture;
            let shifted = editor.submitted && editor.submit_mods.shift;
            let before = Mixed::Same(Rename { key: key.clone(), name: name.clone() });
            let (end, escaped) = rename_input(ctx, &editor, rtl, &mut self.renaming.focused, &mut self.refocus);
            // A pass that only measures keeps no focus, so it neither opens nor ends the hold.
            let real = !row_ui.is_sizing_pass() && !crate::runtime::measure::active(ctx);
            let has_focus = focus::is_focused(ctx, editor.response.id);
            // The field lost the focus with nothing typed (a click elsewhere): the text field reports no event for an
            // unedited name, so the rename ends here, as it does by Tab and as a typed name does by committing.
            let blurred = real && self.renaming.held && !has_focus;
            self.renaming.held |= real && has_focus;
            match end {
                None if !editor.events.is_empty() => {}
                None if escaped => {
                    self.events.push(CollectionEvent::Rename(EditEvent::cancel(gesture, CancelReason::Escape, Some(before))));
                    self.ended_rename = true;
                    self.refocus = true;
                }
                Some(EditEvent::Commit { after, .. }) => {
                    let after = Rename { key: key.clone(), name: after };
                    let applied = after.name.clone();
                    self.events.push(CollectionEvent::Rename(EditEvent::commit(gesture, before, after)));
                    if shifted {
                        self.continue_rename(ctx, applied);
                    } else {
                        self.ended_rename = true;
                    }
                }
                Some(EditEvent::Cancel { reason, .. }) => {
                    self.events.push(CollectionEvent::Rename(EditEvent::cancel(gesture, reason, Some(before))));
                    self.ended_rename = true;
                }
                // Enter on a proposed draft nobody typed in commits it. Enter on the row's own name opens no edit run: the
                // rename ends with no event and no undo step, as a text field's empty commit does.
                None if editor.submitted => {
                    let applied = self.renaming.draft.clone();
                    if self.renaming.proposed {
                        let after = Rename { key: key.clone(), name: applied.clone() };
                        self.events.push(CollectionEvent::Rename(EditEvent::commit(gesture, before, after)));
                    }
                    if shifted {
                        self.continue_rename(ctx, applied);
                    } else {
                        self.ended_rename = true;
                        self.refocus = true;
                    }
                }
                None if editor.left_by_tab.is_some() => {
                    self.events.push(CollectionEvent::Rename(EditEvent::cancel(gesture, CancelReason::Blur, Some(before))));
                    self.ended_rename = true;
                }
                None if blurred => {
                    self.events.push(CollectionEvent::Rename(EditEvent::cancel(gesture, CancelReason::Blur, Some(before))));
                    self.ended_rename = true;
                }
                _ => {}
            }
            if let Some(forward) = editor.left_by_tab.filter(|_| self.ended_rename) {
                // Tab continues the rename on the next renamable row; past the last one focus returns to the collection.
                let renamable = |row: &usize| !self.source.section_stop(*row) && comp.can_rename(&self.source.key(*row));
                let next = if forward { (at + 1..len).find(renamable) } else { (0..at).rev().find(renamable) };
                self.refocus = next.is_none();
                if let Some(next) = next {
                    request::request_rename(ctx, id, &self.source.key(next));
                }
            }
        }
    }
}

/// The width `item` gives its cell in `column` at its ideal width in `ui`; `None` when it has no cell there.
fn cell_width(ctx: &Context, item: &mut impl crate::contract::Show, ui: &Ui, column: &craft_behavior::ColumnKey) -> Option<f32> {
    let (_, cells) = crate::runtime::measure::cell_widths(ctx, || item.measure_in(ui, Proposal::Ideal));
    cells.into_iter().find(|(held, _)| held == column).map(|(_, width)| width)
}

/// Apply the collection's wheel displacement, the page and end moves a descendant left untaken and any reveal left by
/// a descendant. Returns the offset and the moves addressed to the collection that did not page its rows, in order.
fn scroll_rows(core: &mut CxCore<'_>, rect: Rect, scrolls: bool, mut offset: f64, maximum: f64) -> (f64, Vec<(NavMove, MoveMods)>) {
    if let Some(range) = core.scroll_moved(SCROLL) {
        offset = range.start;
    }
    let (ctx, id) = (core.ctx.clone(), core.id);
    let quiet = core.memory.is_peek() || crate::runtime::measure::active(&ctx);
    // The moves are taken once, in the real run (peeked while measuring). A control in a row left a page or end move
    // untaken: the collection, its rows' scroll owner, pages the rows.
    let view = f64::from(rect.height());
    // A region's fallback collection (F-116) takes every move its region passed it as its own keys, page and end
    // included (L-319); the page and end moves a control in the rows left page the rows.
    let (scrolled, mut unpaged) = crate::contract::steps::scroll_and_fallback_moves(&ctx, id, quiet);
    for (mv, mods) in scrolled {
        let span = ScrollSpan::new(0.0..maximum + view, offset..offset + view);
        match craft_behavior::scroll::jump(&span, mv).filter(|_| scrolls) {
            Some(range) => offset = range.start,
            None => unpaged.push((mv, mods)),
        }
    }
    if scrolls {
        // A descendant's reveal, left for this pass: taken against this pass's rows (peeked while measuring).
        if let Some(asked) = if quiet { crate::contract::peek_reveal(&ctx, id) } else { crate::contract::take_reveal(&ctx, id) } {
            // The rows scroll along y only: the reveal moves them along it and passes the rest on.
            let along = Rect::from_min_max(Pos2::new(f32::NEG_INFINITY, rect.min.y), Pos2::new(f32::INFINITY, rect.max.y));
            let (by, shown) = crate::contract::steps::reveal_move(asked, along);
            offset += f64::from(by.y);
            if !quiet {
                crate::contract::Focus::reveal(core, shown);
            }
        }
    }
    (offset.clamp(0.0, maximum), unpaged)
}

/// Record the scrolled rows and their header at the same horizontal offset.
fn note_scroll_regions(ui: &Ui, rect: Rect, content_size: Vec2, offset: ScrollPos, rtl: bool, header: Option<Rect>) {
    let size = content_size.max(rect.size());
    let left = if rtl { rect.max.x + offset.x as f32 - size.x } else { rect.min.x - offset.x as f32 };
    let content = Rect::from_min_size(Pos2::new(left, rect.min.y - offset.y as f32), size);
    crate::runtime::egui::widgets::note_scroll_region(ui.ctx(), rect.intersect(ui.clip_rect()), content);
    // The header row scrolls sideways with the rows.
    if let Some(header) = header {
        let row = Rect::from_x_y_ranges(content.x_range(), header.y_range());
        crate::runtime::egui::widgets::note_scroll_region(ui.ctx(), header.intersect(ui.clip_rect()), row);
    }
}

/// Focus and Escape belong to the editor until the rename ends, independent of the row's key type.
fn rename_input(
    ctx: &Context,
    editor: &crate::contract::EditResponse<String>,
    rtl: bool,
    focused: &mut bool,
    refocus: &mut bool,
) -> (Option<EditEvent<String>>, bool) {
    let id = editor.response.id;
    if !*focused {
        focus::request_now(ctx, focus::FocusTarget::Id(id));
        *focused = true;
    }
    let end = editor.events.iter().rev().find(|event| matches!(event, EditEvent::Commit { .. } | EditEvent::Cancel { .. })).cloned();
    if end.is_some() {
        *refocus = focus::is_focused(ctx, id);
    }
    keys::hold_escape(ctx, id);
    let escaped = editor.events.is_empty() && focus::is_focused(ctx, id) && {
        let mut intents = keys::intents_of(ctx, Some(id), rtl);
        let left = intents.take(Intent::Cancel);
        keys::settle(ctx, Some(id), &intents);
        left
    };
    (end, escaped)
}

/// The height of the first `rows` rows of `layout` over `len` items: the rows it lays out, and the row height `row` for
/// each one it does not have.
pub(super) fn rows_height(layout: &impl ItemLayout, len: usize, rows: u16, row: f32) -> f32 {
    let rows = usize::from(rows);
    let laid = rows.min(len);
    let top = if len == 0 { 0.0 } else { layout.rect(0, ScrollPos::default()).min.y };
    let built = if laid == 0 { 0.0 } else { layout.rect(laid - 1, ScrollPos::default()).max.y - top };
    built + (rows - laid) as f32 * row
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// Step 5 in both runs: what the rows take in the room `measuring` gives. The auto rows in view are measured
    /// first (`measure_ahead`), so the collection takes its size, and places its rows, by this pass's measure.
    fn items_extent(&mut self, comp: &C, measuring: &Measuring<'_, '_>, id: Id) -> Vec2 {
        let (ui, scx) = (measuring.ui, measuring.scx);
        let room = match measuring.at {
            ShowAt::Rect(rect) => rect.size(),
            ShowAt::Flow => crate::layout::flow_room(ui),
        };
        let env = Env::of(ui);
        // The layout reads the component's own motion memory (`StyleCx::exiting`): an item leaving its source keeps
        // its room until its exit ends.
        let scx = &scx.measured_by(ui).owned_by(id, env.motion);
        self.room = room;
        self.indent_step = scx.control().icon;
        let room = Self::inner_room(comp, scx.tokens, room);
        // Where the enclosing body does not give the width, the collection takes its width from its content, whether
        // it fills or fills the width, as a container does. Held in a content-sized column it measures so, and shows
        // across the width the column settles on.
        let held = env.content_column || crate::layout::in_grid_content_column(ui);
        self.from_content = (!env.gives_width() && (crate::runtime::measure::active(ui.ctx()) || !held), env.content.1);
        self.header.measure(scx);
        self.header.read_content(ui.ctx(), id);
        let bounded = room.x.is_finite();
        let item = comp.measures_items().then(|| layout_memory::measured(ui.ctx(), sub(id, "largest")).map(|measured| measured.took)).flatten();
        // Under a body that does not give the width, the measuring run lays the rows out at the width the collection
        // asks for, so the height it reports is the one it has at that width (a grid's rows at its columns).
        let from_content = self.from_content.0 && !self.content_sized(comp);
        let asked = (from_content && !self.source.is_empty()).then(|| self.width_from_content(comp, ui, id, room, item, scx));
        let width = match (bounded, self.content_sized(comp)) {
            (true, false) => asked.unwrap_or(room.x),
            // No rows: the empty state has no ideal width, so it takes the room, or none where the room is unbounded.
            (_, _) if self.source.is_empty() => {
                if bounded {
                    room.x
                } else {
                    0.0
                }
            }
            // An unbounded room (a horizontally scrolling body) takes the widest row, as a content-sized collection does.
            (_, _) => {
                let floor = env.get::<crate::contract::overlay::LayerFloor>().0;
                let widest = layout_memory::measured(ui.ctx(), sub(id, "largest")).map_or(0.0, |measured| measured.took.x);
                // Columns side by side at their own widths: a `Content` column takes its widest cell, not the share of
                // the widest row it would get.
                let columns = if self.header.has_columns() { self.header.content_width() } else { 0.0 };
                widest.max(columns).max(comp.ideal_width(item, self.source.len(), scx)).max(floor).min(room.x)
            }
        };
        let previous = self.layout.take().or_else(|| self.previous.take()).map(|held| Arc::try_unwrap(held).unwrap_or_else(|held| (*held).clone()));
        let mut layout = comp.item_layout_from(previous, &*self.source, width, item, scx);
        // An auto-width layout sets its items side by side: where its width comes from its rows, it takes their run at
        // their ideal widths, not the widest one (a tab strip in a bar's unbounded row).
        let run = (layout.auto_widths() && (asked.is_some() || !bounded) && !self.source.is_empty())
            .then(|| self.run_in(ui, id, room).min(room.x))
            .filter(|run| *run > width);
        let (width, asked) = match run {
            Some(run) => {
                layout = comp.item_layout_from(Some(layout), &*self.source, run, item, scx);
                (run, asked.map(|asked| asked.max(run)))
            }
            None => (width, asked),
        };
        if self.header.has_columns() {
            layout.set_columns(&self.header.columns(width));
        }
        let ctx = ui.ctx();
        rows::measured_heights(ctx, id, |height| layout.measured(&|at| height(rows::measured_id(id, &self.source.key(at)))));
        rows::restore_fits(ctx, id, &mut layout, self.source.len(), |at| key_hash(&self.source.key(at)));
        if layout.auto_widths() {
            rows::measured_widths(ctx, id, |width| {
                for at in 0..self.source.len() {
                    if let Some((ideal, min)) = width(rows::measured_width_id(id, &self.source.key(at))) {
                        layout.set_width(at, ideal, min);
                    }
                }
            });
        }
        self.measure_ahead(comp, measuring, id, &mut layout, (width, room));
        let content = layout.content_size();
        self.content = Vec2::new(content.x as f32, content.y as f32);
        let props = comp.items();
        let row = props.row_height.of(scx);
        let floor = rows_height(&layout, self.source.len(), props.min_rows, row);
        let cap = props.max_rows.map(|rows| rows_height(&layout, self.source.len(), rows, row).max(floor));
        self.columns = layout.column_count() as u32;
        self.layout = Some(Arc::new(layout));
        let empty = if self.source.is_empty() { comp.empty_height(width, ui, scx) } else { 0.0 };
        let rows = self.content.y.max(empty).max(floor);
        let height = cap.map_or(rows, |cap| rows.min(cap)) + self.header.height;
        if !(self.scrolls && room.y.is_finite()) {
            let wide = if from_content { asked.unwrap_or_else(|| self.width_from_content(comp, ui, id, room, item, scx)) } else { self.content.x };
            return Vec2::new(wide, height);
        }
        // A scrolling collection in a bounded room: `Content` takes its rows up to the room, `Fill` the room.
        let wide = if self.content_sized(comp) {
            self.content.x
        } else if from_content {
            asked.unwrap_or_else(|| self.width_from_content(comp, ui, id, room, item, scx))
        } else {
            0.0
        };
        match comp.props(Internal::new()).sizing {
            // Under a body sized from its content along the height, the rows take up to the room, as a content-sized
            // collection's do: the body has no room of its own to hand it.
            Sizing::Fill | Sizing::FillHeight => Vec2::new(wide, if self.from_content.1 || self.capped { height.min(room.y) } else { 0.0 }),
            Sizing::Content | Sizing::FillWidth => Vec2::new(wide, height.min(room.y)),
        }
    }
}

/// Whether a body around the collection, as its scope `env` names it, scrolls along the rows' overflow axis too: the
/// scroll along that axis belongs to the collection, so such a body is a second owner.
fn shares_scroll(props: &CollectionProps, env: &Env) -> bool {
    env.scroll_owner.is_some() && if props.overflow == Axis::X { env.unbounded.0 } else { env.unbounded.1 }
}

/// Whether the collection scrolls its rows itself in the scope `env`: a capped one does, bounded by its rows, inside a
/// scrolling body too; one with the natural height, or inside a body that scrolls along its rows, leaves the scroll to
/// that body. Its rows' scope names it as their scroll owner then (`rows_env`).
pub(super) fn scrolls_rows(props: &CollectionProps, env: &Env) -> bool {
    props.max_rows.is_some() || (!props.natural_height && !shares_scroll(props, env))
}
