//! `BodyLayout` and the layout of a body: the physical egui layout, the measured placement of `Row` End and
//! Center and of each row child on the row's line, and the cursors of `Grid` and of a `Row` that spreads its children.

use crate::runtime::egui::memory::Service;
use crate::runtime::egui::{alloc, child};
use crate::runtime::env::{Env, Shrink};
use crate::runtime::layout_memory;
use crate::runtime::measure;
use crate::runtime::memory::sub;
use craft_behavior::{Axis, ColumnWidth, Flow, GridCursor, GridMeasure};
use egui::{Id, Rect, Ui, Vec2};
use std::borrow::Cow;
use std::collections::HashMap;

/// How a body lays out its children, in reading order: mirrored under a right-to-left direction.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BodyLayout {
    /// Children one under another.
    Stack,
    /// Children side by side. `wrap` continues on a new line; `align` places the block along the row.
    #[non_exhaustive]
    Row {
        /// Whether children continue on a new line.
        wrap: bool,
        /// Where the block of children sits along the row.
        align: RowAlign,
    },
    /// Children row-major over `columns`: a child in flow takes the next cell.
    #[non_exhaustive]
    Grid {
        /// The width rule of each column.
        columns: Cow<'static, [ColumnWidth]>,
    },
}

/// Where a `Row` places its block of children along the row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowAlign {
    /// At the start edge.
    #[default]
    Start,
    /// Centred.
    Center,
    /// At the end edge.
    End,
    /// Spread over the row: the first child at the start edge, the last at the end edge, and the room left shared
    /// equally between the children (a footer's leading slot and its buttons). One child sits at the start edge, and so
    /// do the children of a row that wraps.
    SpaceBetween,
}

impl BodyLayout {
    /// Children one under another.
    pub const fn stack() -> Self {
        BodyLayout::Stack
    }

    /// Children side by side from the start edge, with no wrap.
    pub const fn row() -> Self {
        BodyLayout::Row { wrap: false, align: RowAlign::Start }
    }

    /// Children row-major over `columns`.
    pub const fn grid(columns: &'static [ColumnWidth]) -> Self {
        BodyLayout::Grid { columns: Cow::Borrowed(columns) }
    }

    /// A `Row` that wraps; any other layout is unchanged.
    pub fn wrap(self, on: bool) -> Self {
        match self {
            BodyLayout::Row { align, .. } => BodyLayout::Row { wrap: on, align },
            other => other,
        }
    }

    /// A `Row` with `align`; any other layout is unchanged.
    pub fn align(self, align: RowAlign) -> Self {
        match self {
            BodyLayout::Row { wrap, .. } => BodyLayout::Row { wrap, align },
            other => other,
        }
    }
}

/// The cursors of the `Grid` and spread `Row` bodies running this pass, by body id.
#[derive(Default)]
struct Cursors(HashMap<Id, Cursor>);

/// The number of children that fill the main axis of each `Stack` or `Row` body measured this pass, by body id.
#[derive(Default)]
struct FillCounts(HashMap<Id, u32>);

/// How a child that fills the main axis of the body it sits in sizes along it.
pub(crate) enum MainFill {
    /// The body is being measured: the child takes no room along the axis and is counted.
    Measuring,
    /// The room it takes along the axis.
    Share(f32),
    /// Not a child that shares: it takes the room as before.
    Room,
}

/// What the child `id` of `ui`'s body that fills along `axis` takes there. Called once per such child and run.
pub(crate) fn main_fill(ui: &Ui, id: Id, axis: Axis) -> MainFill {
    let env = Env::of(ui);
    let Some((body, _)) = env.fill_body.filter(|(_, along)| *along == axis) else { return MainFill::Room };
    layout_memory::leaf_fill(ui.ctx(), id, body, axis);
    if let Some(share) = env.fill_share {
        return MainFill::Share(share);
    }
    if measure::active(ui.ctx()) {
        Service::<FillCounts>::of(ui.ctx()).with(|counts| *counts.0.entry(body).or_insert(0) += 1);
        return MainFill::Measuring;
    }
    MainFill::Room
}

