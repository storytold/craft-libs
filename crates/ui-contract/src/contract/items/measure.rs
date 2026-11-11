//! The widest row of a content-sized collection: measured, recorded and read back.

use super::rows;
use super::{ItemsHooks, key_hash};
use crate::contract::collection::RowState;
use crate::contract::run::{Measuring, ShowAt};
use crate::contract::{Collection, Internal, ItemCx, Props, Show as _, Sizing};
use crate::runtime::access::SetPosition;
use crate::runtime::env::{Env, ItemFacts};
use crate::runtime::layout_memory;
use crate::runtime::memory::{MemoryStage, sub};
use craft_behavior::Proposal;
use craft_behavior::layout_memory::measure_tolerance;
use egui::{Id, Pos2, Rect, Ui, Vec2};

impl<'s, 'r, C: Collection> ItemsHooks<'s, 'r, C> {
    /// Whether the collection takes its width from its widest row: `Sizing::Content`. A row's width holds its
    /// column cells at the widths the header resolves for them.
    pub(super) fn content_sized(&self, comp: &C) -> bool {
        comp.props(Internal::new()).sizing == Sizing::Content
    }

    /// The widest ideal width of the rows in this pass: each row is built as `content` builds it, with its column
    /// cells at the widths the header resolves with no room to share (a fixed or toggle column at its own width),
    /// and measured without allocating.
    fn widest_row(&mut self, comp: &C, ui: &Ui, id: Id) -> f32 {
        let ctx = ui.ctx().clone();
        crate::contract::steps::measuring(&ctx, || self.widest_row_in(comp, ui, id))
    }

    /// Builds the rows once to measure the widest and records it under `sub(id, "widest")` for `extent`, when this
    /// pass's width comes from the rows.
    pub(super) fn record_widest(&mut self, comp: &C, at: &Measuring<'_, '_>, id: Id) {
        let room = match at.at {
            ShowAt::Rect(rect) => rect.size(),
            ShowAt::Flow => crate::layout::flow_room(at.ui),
        };
        if self.source.is_empty() || (room.x.is_finite() && !self.content_sized(comp)) {
            return;
        }
        self.header.measure(at.scx);
        self.widest_in(comp, at.ui, id, room);
    }

    /// The widest row in the room `room`: what a record of this pass under `sub(id, "widest")` found when it serves
    /// the room (`Measured::serves`), else the rows built and measured now, and recorded there. The caller measures the
    /// header first.
    pub(super) fn widest_in(&mut self, comp: &C, ui: &Ui, id: Id, room: Vec2) -> f32 {
        let ctx = ui.ctx().clone();
        let key = sub(id, "widest");
        let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(&ctx));
        // The collection's width comes from its rows here: it is not given the width.
        if let Some(found) =
            layout_memory::measured(&ctx, key).filter(|found| layout_memory::Measured::serves(room.x, found.room.x, false, found.took.x, tolerance))
        {
            return found.took.x;
        }
        #[cfg(feature = "testing")]
        crate::runtime::measure::count_widest(&ctx, id);
        // A real run builds only the rows in view, so no kept transcript can hold the widest row: every pass measures
        // it.
        layout_memory::boundary(&ctx, true, || {
            let widest = self.widest_row(comp, ui, id);
            layout_memory::emit_record(&ctx, key, layout_memory::Measured::new(room, Vec2::new(widest, 0.0)));
            widest
        })
    }

    fn widest_row_in(&mut self, comp: &C, ui: &Ui, id: Id) -> f32 {
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let mut memory = MemoryStage::open_peek(&ctx, id);
        let cells: Vec<(craft_behavior::ColumnKey, Rect)> =
            self.header.columns(0.0).into_iter().map(|(key, width)| (key, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 0.0)))).collect();
        let columns: Vec<craft_behavior::ColumnKey> = cells.iter().map(|(key, _)| key.clone()).collect();
        let toggles: Vec<craft_behavior::ColumnKey> = self.header.toggle_columns().map(|(key, _)| key).collect();
        let len = self.source.len();
        let mut widest = 0.0f32;
        for at in 0..len {
            if self.source.section_stop(at) {
                continue;
            }
            let key = self.source.key(at);
            let item_id = sub(id, ("key", key_hash(&key)));
            let position =
                SetPosition::new(at - self.source.section_stops_before(at)).of(len - self.source.section_stops_before(len)).columns(cells.len());
            let mut icx = ItemCx {
                key: &key,
                at,
                rect: Rect::ZERO,
                row: RowState::default(),
                depth: self.source.depth(at),
                set: position,
                row_index: at,
                row_count: len,
                column_count: cells.len(),
                env: env.view(),
                id: sub(item_id, "item"),
                cells: cells.clone(),
                grid: None,
                tip: None,
                openers: Vec::new(),
                item: item_id,
                memory: &mut memory,
            };
            let item = (self.item)(&key, &mut icx);
            let mut item = Props::id(comp.dress(item, &RowState::default()), item_id);
            let facts =
                ItemFacts { item: item_id, flags: crate::style::Flags::empty(), position, columns: columns.clone(), toggles: toggles.clone() };
            let installed = Env::install_for(
                ui,
                env.derive_local(|scoped| {
                    scoped.item = Some(item_id);
                    scoped.row = Some(rows::row_group(item_id));
                    scoped.item_facts = Some(facts);
                }),
            );
            widest = widest.max(item.measure_in(ui, Proposal::Ideal).x);
            Env::restore(&ctx, installed);
        }
        widest
    }
}
