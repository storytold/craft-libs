//! The largest item of a collection: the widest row of a content-sized collection, the cell of a collection that
//! sizes its items from their measure. Measured, recorded, kept while the items' inputs hold, and read back.

use super::{ItemsHooks, key_hash};
use super::{header, rows};
use crate::contract::body::EnvFacts;
use crate::contract::collection::RowState;
use crate::contract::run::{Measuring, ShowAt};
use crate::contract::{Collection, Internal, ItemCx, Props, Show as _, Sizing};
use crate::runtime::access::SetPosition;
use crate::runtime::env::{Env, EnvView, ItemFacts};
use crate::runtime::layout_memory;
use crate::runtime::memory::{MemoryStage, sub};
use crate::style::{StyleCx, UiTokens};
use craft_behavior::Proposal;
use craft_behavior::layout_memory::measure_tolerance;
use egui::{Id, Pos2, Rect, Ui, Vec2};

impl<'s, 'r, C: Collection> ItemsHooks<'s, 'r, C> {
    /// Whether the collection takes its width from its widest row: `Sizing::Content`. A row's width holds its
    /// column cells at the widths the header resolves for them.
    pub(super) fn content_sized(&self, comp: &C) -> bool {
        comp.props(Internal::new()).sizing == Sizing::Content
    }

    /// The padding the collection's props add on every side of its rows (`ShapeHooks::pad`).
    pub(super) fn props_pad(comp: &C, tokens: &UiTokens) -> f32 {
        crate::contract::pad_of(tokens, comp.props(Internal::new()).pad)
    }

    /// The room the header and rows lay out in: `room` less the padding the props add on both sides. Step 5 adds the
    /// padding back and step 7 insets the content rect by it.
    pub(super) fn inner_room(comp: &C, tokens: &UiTokens, room: Vec2) -> Vec2 {
        (room - Vec2::splat(2.0 * Self::props_pad(comp, tokens))).max(Vec2::ZERO)
    }

    /// The width a collection that fills the width takes where it is not given one, in a measuring run under a body
    /// sized from its content: the header's least width, else its largest row's, at least the width it asks for
    /// (`Collection::ideal_width`), up to `room`. The column it sits in holds it at that width and shows it across
    /// the width the column settles on.
    pub(super) fn width_from_content(&mut self, comp: &C, ui: &Ui, id: Id, room: Vec2, item: Option<Vec2>, scx: &StyleCx<'_>) -> f32 {
        let rows = if self.header.has_columns() { self.header.least_width() } else { self.largest_in(comp, ui, id, room).x };
        rows.max(comp.ideal_width(item, scx)).min(room.x)
    }

    /// The largest ideal size of the rows in this pass, and the widest cell each column's items reported: each row is
    /// built as `content` builds it, with its column cells at the widths the header resolves with no room to share (a
    /// fixed or toggle column at its own width), and measured without allocating.
    fn largest_row(&mut self, comp: &C, ui: &Ui, id: Id) -> (Vec2, Vec<(craft_behavior::ColumnKey, f32)>) {
        let ctx = ui.ctx().clone();
        crate::contract::steps::measuring(&ctx, || self.largest_row_in(comp, ui, id))
    }

    /// Builds the rows once to measure the largest and records it under `sub(id, "largest")` for `extent`, when this
    /// pass's width comes from the rows, the collection `measures_items`, or a column takes its widest cell's width.
    pub(super) fn record_largest(&mut self, comp: &C, at: &Measuring<'_, '_>, id: Id) {
        let room = match at.at {
            ShowAt::Rect(rect) => rect.size(),
            ShowAt::Flow => crate::layout::flow_room(at.ui),
        };
        let room = Self::inner_room(comp, &at.env.tokens, room);
        if self.source.is_empty() || (room.x.is_finite() && !self.content_sized(comp) && !comp.measures_items() && !self.header.has_content_columns())
        {
            return;
        }
        self.header.measure(at.scx);
        self.largest_in(comp, at.ui, id, room);
    }