/// The share a child of `ui`'s body that fills along `axis` takes there, once its body knows it: the room the child
/// is measured and shown in along that axis, in the measuring run's second sweep and in the real run alike.
pub(crate) fn fill_share(ui: &Ui, axis: Axis) -> Option<f32> {
    let env = Env::of(ui);
    env.fill_body.filter(|(_, along)| *along == axis).and(env.fill_share)
}

/// Whether `ui` runs the first sweep of a measuring run of a body that shares its main axis: a child that fills along
/// `axis` is only counted there, and its contents run in the second sweep, at its share.
pub(crate) fn counting_fills(ui: &Ui, axis: Axis) -> bool {
    let env = Env::of(ui);
    measure::active(ui.ctx()) && env.fill_body.is_some_and(|(_, along)| along == axis) && env.fill_share.is_none()
}

/// What a leaf of a shrinking row reported in the first sweep: its id, the width it took there, the width it asks for
/// in no room and the least it shrinks to.
struct Reported {
    leaf: Id,
    took: f32,
    ideal: f32,
    least: f32,
}

/// The leaves each `Row` body counting this pass reported, in order, by body id.
#[derive(Default)]
struct ShrinkReports(HashMap<Id, Vec<Reported>>);

/// Reports the widths of the leaf `leaf` of the row `row` that counts them.
pub(crate) fn report_shrink(ctx: &egui::Context, row: Id, leaf: Id, took: f32, ideal: f32, least: f32) {
    let leaf = Reported { leaf, took, ideal, least: least.min(ideal) };
    Service::<ShrinkReports>::run(ctx, |reports| reports.0.entry(row).or_default().push(leaf));
}

/// The width the shrink step of the row `row` gave its leaf `leaf` in this pass's measuring run.
pub(crate) fn shrink_cap(ctx: &egui::Context, row: Id, leaf: Id) -> Option<f32> {
    layout_memory::measured(ctx, shrink_key(row, leaf)).map(|found| found.took.x)
}

fn shrink_key(row: Id, leaf: Id) -> Id {
    sub(row, ("shrink", leaf))
}

/// The widths of the leaves of a row whose leaves, with `fixed` (its other children and gaps), ask for more than
/// `room`: the shortfall comes off the last leaf first, down to its least, then off the one before it, and so on, as
/// a flow gives each child what the ones before it left. A row that fits gives each leaf what it asks for.
fn shrink_widths(leaves: &[Reported], fixed: f32, room: f32) -> Vec<f32> {
    let mut short = fixed + leaves.iter().map(|leaf| leaf.ideal).sum::<f32>() - room;
    let mut widths: Vec<f32> = leaves.iter().map(|leaf| leaf.ideal).collect();
    for (leaf, width) in leaves.iter().zip(widths.iter_mut()).rev() {
        let cut = short.clamp(0.0, leaf.ideal - leaf.least);
        *width -= cut;
        short -= cut;
    }
    widths
}

