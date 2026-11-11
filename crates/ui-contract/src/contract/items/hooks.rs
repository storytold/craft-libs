//! The item step's `ShapeHooks`: the pipeline's calls into a collection's rows.

use super::drag::Drags;
use super::header::HeaderMemory;
use super::{Held, ITEM_NODE, ItemsHooks, LastLayout, Lead, Placement, Renaming, Rows, SCROLL, Scrolled, View, key_hash, section_part};
use super::{request, rows};
use crate::contract::ValueOf;
use crate::contract::collection::{CollectionEvent, Gestures, OpenChange, Rename, RowState, ShownRow};
use crate::contract::cx::{BodyCx, Pointer as _};
use crate::contract::run::{Measuring, ShapeHooks, ShowAt};
use crate::contract::steps::ShapeKind;
use crate::contract::{Bind, Collection, Cx, Internal, ItemCx, ItemOutput as _, PartsCx, Props, Show as _, Sizing};
use crate::primitives::{HiddenContent, ScrollBar};
use crate::runtime::access::{self, Role, SetPosition};
use crate::runtime::egui::{alloc, areas, child, response};
use crate::runtime::env::{AppValues, ArenaId, Env, ItemFacts};
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::{focus, keys, layout_memory, overlay, wheel};
use crate::style::{ComponentStyle as _, StyleCx, SurfaceKind, scroll_bar_look, surface_style};
use craft_behavior::{
    Axis, CancelReason, EditEvent, Intent, ItemLayout, Mixed, Proposal, ScrollPos, ScrollSpan, SelectionModel, Sense, WheelRoom, WheelTaker,
};
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

    fn on_line(&self, comp: &C) -> bool {
        comp.items().overflow == Axis::X
    }

    /// A collection that keeps focus on press senses the press without taking focus.
    fn sense(&self, comp: &C, own: Sense) -> Sense {
        if comp.items().focus_on_press { own } else { own.difference(Sense::TAKE_FOCUS).union(Sense::KEEP_FOCUS) }
    }

    fn key_role(&self, comp: &C) -> craft_behavior::KeyRole {
        let props = comp.items();
        craft_behavior::KeyRole::new()
            .reorderable(props.reorder.is_some())
            .pages(props.pages)
            .rename(props.rename)
            .type_ahead(true)
            .digits(props.digit_keys)
            .space(true)
            .moves(true)
            .deletes(props.deletable)
            .owner_arrows(props.item_keys == crate::contract::collection::ItemKeys::Owner)
            .home_row(self.layout.as_ref().map_or(0, |layout| layout.first_navigable()))
    }

    fn instance_scope(&self, comp: &C) -> Option<crate::runtime::commands::CommandScope> {
        comp.props(Internal::new()).command_scope
    }

    fn read(&mut self, comp: &mut C, stage: &mut MemoryStage, id: Id, env: &Env) {
        let ctx = stage.ctx().clone();
        let natural = comp.items().natural_height;
        // The scroll along the rows' overflow axis belongs to the collection: a body that scrolls along it too is a
        // second owner.
        let overflow = comp.items().overflow;
        let shared = env.scroll_owner.is_some() && if overflow == Axis::X { env.unbounded.0 } else { env.unbounded.1 };
        debug_assert!(
            !shared || natural,
            "a collection owns its scroll: show it outside a body that scrolls along its rows, or give it the natural height (16.3)"
        );
        self.scrolls = !natural && !shared;
        self.overflow = overflow;
        self.selection = match comp.selection() {
            Some(bind) => match bind.get() {
                Some(Mixed::Same(held)) => held,
                _ => C::Selection::default(),
            },
            None => stage.session::<Held<C::Selection>>(sub(id, "selection")).0.clone(),
        };
        self.selection_at_read = self.selection.clone();
        self.previous = stage.session::<LastLayout<C::ItemLayout>>(sub(id, "item_layout")).0.take();
        self.header.read(stage, id, comp.items());
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
        self.renaming = stage.session::<Renaming<C::Key>>(sub(id, "rename")).clone();
        self.last = stage.recall::<View>(sub(id, "view"));
        let scrolled = match comp.scroll() {
            Some(bind) => match bind.get() {
                Some(Mixed::Same(pos)) => Scrolled { y: pos.y, x: pos.x },
                _ => Scrolled::default(),
            },
            None => stage.session::<Scrolled>(sub(id, "scroll")).clone(),
        };
        (self.offset, self.offset_x) = if self.scrolls { (scrolled.y, scrolled.x) } else { (0.0, 0.0) };
        self.offset_at_read = self.offset;
        self.offset_x_at_read = self.offset_x;
        let view = self.last.view.map_or(0.0, |view| view[3] - view[1]);
        if self.scrolls {
            let quiet = stage.is_peek() || crate::runtime::measure::active(&ctx);
            let taken = if quiet { wheel::peek(&ctx, id) } else { wheel::take(&ctx, id) };
            if let Some(share) = taken {
                self.offset -= f64::from(share.scroll.y);
                self.offset_x -= f64::from(share.scroll.x);
                if share.zoom != 1.0
                    && let Some(about) = share.at
                {
                    self.events.push(CollectionEvent::Zoom { factor: share.zoom, about });
                }
            }
            if let (Some(rect), Some(bounds)) =
                (if quiet { crate::contract::peek_reveal(&ctx, id) } else { crate::contract::take_reveal(&ctx, id) }, self.last.view)
            {
                if rect.min.y < bounds[1] {
                    self.offset += f64::from(rect.min.y - bounds[1]);
                } else if rect.max.y > bounds[3] {
                    self.offset += f64::from(rect.max.y - bounds[3]);
                }
            }
            self.offset = self.offset.clamp(0.0, f64::from((self.last.extent - view).max(0.0)));
        }
    }

    fn measured_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>) -> Vec2 {
        let id = cx.core.id;
        self.record_widest(comp, at, id);
        self.extent(comp, at.ui, at.at, id, at.scx)
    }

    fn pass_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, _style: &C::Style, _shown: Option<Arc<ValueOf<C>>>) -> Vec2 {
        let id = at.resolved.id;
        self.record_widest(comp, at, id);
        self.extent(comp, at.ui, at.at, id, at.scx)
    }

    fn extent(&mut self, comp: &C, ui: &Ui, at: ShowAt, id: Id, scx: &StyleCx<'_>) -> Vec2 {
        let room = match at {
            ShowAt::Rect(rect) => rect.size(),
            ShowAt::Flow => crate::layout::flow_room(ui),
        };
        self.room = room;
        self.from_content = if crate::runtime::measure::active(ui.ctx()) { Env::of(ui).content } else { (false, false) };
        self.header.measure(scx);
        let bounded = room.x.is_finite();
        let mut widest = 0.0;
        let width = match (bounded, self.content_sized(comp)) {
            (true, false) => room.x,
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
                let floor = overlay::drawing_parent(ui.ctx()).map_or(0.0, |layer| crate::contract::overlay::floor_width(ui.ctx(), layer));
                widest = layout_memory::measured(ui.ctx(), sub(id, "widest")).map_or(0.0, |measured| measured.took.x);
                widest.max(comp.ideal_width(scx)).max(floor).min(room.x)
            }
        };
        let previous = self.layout.take().or_else(|| self.previous.take()).map(|held| Arc::try_unwrap(held).unwrap_or_else(|held| (*held).clone()));
        let mut layout = comp.item_layout_from(previous, self.source, width, scx);
        if self.header.has_columns() {
            layout.set_columns(&if self.content_sized(comp) { self.header.columns_measured(width, widest) } else { self.header.columns(width) });
        }
        let ctx = ui.ctx();
        rows::measured_heights(ctx, |height| layout.measured(&|at| height(rows::measured_id(id, &self.source.key(at)))));
        if layout.auto_widths() {
            rows::measured_widths(ctx, |width| {
                for at in 0..self.source.len() {
                    if let Some((ideal, min)) = width(rows::measured_width_id(id, &self.source.key(at))) {
                        layout.set_width(at, ideal, min);
                    }
                }
            });
        }
        let content = layout.content_size();
        self.content = Vec2::new(content.x as f32, content.y as f32);
        self.columns = layout.column_count() as u32;
        self.layout = Some(Arc::new(layout));
        let empty = if self.source.is_empty() { comp.empty_height(width, ui, scx) } else { 0.0 };
        let height = self.content.y.max(empty) + self.header.height;
        if !(self.scrolls && room.y.is_finite()) {
            // Under a body sized from its content, a `Fill` collection offers its least width here too: the body
            // has no room of its own to hand it, and the layout above was made at the enclosing room.
            if self.from_content.0 && comp.props(Internal::new()).sizing == Sizing::Fill {
                let wide = if self.header.has_columns() { self.header.least_width() } else { self.widest_in(comp, ui, id, room).min(room.x) };
                return Vec2::new(wide, height);
            }
            return Vec2::new(self.content.x, height);
        }
        // A scrolling collection in a bounded room: `Content` takes its rows up to the room, `Fill` the room.
        match comp.props(Internal::new()).sizing {
            // Under a body sized from its content, a measuring run takes the columns' least width and the rows up to
            // the room, as a content-sized collection does: the body has no room of its own to hand it.
            Sizing::Fill => {
                let (x, y) = self.from_content;
                let wide = if !x {
                    0.0
                } else if self.header.has_columns() {
                    self.header.least_width()
                } else {
                    self.widest_in(comp, ui, id, room).min(room.x)
                };
                Vec2::new(wide, if y { height.min(room.y) } else { 0.0 })
            }
            Sizing::Content | Sizing::FillWidth => Vec2::new(if self.content_sized(comp) { self.content.x } else { 0.0 }, height.min(room.y)),
        }
    }

    fn sized(&self, comp: &C, _: &Ui, _: Id, size: Vec2) -> Vec2 {
        let sizing = comp.props(Internal::new()).sizing;
        // Only a `Fill` collection measures its width from its content under a content-sized body; `FillWidth` takes
        // the room it is given on every run.
        let width_from_content = self.from_content.0 && sizing == Sizing::Fill;
        let width = if self.room.x.is_finite() && !self.content_sized(comp) && !width_from_content { self.room.x.max(size.x) } else { size.x };
        let height = match (self.scrolls && self.room.y.is_finite(), sizing) {
            (true, Sizing::Fill) if !self.from_content.1 => self.room.y,
            (true, Sizing::Fill) => size.y.min(self.room.y),
            (true, Sizing::Content | Sizing::FillWidth) => size.y.min(self.room.y),
            (false, _) => size.y,
        };
        Vec2::new(width, height)
    }

    fn geometry(&mut self, _: &C, content: Rect, rtl: bool) -> Option<Rc<dyn std::any::Any>> {
        let view = self.below_header(content);
        let offset = ScrollPos { x: self.offset_x, y: self.offset.clamp(0.0, self.max_offset(view.height())) };
        let mut layout = self.layout.take()?;
        Arc::make_mut(&mut layout).set_view(view.size(), offset);
        let (_, order) = self.built(&layout, view, offset);
        let rects = order.into_iter().map(|at| (self.source.key(at), self.screen(&layout, view, at, offset, rtl))).collect();
        self.layout = Some(layout);
        Some(Rc::new(Rows { rects, offset }))
    }

    fn parts(&mut self, _: &mut C, _: Rect, pcx: &mut PartsCx<'_, C>) {
        self.header.parts(pcx);
        if !self.scrolls {
            return;
        }
        let view = self.below_header(pcx.content_rect);
        let thickness = scroll_bar_look(pcx.tokens).thickness;
        let track = if pcx.flow.is_rtl() {
            Rect::from_min_max(view.min, Pos2::new(view.min.x + thickness, view.max.y))
        } else {
            Rect::from_min_max(Pos2::new(view.max.x - thickness, view.min.y), view.max)
        };
        let content = f64::from(self.content.y).max(f64::from(view.height()));
        self.offset = self.offset.clamp(0.0, self.max_offset(view.height()));
        let span = ScrollSpan::new(0.0..content, self.offset..self.offset + f64::from(view.height()));
        pcx.scroll(SCROLL, track, Axis::Y, span);
    }

    fn sensed(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        cx.core.collection_selection = Some(Box::new(self.selection.clone()));
        if self.scrolls {
            // The taker covers the whole collection, header row and frame included: the wheel over the header scrolls
            // the rows (Shift sideways).
            let view = self.below_header(cx.core.content_rect);
            let (vertical, horizontal) = (self.overflow == Axis::Y, self.content.x > view.width());
            let max = Vec2::new((self.content.x - view.width()).max(0.0), self.max_offset(view.height()) as f32);
            let offset = Vec2::new(self.offset_x as f32, self.offset as f32);
            wheel::register(
                &cx.core.ctx,
                WheelTaker::new(cx.core.id, cx.core.rect)
                    .vertical((vertical && max.y > 0.0) || horizontal)
                    .horizontal(horizontal)
                    .scrolls(true)
                    .gated(cx.core.env.wheel_gated())
                    .room(WheelRoom::new(offset, max - offset))
                    .zoom(comp.zooms()),
            );
        }
    }

    fn updated(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        use crate::contract::Pointer as _;
        let rect = self.rows_rect(cx);
        let view = rect.height();
        let rtl = cx.core.flow.is_rtl();
        self.xf = comp.props(Internal::new()).transform.map(|zoom| areas::body_map(rect.min, zoom.scale, zoom.offset));
        if let Some(range) = cx.scroll_moved(SCROLL) {
            self.offset = range.start;
        }
        self.offset = self.offset.clamp(0.0, self.max_offset(view));
        let header_held = self.header_step(cx);
        self.offset_x = self.offset_x.clamp(0.0, f64::from((self.content.x - rect.width()).max(0.0)));
        self.open_lead(comp);
        self.follow_lead_source(comp);
        let lead_at = self.lead.key.as_ref().and_then(|key| self.index_of(key));
        let props = comp.items().clone();
        let Some(layout) = self.layout.take() else { return };
        let offset = ScrollPos { x: self.offset_x, y: self.offset };
        let at = Placement { rect, offset, rtl };
        // A press lands on the rows the user saw: the offset of the last pass, before this pass's wheel share (8.3).
        let seen = Placement { offset: ScrollPos { x: self.offset_x_at_read, y: self.offset_at_read }, ..at };
        self.press_step(comp, cx, &props, &layout, seen);
        if props.lead_follows_hover {
            self.hover_lead(comp, cx, &layout, seen);
        }
        self.drag_step(comp, cx, &layout, (rect, offset, rtl), &props);
        self.marquee_step(comp, cx, &layout, (rect, offset, rtl), &props);
        self.navigate(comp, cx, &layout, view, header_held);
        self.switch_page(comp, cx, &props, &layout, view);
        let lead_at = self.lead.key.as_ref().and_then(|key| self.index_of(key)).or(lead_at);
        self.intent_actions(comp, cx, &props, lead_at, header_held);
        self.context_key(cx, &layout, at, lead_at);
        self.delete_key(cx, &props, lead_at);
        self.toggle_by_access(cx, &layout, at);
        self.node_by_access(comp, cx, &layout, at);
        self.reorder_and_rename(comp, cx, &props, lead_at);
        self.requested_rename(comp, cx, &props, &layout, view);
        self.requested_reveal(comp, &layout, view);
        self.type_ahead(comp, cx, &layout, view, lead_at);
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
        self.selection_at_read = self.selection.clone();
    }

    fn content(&mut self, comp: &mut C, ui: &mut Ui, env: &Env, cx: &mut Cx<'_, C>, values: &AppValues) {
        let Some(mut layout) = self.layout.take() else { return };
        let (id, ctx) = (cx.core.id, cx.core.ctx.clone());
        let rect = self.rows_rect(cx);
        let rtl = env.direction.is_rtl();
        let node = matches!(C::ROLE, Role::Node(_));
        let offset = ScrollPos { x: self.offset_x, y: self.offset };
        Arc::make_mut(&mut layout).set_view(rect.size(), offset);
        if self.scrolls {
            let size = self.content.max(rect.size());
            let left = if rtl { rect.max.x + self.offset_x as f32 - size.x } else { rect.min.x - self.offset_x as f32 };
            let content = Rect::from_min_size(Pos2::new(left, rect.min.y - self.offset as f32), size);
            crate::runtime::egui::widgets::note_scroll_region(&ctx, rect.intersect(ui.clip_rect()), content);
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
        let child_size = comp.props(Internal::new()).child_size;
        let base_env = env.derive(|scoped| {
            scoped.values.extend(values);
            scoped.arena = Some(ArenaId::new(id));
            scoped.size = child_size.unwrap_or(cx.core.resolved.size);
        });
        self.header.paint(ui, cx, env, id, rtl);
        let len = self.source.len();
        let tree = matches!(layout.nav(), craft_behavior::NavShape::Tree);
        focus::registration::record_tree(&ctx, id, tree);
        let focused = focus::is_focused(&ctx, id);
        let lead = self.lead.key.clone();
        let mut lead_child = None;
        let striped = comp.items().striped;
        let current: Option<C::Key> = comp.items().current.as_deref().and_then(|key| key.downcast_ref::<C::Key>()).cloned();
        let mut stripe: Option<(usize, usize)> = None;
        let transform = comp.props(Internal::new()).transform;
        let (parent, clip) = (ui.layer_id(), rect.intersect(ui.clip_rect()));
        let forward =
            areas::Forward { id: sub(id, "transform"), sense: response::to_egui(cx.core.root_sense), rect: cx.core.rect.intersect(ui.clip_rect()) };
        let placed = self.selection.placed(self.source);
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
        let hover_at = crate::primitives::sensing::read_outcome(&ctx, ArenaId::new(cx.core.id)).hover.and(ctx.pointer_latest_pos());
        let place_rows = |ui: &mut Ui| {
            for at in order.into_iter() {
                if self.source.section_stop(at) {
                    continue;
                }
                let key = self.source.key(at);
                let item_id = sub(id, ("key", key_hash(&key)));
                let row_group = rows::row_group(item_id);
                row_groups.push(row_group);
                let mut row = self.screen(&layout, rect, at, offset, rtl);
                let state = RowState {
                    selected: placed.holds_row(at),
                    lead: lead.as_ref() == Some(&key),
                    renaming: self.renaming.key.as_ref() == Some(&key),
                    striped: striped && {
                        let run = self.stripe_run(stripe, at);
                        stripe = Some((at, run));
                        run.is_multiple_of(2) && !self.source.is_section(at)
                    },
                    dragging: self.drags.grabbed.as_ref() == Some(&key),
                    drop_target: self.drags.over.as_ref() == Some(&key),
                    current: current.as_ref() == Some(&key),
                    open: self.source.has_children(at) && self.source.open(at),
                    hovered: hover_at.is_some_and(|pos| row.contains(pos)),
                    ..RowState::default()
                };
                let mut flags = crate::style::Flags::empty();
                flags.set(crate::style::Flags::SELECTED, state.selected);
                let child = if state.lead { rows::child_focus(&ctx, row_group) } else { None };
                lead_child = lead_child.or(child);
                // A list that never takes key focus (a popup list: the host forwards its keys) shows its lead always.
                flags.set(crate::style::Flags::LEAD, state.lead && (focused || child.is_some() || !comp.items().focus_on_press));
                flags.set(crate::style::Flags::EXPANDED, state.open);
                flags.set(crate::style::Flags::DRAGGING, state.dragging);
                flags.set(crate::style::Flags::DROP_TARGET, state.drop_target);
                flags.set(crate::style::Flags::CURRENT, state.current);
                let mut position = SetPosition::new(at - self.source.section_stops_before(at))
                    .of(len - self.source.section_stops_before(len))
                    .columns(columns.len());
                if tree {
                    position = position.level(self.source.depth(at) + 1);
                }
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
                    row: state,
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
                    openers: Vec::new(),
                    item: item_id,
                    memory: &mut *cx.core.memory,
                };
                icx.memory.claim(icx.id);
                let item = (self.item)(&key, &mut icx);
                let mut item = comp.dress(item, &state);
                let tip = icx.tip.take();
                let openers = std::mem::take(&mut icx.openers);
                drop(icx);
                let builder = UiBuilder::new().max_rect(row).layout(child::column_layout(rtl)).scope_id(sub(item_id, "row"));
                let builder = if node { access::with_parent(builder, id) } else { builder };
                let mut row_ui = alloc::child(ui, builder);
                // A scrolled row slides under the frozen rows: it never paints into their band.
                let below_frozen = Rect::from_min_max(Pos2::new(rect.min.x, rect.min.y + layout.frozen_band(at)), rect.max);
                row_ui.set_clip_rect(below_frozen.intersect(ui.clip_rect()));
                let installed = Env::install_for(
                    &row_ui,
                    base_env.derive_local(|scoped| {
                        scoped.item = Some(item_id);
                        scoped.row = Some(row_group);
                        scoped.item_facts = Some(ItemFacts { item: item_id, flags, position, columns: column_keys, toggles: toggle_keys });
                    }),
                );
                item = Props::id(item, item_id);
                if layout.is_auto(at) {
                    let measured = item.measure_in(&row_ui, Proposal::Width(row.width())).y;
                    rows::record_measure(&ctx, rows::measured_id(id, &key), Vec2::new(row.width(), measured));
                    if measured != row.height() {
                        Arc::make_mut(&mut layout).set_height(at, measured);
                        row = self.screen(&layout, rect, at, offset, rtl);
                    }
                }
                if layout.auto_widths() {
                    let ideal = item.measure_in(&row_ui, Proposal::Ideal).x;
                    let min = item.measure_in(&row_ui, Proposal::Min).x;
                    rows::record_measure(&ctx, rows::measured_width_id(id, &key), Vec2::new(ideal, min));
                    Arc::make_mut(&mut layout).set_width(at, ideal, min);
                    row = self.screen(&layout, rect, at, offset, rtl);
                }
                shown_rows.push(ShownRow::new(key.clone(), row, shown_cells.clone()));
                let out = match tip {
                    Some(tip) => item.show_at_with_tip(&mut row_ui, row, tip),
                    None => item.show_at(&mut row_ui, row),
                };
                // After the row is shown: its toggle cells have recorded their names by then.
                self.report_toggle_cells(&ctx, (id, item_id), (at, &key), (shown_cells.as_slice(), cx.core.resolved.enabled), &cx.core.env);
                let label = out.label_rect().unwrap_or(row);
                crate::runtime::keys::redirect_node(&ctx, item_id, id, ITEM_NODE.at(key_hash(&key)));
                if let Some(row) = shown_rows.last_mut() {
                    row.openers = self.item_openers(&row_ui, &ctx, id, cx.core.resolved.enabled, openers);
                }
                if let Some(ask) = out.open_request() {
                    self.events.push(CollectionEvent::Open(OpenChange { key: key.clone(), open: ask.open, all: ask.all }));
                }
                if let Some(event) = out.into_event() {
                    self.events.push(CollectionEvent::Item(key.clone(), event));
                }
                if state.renaming {
                    let mut draft = std::mem::take(&mut self.renaming.draft);
                    let name = self.source.text(at).map(|text| text.into_owned()).unwrap_or_default();
                    let editor = comp.rename_editor(&mut row_ui, Bind::bound(&mut draft), label, &name);
                    self.renaming.draft = draft;
                    if let Some(editor) = editor {
                        if !self.renaming.focused {
                            focus::request_now(&ctx, focus::FocusTarget::Id(editor.response.id));
                            self.renaming.focused = true;
                        }
                        let gesture = self.renaming.gesture;
                        let before = Mixed::Same(Rename { key: key.clone(), name: name.clone() });
                        // The editor's gesture end, if the pass has one, ends the rename.
                        let end =
                            editor.events.iter().rev().find(|event| matches!(event, EditEvent::Commit { .. } | EditEvent::Cancel { .. })).cloned();
                        let edited = !editor.events.is_empty();
                        if end.is_some() {
                            self.refocus = focus::is_focused(&ctx, editor.response.id);
                        }
                        // The editor claims Escape for the whole rename, as a text part with a draft does: Escape does not leave the
                        // row. One the editor leaves untaken (it holds no draft yet) cancels the rename.
                        keys::hold_escape(&ctx, editor.response.id);
                        let escaped = !edited && focus::is_focused(&ctx, editor.response.id) && {
                            let mut intents = keys::intents_of(&ctx, Some(editor.response.id), rtl);
                            let left = intents.take(Intent::Cancel);
                            keys::settle(&ctx, Some(editor.response.id), &intents);
                            left
                        };
                        match end {
                            None if edited => {}
                            None if escaped => {
                                self.events.push(CollectionEvent::Rename(EditEvent::cancel(gesture, CancelReason::Escape, Some(before))));
                                self.ended_rename = true;
                                self.refocus = true;
                            }
                            Some(EditEvent::Commit { after, .. }) => {
                                let after = Rename { key: key.clone(), name: after };
                                self.events.push(CollectionEvent::Rename(EditEvent::commit(gesture, before, after)));
                                self.ended_rename = true;
                            }
                            Some(EditEvent::Cancel { reason, .. }) => {
                                self.events.push(CollectionEvent::Rename(EditEvent::cancel(gesture, reason, Some(before))));
                                self.ended_rename = true;
                            }
                            // Enter on a name nobody typed in opens no edit run: the rename ends with no event and no undo step, as a
                            // text field's empty commit does.
                            None if editor.submitted => {
                                self.ended_rename = true;
                                self.refocus = true;
                            }
                            None if editor.left_by_tab.is_some() => {
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
                                request::request_rename(&ctx, id, &self.source.key(next));
                            }
                        }
                    }
                }
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
        self.paint_marquee(ui, env, &layout, (rect, offset, rtl));
        focus::register_row_groups(&ctx, id, rtl, row_groups);
        self.shown = shown_rows;
        cx.core.grid = crate::contract::GridPlace::grid(len, columns.len());
        let size = layout.content_size();
        self.content = Vec2::new(size.x as f32, size.y as f32);
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
                let item_id = sub(id, ("key", key_hash(&key)));
                let cell = (sheet && self.selection.lead_column().is_some()).then_some(self.lead.column);
                access::set_focus(&ctx, crate::runtime::access::items::focus_node(item_id, lead_child, cell));
            }
        }
        if self.source.is_empty() {
            let mut open = true;
            let builder = UiBuilder::new().max_rect(rect).layout(child::column_layout(rtl)).scope_id(sub(id, "empty"));
            let mut empty_ui = alloc::child(ui, builder);
            let installed = Env::install_for(&empty_ui, env.derive(|scoped| scoped.values.extend(values)));
            let (resolved, view) = (cx.core.resolved, cx.core.env);
            let mut bcx = BodyCx::new(sub(id, "body"), view, resolved, rect, &mut *cx.core.memory, &mut open, None);
            comp.empty(&mut empty_ui, &mut bcx);
            Env::restore(&ctx, installed);
        }
        if self.scrolls {
            let look = scroll_bar_look(&env.tokens);
            // The rows blend into the surface the collection paints, else the one it sits on.
            let own = cx.style.surface().and_then(|surface| surface.kind_in(&cx.core.state));
            let background = surface_style(&env.tokens, SurfaceKind::under(own, env.surface)).fill;
            let hidden = HiddenContent::new(self.content, rect.size(), Vec2::new(self.offset_x as f32, self.offset as f32), rtl);
            ui.painter().extend(ScrollBar::fades(&look, rect, hidden, background));
            let thickness = look.thickness;
            let track = if rtl {
                Rect::from_min_max(rect.min, Pos2::new(rect.min.x + thickness, rect.max.y))
            } else {
                Rect::from_min_max(Pos2::new(rect.max.x - thickness, rect.min.y), rect.max)
            };
            let content = f64::from(self.content.y).max(f64::from(rect.height()));
            if content > f64::from(rect.height()) {
                let span = ScrollSpan::new(0.0..content, self.offset..self.offset + f64::from(rect.height()));
                let (at, len) = craft_behavior::scroll::thumb(&span, track.height(), look.min_thumb);
                let thumb = crate::contract::cx::thumb_rect(track, Axis::Y, rtl, at, len);
                let result = cx.core.results.get(&(SCROLL, Some(crate::contract::cx::THUMB))).cloned().unwrap_or(crate::primitives::PartResult::IDLE);
                let state = crate::contract::steps::part_state(&result, cx.core.resolved.enabled, None);
                ui.painter().extend(ScrollBar::shapes(&look, track, thumb, &state));
            }
        }
        self.layout = Some(layout);
    }

    fn settle(&mut self, comp: &mut C, cx: &mut Cx<'_, C>) {
        let id = cx.core.id;
        // A collection's focus move reveals through its lead (`page_reveal`), never the collection's whole rect.
        crate::runtime::focus::take_reveal_of(&cx.core.ctx, id);
        if self.scrolls && (self.offset != self.offset_at_read || self.offset_x != self.offset_x_at_read) {
            match comp.scroll() {
                Some(mut bind) => {
                    let (label, doc_key) = (cx.core.label_arc(), cx.core.resolved.doc_key);
                    let (before, after) =
                        (ScrollPos { x: self.offset_x_at_read, y: self.offset_at_read }, ScrollPos { x: self.offset_x, y: self.offset });
                    bind.apply(&EditEvent::set(Mixed::Same(before), after, label, doc_key));
                }
                None => {
                    let scrolled = cx.core.memory.session::<Scrolled>(sub(id, "scroll"));
                    (scrolled.y, scrolled.x) = (self.offset, self.offset_x);
                }
            }
        }
        *cx.core.memory.session::<Lead<C::Key>>(sub(id, "lead")) = self.lead.clone();
        cx.core.memory.session::<LastLayout<C::ItemLayout>>(sub(id, "item_layout")).0 = self.layout.clone();
        *cx.core.memory.session::<HeaderMemory>(sub(id, "columns")) = self.header.persist();
        *cx.core.memory.session::<Drags<C::Key>>(sub(id, "drags")) = self.drags.clone();
        self.header.report(&cx.core.ctx, id, cx);
        *cx.core.memory.session::<Renaming<C::Key>>(sub(id, "rename")) = self.renaming.clone();
        cx.core.collection_selection = Some(Box::new(self.selection.clone()));
        cx.core.collection_events = Some(Box::new((std::mem::take(&mut self.events), std::mem::take(&mut self.shown), self.activation_clicks)));
        cx.core.visible_rows = self.visible.clone();
        cx.core.gestures = Gestures::of(comp.items());
    }

    fn record(&mut self, stage: &mut MemoryStage, _: &Context, id: Id) {
        let view = stage.shown::<View>(sub(id, "view"));
        view.extent = self.content.y;
    }
}
