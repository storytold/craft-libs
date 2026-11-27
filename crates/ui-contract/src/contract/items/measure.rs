//! The largest item of a collection: the widest row of a content-sized collection, the cell of a collection that
//! sizes its items from their measure. Measured, recorded, kept while the items' inputs hold, and read back.

use super::{ItemsHooks, key_hash};
use super::{header, rows};
use crate::contract::body::EnvFacts;
use crate::contract::collection::RowState;
use crate::contract::run::{Measuring, ShowAt};
use crate::contract::{Collection, Internal, ItemCx, Props, Show as _};
use crate::runtime::access::SetPosition;
use crate::runtime::env::ArenaId;
use crate::runtime::env::{Env, EnvView, ItemFacts, ScrollOwner};
use crate::runtime::layout_memory;
use crate::runtime::memory::{MemoryStage, sub};
use crate::style::{Flags, SurfaceKind};
use crate::style::{StyleCx, UiTokens};
use craft_behavior::layout_memory::measure_tolerance;
use craft_behavior::{ItemLayout as _, Proposal, ScrollPos, SelectionModel as _};
use egui::{Id, Pos2, Rect, Ui, Vec2};

impl<'s, 'r, C: Collection> ItemsHooks<'s, 'r, C> {
    /// Whether the collection takes its width from its widest row: `Sizing::Content`. A row's width holds its
    /// column cells at the widths the header resolves for them.
    pub(super) fn content_sized(&self, comp: &C) -> bool {
        comp.props(Internal::new()).sizing.content_width()
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
    pub(super) fn width_from_content(&mut self, comp: &C, at: (&Ui, &Env), id: Id, room: Vec2, item: Option<Vec2>, scx: &StyleCx<'_>) -> f32 {
        let rows = if self.header.has_columns() { self.header.least_width() } else { self.largest_in(comp, at, id, room).x };
        rows.max(comp.ideal_width(item, self.source.len(), scx)).min(room.x)
    }

    /// The largest ideal size of the rows in this pass, the sum of their ideal widths, and the widest cell each column's
    /// items reported: each row is built as `content` builds it, with its column cells at the widths the header
    /// resolves with no room to share (a fixed or toggle column at its own width), and measured without allocating.
    fn largest_row(&mut self, comp: &C, (ui, env): (&Ui, &Env), id: Id) -> (Vec2, f32, Vec<(craft_behavior::ColumnKey, f32)>) {
        let ctx = ui.ctx().clone();
        crate::contract::steps::measuring(&ctx, || self.largest_row_in(comp, (ui, env), id))
    }

    /// Builds the rows once to measure the largest and records it under `sub(id, "largest")` for `extent`, when this
    /// pass's width comes from the rows, the collection `measures_items`, or a column takes its widest cell's width.
    pub(super) fn record_largest(&mut self, comp: &C, at: &Measuring<'_, '_>, id: Id) {
        let room = match at.at {
            ShowAt::Rect(rect) => rect.size(),
            ShowAt::Flow => crate::layout::flow_room(at.ui, at.place),
        };
        let room = Self::inner_room(comp, &at.env.tokens, room);
        if self.source.is_empty() || (room.x.is_finite() && !self.content_sized(comp) && !comp.measures_items() && !self.header.has_content_columns())
        {
            return;
        }
        self.header.measure(at.scx);
        self.largest_in(comp, (at.ui, at.place), id, room);
    }

    /// The largest row in the room `room`: what a record of this pass under `sub(id, "largest")` found when it serves
    /// the room (`Measured::serves`), else what the measure kept from an earlier pass gives while the items' inputs
    /// hold, else the rows built and measured now, recorded there and kept. The widest cell of each `Content` column,
    /// or its header label when wider, is recorded with it (`header::content_id`). The caller measures the header first.
    /// `env` is the env `ui`'s children are placed under.
    pub(super) fn largest_in(&mut self, comp: &C, (ui, env): (&Ui, &Env), id: Id, room: Vec2) -> Vec2 {
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
        let inputs = self.items_inputs(comp, env, room);
        if let Some(inputs) = &inputs
            && crate::runtime::measure::measuring(&ctx, || layout_memory::kept_serves(&ctx, key, inputs))
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
            let (largest, run, cells) = self.largest_row(comp, (ui, env), id);
            layout_memory::emit_record(&ctx, key, layout_memory::Measured::new(room, largest));
            layout_memory::emit_record(&ctx, run_id(id), layout_memory::Measured::new(room, Vec2::new(run, 0.0)));
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
    fn items_inputs(&self, comp: &C, env: &Env, room: Vec2) -> Option<ItemsInputs> {
        let content = (comp.items().rows_stamp, self.source.revision());
        (content.0.is_some() || content.1.is_some()).then(|| ItemsInputs {
            room,
            count: self.source.len(),
            content,
            dress: comp.dress_inputs(),
            env: EnvFacts::of(env),
        })
    }

    /// The length of the rows side by side at their ideal widths, as the last measure of the largest row in the room
    /// `room` found them (`largest_in`); 0 before one.
    pub(super) fn run_in(&self, ui: &Ui, id: Id, room: Vec2) -> f32 {
        let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(ui.ctx()));
        layout_memory::measured(ui.ctx(), run_id(id))
            .filter(|found| layout_memory::Measured::serves(room.x, found.room.x, false, found.took.x, tolerance))
            .map_or(0.0, |found| found.took.x)
    }

    fn largest_row_in(&mut self, comp: &C, (ui, env): (&Ui, &Env), id: Id) -> (Vec2, f32, Vec<(craft_behavior::ColumnKey, f32)>) {
        let ctx = ui.ctx().clone();
        let mut memory = MemoryStage::open_peek(&ctx, id);
        let cells: Vec<(craft_behavior::ColumnKey, Rect)> =
            self.header.columns(0.0).into_iter().map(|(key, width)| (key, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 0.0)))).collect();
        let mut largest = Vec2::ZERO;
        let mut run = 0.0;
        let mut widest: Vec<(craft_behavior::ColumnKey, f32)> = Vec::new();
        for at in 0..self.source.len() {
            if self.source.section_stop(at) {
                continue;
            }
            let row = RowMeasure::new(at, Rect::ZERO, cells.clone());
            let (size, cells) = self.measure_row(comp, ui, (id, env.view(), env), &mut memory, row, |measure, _, _| measure(Proposal::Ideal));
            largest = largest.max(size);
            run += size.x;
            for (column, width) in cells {
                match widest.iter_mut().find(|(held, _)| *held == column) {
                    Some((_, held)) => *held = held.max(width),
                    None => widest.push((column, width)),
                }
            }
        }
        (largest, run, widest)
    }

    /// Builds the item of `row.at` as the collection `id` builds it, with the row state `row.state` and flags
    /// `row.flags`, under `installed` (`view` is what the item builder sees), and hands `measure` a measure of it that
    /// does not allocate: its size at a proposal and the widths its column cells reported, whether the item declared
    /// a detail body, which the measure leaves out, and whether it is as wide as it measures (`rows::fits`). The item is
    /// built once however many proposals `measure` asks. Item memory goes to `memory`.
    pub(super) fn measure_row<R>(
        &mut self,
        comp: &C,
        ui: &Ui,
        (id, view, installed): (Id, EnvView<'_>, &Env),
        memory: &mut MemoryStage,
        row: RowMeasure,
        measure: impl FnOnce(&mut dyn FnMut(Proposal) -> (Vec2, Vec<(craft_behavior::ColumnKey, f32)>), bool, bool) -> R,
    ) -> R {
        let ctx = ui.ctx().clone();
        let RowMeasure { at, rect, cells, state, flags, tree } = row;
        let len = self.source.len();
        let key = self.source.key(at);
        let item_id = sub(id, ("key", key_hash(&key)));
        let position = self.row_position(at, cells.len(), tree);
        let columns: Vec<craft_behavior::ColumnKey> = cells.iter().map(|(key, _)| key.clone()).collect();
        let column_count = cells.len();
        let mut icx = ItemCx {
            key: &key,
            at,
            rect,
            row: state.clone(),
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
            detail: None,
            openers: Vec::new(),
            item: item_id,
            memory,
        };
        #[cfg(feature = "testing")]
        crate::runtime::measure::count_row_build(&ctx, item_id);
        let item = (self.item)(&key, &mut icx);
        let detail = icx.detail.is_some();
        let mut item = Props::id(comp.dress(item, &state), item_id);
        let fits = rows::fits(&item);
        let facts = ItemFacts {
            item: item_id,
            flags,
            position,
            columns,
            toggles: self.header.toggle_columns().map(|(key, _)| key).collect(),
            toggle_keys: Vec::new(),
            closed_toggles: Vec::new(),
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
        let measured = measure(&mut |proposal| crate::runtime::measure::cell_widths(&ctx, || item.measure_in(ui, proposal)), detail, fits);
        Env::restore(&ctx, env);
        measured
    }

    /// Measures the auto rows in the view the collection takes that this pass has not measured, before step 5 sizes
    /// the collection from its layout: their ideal and minimum widths in a layout of auto widths, then their heights at
    /// their widths. Each measure is recorded (`rows::record_measure`) and set in `layout`, so this pass places every
    /// row in view by its measure, and the other runs of the pass take it without measuring again. The view is the
    /// one step 5 sizes: `width` wide, as high as the rows (capped by `max_rows`), and no higher than `room` less the
    /// header when the rows scroll in a bounded room, at the offset step 4 read. It repeats while measured rows bring
    /// other unmeasured rows into view; each row is measured at most once.
    pub(super) fn measure_ahead(&mut self, comp: &C, measuring: &Measuring<'_, '_>, id: Id, layout: &mut C::ItemLayout, (width, room): (f32, Vec2)) {
        if self.source.is_empty() {
            return;
        }
        let ctx = measuring.ui.ctx().clone();
        let tolerance = measure_tolerance(crate::runtime::egui::scale::pixels_per_point(&ctx));
        let props = comp.items();
        let (row, max_rows, striped) = (props.row_height.of(measuring.scx), props.max_rows, props.striped);
        let tree = matches!(layout.nav(), craft_behavior::NavShape::Tree);
        let known = self.known(comp, true);
        let env = Self::rows_env(comp, measuring.env, id, measuring.resolved.size, comp.props(Internal::new()).surface);
        let widths = layout.auto_widths();
        let mut memory = None;
        // How many times each row was measured here: its widths once, its height again only at a width the later rows'
        // widths moved it to.
        let mut done: Vec<(usize, u8)> = Vec::new();
        loop {
            let content = layout.content_size().y as f32;
            let mut high = max_rows.map_or(content, |rows| content.min(super::hooks::rows_height(&*layout, self.source.len(), rows, row)));
            if self.scrolls && room.y.is_finite() {
                high = high.min((room.y - self.header.height).max(0.0));
            }
            let mut in_view = Vec::new();
            layout.visible(ScrollPos { x: self.offset_x, y: self.offset.max(0.0) }, Vec2::new(width, high), &mut in_view);
            in_view.sort_unstable();
            let source = &*self.source;
            // The rows of the view this pass has not measured: (row, its widths, its height).
            let todo: Vec<(usize, bool, bool)> = rows::measured_this_pass(&ctx, id, |fresh| {
                in_view
                    .into_iter()
                    .filter(|&at| layout.is_auto(at) && !source.section_stop(at) && done.iter().all(|&(held, times)| held != at || times < 2))
                    .filter_map(|at| {
                        let key = source.key(at);
                        let wide = widths && fresh(rows::measured_width_id(id, &key)).is_none();
                        let row_width = layout.rect(at, ScrollPos::default()).width();
                        let high = fresh(rows::measured_id(id, &key)).is_none_or(|size| (size.x - row_width).abs() > tolerance);
                        (wide || high).then_some((at, wide, high))
                    })
                    .collect()
            });
            if todo.is_empty() {
                return;
            }
            let memory = memory.get_or_insert_with(|| MemoryStage::open_peek(&ctx, id));
            let mut stripe: Option<(usize, usize)> = None;
            for (at, wide, high) in todo {
                match done.iter_mut().find(|(held, _)| *held == at) {
                    Some((_, times)) => *times += 1,
                    None => done.push((at, 1)),
                }
                let odd = striped && {
                    let run = self.stripe_run(stripe, at);
                    stripe = Some((at, run));
                    run.is_multiple_of(2) && !self.source.is_section(at)
                };
                let (state, flags) = self.known_row(at, &known, odd);
                let mut cells = Vec::new();
                layout.cells(at, ScrollPos::default(), &mut cells);
                let row_measure = RowMeasure { at, rect: layout.rect(at, ScrollPos::default()), cells, state, flags, tree };
                let key = self.source.key(at);
                let (width_id, height_id) = (rows::measured_width_id(id, &key), rows::measured_id(id, &key));
                let installed = (id, env.view(), &env);
                // The rows' measure is the collection's own, as its largest-row probe is: no transcript around takes it.
                // It runs as a measure; the layout takes its results in the run that called it, so the run's own
                // placement (a strip's report of its tabs) follows them.
                let row_width = layout.rect(at, ScrollPos::default()).width();
                let (widths, height, detail) = layout_memory::boundary(&ctx, true, || {
                    crate::contract::steps::measuring(&ctx, || {
                        self.measure_row(comp, measuring.ui, installed, memory, row_measure, |measure, detail, fits| {
                            // A row of auto widths takes its ideal width unless the fit over every row's widths shrinks it:
                            // its height is measured there, and again only where the fit settles on another width. A row
                            // with a detail body is as high as its leaf and its body, which the shown row records. A row
                            // sized to its content is as wide as it measures there.
                            let widths = wide.then(|| Vec2::new(measure(Proposal::Ideal).0.x, measure(Proposal::Min).0.x));
                            let row_width = widths.map_or(row_width, |widths| widths.x);
                            let height = (!detail && (high || wide)).then(|| {
                                let size = measure(Proposal::Width(row_width)).0;
                                (Vec2::new(row_width, size.y), fits.then_some(size.x))
                            });
                            (widths, height, detail)
                        })
                    })
                });
                if detail && let Some((_, times)) = done.iter_mut().find(|(held, _)| *held == at) {
                    // Its height stays unmeasured this pass: once is enough.
                    *times = 2;
                }
                if let Some(widths) = widths {
                    rows::record_measure(&ctx, id, width_id, widths);
                    layout.set_width(at, widths.x, widths.y);
                }
                if let Some((height, fit)) = height {
                    #[cfg(feature = "testing")]
                    crate::runtime::measure::count_row_measure(&ctx, height_id);
                    rows::record_measure(&ctx, id, height_id, height);
                    layout.set_height(at, height.y);
                    if rows::keep_fit(&ctx, id, key_hash(&key), fit) {
                        layout.set_fit(at, fit);
                    }
                }
            }
        }
    }

    /// The place of the row `at` in the collection's set, among `columns` columns, with its level in a tree.
    pub(super) fn row_position(&self, at: usize, columns: usize, tree: bool) -> SetPosition {
        let len = self.source.len();
        let position = SetPosition::new(at - self.source.section_stops_before(at)).of(len - self.source.section_stops_before(len)).columns(columns);
        if tree { position.level(self.source.depth(at) + 1) } else { position }
    }

    /// The env the rows lay out under, from the collection's env `env`: the collection's arena, the child size (else
    /// the collection's `size`), the child glyph role, and the surface the rows sit on (`own`, else the one the
    /// collection sits on). The rows lay out their own content: no line or first line of the body around reaches them.
    /// A collection that scrolls its rows is their scroll owner: a reveal in a row comes to it. The content step adds
    /// the app values its body gives (the row-hook contract on `ItemCx`).
    pub(super) fn rows_env(comp: &C, env: &Env, id: Id, size: craft_theme::ControlSize, own: Option<SurfaceKind>) -> Env {
        let (child_size, child_glyphs) = (comp.props(Internal::new()).child_size, comp.props(Internal::new()).child_glyphs);
        let owns = super::hooks::scrolls_rows(comp.items(), env);
        env.derive(|scoped| {
            if owns {
                scoped.scroll_owner = Some(ScrollOwner { id, view: false });
            }
            scoped.arena = Some(ArenaId::new(id));
            scoped.size = child_size.unwrap_or(size);
            if let Some(role) = child_glyphs {
                scoped.glyphs = role;
            }
            scoped.surface = SurfaceKind::under(own, scoped.surface);
            scoped.row_line = None;
            scoped.first_line = None;
        })
    }

    /// The state of the row `at` its measure reads, from `known` (the row-hook contract on `ItemCx`): selected, lead,
    /// renaming, current, open, striped (`striped`, which the caller runs along the rows), and its place among
    /// sub-rows and the last row; with the `SELECTED`, `EXPANDED` and `CURRENT` flags. The content step adds the
    /// pointer, gesture and focus state.
    pub(super) fn known_row(&self, at: usize, known: &Known<C::Key>, striped: bool) -> (RowState, Flags) {
        let key = self.source.key(at);
        let len = self.source.len();
        let state = RowState {
            selected: known.placed.holds_row(at),
            lead: known.lead.as_ref() == Some(&key),
            renaming: known.renaming.as_ref() == Some(&key),
            striped,
            current: known.current.as_ref() == Some(&key),
            open: self.source.has_children(at) && self.source.open(at),
            sub_row: self.source.sub_row(at),
            continued: at + 1 < len && self.source.sub_row(at + 1).is_some(),
            last: at + 1 == len,
            ..RowState::default()
        };
        let mut flags = Flags::empty();
        flags.set(Flags::SELECTED, state.selected);
        flags.set(Flags::EXPANDED, state.open);
        flags.set(Flags::CURRENT, state.current);
        (state, flags)
    }

    /// The facts `known_row` reads: the selection, lead and renaming row as step 4 read them (`at_read`), or as the
    /// pass leaves them.
    pub(super) fn known(&self, comp: &C, at_read: bool) -> Known<C::Key> {
        let current = comp.items().current.as_deref().and_then(|key| key.downcast_ref::<C::Key>()).cloned();
        if at_read {
            Known {
                placed: self.selection_at_read.placed(&*self.source),
                lead: self.lead_at_read.clone(),
                renaming: self.renaming_at_read.clone(),
                current,
            }
        } else {
            Known { placed: self.selection.placed(&*self.source), lead: self.lead.key.clone(), renaming: self.renaming.key.clone(), current }
        }
    }
}

/// Where the measure of the largest row of the collection `id` records the length of its rows side by side.
fn run_id(id: Id) -> Id {
    sub(id, "run")
}

/// What the state of a row known before the pass senses input comes from (`ItemsHooks::known_row`).
pub(super) struct Known<K> {
    pub(super) placed: craft_behavior::PlacedSelection,
    pub(super) lead: Option<K>,
    pub(super) renaming: Option<K>,
    pub(super) current: Option<K>,
}

/// A row to build and measure (`measure_row`): its position, rect and column cells, its state and flags, and whether
/// the collection is a tree.
pub(super) struct RowMeasure {
    pub(super) at: usize,
    pub(super) rect: Rect,
    pub(super) cells: Vec<(craft_behavior::ColumnKey, Rect)>,
    pub(super) state: RowState,
    pub(super) flags: Flags,
    pub(super) tree: bool,
}

impl RowMeasure {
    /// The row `at` at `rect` with `cells`, in the default state.
    pub(super) fn new(at: usize, rect: Rect, cells: Vec<(craft_behavior::ColumnKey, Rect)>) -> Self {
        RowMeasure { at, rect, cells, state: RowState::default(), flags: Flags::empty(), tree: false }
    }
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