/// Runs `body`, the children of the unwrapped `Row` body `id`, in `ui`, the block they are placed in, with the row's
/// shrink step: a row fits its room by narrowing the leaves that can shrink (elidable text, fields, selects), never
/// past the least each reports (`Proposal::Min`).
///
/// A measuring run sweeps the children once in a child that takes no room, each leaf reporting its widths, and
/// records the width `shrink_widths` gives each leaf for the rest of the pass. When a leaf took another width there,
/// it sweeps them again at those widths. The real run shows each leaf at most at its width. Every sweep runs in a
/// child of `ui` with its layout, so the children get the same ids in each, and the records a measuring run leaves
/// are the ones the real run's children read and check.
fn run_shrinking<R>(ui: &mut Ui, id: Id, mut body: impl FnMut(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let layout = *ui.layout();
    let sweep = |ui: &mut Ui, shrink: Shrink, body: &mut dyn FnMut(&mut Ui) -> R| {
        let installed = Env::install_for(ui, Env::of(ui).derive_local(|scoped| scoped.shrink = Some(shrink)));
        let inner = body(ui);
        Env::restore(&ctx, installed);
        inner
    };
    if !measure::active(&ctx) {
        return child::block(ui, None, layout, |ui| sweep(ui, Shrink::Apply(id), &mut body)).0;
    }
    let room = flow_room(ui).x;
    let first = crate::contract::keeping_ids(&ctx, || {
        let mut first = ui.new_child(egui::UiBuilder::new().layout(layout));
        Service::<ShrinkReports>::run(&ctx, |reports| reports.0.insert(id, Vec::new()));
        let inner = sweep(&mut first, Shrink::Count(id), &mut body);
        let leaves = Service::<ShrinkReports>::run(&ctx, |reports| reports.0.remove(&id)).unwrap_or_default();
        let rect = first.min_rect();
        let fixed = rect.width() - leaves.iter().map(|leaf| leaf.took).sum::<f32>();
        let widths = if room.is_finite() { shrink_widths(&leaves, fixed, room) } else { leaves.iter().map(|leaf| leaf.ideal).collect() };
        let mut again = false;
        for (leaf, width) in leaves.iter().zip(widths) {
            layout_memory::emit_record(&ctx, shrink_key(id, leaf.leaf), layout_memory::Measured::new(Vec2::new(room, 0.0), Vec2::new(width, 0.0)));
            // Half a point: less than a pixel shows at any scale the box runs.
            again |= (leaf.took - width).abs() >= 0.5;
        }
        ((inner, rect, again), again)
    });
    match first {
        (_, _, true) => child::block(ui, None, layout, |ui| sweep(ui, Shrink::Apply(id), &mut body)).0,
        (inner, rect, false) => {
            ui.advance_cursor_after_rect(rect);
            inner
        }
    }
}

/// Runs `body` as the content of the `Stack` (along y) or unwrapped `Row` (along x) body `id` in `ui`, the block
/// it is placed in, when the body is given that axis.
///
/// A measuring run sweeps the children once, the children that fill the main axis counted at no extent, and records
/// what the others and the gaps took with the number that fill. With a child that fills, it sweeps them again: the
/// others reuse what the first sweep recorded, and each child that fills is measured once, at its equal share of the
/// rest. The real run gives each the same share from that record.
fn run_sharing<R>(ui: &mut Ui, id: Id, axis: Axis, layout: egui::Layout, mut body: impl FnMut(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let env = Env::of(ui);
    let given = if axis == Axis::Y { !env.content.1 } else { !env.content.0 };
    if !given {
        return child::block(ui, None, layout, body).0;
    }
    let key = sub(id, "share");
    let along = |size: Vec2| if axis == Axis::Y { size.y } else { size.x };
    let sweep = |ui: &mut Ui, share: Option<f32>, body: &mut dyn FnMut(&mut Ui) -> R| {
        let installed = Env::install_for(
            ui,
            Env::of(ui).derive_local(|scoped| {
                scoped.fill_body = Some((id, axis));
                scoped.fill_share = share;
            }),
        );
        let inner = body(ui);
        Env::restore(&ctx, installed);
        inner
    };
    if !measure::active(&ctx) {
        let (result, _) = child::block(ui, None, layout, |ui| {
            let room = along(ui.available_rect_before_wrap().size());
            let share = layout_memory::measured(&ctx, key)
                .filter(|found| found.took.y >= 1.0 && room.is_finite())
                .map(|found| ((room - found.took.x) / found.took.y).max(0.0));
            sweep(ui, share, &mut body)
        });
        return result;
    }
    // The first sweep runs in a child that takes no room yet: with a child that fills, the second sweep replaces it
    // under the same ids.
    let first = crate::contract::keeping_ids(&ctx, || {
        let mut first = ui.new_child(egui::UiBuilder::new().layout(layout));
        let room = along(first.available_rect_before_wrap().size());
        Service::<FillCounts>::run(&ctx, |counts| counts.0.insert(id, 0));
        let inner = sweep(&mut first, None, &mut body);
        let fills = Service::<FillCounts>::run(&ctx, |counts| counts.0.remove(&id)).unwrap_or(0);
        let fixed = along(first.min_rect().size());
        layout_memory::emit_record(&ctx, key, layout_memory::Measured::new(Vec2::ZERO, Vec2::new(fixed, fills as f32)));
        let share = (fills > 0 && room.is_finite()).then(|| ((room - fixed) / fills as f32).max(0.0));
        let rect = first.min_rect();
        ((inner, rect, share), share.is_some())
    });
    match first {
        (_, _, Some(share)) => child::block(ui, None, layout, |ui| sweep(ui, Some(share), &mut body)).0,
        (inner, rect, None) => {
            ui.advance_cursor_after_rect(rect);
            inner
        }
    }
}

/// What places the children in flow of a body that places them itself.
enum Cursor {
    Grid(GridCursor),
    Spread(Spread),
}

impl Cursor {
    fn cell(&mut self, size: Vec2) -> Rect {
        match self {
            Cursor::Grid(grid) => grid.cell(size),
            Cursor::Spread(row) => row.cell(size),
        }
    }

    fn room(&self) -> Vec2 {
        match self {
            Cursor::Grid(grid) => grid.room(),
            Cursor::Spread(row) => row.room(),
        }
    }

    fn region(&self) -> Rect {
        match self {
            Cursor::Grid(grid) => grid.region(),
            Cursor::Spread(row) => row.region(),
        }
    }

    fn fill_room(&self) -> Vec2 {
        match self {
            Cursor::Grid(grid) => grid.fill_room(),
            Cursor::Spread(row) => row.room(),
        }
    }

    fn in_content_column(&self) -> bool {
        matches!(self, Cursor::Grid(grid) if grid.in_content_column())
    }
}

/// The cursor of a `SpaceBetween` row: each child in flow takes the next place along the row, and the room the
/// children left in the measuring run is shared between them. It places from this pass's measuring run, as `End` and `Center` do.
struct Spread {
    flow: Flow,
    gap: f32,
    /// The space between two children: the gap and the share of the room left.
    between: f32,
    /// How far from the start edge the last child placed ends.
    at: f32,
    /// The width of the last child placed.
    last: f32,
    /// What the children took this pass at the gap alone: their widths and the gaps between them.
    natural: f32,
    count: usize,
}

impl Spread {
    /// A cursor over `flow` with `gap` between children, spreading by `last`: the natural width and the number of
    /// the children the measuring run found.
    fn new(flow: Flow, gap: f32, last: Option<Vec2>) -> Self {
        let width = flow.rect().width();
        let between = last.filter(|last| last.y >= 2.0 && last.x < width).map_or(gap, |last| gap + (width - last.x) / (last.y - 1.0));
        Spread { flow, gap, between, at: 0.0, last: 0.0, natural: 0.0, count: 0 }
    }

    fn start(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.at + self.between }
    }

    fn cell(&mut self, size: Vec2) -> Rect {
        let start = self.start();
        let span = self.flow.span(start, size.x);
        self.natural += size.x + if self.count == 0 { 0.0 } else { self.gap };
        (self.at, self.last, self.count) = (start + size.x, size.x, self.count + 1);
        Rect::from_min_size(span.min, Vec2::new(span.width(), size.y))
    }

    fn room(&self) -> Vec2 {
        let rect = self.flow.rect();
        Vec2::new((rect.width() - self.start()).max(0.0), rect.height())
    }

    fn region(&self) -> Rect {
        self.flow.span(self.start(), self.room().x)
    }

    /// What `new` spreads by next pass.
    fn measured(&self) -> Vec2 {
        Vec2::new(self.natural, self.count as f32)
    }
}

/// What a child shown in flow is, for where it sits in the height of its slot.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InFlow {
    /// Whether the child fills its cell's height in a `Grid` (declares `Sizing::Fill`).
    pub fill: bool,
    /// Whether the child is a line itself: a control, a `Row` body that does not wrap, or a collection whose rows run
    /// along X. Anything else is a block or a column.
    pub line: bool,
}