    /// The largest row in the room `room`: what a record of this pass under `sub(id, "largest")` found when it serves
    /// the room (`Measured::serves`), else what the measure kept from an earlier pass gives while the items' inputs
    /// hold, else the rows built and measured now, recorded there and kept. The widest cell of each `Content` column,
    /// or its header label when wider, is recorded with it (`header::content_id`). The caller measures the header first.
    pub(super) fn largest_in(&mut self, comp: &C, ui: &Ui, id: Id, room: Vec2) -> Vec2 {
        let ctx = ui.ctx().clone();
        let key = sub(id, "largest");
        let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(&ctx));
        let found = |ctx: &egui::Context| {
            // The collection's width comes from its rows here: it is not given the width.
            layout_memory::measured(ctx, key).filter(|found| layout_memory::Measured::serves(room.x, found.room.x, false, found.took.x, tolerance))
        };
        if let Some(found) = found(&ctx) {
            return found.took;
        }
        // A kept measure serves only the outermost measuring run, which this opens when none is open.
        let inputs = self.items_inputs(comp, ui, room);
        if let Some(inputs) = &inputs
            && crate::runtime::measure::run(&ctx, || layout_memory::kept_serves(&ctx, key, inputs))
            && let Some(found) = found(&ctx)
        {
            return found.took;
        }
        #[cfg(feature = "testing")]
        crate::runtime::measure::count_widest(&ctx, id);
        layout_memory::boundary(&ctx, true, || {
            if inputs.is_some() {
                layout_memory::begin_measure(&ctx, key);
            }
            let (largest, cells) = self.largest_row(comp, ui, id);
            layout_memory::emit_record(&ctx, key, layout_memory::Measured::new(room, largest));
            let env = Env::of(ui);
            let content: Vec<(usize, craft_behavior::ColumnKey)> = self.header.content_columns().map(|(at, key)| (at, key.clone())).collect();
            for (at, column) in content {
                let widest = cells.iter().find(|(held, _)| *held == column).map_or(0.0, |(_, width)| *width);
                let wide = widest.max(self.header.label_width(ui, &env.view(), at));
                layout_memory::emit_record(&ctx, header::content_id(id, &column), layout_memory::Measured::new(room, Vec2::new(wide, 0.0)));
            }
            if let Some(inputs) = inputs {
                layout_memory::end_measure(&ctx, key, inputs);
            }
            largest
        })
    }

    /// What the items' measure reads besides the rows' content: the room, the item count, the env and what `dress`
    /// reads of the collection. `None` when the source declares no content version (rows stamp or revision): the rows
    /// may change unseen, so every pass measures them.
    fn items_inputs(&self, comp: &C, ui: &Ui, room: Vec2) -> Option<ItemsInputs> {
        let content = (comp.items().rows_stamp, self.source.revision());
        (content.0.is_some() || content.1.is_some()).then(|| ItemsInputs {
            room,
            count: self.source.len(),
            content,
            dress: comp.dress_inputs(),
            env: EnvFacts::of(&Env::of(ui)),
        })
    }

    fn largest_row_in(&mut self, comp: &C, ui: &Ui, id: Id) -> (Vec2, Vec<(craft_behavior::ColumnKey, f32)>) {
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let mut memory = MemoryStage::open_peek(&ctx, id);
        let cells: Vec<(craft_behavior::ColumnKey, Rect)> =
            self.header.columns(0.0).into_iter().map(|(key, width)| (key, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 0.0)))).collect();
        let mut largest = Vec2::ZERO;
        let mut widest: Vec<(craft_behavior::ColumnKey, f32)> = Vec::new();
        for at in 0..self.source.len() {
            if self.source.section_stop(at) {
                continue;
            }
            let row = RowMeasure { at, rect: Rect::ZERO, cells: cells.clone(), proposal: Proposal::Ideal };
            let (size, cells) = self.measure_row(comp, ui, (id, env.view(), &env), &mut memory, row);
            largest = largest.max(size);
            for (column, width) in cells {
                match widest.iter_mut().find(|(held, _)| *held == column) {
                    Some((_, held)) => *held = held.max(width),
                    None => widest.push((column, width)),
                }
            }
        }
        (largest, widest)
    }

    /// Builds the item of `row.at` as the collection `id` builds it, with the default row state, under `installed`
    /// (`view` is what the item builder sees), and measures it at `row.proposal` without allocating: its size and the
    /// widths its column cells reported. Item memory goes to `memory`.
    pub(super) fn measure_row(
        &mut self,
        comp: &C,
        ui: &Ui,
        (id, view, installed): (Id, EnvView<'_>, &Env),
        memory: &mut MemoryStage,
        row: RowMeasure,
    ) -> (Vec2, Vec<(craft_behavior::ColumnKey, f32)>) {
        let ctx = ui.ctx().clone();
        let RowMeasure { at, rect, cells, proposal } = row;
        let len = self.source.len();
        let key = self.source.key(at);
        let item_id = sub(id, ("key", key_hash(&key)));
        let position =
            SetPosition::new(at - self.source.section_stops_before(at)).of(len - self.source.section_stops_before(len)).columns(cells.len());
        let columns: Vec<craft_behavior::ColumnKey> = cells.iter().map(|(key, _)| key.clone()).collect();
        let column_count = cells.len();
        let mut icx = ItemCx {
            key: &key,
            at,
            rect,
            row: RowState::default(),
            depth: self.source.depth(at),
            set: position,
            row_index: at,
            row_count: len,
            column_count,
            env: view,
            id: sub(item_id, "item"),
            cells,
            grid: None,
            tip: None,
            openers: Vec::new(),
            item: item_id,
            memory,
        };
        let item = (self.item)(&key, &mut icx);
        let mut item = Props::id(comp.dress(item, &RowState::default()), item_id);
        let facts = ItemFacts {
            item: item_id,
            flags: crate::style::Flags::empty(),
            position,
            columns,
            toggles: self.header.toggle_columns().map(|(key, _)| key).collect(),
            toggle_keys: Vec::new(),
            aligns: self.header.aligns(),
            pressed: None,
            menu_open: false,
        };
        let env = Env::install_for(
            ui,
            installed.derive_local(|scoped| {
                scoped.item = Some(item_id);
                scoped.row = Some(rows::row_group(item_id));
                scoped.item_facts = Some(std::sync::Arc::new(facts));
            }),
        );
        let measured = crate::runtime::measure::cell_widths(&ctx, || item.measure_in(ui, proposal));
        Env::restore(&ctx, env);
        measured
    }
}

/// A row to build and measure (`measure_row`): its position, rect and column cells, and the proposal it is measured at.
pub(super) struct RowMeasure {
    pub(super) at: usize,
    pub(super) rect: Rect,
    pub(super) cells: Vec<(craft_behavior::ColumnKey, Rect)>,
    pub(super) proposal: Proposal,
}

/// The inputs a kept measure of the items serves under.
#[derive(Clone, PartialEq)]
struct ItemsInputs {
    room: Vec2,
    count: usize,
    /// The rows stamp and the source's revision.
    content: (Option<u64>, Option<u64>),
    dress: u64,
    env: EnvFacts,
}