impl InFlow {
    /// A control on the line that does not fill its cell.
    pub const LINE: InFlow = InFlow { fill: false, line: true };
}

/// The rect a child of measured `size` takes in flow in `ui`: the next cell of the `Grid` or spread `Row` body that
/// encloses it directly, else the next allocation of `ui`'s layout. A direct child of an unwrapped `Row` takes the
/// row's line: a line sits at its vertical centre, a block or column at its top. In a `Grid` cell a child that
/// `fill`s takes the cell's whole height from the cell's top, so the cards of one row share their top and bottom
/// edges; any other child sits at the vertical centre of its cell.
#[doc(hidden)]
pub fn place_in_flow(ui: &mut Ui, size: Vec2) -> Rect {
    place_in_cell(ui, size, InFlow::LINE)
}

/// `place_in_flow` for a child that is `child`.
pub(crate) fn place_in_cell(ui: &mut Ui, size: Vec2, child: InFlow) -> Rect {
    let env = Env::of(ui);
    let slot = Vec2::new(size.x, env.row_line.map_or(size.y, |line| line.max(size.y)));
    let (rect, in_cell) = take_in(ui, env.grid, slot);
    if child.fill && in_cell {
        return rect;
    }
    let spare = (rect.height() - size.y).max(0.0);
    let (above, below) = if env.row_line.is_some() && !child.line { (0.0, spare) } else { (spare / 2.0, spare / 2.0) };
    Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + above), egui::pos2(rect.max.x, rect.max.y - below))
}

/// Takes `size` in flow in `ui`: the next cell of the `Grid` or spread `Row` body that encloses it directly, else the
/// next allocation of `ui`'s layout. Returns the rect taken and whether it is a cell.
pub(crate) fn take_in_flow(ui: &mut Ui, size: Vec2) -> (Rect, bool) {
    take_in(ui, Env::of(ui).grid, size)
}

/// `take_in_flow`, with `grid` the body of `ui`'s env that places its children itself.
fn take_in(ui: &mut Ui, grid: Option<Id>, size: Vec2) -> (Rect, bool) {
    let cell = grid.and_then(|body| Service::<Cursors>::of(ui.ctx()).with(|cursors| cursors.0.get_mut(&body).map(|cursor| cursor.cell(size))));
    match cell {
        Some(cell) => {
            alloc::advance_past(ui, cell);
            (cell, true)
        }
        None => (alloc::space(ui, size), false),
    }
}

/// The region the next child shown in flow in `ui` measures and is placed in: that of the next cell of the `Grid` or
/// spread `Row` body that encloses `ui` directly, else the rest of `ui`'s layout.
pub(crate) fn flow_region(ui: &Ui) -> Rect {
    let cell = Env::of(ui).grid.and_then(|body| Service::<Cursors>::of(ui.ctx()).with(|cursors| cursors.0.get(&body).map(Cursor::region)));
    cell.unwrap_or_else(|| ui.available_rect_before_wrap())
}

/// The room a child shown in flow in `ui` measures and sizes in: the room of the next cell of the `Grid` or spread
/// `Row` body that encloses `ui` directly, else the rest of `ui`'s layout.
pub(crate) fn flow_room(ui: &Ui) -> Vec2 {
    let cell = Env::of(ui).grid.and_then(|body| Service::<Cursors>::of(ui.ctx()).with(|cursors| cursors.0.get(&body).map(Cursor::room)));
    cell.unwrap_or_else(|| open_room(ui))
}

/// The rest of `ui`'s layout (the room offered to the container, less the earlier siblings and gaps); unbounded along the axes of the scrolling body that encloses it, whose region is only
/// as large as egui can make it.
fn open_room(ui: &Ui) -> Vec2 {
    let (x, y) = Env::of(ui).unbounded;
    let mut room = ui.available_size_before_wrap();
    // A body sized from its content offers its children the width it was offered, and a row body that placed its
    // block behind a lead the width of its band; either less what they took.
    let env = Env::of(ui);
    if let Some(offered) = env.offered.or(env.row_room) {
        room.x += offered - ui.max_rect().width();
    }
    Vec2::new(if x { f32::INFINITY } else { room.x }, if y { f32::INFINITY } else { room.y })
}

/// The room a child that fills its width fills in `ui`: `flow_room`, except that in a `Content` column of a `Grid` it
/// is the column's width, so the child never measures as the room and pushes the next column out of it.
pub(crate) fn flow_fill_room(ui: &Ui) -> Vec2 {
    let cell = Env::of(ui).grid.and_then(|body| Service::<Cursors>::of(ui.ctx()).with(|cursors| cursors.0.get(&body).map(Cursor::fill_room)));
    cell.unwrap_or_else(|| open_room(ui))
}

/// Whether the next child shown in flow in `ui` goes in a `Content` column of the `Grid` body that encloses `ui`
/// directly: like a content-sized column, a child that fills its width is placed at its measured width there.
pub(crate) fn in_grid_content_column(ui: &Ui) -> bool {
    Env::of(ui).grid.is_some_and(|body| Service::<Cursors>::of(ui.ctx()).with(|cursors| cursors.0.get(&body).is_some_and(Cursor::in_content_column)))
}

fn column_id(grid: Id, index: usize) -> Id {
    sub(grid, ("column", index))
}

fn row_id(grid: Id, index: usize) -> Id {
    sub(grid, ("row", index))
}

/// What the grid measured in this pass's measuring run.
fn measured_tracks(ctx: &egui::Context, sizing: bool, grid: Id, columns: usize) -> GridMeasure {
    let rows = this_pass(ctx, sizing, sub(grid, "rows")).map_or(0, |count| count.x as usize);
    GridMeasure::new(
        (0..columns).map(|i| this_pass(ctx, sizing, column_id(grid, i)).map_or(0.0, |size| size.x)).collect(),
        (0..rows).map(|i| this_pass(ctx, sizing, row_id(grid, i)).map_or(0.0, |size| size.y)).collect(),
    )
}

/// What this pass's measuring run found under `id`. A measuring run itself reads nothing.
fn this_pass(ctx: &egui::Context, sizing: bool, id: Id) -> Option<Vec2> {
    if sizing { None } else { layout_memory::measured(ctx, id).map(|found| found.took) }
}

/// The line of the unwrapped `Row` body `id`: the height of its tallest child in this pass's measuring run.
fn row_line(ctx: &egui::Context, sizing: bool, id: Id) -> Option<f32> {
    this_pass(ctx, sizing, sub(id, "line")).map(|took| took.y).filter(|line| *line > 0.0)
}

/// The line an unwrapped `Row` sets its children on: its own, or the line it shares with its container's regions
/// when that is taller.
fn on_line(own: Option<f32>, shared: Option<f32>) -> Option<f32> {
    match (own, shared) {
        (Some(own), Some(shared)) => Some(own.max(shared)),
        (own, shared) => own.or(shared),
    }
}

/// Records the line of the unwrapped `Row` body `id` from what its children took in `ui`, the block they ran in; a
/// real run records nothing. The last sweep of a measuring run gives the line.
fn record_line(ui: &Ui, sizing: bool, id: Id) {
    found(ui.ctx(), sizing, sub(id, "line"), Vec2::ZERO, Vec2::new(0.0, ui.min_rect().height()));
}

/// Records what a measuring run found under `id`; a real run records nothing.
fn found(ctx: &egui::Context, sizing: bool, id: Id, room: Vec2, took: Vec2) {
    if sizing {
        layout_memory::emit_record(ctx, id, layout_memory::Measured::new(room, took));
    }
}

/// Runs `body` in a child `Ui` laid out by `layout` under `id`, in the direction of `ui`'s env. `gap` is the
/// space between the cells of a `Grid`.
///
/// Placement comes from this pass's measuring run: `Row` with `End` or `Center` places its block from the size the
/// run found, `SpaceBetween` shares the room its children left there between them, and `Grid` places its cells from
/// the widest child per column and tallest per row there. Without a measuring run they place as `Start`, spread at
/// the gap and size columns from the cells as they come.
pub(crate) fn run_body<R>(ui: &mut Ui, id: Id, layout: &BodyLayout, gap: Vec2, body: impl FnMut(&mut Ui) -> R) -> R {
    run_body_measured(ui, id, layout, gap, None, body).0
}

/// `run_body`, with the width the children take at the gap alone for a `Row` that places its block itself (`End`,
/// `Center`, `SpaceBetween`): the width the body asks for, where the room it is placed in spans wider. `shared` is the
/// line the body shares with its container's inline header or end region: an unwrapped `Row` sets its children on it.
pub(crate) fn run_body_measured<R>(
    ui: &mut Ui,
    id: Id,
    layout: &BodyLayout,
    gap: Vec2,
    shared: Option<f32>,
    mut body: impl FnMut(&mut Ui) -> R,
) -> (R, Option<f32>) {
    let rtl = Env::of(ui).direction.is_rtl();
    let ctx = ui.ctx().clone();
    let sizing = measure::active(&ctx);
    match layout {
        BodyLayout::Stack => (run_sharing(ui, id, Axis::Y, child::column_layout(rtl), body), None),
        BodyLayout::Row { wrap: false, align: RowAlign::SpaceBetween } => {
            let (inner, _) = child::block(ui, None, child::row_layout(rtl, false), |ui| {
                let flow = Flow::new(ui.available_rect_before_wrap(), rtl);
                let spread = Spread::new(flow, gap.x, this_pass(&ctx, sizing, id));
                Service::<Cursors>::run(&ctx, |cursors| cursors.0.insert(id, Cursor::Spread(spread)));
                let line = on_line(row_line(&ctx, sizing, id), shared);
                let installed = Env::install_for(
                    ui,
                    Env::of(ui).derive_local(|scoped| {
                        scoped.grid = Some(id);
                        scoped.row_line = line;
                    }),
                );
                let inner = body(ui);
                Env::restore(&ctx, installed);
                record_line(ui, sizing, id);
                inner
            });
            let measured = match Service::<Cursors>::run(&ctx, |cursors| cursors.0.remove(&id)) {
                Some(Cursor::Spread(spread)) => Some(spread.measured()),
                _ => None,
            };
            if let Some(measured) = measured {
                found(&ctx, sizing, id, ui.available_rect_before_wrap().size(), measured);
            }
            (inner, measured.map(|measured| measured.x))
        }
        BodyLayout::Row { wrap: false, align: RowAlign::Start } => {
            let room = ui.available_rect_before_wrap();
            let line = on_line(row_line(&ctx, sizing, id), shared);
            let inner = run_sharing(ui, id, Axis::X, child::row_layout(rtl, false), |ui| {
                let installed = Env::install_for(
                    ui,
                    Env::of(ui).derive_local(|scoped| {
                        scoped.row_room = Some(room.width());
                        scoped.row_line = line;
                    }),
                );
                let inner = run_shrinking(ui, id, &mut body);
                Env::restore(&ctx, installed);
                record_line(ui, sizing, id);
                inner
            });
            (inner, None)
        }
        BodyLayout::Row { wrap, align } => {
            let room = ui.available_rect_before_wrap();
            let rect = match *align {
                RowAlign::Start | RowAlign::SpaceBetween => None,
                RowAlign::Center | RowAlign::End => {
                    let last = this_pass(&ctx, sizing, id);
                    // Without a measuring run the block is placed as `Start`.
                    last.filter(|last| last.x > 0.0 && last.x < room.width()).map(|last| {
                        let spare = room.width() - last.x;
                        let lead = if *align == RowAlign::Center { spare / 2.0 } else { spare };
                        Flow::new(room, rtl).take_start(lead).1.rect()
                    })
                }
            };
            // A wrapped row has a line per wrap, so its children stay on the top of each.
            let line = if *wrap { None } else { on_line(row_line(&ctx, sizing, id), shared) };
            let (inner, took) = child::block(ui, rect, child::row_layout(rtl, *wrap), |ui| {
                let installed = Env::install_for(
                    ui,
                    Env::of(ui).derive_local(|scoped| {
                        scoped.row_room = Some(room.width());
                        scoped.row_line = line;
                    }),
                );
                let inner = if *wrap { body(ui) } else { run_shrinking(ui, id, &mut body) };
                Env::restore(&ctx, installed);
                if !*wrap {
                    record_line(ui, sizing, id);
                }
                inner
            });
            if *align != RowAlign::Start {
                found(&ctx, sizing, id, room.size(), took.size());
            }
            (inner, (*align != RowAlign::Start).then(|| took.width()))
        }
        BodyLayout::Grid { columns } => {
            let inner = child::block(ui, None, child::column_layout(rtl), |ui| {
                let top = ui.available_rect_before_wrap().top();
                let last = measured_tracks(&ctx, sizing, id, columns.len());
                let content_run = measure::active(&ctx) && Env::of(ui).content.0;
                // A measuring run under a body sized from its content gives each fill column the width its cells
                // take, as a content column: the room it would share is not the body's to claim.
                let cursor = if content_run {
                    GridCursor::sized_by_content(Flow::new(ui.available_rect_before_wrap(), rtl), columns, gap, &last.rows)
                } else {
                    GridCursor::new(Flow::new(ui.available_rect_before_wrap(), rtl), columns, gap, &last)
                };
                Service::<Cursors>::run(&ctx, |cursors| cursors.0.insert(id, Cursor::Grid(cursor)));
                let installed = Env::install_for(ui, Env::of(ui).derive_local(|scoped| scoped.grid = Some(id)));
                let inner = body(ui);
                Env::restore(&ctx, installed);
                // Cells place from last pass's rows, so a measuring run's rows all start at the top: the grid reaches
                // as far down as the rows this pass measured, whichever pass it is.
                let rows = Service::<Cursors>::run(&ctx, |cursors| match cursors.0.get(&id) {
                    Some(Cursor::Grid(cursor)) => {
                        cursor.measured().rows.iter().sum::<f32>() + gap.y * cursor.measured().rows.len().saturating_sub(1) as f32
                    }
                    _ => 0.0,
                });
                alloc::reach_down_to(ui, top + rows);
                inner
            })
            .0;
            if let Some(Cursor::Grid(cursor)) = Service::<Cursors>::run(&ctx, |cursors| cursors.0.remove(&id)) {
                let measured = cursor.measured();
                let room = ui.available_rect_before_wrap().size();
                for (i, width) in measured.columns.iter().enumerate() {
                    found(&ctx, sizing, column_id(id, i), room, Vec2::new(*width, 0.0));
                }
                for (i, height) in measured.rows.iter().enumerate() {
                    found(&ctx, sizing, row_id(id, i), room, Vec2::new(0.0, *height));
                }
                found(&ctx, sizing, sub(id, "rows"), room, Vec2::new(measured.rows.len() as f32, 0.0));
            }
            (inner, None)
        }
    }
}
