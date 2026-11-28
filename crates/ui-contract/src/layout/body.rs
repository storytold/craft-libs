//! `BodyLayout` and the layout of a body: the physical egui layout, the measured placement of `Row` End and
//! Center and of each row child on the row's line, and the cursors of `Grid` and of a `Row` that spreads its children.

use super::column_group::{ColumnGroup, Columns};
use crate::contract::InBar;
use crate::runtime::access::scope;
use crate::runtime::egui::{alloc, child};
use crate::runtime::env::{ColumnSweep, Env, RowLine, Shrink};
use crate::runtime::layout_memory;
use crate::runtime::measure;
use crate::runtime::memory::sub;
use crate::runtime::pass_state;
use craft_behavior::layout_memory::measure_tolerance;
use craft_behavior::{Axis, ColumnWidth, Flow, GridCursor, GridMeasure};
use egui::{Id, IdMap, Rect, Ui, Vec2};
use std::borrow::Cow;

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
        /// What the grid does when its columns do not fit beside each other.
        fit: GridFit,
        /// Where a cell shorter than its row stands in it.
        cells: CellAlign,
    },
}

/// Where a `Grid` sets a cell that is shorter than its row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CellAlign {
    /// In the row's middle: a form's label beside a taller control.
    #[default]
    Center,
    /// At the row's top: panels of different heights side by side whose first lines stand level (the Color Picker's
    /// field groups, its buttons beside its compare block).
    Top,
}

/// What a `Grid` does when the least widths of its columns do not fit beside each other in its room.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GridFit {
    /// The cells go one under another across the room, so no column is cut: a form that reflows in a narrow panel.
    #[default]
    Stack,
    /// The columns stay beside each other at their least widths and the grid overflows its room: a component whose
    /// arrangement is fixed (the Color Picker's channel groups), which a narrow room must not rearrange.
    Fixed,
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

    /// Children row-major over `columns`: a `'static` slice, or columns built in the pass. The cells are the body's
    /// craft children: an egui container shown in the body is no cell, and egui places it below the cells placed so
    /// far. The rows after it are placed over it: show it as the last child, or put it in a craft body (`Row`), which
    /// takes a cell.
    pub fn grid(columns: impl Into<Cow<'static, [ColumnWidth]>>) -> Self {
        BodyLayout::Grid { columns: columns.into(), fit: GridFit::Stack, cells: CellAlign::Center }
    }

    /// A `Grid` with `fit`; any other layout is unchanged.
    pub fn fit(self, fit: GridFit) -> Self {
        match self {
            BodyLayout::Grid { columns, cells, .. } => BodyLayout::Grid { columns, fit, cells },
            other => other,
        }
    }

    /// A `Grid` with `cells`; any other layout is unchanged.
    pub fn cells(self, cells: CellAlign) -> Self {
        match self {
            BodyLayout::Grid { columns, fit, .. } => BodyLayout::Grid { columns, fit, cells },
            other => other,
        }
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
pub(crate) struct Cursors(IdMap<Cursor>);

/// The number of children that fill the main axis of each `Stack` or `Row` body measured this pass, by body id.
#[derive(Default)]
pub(crate) struct FillCounts(IdMap<u32>);

/// The memoized views running this pass whose content holds a direct child that fills the axis the body around the
/// view shares (`Scoped::fill_view`), by view id, with that axis.
#[derive(Default)]
pub(crate) struct ViewFills(IdMap<Axis>);

/// What makes a child of a content-sized `Stack` column a held child: it measures at its own width while the column
/// settles its width from the others, and the column's second sweep gives it the settled width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeldKind {
    /// A fill-width child: it shows across the width the column settles on.
    Fill,
    /// A picture whose height follows from its width that took its least for want of room
    /// (`MeasureCx::picture_size`): it shows at the width it sizes at in the column's width, which may be narrower than
    /// the column.
    Picture,
}

/// The held children of each content-sized `Stack` column in its measuring run, by column id: each child's kind and
/// the narrowest width it measured at in the column's first sweep, by child id.
#[derive(Default)]
pub(crate) struct HeldWidths(IdMap<IdMap<(HeldKind, f32)>>);

/// Reports a held child `child` of the content-sized column `column` in its first sweep: its kind and the `width` it
/// measured at. A column with no first sweep open takes no report.
pub(crate) fn report_held(ctx: &egui::Context, column: Id, child: Id, kind: HeldKind, width: f32) {
    pass_state::with::<HeldWidths, _>(ctx, |held| {
        if let Some(children) = held.0.get_mut(&column) {
            let entry = children.entry(child).or_insert((kind, width));
            entry.1 = entry.1.min(width);
        }
    });
}

/// The kind of the held child `child` of the column `column`, as the column's first sweep reported it, while the
/// column's measuring run runs.
pub(crate) fn held_kind(ctx: &egui::Context, column: Id, child: Id) -> Option<HeldKind> {
    pass_state::with::<HeldWidths, _>(ctx, |held| held.0.get(&column).and_then(|children| children.get(&child)).map(|(kind, _)| *kind))
}

/// What the first child a measuring sweep placed in flow in each open target reported (`Scoped::first_line`): the y
/// of its first line's centre, or `None` for a child with no line of its own. A container's measuring sweep opens and
/// closes its header's and body's targets; a body's sweep starts its own target afresh.
#[derive(Default)]
pub(crate) struct FirstLines(IdMap<Option<f32>>);

/// What the children of each unwrapped `Row` body took in the sweep of a measuring run running now, by body id.
#[derive(Default)]
pub(crate) struct RowLines(IdMap<LineTook>);

/// What the children of a row took about its line.
#[derive(Clone, Copy, Default)]
pub(crate) struct LineTook {
    /// The lowest centre among the one-line children, from their tops.
    centre: Option<f32>,
    /// How far the one-line children reach below their centre, at most.
    below: f32,
    /// The height of the tallest child of more than one line, or with no line of its own.
    block: f32,
}

impl LineTook {
    fn add(&mut self, first: Option<f32>, height: f32) {
        match first {
            Some(first) => {
                self.centre = Some(self.centre.map_or(first, |centre| centre.max(first)));
                self.below = self.below.max(height - first);
            }
            None => self.block = self.block.max(height),
        }
    }

    /// The height of the row once every child sits on the line.
    fn height(self) -> f32 {
        self.centre.map_or(0.0, |centre| centre + self.below).max(self.block)
    }
}

/// The children of each wrapping `Row` body in the sweep or run under way, by body id.
#[derive(Default)]
pub(crate) struct WrapRows(IdMap<WrapTook>);

/// What the children of a wrapping row took so far, and the lines the real run sets them on.
#[derive(Default)]
struct WrapTook {
    /// Each child placed so far: its line, counted from the row's first, and its height.
    children: Vec<(usize, f32)>,
    /// The top of the last child's slot.
    top: Option<f32>,
    /// In a real run, each child's line and that line's height in this pass's measuring run, while every child so far
    /// went on the line it went on there: `None` in a measuring run, without one, and once a child did not.
    planned: Option<Vec<(usize, f32)>>,
}

/// Starts the children of the wrapping `Row` body `id` afresh; a real run reads the lines this pass's measuring run
/// found for them.
fn open_wraps(ctx: &egui::Context, sizing: bool, id: Id) {
    let count = this_pass(ctx, sizing, sub(id, "wraps")).map_or(0, |count| count.x as usize);
    let planned =
        (count > 0).then(|| (0..count).map(|i| this_pass(ctx, sizing, sub(id, ("wrap", i))).map(|at| (at.x as usize, at.y))).collect()).flatten();
    pass_state::with::<WrapRows, _>(ctx, |rows| rows.0.insert(id, WrapTook { planned, ..WrapTook::default() }));
}

/// Ends the children of the wrapping `Row` body `id`, `room` wide; a measuring run records each child's line and the
/// height of its tallest child.
fn close_wraps(ctx: &egui::Context, sizing: bool, id: Id, room: Vec2) {
    let took = pass_state::with::<WrapRows, _>(ctx, |rows| rows.0.remove(&id)).unwrap_or_default();
    if !sizing {
        return;
    }
    let mut heights: Vec<f32> = Vec::new();
    for &(line, height) in &took.children {
        heights.resize(heights.len().max(line + 1), 0.0);
        if let Some(tallest) = heights.get_mut(line) {
            *tallest = tallest.max(height);
        }
    }
    found(ctx, true, sub(id, "wraps"), room, Vec2::new(took.children.len() as f32, 0.0));
    for (i, &(line, _)) in took.children.iter().enumerate() {
        found(ctx, true, sub(id, ("wrap", i)), room, Vec2::new(line as f32, heights.get(line).copied().unwrap_or_default()));
    }
}

/// Whether egui puts the next child `width` wide in `ui`, a wrapping row's block, on a new line (`Layout::next_frame`):
/// the line holds something already and the child does not fit the rest of it.
fn wraps(ui: &Ui, width: f32) -> bool {
    let (cursor, max) = (ui.cursor(), ui.max_rect());
    let begun = if ui.layout().main_dir() == egui::Direction::RightToLeft { cursor.right() < max.right() } else { max.left() < cursor.left() };
    begun && ui.available_rect_before_wrap().width() < width
}

/// Counts the next child of `size` placed in `ui`, the block of the wrapping `Row` body that places the children under
/// `env`, on its line: the line egui sets it on, or a line another allocation started. Returns the height of that line
/// in this pass's measuring run while each child so far went on the line it went on there; `None` outside a wrapping
/// row, in a measuring run, and once the lines differ.
fn wrap_line(ui: &Ui, env: &Env, size: Vec2) -> Option<f32> {
    let body = env.flow_body.filter(|_| ui.layout().main_wrap)?;
    let (starts, top) = (wraps(ui, size.x), ui.cursor().top());
    pass_state::with::<WrapRows, _>(ui.ctx(), |rows| {
        let took = rows.0.get_mut(&body)?;
        let line = match (took.children.last(), took.top) {
            (Some(&(last, _)), Some(was)) if starts || top > was + ONE_LINE_SLACK => last + 1,
            (Some(&(last, _)), _) => last,
            (None, _) => 0,
        };
        let i = took.children.len();
        took.children.push((line, size.y));
        let planned = took.planned.as_ref().and_then(|planned| planned.get(i)).filter(|(at, _)| *at == line).map(|(_, height)| *height);
        if planned.is_none() {
            took.planned = None;
        }
        planned
    })
}

/// Notes the top of the `slot` a child took in `ui`, the block of the wrapping `Row` body that places the children
/// under `env`.
fn wrap_took(ui: &Ui, env: &Env, slot: Rect) {
    if let Some(body) = env.flow_body.filter(|_| ui.layout().main_wrap) {
        pass_state::with::<WrapRows, _>(ui.ctx(), |rows| rows.0.get_mut(&body).map(|took| took.top = Some(slot.top())));
    }
}

/// Opens the first-line targets `targets` of a container's measuring sweep (`Scoped::first_line`).
pub(crate) fn open_first_lines(ctx: &egui::Context, targets: &[Id]) {
    pass_state::with::<FirstLines, _>(ctx, |lines| {
        for target in targets {
            lines.0.remove(target);
        }
    });
}

/// Closes `target`: what the first child placed in flow there reported, `None` when nothing was.
pub(crate) fn close_first_line(ctx: &egui::Context, target: Id) -> Option<Option<f32>> {
    pass_state::with::<FirstLines, _>(ctx, |lines| lines.0.remove(&target))
}

/// Reports a child's first line `at` to `target`, unless an earlier child of the sweep did.
fn report_first_line(ctx: &egui::Context, target: Id, at: Option<f32>) {
    pass_state::with::<FirstLines, _>(ctx, |lines| {
        lines.0.entry(target).or_insert(at);
    });
}

/// The bodies whose flow has placed nothing yet in the sweep or run under way, by body id, each with whether a rule
/// precedes the body's start (`Scoped::after_rule`).
#[derive(Default)]
pub(crate) struct FlowStarts(IdMap<bool>);

/// Where the next child placed in flow under `env` starts: `Some(edge)` while the body that places it has placed
/// nothing yet in this sweep or run, with `after_rule` when a rule precedes that body's start (`Scoped::after_rule`);
/// `None` once a child took room there, and outside a body.
pub(crate) fn flow_start(ctx: &egui::Context, env: &Env) -> Option<bool> {
    let body = env.flow_body?;
    pass_state::with::<FlowStarts, _>(ctx, |starts| starts.0.get(&body).copied())
}

/// Whether the next child placed in flow under `env` starts its body's flow (`flow_start`).
pub(crate) fn at_flow_start(ctx: &egui::Context, env: &Env) -> bool {
    flow_start(ctx, env).is_some()
}

/// A child took room in the flow of the body that places the children under `env`: its flow start closes.
pub(crate) fn close_flow_start(ctx: &egui::Context, env: &Env) {
    if let Some(body) = env.flow_body {
        pass_state::with::<FlowStarts, _>(ctx, |starts| starts.0.remove(&body));
    }
}

/// What the labelled rows of a bracketed `Row` (`ContainerProps::bracket`) reported of their controls in the sweep or
/// run under way, and the rect of the row's last child in flow, by the row's layout id.
#[derive(Default)]
pub(crate) struct Brackets(IdMap<BracketTook>);

/// The controls of a bracket's rows and the child after them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BracketTook {
    /// The centre line of the first row's control.
    pub(crate) first: Option<f32>,
    /// The centre line of the last row's control.
    pub(crate) last: Option<f32>,
    /// How far the rows' controls reach toward the row's end edge, at most: right, or left under right-to-left.
    pub(crate) end: Option<f32>,
    /// The rect of the row's last child placed in flow: the control in the bracket's middle.
    pub(crate) middle: Option<Rect>,
}

impl BracketTook {
    /// The centre line between the first and the last row's controls, once two rows reported.
    pub(crate) fn centre(&self) -> Option<f32> {
        self.first.zip(self.last).filter(|(first, last)| last > first).map(|(first, last)| (first + last) / 2.0)
    }
}

/// Reports the `control` rect of a labelled row to the bracketed row `row`, its end edge `rtl`-aware.
pub(crate) fn report_bracket_control(ctx: &egui::Context, row: Id, control: Rect, rtl: bool) {
    pass_state::with::<Brackets, _>(ctx, |brackets| {
        if let Some(took) = brackets.0.get_mut(&row) {
            let centre = control.center().y;
            took.first.get_or_insert(centre);
            took.last = Some(centre);
            let end = if rtl { -control.min.x } else { control.max.x };
            took.end = Some(took.end.map_or(end, |was: f32| was.max(end)));
        }
    });
}

/// What the rows of the bracketed row `row` reported in the run that just ended; `None` outside a bracketed row.
pub(crate) fn take_bracket(ctx: &egui::Context, row: Id) -> Option<BracketTook> {
    pass_state::with::<Brackets, _>(ctx, |brackets| brackets.0.remove(&row))
}

/// How a child that fills the main axis of the body it sits in sizes along it.
enum MainFill {
    /// The body is being measured: the child takes no room along the axis and is counted.
    Measuring,
    /// The room it takes along the axis, and the child's least along it.
    Share(f32, f32),
    /// Not a child that shares: it takes the room as before; the child's least along the axis.
    Room(f32),
}

/// The size the child `id` of `ui`'s body takes along `axis`: one fill rule for leaves, containers and the
/// collection's fill width. On a bounded axis `Room(least)` gives `least.max(offered)` and `Share(s, least)` gives
/// `least.max(s)`; on an axis the body scrolls or sizes from its content it is `measured` (`measured.max(share)` in a
/// scrolled share); a fill child along y in a content-height row line takes `measured.max(line)`.
///
/// `given` is the caller's own fact: whether the child is given `axis` (a leaf where its body does not size from its
/// content, a container given the axis or held across its column, a collection where it fills). `offered` is the room
/// the flow offers it along `axis`, `measured` what it measured. `env` is the env `ui`'s children are placed under.
pub(crate) fn fill_size(ui: &Ui, env: &Env, id: Id, axis: Axis, measured: f32, offered: f32, given: bool) -> f32 {
    // Across an unwrapped row's content-height line a child that fills is shown across the line the row settled on in
    // its measuring run, where the child measured at its own size.
    if axis == Axis::Y
        && env.content.1
        && let Some(line) = env.row_line.and_then(|line| line.height)
    {
        return measured.max(line);
    }
    if !given {
        return measured;
    }
    if let Some((view, along)) = env.fill_view.filter(|(_, along)| *along == axis) {
        pass_state::with::<ViewFills, _>(ui.ctx(), |fills| fills.0.insert(view, along));
    }
    // In a body that shares the axis and scrolls along it, a share never cuts the child's content: what is past the
    // view scrolls.
    if env.fill_body.is_some_and(|(_, along)| along == axis) && view_room(ui, env, axis).is_some() {
        return match main_fill(ui, env, id, axis) {
            MainFill::Measuring => 0.0,
            MainFill::Share(share, _) => measured.max(share),
            MainFill::Room(_) => measured,
        };
    }
    if !offered.is_finite() {
        return measured;
    }
    match main_fill(ui, env, id, axis) {
        MainFill::Measuring => 0.0,
        MainFill::Share(share, least) => least.max(share),
        MainFill::Room(least) => least.max(offered),
    }
}

/// What the child `id` of `ui`'s body that fills along `axis` takes there, with its least along it
/// (`layout_memory::fill_least`). Called once per such child and run.
fn main_fill(ui: &Ui, env: &Env, id: Id, axis: Axis) -> MainFill {
    let Some((body, _)) = env.fill_body.filter(|(_, along)| *along == axis) else { return MainFill::Room(0.0) };
    layout_memory::leaf_fill(ui.ctx(), id, body, axis);
    if let Some(share) = env.fill_share {
        return MainFill::Share(share, layout_memory::fill_least(ui.ctx(), id, axis));
    }
    if measure::active(ui.ctx()) {
        count_fill(ui.ctx(), body);
        return MainFill::Measuring;
    }
    MainFill::Room(layout_memory::fill_least(ui.ctx(), id, axis))
}

/// Counts one child that fills the main axis of the sharing body `body`, in the first sweep of its measuring run.
fn count_fill(ctx: &egui::Context, body: Id) {
    pass_state::with::<FillCounts, _>(ctx, |counts| *counts.0.entry(body).or_insert(0) += 1);
}

/// Counts the memoized view whose content fills the main axis of the sharing body that places children under `env` as
/// one child that fills it, in the first sweep of the body's measuring run (`counting_fills`), as a child shown there
/// directly is counted.
pub(crate) fn count_fill_view(ctx: &egui::Context, env: &Env) {
    if let Some((body, _)) = env.fill_body {
        count_fill(ctx, body);
    }
}

/// Opens the record of whether the content of the memoized view `view` fills the axis its body shares, before its run.
pub(crate) fn open_view_fill(ctx: &egui::Context, view: Id) {
    pass_state::with::<ViewFills, _>(ctx, |fills| fills.0.remove(&view));
}

/// The axis the content of the memoized view `view` filled in the run that just ended, as `fill_size` noted it; closes
/// the record.
pub(crate) fn take_view_fill(ctx: &egui::Context, view: Id) -> Option<Axis> {
    pass_state::with::<ViewFills, _>(ctx, |fills| fills.0.remove(&view))
}

/// The share a child placed under `env` that fills its body along `axis` takes there, once its body knows it: the room
/// the child is measured and shown in along that axis, in the measuring run's second sweep and in the real run alike.
pub(crate) fn fill_share(env: &Env, axis: Axis) -> Option<f32> {
    env.fill_body.filter(|(_, along)| *along == axis).and(env.fill_share)
}

/// Whether the children placed under `env` run in the first sweep of a measuring run of a body that shares its main
/// axis: a child that fills along `axis` is only counted there, and its contents run in the second sweep, at its share.
pub(crate) fn counting_fills(ctx: &egui::Context, env: &Env, axis: Axis) -> bool {
    measure::active(ctx) && env.fill_body.is_some_and(|(_, along)| along == axis) && env.fill_share.is_none()
}

/// What a unit of a shrinking row reported in the first sweep: its id, the width it took there, the width it asks for
/// in no room, the least at which its content shows whole and the least it shrinks to, and whether its content is
/// made to be cut (`Component::cut_first`). A body unit reports what it took as all three; the row asks it for its
/// least (`Shrink::Least`) only when the row is short. A levelled unit (`levelled_unit`) reports its width at each of
/// its levels, widest first, in `levels`; a continuous unit leaves it empty.
struct Reported {
    leaf: Id,
    took: f32,
    widths: ShrinkUnit,
    whole: f32,
    body: bool,
    cut_first: bool,
    /// The rank by which the unit leaves whole before the cut-first drops; `None` never leaves by rank.
    rank: Option<u8>,
    levels: Vec<f32>,
}

/// The widths of a unit of a row that shrinks to fit: the width it asks for and the least it narrows to.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShrinkUnit {
    /// The width the unit takes where the row has room.
    pub ideal: f32,
    /// The least width the unit narrows to.
    pub least: f32,
}

impl ShrinkUnit {
    /// A unit that asks for `ideal` and narrows to `least`.
    pub const fn new(ideal: f32, least: f32) -> Self {
        ShrinkUnit { ideal, least }
    }
}

/// The units each `Row` body counting this pass reported, in order, by body id.
#[derive(Default)]
pub(crate) struct ShrinkReports(IdMap<Vec<Reported>>);

/// The least widths the body units of each `Row` body asking for them this pass reported, by row id.
#[derive(Default)]
pub(crate) struct LeastReports(IdMap<IdMap<f32>>);

/// How much the leaves that fill each `Row` body under a label need at their least beyond what they took in its first
/// sweep (less where they took more), by row id: the row adds it to its units' least widths as the least its label
/// leaves it.
#[derive(Default)]
pub(crate) struct FillLeasts(IdMap<f32>);

/// How far the units of each `Row` body asked for one this pass reach past the row's room at their ideal widths, by
/// row id: `None` for a row whose units fit.
#[derive(Default)]
pub(crate) struct RowShortfalls(IdMap<Option<f32>>);

/// Asks the `Row` body `row` for its shortfall in this pass's measuring run (`take_shortfall`).
pub(crate) fn ask_shortfall(ctx: &egui::Context, row: Id) {
    pass_state::with::<RowShortfalls, _>(ctx, |asked| asked.0.insert(row, None));
}

/// How far the units of the `Row` body `row` reached past its room at their ideal widths in the measuring run since
/// `ask_shortfall`: `None` when they fit or the row did not run.
pub(crate) fn take_shortfall(ctx: &egui::Context, row: Id) -> Option<f32> {
    pass_state::with::<RowShortfalls, _>(ctx, |asked| asked.0.remove(&row).flatten())
}

/// Reports the widths of the unit `leaf` of the row `row` that counts them: a leaf's whole and least widths, or `None`
/// for a body, and how it leaves a row too narrow for it: whether its content is made to be cut, and its rank.
pub(crate) fn report_shrink(
    ctx: &egui::Context,
    row: Id,
    leaf: Id,
    took: f32,
    ideal: f32,
    whole_least: Option<(f32, f32)>,
    (cut_first, rank): (bool, Option<u8>),
) {
    let (whole, least) = whole_least.map_or((ideal, ideal), |(whole, least)| (whole.min(ideal), least.min(ideal)));
    let widths = ShrinkUnit { ideal, least: least.min(whole) };
    let leaf = Reported { leaf, took, widths, whole, body: whole_least.is_none(), cut_first, rank, levels: Vec::new() };
    pass_state::with::<ShrinkReports, _>(ctx, |reports| reports.0.entry(row).or_default().push(leaf));
}

/// Reports the widths of the levelled unit `unit` of the row `row` that counts them, one per level, widest first. The
/// unit took the first (widest) in the counting sweep.
fn report_levels(ctx: &egui::Context, row: Id, unit: Id, levels: Vec<f32>) {
    let ideal = levels.first().copied().unwrap_or_default();
    let least = levels.last().copied().unwrap_or_default().min(ideal);
    let leaf =
        Reported { leaf: unit, took: ideal, widths: ShrinkUnit { ideal, least }, whole: ideal, body: false, cut_first: false, rank: None, levels };
    pass_state::with::<ShrinkReports, _>(ctx, |reports| reports.0.entry(row).or_default().push(leaf));
}

/// The level of each unit of a row whose units show in discrete levels, for `room`. `units` holds each unit's width
/// at each of its levels, widest (level 0) first. The units give way from the end: first each steps down to its level
/// 1, then each to its level 2, and so on, stopping as soon as the row fits. Then, from the start, each unit takes back
/// a level while the row still fits: stepping a wide unit down can free room an earlier unit no longer needed to give
/// up. A unit with fewer levels stays at its last. A width that is not a number never fits, so it gives nothing back.
///
/// Worked through for units `[300, 250, 60]`, `[500, 500, 60]`, `[60]`, `[60]` in 800: the total 920 does not fit;
/// stepping to level 1 from the end leaves 870, still too wide; stepping to level 2 from the end reaches the second
/// unit (60), and the row fits at 430. Giving back from the start: the first unit takes level 0 (480); the second
/// cannot (920 > 800). The levels are `[0, 2, 0, 0]`.
pub fn plan_levels(units: &[Vec<f32>], room: f32) -> Vec<usize> {
    let width = |unit: &[f32], level: usize| unit.get(level).or(unit.last()).copied().unwrap_or_default();
    let deepest = units.iter().map(|unit| unit.len().saturating_sub(1)).max().unwrap_or_default();
    let mut levels = vec![0; units.len()];
    let mut total: f32 = units.iter().map(|unit| width(unit, 0)).sum();
    for target in 1..=deepest {
        for (unit, level) in units.iter().zip(levels.iter_mut()).rev() {
            if total <= room {
                break;
            }
            if target < unit.len() {
                total += width(unit, target) - width(unit, *level);
                *level = target;
            }
        }
    }
    for (unit, level) in units.iter().zip(levels.iter_mut()) {
        while *level > 0 {
            let next = total + width(unit, *level - 1) - width(unit, *level);
            if !next.is_finite() || next > room {
                break;
            }
            total = next;
            *level -= 1;
        }
    }
    levels
}

/// Shows `show`, one unit of the unwrapped `Row` body around it that fits in discrete levels (a ribbon group: full,
/// compact, collapsed), at the level the row's shrink step gave it. `levels` is how many levels the unit has; `show`
/// shows it at a level from 0 (the widest) to `levels - 1`, and must be free of side effects in a measuring run, as
/// every row body is: the row's counting sweep runs it once per level to measure each, in an invisible sizing pass.
/// `key` names the unit within its row and stays the same in every pass.
///
/// The row gives its levelled units their levels before its continuous units narrow (`plan_levels`, in the room the
/// continuous units leave at their least), and the continuous units narrow by their own rule in what is left. The level
/// is kept with the row's measuring run, so a pass without one (nothing the row measures changed) shows each unit once,
/// at its kept level. Outside a shrinking row the unit shows at level 0.
///
/// The unit shows on the row's line: a child of it that fills its height takes the line's height, as a direct child of
/// the row does (`fill_size`). Its children are placed in the unit's own flow, not on slots of the line, and report no
/// line to the row: a unit of two controls stays one line high. At its level it runs in the width the shrink step gave
/// that level, a bounded room even in a row that scrolls, so its content lays out as in any room of that width: a
/// centred row in it centres across its column, not in the view the row scrolls in.
pub fn levelled_unit(ui: &mut Ui, key: Id, levels: usize, mut show: impl FnMut(&mut Ui, usize)) {
    let ctx = ui.ctx().clone();
    let env = Env::of(ui);
    let shrink = env.shrink;
    let rtl = env.direction.is_rtl();
    // The unit's content is no unit of the row: it runs in a room of its own, on the row's line only to fill it. Its
    // children take no slots on the line and report nothing to the row: the unit's block is what stands in the row.
    let line = env.row_line.map(|line| RowLine { places: false, ..line });
    let own_room = env.derive_local(|scoped| {
        scoped.start_room();
        scoped.row_line = line;
    });
    match shrink {
        Some(Shrink::Count(row)) => {
            let mut widths = Vec::with_capacity(levels);
            let mut widest = Vec2::ZERO;
            // Each level measures at what it takes in no room: unbounded along x, in a region as wide as the row's,
            // and off the row's line, which a probe is not placed on. An infinite egui region would put
            // right-to-left content's start edge at infinity.
            let no_room = own_room.derive_local(|scoped| {
                scoped.unbounded.0 = true;
                scoped.row_line = None;
            });
            let width = ui.max_rect().width();
            for level in 0..levels.max(1) {
                // Every level shows the same bodies under the same ids: each measures as a probe, so no level reuses a
                // record another level left, and none serves the sweep at the levels given, which records its own.
                let (_, took) = layout_memory::probing(&ctx, || {
                    crate::runtime::egui::sizing::run_sized(ui, width, child::column_layout(rtl), |ui| {
                        Env::run_in(ui, no_room.clone(), |ui| show(ui, level))
                    })
                });
                if level == 0 {
                    widest = took;
                }
                widths.push(took.x);
            }
            // The counting sweep places the unit at its widest; the row sweeps again at the levels it gives.
            alloc::space(ui, widest);
            report_levels(&ctx, row, key, widths);
        }
        // The probe for the leasts of body units asks nothing of a levelled unit: its least is its last level.
        Some(Shrink::Least(_)) => {}
        Some(Shrink::Apply(row)) => {
            let (level, width) = level_of(&ctx, row, key);
            let level = level.min(levels.saturating_sub(1));
            match width {
                Some(width) => {
                    let bounded = own_room.derive_local(|scoped| scoped.unbounded.0 = false);
                    // Half a pixel past the width the level measured at, as a bounded re-run of a measured body is given:
                    // content that took exactly that width lays out the same here, never wrapping on a rounding.
                    let width = width + measure_tolerance(crate::runtime::egui::scale::pixels_per_point(&ctx));
                    let rect = Flow::new(ui.available_rect_before_wrap(), rtl).span(0.0, width);
                    child::block(ui, Some(rect), child::column_layout(rtl), |ui| {
                        let installed = Env::install_for(ui, bounded);
                        show(ui, level);
                        Env::restore(&ctx, installed);
                    });
                }
                None => {
                    let installed = Env::install_for(ui, own_room);
                    show(ui, level);
                    Env::restore(&ctx, installed);
                }
            }
        }
        Some(Shrink::Fit) | None => {
            let installed = Env::install_for(ui, own_room);
            show(ui, 0);
            Env::restore(&ctx, installed);
        }
    }
}

/// The level the shrink step of the row `row` gave its levelled unit `unit`, with the unit's width at that level: 0 and
/// no width without a record. The record keeps the level as the measure's height, which a row's shrink record has no
/// other use for.
fn level_of(ctx: &egui::Context, row: Id, unit: Id) -> (usize, Option<f32>) {
    // A level is a small whole number written as `level as f32`: the cast back is exact.
    layout_memory::shrink(ctx, shrink_key(row, unit))
        .map_or((0, None), |(found, _)| (found.took.y.max(0.0) as usize, Some(found.took.x).filter(|width| width.is_finite() && *width > 0.0)))
}

/// Reports the least width `least` of the leaf `leaf` that fills the row `row`, which took `took` in the row's first
/// sweep, to a row that sums its controls' least widths for its label (`FillLeasts`).
pub(crate) fn report_fill_least(ctx: &egui::Context, row: Id, took: f32, least: f32) {
    pass_state::with::<FillLeasts, _>(ctx, |reports| {
        if let Some(sum) = reports.0.get_mut(&row) {
            *sum += least - took;
        }
    });
}

/// Reports the least width of the body unit `body` of the row `row` that asks for it.
pub(crate) fn report_least(ctx: &egui::Context, row: Id, body: Id, least: f32) {
    pass_state::with::<LeastReports, _>(ctx, |reports| {
        if let Some(leasts) = reports.0.get_mut(&row) {
            leasts.insert(body, least);
        }
    });
}

/// What the shrink step of a row gave one of its units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShrinkCap {
    /// The width the unit shows at most.
    pub width: f32,
    /// The unit dropped out of the row (`shrink_or_drop`): it shows nothing and takes no place.
    pub dropped: bool,
    /// The row gave the unit less than it asks for in no room, or dropped it: what the unit shows depends on the row's
    /// room. A unit the row did not narrow shows as in no room.
    pub narrowed: bool,
}

/// What the shrink step of the row `row` gave its unit `leaf` in this pass's measuring run.
pub(crate) fn shrink_cap(ctx: &egui::Context, row: Id, leaf: Id) -> Option<ShrinkCap> {
    layout_memory::shrink(ctx, shrink_key(row, leaf)).map(|(found, given)| ShrinkCap {
        width: found.took.x,
        dropped: given.dropped,
        narrowed: given.narrowed,
    })
}

fn shrink_key(row: Id, leaf: Id) -> Id {
    sub(row, ("shrink", leaf))
}

/// Which units of a row too long for its room give way first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShrinkOrder {
    /// Every unit gives the same share of what it can give, so all reach their least together.
    #[default]
    Even,
    /// The last unit in reading order goes to its least before the one before it gives anything: a bar gives way from
    /// its end, where the members that matter least sit.
    FromEnd,
}

/// The widths of the units of a row whose units, with `fixed` (its other children and gaps), ask for more than `room`:
/// the shortfall comes off the units in `order`, never past the least each reports (its ideal less what it can give).
/// A row that fits gives each unit its ideal, and so does a row that does not fit even with every unit at its least:
/// it overflows (or its body scrolls) at its natural widths, instead of cutting its units to their least and
/// overflowing all the same.
pub fn shrink_widths(units: &[ShrinkUnit], fixed: f32, room: f32, order: ShrinkOrder) -> Vec<f32> {
    let short = fixed + units.iter().map(|unit| unit.ideal).sum::<f32>() - room;
    let slack: f32 = units.iter().map(|unit| unit.ideal - unit.least).sum();
    // Half a point: less than a pixel shows at any scale the box runs.
    let fits_at_least = short - slack < 0.5;
    if short <= 0.0 || slack <= 0.0 || !fits_at_least {
        return units.iter().map(|unit| unit.ideal).collect();
    }
    match order {
        ShrinkOrder::Even => {
            let share = (short / slack).min(1.0);
            units.iter().map(|unit| unit.ideal - (unit.ideal - unit.least) * share).collect()
        }
        ShrinkOrder::FromEnd => {
            let mut left = short;
            let mut widths: Vec<f32> = units
                .iter()
                .rev()
                .map(|unit| {
                    let given = (unit.ideal - unit.least).min(left);
                    left -= given;
                    unit.ideal - given
                })
                .collect();
            widths.reverse();
            widths
        }
    }
}

/// The widths of the units of a row too long for its room, in turns. First every unit gives the room it holds beyond
/// its content, down to its `whole` width, in `order`: a field gives its blank length before a readout beside it
/// wraps. Only when that does not cover the shortfall does content give way, from the whole widths: the units whose
/// content is made to be cut (`first`) give what they can, and the others only the rest. A row that does not fit
/// even with every unit at its least keeps the natural widths, as `shrink_widths` does.
fn shrink_in_turns(units: &[ShrinkUnit], whole: &[f32], first: &[bool], fixed: f32, room: f32, order: ShrinkOrder) -> Vec<f32> {
    let at_whole: Vec<ShrinkUnit> =
        units.iter().zip(whole).map(|(unit, whole)| ShrinkUnit { ideal: unit.ideal, least: whole.max(unit.least).min(unit.ideal) }).collect();
    let slack = |units: &[ShrinkUnit]| units.iter().map(|unit| unit.ideal - unit.least).sum::<f32>();
    let short = fixed + units.iter().map(|unit| unit.ideal).sum::<f32>() - room;
    // Half a point: less than a pixel shows at any scale the box runs.
    if short - slack(&at_whole) < 0.5 {
        return shrink_widths(&at_whole, fixed, room, order);
    }
    let cut: Vec<ShrinkUnit> = at_whole.iter().zip(units).map(|(whole, unit)| ShrinkUnit { ideal: whole.least, least: unit.least }).collect();
    if short - slack(&at_whole) - slack(&cut) >= 0.5 {
        return units.iter().map(|unit| unit.ideal).collect();
    }
    shrink_widths_in_turn(&cut, first, fixed, room, order)
}

/// The widths of the units of a row too long for its room, as `shrink_in_turns` gives them, in a row that `drops`
/// (one whose body does not scroll): when the units do not fit even at their least, the units whose content is made
/// to be cut (`first`) drop out from the end, each with one `gap`, until the rest fits at their least. A unit that
/// drops gets no width and takes no place, so nothing overflows into a neighbour; controls never drop. A row that
/// scrolls keeps every unit and scrolls instead.
///
/// Before those drops, the units that carry a rank (`ranks`) leave whole, scrolling row or not: the lowest rank first,
/// the last of a rank first, until the kept units fit at their ideal widths in `rank_room` (at most `room`). A unit
/// with no rank never leaves by rank.
fn shrink_or_drop(
    units: &[ShrinkUnit],
    whole: &[f32],
    (first, ranks): (&[bool], &[Option<u8>]),
    (fixed, gap): (f32, f32),
    (room, rank_room): (f32, f32),
    order: ShrinkOrder,
    drops: bool,
) -> Vec<ShrinkCap> {
    let mut kept = vec![true; units.len()];
    let mut fixed = fixed;
    // Half a point: less than a pixel shows at any scale the box runs.
    let fits_at = |kept: &[bool], fixed: f32, width: fn(&ShrinkUnit) -> f32, room: f32| {
        fixed + units.iter().zip(kept).filter(|(_, kept)| **kept).map(|(unit, _)| width(unit)).sum::<f32>() - room < 0.5
    };
    let fits = |kept: &[bool], fixed: f32| fits_at(kept, fixed, |unit| unit.least, room);
    while !fits_at(&kept, fixed, |unit| unit.ideal, rank_room.min(room)) {
        let Some(rank) = (0..units.len()).filter(|at| kept[*at]).filter_map(|at| ranks[at]).min() else { break };
        let Some(at) = (0..units.len()).rev().find(|at| kept[*at] && ranks[*at] == Some(rank)) else { break };
        kept[at] = false;
        fixed -= gap;
    }
    while drops && !fits(&kept, fixed) {
        let Some(at) = (0..units.len()).rev().find(|at| kept[*at] && first[*at]) else { break };
        kept[at] = false;
        fixed -= gap;
    }
    let pick = |all: &[f32]| -> Vec<f32> { all.iter().zip(&kept).filter(|(_, kept)| **kept).map(|(value, _)| *value).collect() };
    let shown: Vec<ShrinkUnit> = units.iter().zip(&kept).filter(|(_, kept)| **kept).map(|(unit, _)| *unit).collect();
    let shown_first: Vec<bool> = first.iter().zip(&kept).filter(|(_, kept)| **kept).map(|(first, _)| *first).collect();
    let mut widths = shrink_in_turns(&shown, &pick(whole), &shown_first, fixed, room, order).into_iter();
    kept.iter()
        .zip(units)
        .map(|(kept, unit)| match kept {
            true => {
                let width = widths.next().unwrap_or_default();
                ShrinkCap { width, dropped: false, narrowed: unit.ideal - width >= 0.5 }
            }
            false => ShrinkCap { width: 0.0, dropped: true, narrowed: true },
        })
        .collect()
}

/// `shrink_widths` in two turns: the units whose content is made to be cut (`first`) give what they can of the
/// shortfall, and the others give only the rest. A row that does not fit even with every unit at its least keeps the
/// natural widths, as `shrink_widths` does.
fn shrink_widths_in_turn(units: &[ShrinkUnit], first: &[bool], fixed: f32, room: f32, order: ShrinkOrder) -> Vec<f32> {
    let pick = |turn: bool| -> Vec<ShrinkUnit> { units.iter().zip(first).filter(|(_, first)| **first == turn).map(|(unit, _)| *unit).collect() };
    let (cut, rest) = (pick(true), pick(false));
    let slack = |units: &[ShrinkUnit]| units.iter().map(|unit| unit.ideal - unit.least).sum::<f32>();
    let short = fixed + units.iter().map(|unit| unit.ideal).sum::<f32>() - room;
    // Half a point: less than a pixel shows at any scale the box runs.
    if short <= 0.0 || slack(&cut) <= 0.0 || short - slack(&cut) - slack(&rest) >= 0.5 {
        return shrink_widths(units, fixed, room, order);
    }
    let ideal = |units: &[ShrinkUnit]| units.iter().map(|unit| unit.ideal).sum::<f32>();
    // The cut units take the shortfall against the others at their ideal, down to their least.
    let cut_widths =
        if short >= slack(&cut) { cut.iter().map(|unit| unit.least).collect() } else { shrink_widths(&cut, fixed + ideal(&rest), room, order) };
    let rest_widths = shrink_widths(&rest, fixed + cut_widths.iter().sum::<f32>(), room, order);
    let (mut cut_widths, mut rest_widths) = (cut_widths.into_iter(), rest_widths.into_iter());
    first.iter().map(|first| if *first { cut_widths.next() } else { rest_widths.next() }.unwrap_or_default()).collect()
}

/// Runs `body`, the children of the unwrapped `Row` body `id`, in `ui`, the block they are placed in, with the row's
/// shrink step: a row fits its room by narrowing its units, first by the room they hold beyond their content (a leaf's
/// measure past its whole width), then by their content, never past the least each reports, evenly or, in a bar, from
/// its end, and a row that cannot fit even at its units' leasts keeps their natural widths, except that a row whose
/// body does not scroll first drops its units made to be cut, from the end (`shrink_or_drop`). A unit is a leaf that
/// can shrink (elidable text, fields, selects), whose least is its `Proposal::Min`, or a body that does not fill the
/// row (a captioned field, a nested row), whose least is what its content takes unbounded with every leaf at its least.
///
/// A measuring run sweeps the children once in a child that takes no room, each unit reporting its widths. When the
/// units ask for more than the room and a body is among them, it sweeps them again as a probe (`Shrink::Least`) for
/// the bodies' leasts. It records the width `shrink_in_turns` gives each unit for the rest of the pass, and when a unit
/// took another width there, or the row probed, it sweeps them again at those widths. The real run shows each unit at
/// most at its width. Every sweep runs in a child of `ui` with its layout, so the children get the same ids in each,
/// and the records a measuring run leaves are the ones the real run's children read and check. `place` is the env
/// installed for `ui`.
fn run_shrinking<R>(ui: &mut Ui, place: &Env, id: Id, mut body: impl FnMut(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let layout = *ui.layout();
    let mut env = place.clone();
    let inherited = (env.unbounded.0, env.least);
    let rank_short = env.rank_short;
    let mut sweep = |ui: &mut Ui, shrink: Shrink, body: &mut dyn FnMut(&mut Ui) -> R| {
        let scoped = env.local_mut();
        scoped.rank_short = None;
        scoped.shrink = Some(shrink);
        let least = matches!(shrink, Shrink::Least(_));
        scoped.unbounded.0 = inherited.0 || least;
        scoped.least = inherited.1 || least;
        let installed = Env::install_for(ui, env.clone());
        let inner = body(ui);
        Env::restore(&ctx, installed);
        inner
    };
    if !measure::active(&ctx) {
        return child::block(ui, None, layout, |ui| sweep(ui, Shrink::Apply(id), &mut body)).0;
    }
    let room = shrink_room(ui, place);
    let order = if place.values.get::<InBar>().0 { ShrinkOrder::FromEnd } else { ShrinkOrder::Even };
    let (drops, gap) = (view_room(ui, place, Axis::X).is_none(), ui.spacing().item_spacing.x);
    // Records the width each unit gets, and returns whether one took another width in the first sweep. The levelled
    // units take their levels first, in the room the continuous units leave at their least; the continuous units then
    // narrow by their own rule with the levelled units' widths held. A row without levelled units holds nothing more.
    let settle = |leaves: &[Reported], fixed: f32| {
        let (levelled, continuous): (Vec<&Reported>, Vec<&Reported>) = leaves.iter().partition(|leaf| !leaf.levels.is_empty());
        let units: Vec<ShrinkUnit> = continuous.iter().map(|leaf| leaf.widths).collect();
        let whole: Vec<f32> = continuous.iter().map(|leaf| leaf.whole).collect();
        let first: Vec<bool> = continuous.iter().map(|leaf| leaf.cut_first).collect();
        let ranks: Vec<Option<u8>> = continuous.iter().map(|leaf| leaf.rank).collect();
        let level_widths: Vec<Vec<f32>> = levelled.iter().map(|leaf| leaf.levels.clone()).collect();
        let levels = if room.is_finite() {
            plan_levels(&level_widths, room - fixed - units.iter().map(|unit| unit.least).sum::<f32>())
        } else {
            vec![0; levelled.len()]
        };
        let width_at = |leaf: &Reported, level: usize| leaf.levels.get(level).copied().unwrap_or_default();
        let held: f32 = levelled.iter().zip(&levels).map(|(leaf, level)| width_at(leaf, *level)).sum();
        // The ranked units give up `rank_short` of the row's ideal width; a row offered no bound (a content-sized row's
        // measuring run) still drops them for it.
        let ideal = fixed + held + units.iter().map(|unit| unit.ideal).sum::<f32>();
        let rank_room = rank_short.map_or(f32::INFINITY, |short| ideal - short);
        let widths = if room.is_finite() || rank_room.is_finite() {
            shrink_or_drop(&units, &whole, (&first, &ranks), (fixed + held, gap), (room, rank_room), order, drops)
        } else {
            units.iter().map(|unit| ShrinkCap { width: unit.ideal, dropped: false, narrowed: false }).collect()
        };
        // A levelled unit took no place of its own content in the first sweep (it measured each level apart), so the
        // records its content's run leaves for the real run come from a sweep at the levels given.
        let mut again = !levelled.is_empty();
        for (leaf, cap) in continuous.iter().zip(widths) {
            let measured = layout_memory::Measured::new(Vec2::new(room, 0.0), Vec2::new(cap.width, 0.0));
            let given = layout_memory::ShrinkGiven { dropped: cap.dropped, narrowed: cap.narrowed };
            layout_memory::emit_shrink(&ctx, shrink_key(id, leaf.leaf), measured, given);
            // Half a point: less than a pixel shows at any scale the box runs.
            again |= (leaf.took - cap.width).abs() >= 0.5;
        }
        for (leaf, level) in levelled.iter().zip(levels) {
            // The level travels as the record's height (`level_of`).
            let measured = layout_memory::Measured::new(Vec2::new(room, 0.0), Vec2::new(width_at(leaf, level), level as f32));
            let given = layout_memory::ShrinkGiven { dropped: false, narrowed: level > 0 };
            layout_memory::emit_shrink(&ctx, shrink_key(id, leaf.leaf), measured, given);
        }
        again
    };
    // A row that is the first control under a label holds every control the label's placement must leave room for.
    let labelled = scope::label_awaiting_min(&ctx);
    let (inner, rect, mut leaves, fixed, fills, probe, again) = crate::contract::keeping_ids(&ctx, || {
        let mut first = laid_out_child(ui, layout);
        pass_state::with::<ShrinkReports, _>(&ctx, |reports| reports.0.insert(id, Vec::new()));
        if labelled.is_some() {
            pass_state::with::<FillLeasts, _>(&ctx, |reports| reports.0.insert(id, 0.0));
        }
        let inner = sweep(&mut first, Shrink::Count(id), &mut body);
        let leaves = pass_state::with::<ShrinkReports, _>(&ctx, |reports| reports.0.remove(&id)).unwrap_or_default();
        let fills = pass_state::with::<FillLeasts, _>(&ctx, |reports| reports.0.remove(&id)).unwrap_or_default();
        let rect = first.min_rect();
        let fixed = rect.width() - leaves.iter().map(|leaf| leaf.took).sum::<f32>();
        let shortfall = fixed + leaves.iter().map(|leaf| leaf.widths.ideal).sum::<f32>() - room;
        let short = shortfall >= 0.5;
        if short {
            pass_state::with::<RowShortfalls, _>(&ctx, |asked| asked.0.get_mut(&id).map(|slot| *slot = Some(shortfall)));
        }
        // A row under a label asks its bodies for their least whatever its room: the label's placement needs it.
        let probe = (labelled.is_some() || (room.is_finite() && short)) && leaves.iter().any(|leaf| leaf.body);
        let again = !probe && settle(&leaves, fixed);
        ((inner, rect, leaves, fixed, fills, probe, again), probe || again)
    });
    let again = again || probe;
    if probe {
        #[cfg(feature = "testing")]
        measure::count_least(&ctx, id);
        pass_state::with::<LeastReports, _>(&ctx, |reports| reports.0.insert(id, IdMap::default()));
        crate::contract::keeping_ids(&ctx, || {
            layout_memory::probing(&ctx, || {
                let mut least = laid_out_child(ui, layout);
                sweep(&mut least, Shrink::Least(id), &mut body);
            });
            ((), true)
        });
        let leasts = pass_state::with::<LeastReports, _>(&ctx, |reports| reports.0.remove(&id)).unwrap_or_default();
        for leaf in leaves.iter_mut().filter(|leaf| leaf.body) {
            if let Some(least) = leasts.get(&leaf.leaf) {
                leaf.widths.least = least.min(leaf.widths.ideal);
            }
        }
        settle(&leaves, fixed);
    }
    if let Some(container) = labelled {
        scope::record_row_min(&ctx, container, fixed + fills + leaves.iter().map(|leaf| leaf.widths.least).sum::<f32>());
    }
    if again {
        child::block(ui, None, layout, |ui| sweep(ui, Shrink::Apply(id), &mut body)).0
    } else {
        ui.advance_cursor_after_rect(rect);
        inner
    }
}

/// In a body that scrolls along `axis`, what its view shows along it from the start of the rest of `ui`'s layout: along
/// x the width a row there shrinks to fit and a child that fills its width fills, along y the height the children that
/// fill a `Stack` there share. `None` outside such a body.
pub(crate) fn view_room(ui: &Ui, env: &Env, axis: Axis) -> Option<f32> {
    let scrolls = |view: &crate::runtime::lazy::View| if axis == Axis::X { env.unbounded.0 && view.axes.0 } else { env.unbounded.1 && view.axes.1 };
    env.lazy.filter(scrolls).map(|view| {
        let start = view.start_of(ui.available_rect_before_wrap());
        let (length, at) = if axis == Axis::X { (view.rect.width(), start.x) } else { (view.rect.height(), start.y) };
        (length - at).max(0.0)
    })
}

/// The width the leaves of a row in `ui` shrink to fit: the row's flow room, except in a body that scrolls along x,
/// where it is what the view shows from the row's start. There the leaves narrow to their least before the body
/// scrolls, and when even their least does not fit they keep their natural widths and the body scrolls.
fn shrink_room(ui: &Ui, env: &Env) -> f32 {
    view_room(ui, env, Axis::X).unwrap_or_else(|| flow_room(ui, env).x)
}

/// Runs `body` as the content of the `Stack` (along y) or unwrapped `Row` (along x) body `id` in `ui`, the block
/// it is placed in, when the body is given that axis. A measuring run is `measure_sharing`'s; the real run gives each
/// child that fills the share its record holds. `body` gets the env installed for the `Ui` it runs in.
fn run_sharing<R>(ui: &mut Ui, env: &Env, id: Id, axis: Axis, layout: egui::Layout, mut body: impl FnMut(&mut Ui, &Env) -> R) -> R {
    let ctx = ui.ctx().clone();
    if measure::active(&ctx) {
        let (inner, rect) = measure_sharing(ui, env, id, axis, layout, &mut body);
        ui.advance_cursor_after_rect(rect);
        return inner;
    }
    if !shares_along(env, axis) {
        return child::block(ui, None, layout, |ui| Env::run_in(ui, env.clone(), |ui| body(ui, env))).0;
    }
    let key = sub(id, "share");
    child::block(ui, None, layout, |ui| {
        let room = share_room(ui, env, axis);
        let share = layout_memory::measured(&ctx, key)
            .filter(|found| found.took.y >= 1.0 && room.is_finite())
            .map(|found| ((room - found.took.x) / found.took.y).max(0.0));
        share_sweep(ui, &mut env.clone(), id, axis, share, &mut body)
    })
    .0
}

/// A child of `ui` with `layout`, built outside the generic sweeps that run a body in it, on the heap: the body's
/// children run in it.
fn laid_out_child(ui: &mut Ui, layout: egui::Layout) -> Box<Ui> {
    Box::new(ui.new_child(egui::UiBuilder::new().layout(layout)))
}

/// Whether a body given `axis` shares it among the children that fill it.
fn shares_along(env: &Env, axis: Axis) -> bool {
    if axis == Axis::Y { !env.content.1 } else { !env.content.0 }
}

/// The room along `axis` the children that fill a body in `ui` share. In a body that scrolls along the axis they share
/// what the view shows: shorter than the view, the content leaves them its rest; longer, they take their least and the
/// body scrolls (`MainFill::Share`).
fn share_room(ui: &Ui, env: &Env, axis: Axis) -> f32 {
    view_room(ui, env, axis).unwrap_or_else(|| {
        let size = ui.available_rect_before_wrap().size();
        if axis == Axis::Y { size.y } else { size.x }
    })
}

/// One sweep of the children of the sharing body `id` in `ui`, each child that fills `axis` given `share`.
fn share_sweep<R>(ui: &mut Ui, env: &mut Env, id: Id, axis: Axis, share: Option<f32>, body: &mut dyn FnMut(&mut Ui, &Env) -> R) -> R {
    let scoped = env.local_mut();
    scoped.fill_body = Some((id, axis));
    scoped.fill_share = share;
    let installed = Env::install_for(ui, env.clone());
    let inner = body(ui, env);
    Env::restore(ui.ctx(), installed);
    inner
}

/// `run_sharing`'s measuring run: its sweeps, each in a child of `ui` that takes no room, under the ids the real run
/// gives the children. Returns the last sweep's result and the rect it took, which the caller places.
///
/// The first sweep counts the children that fill at no extent and records what the others and the gaps took with the
/// number that fill. With a child that fills, a second sweep replaces it under the same ids: the others reuse what the
/// first recorded, and each child that fills is measured once, at its equal share of the rest.
fn measure_sharing<R>(ui: &mut Ui, env: &Env, id: Id, axis: Axis, layout: egui::Layout, body: &mut dyn FnMut(&mut Ui, &Env) -> R) -> (R, Rect) {
    let ctx = ui.ctx().clone();
    let in_child = |ui: &mut Ui, body: &mut dyn FnMut(&mut Ui, &Env) -> R| {
        let mut child = laid_out_child(ui, layout);
        let inner = Env::run_in(&mut child, env.clone(), |ui| body(ui, env));
        (inner, child.min_rect())
    };
    if !shares_along(env, axis) {
        return in_child(ui, body);
    }
    let key = sub(id, "share");
    let along = |size: Vec2| if axis == Axis::Y { size.y } else { size.x };
    let mut sweep_env = env.clone();
    let first = crate::contract::keeping_ids(&ctx, || {
        let (inner, rect, room) = {
            let mut first = laid_out_child(ui, layout);
            let room = share_room(&first, env, axis);
            pass_state::with::<FillCounts, _>(&ctx, |counts| counts.0.insert(id, 0));
            let inner = share_sweep(&mut first, &mut sweep_env, id, axis, None, body);
            (inner, first.min_rect(), room)
        };
        let fills = pass_state::with::<FillCounts, _>(&ctx, |counts| counts.0.remove(&id)).unwrap_or(0);
        let fixed = along(rect.size());
        layout_memory::emit_record(&ctx, key, layout_memory::Measured::new(Vec2::ZERO, Vec2::new(fixed, fills as f32)));
        let share = (fills > 0 && room.is_finite()).then(|| ((room - fixed) / fills as f32).max(0.0));
        ((inner, rect, share), share.is_some())
    });
    match first {
        (_, _, Some(share)) => in_child(ui, &mut |ui, _| share_sweep(ui, &mut sweep_env, id, axis, Some(share), body)),
        (inner, rect, None) => (inner, rect),
    }
}

/// Runs `body` as the content of the content-sized `Stack` column `id` in `ui`, the block it is placed in, whose width
/// is bounded: each sweep is `run_sharing`'s, with the column's sweep (`ColumnSweep`) in its children's env. It adds no
/// `Ui` of its own, so the children have the ids `run_sharing` gives them when the column's width is not bounded.
///
/// A held child (`HeldKind`: a fill-width child, or a picture that took its least for want of room) measures at its
/// own width while the column settles its width from the others, so it may measure narrower than it shows. A
/// measuring run sweeps the children once, each held child reporting the width it measured at (`report_held`). When
/// one of them measured narrower than the column settled on, it sweeps them again with that width, at which the held
/// children measure in the column's width and the others reuse their records; else the first sweep stands, and each
/// held child keeps the record it made at its own width, which is the column's. It records which
/// (`sub(id, "column")`): the real run gives the held children the width the column shows at only after a second
/// sweep, so each shows as its records were made.
// PLAN: the second sweep's need is decided once for the column, not per held container at body/hooks.rs's reuse
// rule: a container that reused its content-sized record there would be laid out across in the real run under records
// not made across.
fn run_column<R>(ui: &mut Ui, env: &Env, id: Id, layout: egui::Layout, mut body: impl FnMut(&mut Ui, &Env) -> R) -> R {
    let ctx = ui.ctx().clone();
    let key = sub(id, "column");
    let column_env = |column: Option<ColumnSweep>| env.derive_local(|scoped| scoped.column = column);
    let sweep = |ui: &mut Ui, column: Option<ColumnSweep>, body: &mut dyn FnMut(&mut Ui, &Env) -> R| {
        let env = column_env(column);
        let installed = Env::install_for(ui, env.clone());
        let inner = run_sharing(ui, &env, id, Axis::Y, layout, body);
        Env::restore(&ctx, installed);
        inner
    };
    if !measure::active(&ctx) {
        let across = this_pass(&ctx, false, key).is_some_and(|took| took.x > 0.0);
        let column = across.then(|| ColumnSweep::Column(id, flow_room(ui, env).x));
        return sweep(ui, column, &mut body);
    }
    let (inner, rect, again) = crate::contract::keeping_ids(&ctx, || {
        pass_state::with::<HeldWidths, _>(&ctx, |held| held.0.insert(id, IdMap::default()));
        let env = column_env(Some(ColumnSweep::Count(id)));
        let installed = Env::install_for(ui, env.clone());
        let (inner, rect) = measure_sharing(ui, &env, id, Axis::Y, layout, &mut body);
        Env::restore(&ctx, installed);
        let narrowest = pass_state::with::<HeldWidths, _>(&ctx, |held| {
            held.0.get(&id).map_or(f32::INFINITY, |children| children.values().fold(f32::INFINITY, |least, (_, width)| least.min(*width)))
        });
        // Half a point: less than a pixel shows at any scale the box runs.
        let again = narrowest < rect.width() - 0.5;
        ((inner, rect, again), again)
    });
    let column = rect.width();
    // 0 when the first sweep stands, so nothing an earlier run of the pass left is read.
    found(&ctx, true, key, Vec2::ZERO, Vec2::new(if again { column } else { 0.0 }, 0.0));
    // The second sweep reads the held children's kinds (`held_kind`); the column's record closes after it.
    let inner = if again {
        sweep(ui, Some(ColumnSweep::Column(id, column)), &mut body)
    } else {
        ui.advance_cursor_after_rect(rect);
        inner
    };
    pass_state::with::<HeldWidths, _>(&ctx, |held| held.0.remove(&id));
    inner
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

    /// The grid column the next child goes in; `None` for a spread row.
    fn grid_column(&self) -> Option<usize> {
        match self {
            Cursor::Grid(grid) => Some(grid.column()),
            Cursor::Spread(_) => None,
        }
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
    /// The width each child placed took, in order.
    widths: Vec<f32>,
    /// The least widths of the children by index, as this pass's probe of a short row found them; empty without one.
    /// A child's room leaves the leasts of the children after it.
    leasts: Vec<f32>,
    /// The sweep is the probe: every child measures unbounded, at its least.
    probe: bool,
}

impl Spread {
    /// A cursor over `flow` with `gap` between children, spreading by `last` (the natural width and the number of the
    /// children the measuring run found), each child's room leaving `leasts` for the children after it. A `probe`
    /// sweep offers each child unbounded room.
    fn new(flow: Flow, gap: f32, last: Option<Vec2>, leasts: Vec<f32>, probe: bool) -> Self {
        let width = flow.rect().width();
        let between = last.filter(|last| last.y >= 2.0 && last.x < width).map_or(gap, |last| gap + (width - last.x) / (last.y - 1.0));
        Spread { flow, gap, between, at: 0.0, last: 0.0, natural: 0.0, count: 0, widths: Vec::new(), leasts, probe }
    }

    fn start(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.at + self.between }
    }

    fn cell(&mut self, size: Vec2) -> Rect {
        let start = self.start();
        let span = self.flow.span(start, size.x);
        self.natural += size.x + if self.count == 0 { 0.0 } else { self.gap };
        (self.at, self.last, self.count) = (start + size.x, size.x, self.count + 1);
        self.widths.push(size.x);
        Rect::from_min_size(span.min, Vec2::new(span.width(), size.y))
    }

    /// The width the next child (index `count`) may take: the rest of the row less the least of each child after it,
    /// with the gap before it.
    fn rest(&self) -> f32 {
        let after: f32 = self.leasts.iter().skip(self.count + 1).map(|least| least + self.gap).sum();
        (self.flow.rect().width() - self.start() - after).max(0.0)
    }

    fn room(&self) -> Vec2 {
        let height = self.flow.rect().height();
        Vec2::new(if self.probe { f32::INFINITY } else { self.rest() }, height)
    }

    fn region(&self) -> Rect {
        self.flow.span(self.start(), self.rest())
    }

    /// What `new` spreads by next pass.
    fn measured(&self) -> Vec2 {
        Vec2::new(self.natural, self.count as f32)
    }
}

/// Runs `body`, the children of the `SpaceBetween` row `id` with `gap` between them, in `ui`, the block they are
/// placed in, under the row's env. Returns what `body` returned and the width the children take at the gap alone.
///
/// A measuring run sweeps the children once. When they do not fit the row (two or more children wider than it), it
/// sweeps them again as a probe, each unbounded at its least (`Scoped::least`), and then once more: each child measures
/// in the room the leasts of the children after it leave, so a child that wraps leaves the others their place. It
/// records the natural width and count, and the leasts (none for a row that fits). The real run, including one a kept
/// transcript serves, builds the same cursor from those records, so each child measures in the same room in both.
/// Every sweep runs in a child of `ui` with its layout, so the children get the same ids in each.
/// `env` is the row's env, installed for `ui`.
fn run_spread<R>(ui: &mut Ui, env: &Env, id: Id, gap: f32, rtl: bool, body: &mut dyn FnMut(&mut Ui) -> R) -> (R, Option<f32>) {
    let ctx = ui.ctx().clone();
    let layout = child::row_layout(rtl, false);
    let sweep = |ui: &mut Ui, env: &Env, last: Option<Vec2>, leasts: Vec<f32>, probe: bool, body: &mut dyn FnMut(&mut Ui) -> R| {
        let flow = Flow::new(ui.available_rect_before_wrap(), rtl);
        pass_state::with::<Cursors, _>(&ctx, |cursors| cursors.0.insert(id, Cursor::Spread(Spread::new(flow, gap, last, leasts, probe))));
        // The children run in the sweep's own `Ui`: the row's env is installed for it, so they are its direct children.
        let inner = Env::run_in(ui, env.clone(), body);
        let spread = match pass_state::with::<Cursors, _>(&ctx, |cursors| cursors.0.remove(&id)) {
            Some(Cursor::Spread(spread)) => Some(spread),
            _ => None,
        };
        (inner, spread)
    };
    let natural = |spread: &Option<Spread>| spread.as_ref().map(|spread| spread.measured().x);
    let leasts_id = sub(id, "leasts");
    if !measure::active(&ctx) {
        let count = this_pass(&ctx, false, leasts_id).map_or(0, |count| count.x as usize);
        let leasts = (0..count).map(|i| this_pass(&ctx, false, sub(id, ("least", i))).map_or(0.0, |least| least.x)).collect();
        let last = this_pass(&ctx, false, id);
        let (inner, spread) = child::block(ui, None, layout, |ui| sweep(ui, env, last, leasts, false, body)).0;
        return (inner, natural(&spread));
    }
    let room = ui.available_rect_before_wrap().size();
    let ((inner, spread), rect, short) = crate::contract::keeping_ids(&ctx, || {
        let mut first = laid_out_child(ui, layout);
        let (inner, spread) = sweep(&mut first, env, None, Vec::new(), false, body);
        // Half a point: less than a pixel shows at any scale the box runs.
        let short = spread.as_ref().is_some_and(|spread| spread.count >= 2 && room.x.is_finite() && spread.natural > room.x + 0.5);
        (((inner, spread), first.min_rect(), short), short)
    });
    let (inner, spread, leasts) = if short {
        #[cfg(feature = "testing")]
        measure::count_least(&ctx, id);
        let probe = env.derive_local(|scoped| {
            scoped.unbounded.0 = true;
            scoped.least = true;
        });
        let leasts = crate::contract::keeping_ids(&ctx, || {
            let leasts = layout_memory::probing(&ctx, || {
                let mut least = laid_out_child(ui, layout);
                let (_, spread) = sweep(&mut least, &probe, None, Vec::new(), true, body);
                spread.map_or_else(Vec::new, |spread| spread.widths)
            });
            (leasts, true)
        });
        let (inner, spread) = child::block(ui, None, layout, |ui| sweep(ui, env, None, leasts.clone(), false, body)).0;
        (inner, spread, leasts)
    } else {
        ui.advance_cursor_after_rect(rect);
        (inner, spread, Vec::new())
    };
    if let Some(spread) = &spread {
        found(&ctx, true, id, room, spread.measured());
    }
    // Recorded for a row that fits too, so the real run reads no leasts an earlier run of the pass left.
    found(&ctx, true, leasts_id, room, Vec2::new(leasts.len() as f32, 0.0));
    for (i, least) in leasts.iter().enumerate() {
        found(&ctx, true, sub(id, ("least", i)), room, Vec2::new(*least, 0.0));
    }
    (inner, natural(&spread))
}

/// What a child shown in flow is, for where it sits in the height of its slot.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InFlow {
    /// Whether the child fills its cell's height in a `Grid` (declares `Sizing::Fill`).
    pub fill: bool,
    /// How far below the child's top the centre of its first line lies: half a control's height, a container's
    /// first child's first line, a `Row` body's line. `None` for a child with no line of its own: a collection whose
    /// rows run along Y, or a container whose first child is one. The child is one line when that centre lies at
    /// half its height (`one_line`).
    pub first: Option<f32>,
    /// `first` is the line of the child's own unwrapped `Row` body, of the control of a labelled control's row, or of
    /// the first control of a container labelled above its body: the child sits on a row's line by it even when it is
    /// more than one line (a nested row with a child of two lines, a check box whose label has a detail line under it,
    /// a control or a field under its label).
    pub line: bool,
}

/// How far a one-line child's first-line centre may lie from half its height: half a pixel at the scales the box runs.
const ONE_LINE_SLACK: f32 = 0.5;

/// The centre of the line of a child `height` tall whose first line's centre lies `first` below its top, when the
/// child is one line: a control, a row of one-line children, a container around one line (a field with its label
/// beside its control). A child of more than one line (a column, a field with a help line below) has its first line
/// above its middle.
fn one_line(first: Option<f32>, height: f32) -> Option<f32> {
    first.filter(|first| (2.0 * first - height).abs() <= ONE_LINE_SLACK)
}

/// The rect a child of measured `size` takes in flow in `ui`: the next cell of the `Grid` or spread `Row` body that
/// encloses it directly, else the next allocation of `ui`'s layout. A direct child of an unwrapped `Row` that is one
/// line sits with its centre on the row's line; a child of more than one line (a column, a field with a help line)
/// sits at the row's top. A direct child of a wrapping `Row` takes the height of its line, at the line's middle when it
/// is one line, else at its top. In a `Grid` cell a child that `fill`s takes the
/// cell's whole height from the cell's top, so the cards of one row share their top and bottom edges; any other child
/// sits at the vertical centre of its cell.
#[doc(hidden)]
pub fn place_in_flow(ui: &mut Ui, size: Vec2) -> Rect {
    let env = Env::of(ui);
    place_in_cell(ui, &env, size, InFlow { fill: false, first: Some(size.y / 2.0), line: false })
}

/// `place_in_flow` for a child that is `child`. In a measuring run the child reports its line to the row around it
/// and its first line to the container whose header or body it starts. `env` is the env `ui`'s children are placed under.
pub(crate) fn place_in_cell(ui: &mut Ui, env: &Env, size: Vec2, child: InFlow) -> Rect {
    let (line, target) = (env.row_line.filter(|line| line.places), env.first_line);
    let child = InFlow { first: if child.line { child.first } else { one_line(child.first, size.y) }, ..child };
    // A one-line child of a bracketed row sits with its centre on the bracket's middle, between the controls its
    // rows reported before it.
    let bracket = line.filter(|line| env.bracket == Some(line.body)).and_then(|line| {
        let took = pass_state::with::<Brackets, _>(ui.ctx(), |brackets| brackets.0.get(&line.body).copied())?;
        Some((line.body, took.centre().map(|centre| centre - ui.available_rect_before_wrap().top())))
    });
    let centre = bracket.and_then(|(_, centre)| centre).or_else(|| line.and_then(|line| line.centre));
    let above = centre.zip(child.first).map_or(0.0, |(centre, first)| (centre - first).max(0.0));
    // A child of a wrapping row takes the height of its line; a one-line child sits at its middle.
    let wrapped = if line.is_none() { wrap_line(ui, env, size) } else { None };
    let height = line.and_then(|line| line.height).or(wrapped);
    let slot = Vec2::new(size.x, height.map_or(above + size.y, |height| height.max(above + size.y)));
    let (rect, in_cell) = take_in_flow(ui, env, slot);
    wrap_took(ui, env, rect);
    let placed = if child.fill && in_cell {
        rect
    } else {
        let spare = (rect.height() - size.y).max(0.0);
        let (above, below) = match line {
            Some(_) => (above, (rect.height() - above - size.y).max(0.0)),
            None => {
                let top = if wrapped.is_some() { one_line(child.first, size.y).is_none() } else { env.grid_top };
                if top { (0.0, spare) } else { (spare / 2.0, spare / 2.0) }
            }
        };
        Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + above), egui::pos2(rect.max.x, rect.max.y - below))
    };
    if let Some((row, _)) = bracket {
        pass_state::with::<Brackets, _>(ui.ctx(), |brackets| brackets.0.get_mut(&row).map(|took| took.middle = Some(placed)));
    }
    if measure::active(ui.ctx()) {
        if let Some(line) = line {
            pass_state::with::<RowLines, _>(ui.ctx(), |rows| {
                if let Some(took) = rows.0.get_mut(&line.body) {
                    took.add(child.first, size.y);
                }
            });
        }
        if let Some(target) = target {
            report_first_line(ui.ctx(), target, child.first.map(|first| placed.min.y + first));
        }
    }
    placed
}

/// Takes `size` in flow in `ui`, whose children are placed under `env`: the next cell of the `Grid` or spread `Row` body
/// that encloses it directly, else the next allocation of `ui`'s layout; the body's flow start closes. Returns the rect
/// taken and whether it is a cell.
pub(crate) fn take_in_flow(ui: &mut Ui, env: &Env, size: Vec2) -> (Rect, bool) {
    close_flow_start(ui.ctx(), env);
    match cell_in(ui, env.grid, size) {
        Some(cell) => (cell, true),
        None => (alloc::space(ui, size), false),
    }
}

/// Leaves the next cell of the `Grid` body that encloses `ui` directly empty: the next child takes the cell after it.
/// A row of a grid whose first column holds a control in some rows (a radio) leaves it empty in the others, so the
/// cells after it stay in their columns. Outside a grid it does nothing.
pub fn empty_cell(ui: &mut Ui) {
    let env = Env::of(ui);
    let grid = env.grid.filter(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| matches!(cursors.0.get(body), Some(Cursor::Grid(_)))));
    if grid.is_some() {
        take_in_flow(ui, &env, Vec2::ZERO);
    }
}

/// Takes the next cell of the `Grid` or spread `Row` body that encloses `ui` directly for a child of `size`, and returns
/// it; `None`, taking nothing, outside such a body. `env` is the env `ui`'s children are placed under.
pub(crate) fn take_cell(ui: &mut Ui, env: &Env, size: Vec2) -> Option<Rect> {
    let cell = cell_in(ui, env.grid, size);
    if cell.is_some() {
        close_flow_start(ui.ctx(), env);
    }
    cell
}

fn cell_in(ui: &mut Ui, grid: Option<Id>, size: Vec2) -> Option<Rect> {
    let cell = grid.and_then(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get_mut(&body).map(|cursor| cursor.cell(size))))?;
    alloc::advance_past(ui, cell);
    Some(cell)
}

/// The region the next child shown in flow in `ui` measures and is placed in: that of the next cell of the `Grid` or
/// spread `Row` body that encloses `ui` directly, else the rest of `ui`'s layout. `env` is the env `ui`'s children are
/// placed under, as for every helper here that takes one.
pub(crate) fn flow_region(ui: &Ui, env: &Env) -> Rect {
    let cell = env.grid.and_then(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get(&body).map(Cursor::region)));
    cell.unwrap_or_else(|| ui.available_rect_before_wrap())
}

/// The width a child placed under `env` takes alone in the `Row` body that places it: the room the row is offered,
/// whatever its earlier children took, unbounded in a body that scrolls along x. `None` outside such a row.
pub(crate) fn row_alone_room(env: &Env) -> Option<f32> {
    env.row_room.map(|room| if env.unbounded.0 { f32::INFINITY } else { room })
}

/// The room a child shown in flow in `ui` measures and sizes in: the room of the next cell of the `Grid` or spread
/// `Row` body that encloses `ui` directly, else the rest of `ui`'s layout.
pub(crate) fn flow_room(ui: &Ui, env: &Env) -> Vec2 {
    let cell = env.grid.and_then(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get(&body).map(Cursor::room)));
    cell.unwrap_or_else(|| open_room(ui, env))
}

/// The rest of `ui`'s layout (the room offered to the container, less the earlier siblings and gaps; a whole line in a row
/// that wraps); unbounded along the axes of the scrolling body that encloses it, whose region is only as large as egui
/// can make it.
fn open_room(ui: &Ui, env: &Env) -> Vec2 {
    let (x, y) = env.unbounded;
    let mut room = ui.available_size_before_wrap();
    // In a row that wraps, a child that does not fit the rest of the line goes on the next one: its room is a line.
    let layout = ui.layout();
    if layout.main_wrap && layout.is_horizontal() {
        room.x = ui.max_rect().width();
    }
    // A body sized from its content offers its children the width it was offered, and a row body that placed its
    // block behind a lead the width of its band; either less what they took.
    if let Some(offered) = env.offered.or(env.row_room) {
        room.x += offered - ui.max_rect().width();
    }
    Vec2::new(if x { f32::INFINITY } else { room.x }, if y { f32::INFINITY } else { room.y })
}

/// The room a child that fills its width fills in `ui`: `flow_room`, except that in a `Content` column of a `Grid` it
/// is the column's width, so the child never measures as the room and pushes the next column out of it.
pub(crate) fn flow_fill_room(ui: &Ui, env: &Env) -> Vec2 {
    let cell = env.grid.and_then(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get(&body).map(Cursor::fill_room)));
    cell.unwrap_or_else(|| open_room(ui, env))
}

/// `flow_fill_room` for a leaf that fills: along x in a body that scrolls along x it is what the view shows from the
/// leaf's start, so the leaf fills the view and what is past it scrolls; along y the same in a `Stack` body that
/// scrolls along y, whose children that fill share it.
pub(crate) fn leaf_fill_room(ui: &Ui, env: &Env) -> Vec2 {
    let room = flow_fill_room(ui, env);
    let x = view_room(ui, env, Axis::X).filter(|_| room.x.is_infinite());
    let y = view_room(ui, env, Axis::Y).filter(|_| room.y.is_infinite() && shares_y(env));
    Vec2::new(x.unwrap_or(room.x), y.unwrap_or(room.y))
}

/// Whether the children placed under `env` are the children of a `Stack` body that shares its height among those that
/// fill.
pub(crate) fn shares_y(env: &Env) -> bool {
    env.fill_body.is_some_and(|(_, along)| along == Axis::Y)
}

/// Whether the next child placed under `env` goes in a `Content` column of the `Grid` body that places it: like a
/// content-sized column, a child that fills its width is placed at its measured width there.
pub(crate) fn in_grid_content_column(ctx: &egui::Context, env: &Env) -> bool {
    env.grid.is_some_and(|body| pass_state::with::<Cursors, _>(ctx, |cursors| cursors.0.get(&body).is_some_and(Cursor::in_content_column)))
}

/// The cell the next child placed under `env` takes, when the body that places it is a `Grid`: the grid's body, the
/// cell's column and whether the column is a `Content` one.
pub(crate) fn grid_cell(ctx: &egui::Context, env: &Env) -> Option<(Id, usize, bool)> {
    let grid = env.grid?;
    pass_state::with::<Cursors, _>(ctx, |cursors| {
        let cursor = cursors.0.get(&grid)?;
        Some((grid, cursor.grid_column()?, cursor.in_content_column()))
    })
}

fn column_id(grid: Id, index: usize) -> Id {
    sub(grid, ("column", index))
}

fn row_id(grid: Id, index: usize) -> Id {
    sub(grid, ("row", index))
}

fn least_id(grid: Id, index: usize) -> Id {
    sub(grid, ("least", index))
}

/// What the grid measured in this pass's measuring run.
fn measured_tracks(ctx: &egui::Context, sizing: bool, grid: Id, columns: usize) -> GridMeasure {
    let rows = this_pass(ctx, sizing, sub(grid, "rows")).map_or(0, |count| count.x as usize);
    let least: Vec<f32> = (0..columns).map_while(|i| this_pass(ctx, sizing, least_id(grid, i)).map(|size| size.x)).collect();
    GridMeasure::new(
        (0..columns).map(|i| this_pass(ctx, sizing, column_id(grid, i)).map_or(0.0, |size| size.x)).collect(),
        (0..rows).map(|i| this_pass(ctx, sizing, row_id(grid, i)).map_or(0.0, |size| size.y)).collect(),
    )
    .with_least(least)
}

/// The one column a grid lays its cells in, one under another, when its columns do not fit beside each other.
const STACKED: &[ColumnWidth] = &[ColumnWidth::fill(1.0)];

/// Runs `body`, the cells of the `Grid` body `id` over `columns`, in `ui`. Each column is a column group
/// (`ColumnGroup::grid`): the labels beside the controls of one column share a width.
///
/// A measuring run sweeps the cells once, each `Content` column in the room the other columns leave at their least
/// (`GridCursor::each_alone`). When the columns do not fit beside each other (`GridCursor::short`), it sweeps them again
/// as a probe, every leaf at its least and no column bounded, which gives each column its least, and then once more:
/// each column keeps at least its least, a `Content` column measuring in the room the others leave at theirs. When
/// the leasts do not fit beside each other, that last sweep lays the cells out one under another across the room
/// instead, so no column is cut. A `GridFit::Fixed` grid sweeps its cells once at their own sizes in unbounded room,
/// whatever the room and the probe, and overflows the room. The real run places the cells from the widths, leasts and
/// layout the measuring run recorded.
fn run_grid<R>(
    ui: &mut Ui,
    env: &Env,
    id: Id,
    (columns, fit, cells): (&[ColumnWidth], GridFit, CellAlign),
    gap: Vec2,
    mut body: impl FnMut(&mut Ui) -> R,
) -> R {
    let ctx = ui.ctx().clone();
    let rtl = env.direction.is_rtl();
    let layout = child::column_layout(rtl);
    // One sweep of the cells in `ui` over `columns`, with the cursor `make` given the flow of `ui`'s layout and the
    // column groups opened at `seed`; returns what the cursor measured, and from a measuring sweep the groups' widths
    // when they have to settle (`ColumnGroup::grid`).
    let mut sweep = |ui: &mut Ui, scoped: &Env, seed: Option<Vec<Columns>>, columns: &[ColumnWidth], make: &dyn Fn(Flow) -> GridCursor| {
        let top = ui.available_rect_before_wrap().top();
        let cursor = make(Flow::new(ui.available_rect_before_wrap(), rtl));
        let count = columns.len();
        pass_state::with::<Cursors, _>(&ctx, |cursors| cursors.0.insert(id, Cursor::Grid(cursor)));
        let installed = Env::install_for(
            ui,
            scoped.derive_local(|scoped| {
                scoped.grid = Some(id);
                scoped.grid_top = cells == CellAlign::Top;
                scoped.flow_body = Some(id);
            }),
        );
        let (inner, unsettled) = ColumnGroup::grid(ui, id, count, seed, &mut body);
        Env::restore(&ctx, installed);
        let cursor = match pass_state::with::<Cursors, _>(&ctx, |cursors| cursors.0.remove(&id)) {
            Some(Cursor::Grid(cursor)) => Some(cursor),
            _ => None,
        };
        let short = cursor.as_ref().is_some_and(GridCursor::short);
        let measured = cursor.map(|cursor| cursor.measured().clone()).unwrap_or_default();
        // Cells place from last pass's rows, so a measuring run's rows all start at the top: the grid reaches as far
        // down as the rows this pass measured, whichever pass it is.
        let rows = measured.rows.iter().sum::<f32>() + gap.y * measured.rows.len().saturating_sub(1) as f32;
        alloc::reach_down_to(ui, top + rows);
        (inner, measured, short, unsettled)
    };
    if !measure::active(&ctx) {
        let stacked = this_pass(&ctx, false, sub(id, "stacked")).is_some_and(|stacked| stacked.x > 0.0);
        let columns = if stacked { STACKED } else { columns };
        let last = measured_tracks(&ctx, false, id, columns.len());
        // A fixed grid lays its cells out over a room at least as wide as its columns, whatever room it was given.
        let at = (fit == GridFit::Fixed).then(|| fixed_room(ui, &last.columns, gap.x, rtl));
        return child::block(ui, at, layout, |ui| sweep(ui, env, None, columns, &|flow| GridCursor::new(flow, columns, gap, &last)).0).0;
    }
    // A fixed grid's columns are its cells' own widths, never their leasts, also inside a parent's probe for its least:
    // a probe in unbounded room finds them, settled across the cells' column groups, and the grid lays its cells out
    // over a room at least that wide, so it neither stacks nor squeezes them and its least width is its width. The room
    // then clips or scrolls it.
    if fit == GridFit::Fixed {
        let natural = env.derive_local(|scoped| {
            scoped.unbounded.0 = true;
            scoped.least = false;
        });
        let mut probe = |ui: &mut Ui, seed: Option<Vec<Columns>>| {
            crate::contract::keeping_ids(&ctx, || {
                let found = layout_memory::probing(&ctx, || {
                    let mut probe = laid_out_child(ui, layout);
                    let (_, measured, _, unsettled) = sweep(&mut probe, &natural, seed, columns, &|flow| GridCursor::unbounded(flow, columns, gap));
                    (measured.columns, unsettled)
                });
                (found, true)
            })
        };
        let (mut widths, settled) = probe(ui, None);
        if let Some(seed) = settled.clone() {
            widths = probe(ui, Some(seed)).0;
        }
        let at = fixed_room(ui, &widths, gap.x, rtl);
        let own = env.derive_local(|scoped| scoped.least = false);
        let last = GridMeasure::default().with_least(widths.clone());
        let (inner, measured, ..) =
            child::block(ui, Some(at), layout, |ui| sweep(ui, &own, settled, columns, &|flow| GridCursor::new(flow, columns, gap, &last))).0;
        let room = ui.available_rect_before_wrap().size();
        for (i, width) in measured.columns.iter().enumerate() {
            found(&ctx, true, column_id(id, i), room, Vec2::new(*width, 0.0));
        }
        for (i, width) in widths.iter().enumerate() {
            found(&ctx, true, least_id(id, i), room, Vec2::new(*width, 0.0));
        }
        for (i, height) in measured.rows.iter().enumerate() {
            found(&ctx, true, row_id(id, i), room, Vec2::new(0.0, *height));
        }
        found(&ctx, true, sub(id, "rows"), room, Vec2::new(measured.rows.len() as f32, 0.0));
        found(&ctx, true, sub(id, "stacked"), room, Vec2::ZERO);
        return inner;
    }
    // A measuring run under a body sized from its content gives each fill column the width its cells take, as a
    // content column: the room it would share is not the body's to claim.
    let content_run = env.content.0;
    let cursor = |columns: &[ColumnWidth], last: &GridMeasure, flow: Flow| match content_run {
        true => GridCursor::sized_by_content(flow, columns, gap, last),
        false => GridCursor::new(flow, columns, gap, last),
    };
    let none = GridMeasure::default();
    // The first sweep, each column alone. When a cell read a label column narrower than its group settled on, the
    // sweep is taken back and runs once more with the groups opened at the widths it found.
    let mut alone = |ui: &mut Ui, seed: Option<Vec<Columns>>| {
        let seeded = seed.is_some();
        crate::contract::keeping_ids(&ctx, || {
            let (out, kept) = layout_memory::draft(
                &ctx,
                || {
                    let mut first = laid_out_child(ui, layout);
                    let (inner, measured, short, unsettled) =
                        sweep(&mut first, env, seed, columns, &|flow| cursor(columns, &none, flow).each_alone());
                    (inner, measured, first.min_rect(), short, unsettled)
                },
                |out| seeded || out.4.is_none(),
            );
            let restore = !kept || out.3;
            (out, restore)
        })
    };
    let mut first = alone(ui, None);
    if let Some(widths) = first.4.take() {
        first = alone(ui, Some(widths));
    }
    let (inner, measured, rect, short, _) = first;
    let record = |measured: &GridMeasure, least: &[f32], stacked: bool, room: Vec2| {
        for (i, width) in measured.columns.iter().enumerate() {
            found(&ctx, true, column_id(id, i), room, Vec2::new(*width, 0.0));
        }
        for (i, width) in least.iter().enumerate() {
            found(&ctx, true, least_id(id, i), room, Vec2::new(*width, 0.0));
        }
        for (i, height) in measured.rows.iter().enumerate() {
            found(&ctx, true, row_id(id, i), room, Vec2::new(0.0, *height));
        }
        found(&ctx, true, sub(id, "rows"), room, Vec2::new(measured.rows.len() as f32, 0.0));
        found(&ctx, true, sub(id, "stacked"), room, Vec2::new(f32::from(u8::from(stacked)), 0.0));
    };
    if !short {
        ui.advance_cursor_after_rect(rect);
        record(&measured, &[], false, ui.available_rect_before_wrap().size());
        return inner;
    }
    #[cfg(feature = "testing")]
    measure::count_least(&ctx, id);
    let probe = env.derive_local(|scoped| {
        scoped.unbounded.0 = true;
        scoped.least = true;
    });
    let least = crate::contract::keeping_ids(&ctx, || {
        let least = layout_memory::probing(&ctx, || {
            let mut least = laid_out_child(ui, layout);
            sweep(&mut least, &probe, None, columns, &|flow| GridCursor::unbounded(flow, columns, gap)).1.columns
        });
        (least, true)
    });
    // A fixed column is as wide as the app made it, whatever its cells take.
    let least: Vec<f32> = (columns.iter().enumerate())
        .map(|(i, kind)| match kind {
            ColumnWidth::Fixed(width) => *width,
            _ => least.get(i).copied().unwrap_or(0.0),
        })
        .collect();
    let gaps = gap.x * columns.len().saturating_sub(1) as f32;
    let stacked = least.iter().sum::<f32>() + gaps > ui.available_rect_before_wrap().width() + 0.5;
    let (columns, last) = match stacked {
        true => (STACKED, GridMeasure::default()),
        false => (columns, GridMeasure::default().with_least(least.clone())),
    };
    let (inner, measured, ..) = child::block(ui, None, layout, |ui| sweep(ui, env, None, columns, &|flow| cursor(columns, &last, flow))).0;
    record(&measured, &last.least, stacked, ui.available_rect_before_wrap().size());
    inner
}

/// The room a fixed grid of columns `widths` lays its cells out over in `ui`: the room `ui` offers, widened to the
/// columns and the gaps between them at the reading start.
fn fixed_room(ui: &Ui, widths: &[f32], gap: f32, rtl: bool) -> Rect {
    let room = ui.available_rect_before_wrap();
    let width = room.width().max(widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32);
    if rtl {
        Rect::from_min_max(egui::pos2(room.max.x - width, room.min.y), room.max)
    } else {
        Rect::from_min_size(room.min, Vec2::new(width, room.height()))
    }
}

/// What this pass's measuring run found under `id`. A measuring run itself reads nothing.
fn this_pass(ctx: &egui::Context, sizing: bool, id: Id) -> Option<Vec2> {
    if sizing { None } else { layout_memory::measured(ctx, id).map(|found| found.took) }
}

/// The line of the unwrapped `Row` body `id` in this pass's measuring run, with the line it shares already in it: how
/// far below its top the line's centre lies, and the row's height once its children sit on it. A line measured while
/// the row shared another line (a row centred in another height) is not this pass's.
fn row_line(ctx: &egui::Context, sizing: bool, id: Id, shared: Option<f32>) -> Option<(f32, f32)> {
    let found = if sizing { None } else { layout_memory::measured(ctx, sub(id, "line")) }?;
    let same = (found.room.y - shared.unwrap_or(0.0)).abs() <= ONE_LINE_SLACK;
    Some((found.took.x, found.took.y)).filter(|(_, height)| same && *height > 0.0)
}

/// The line the members of the unwrapped `Row` body `body` sit on in this pass's measuring run, with the line it shares
/// with its container's regions already in it: what the container's end region sets its children on, so the end
/// content meets the members' line as the members meet each other. `None` for any other body.
pub(crate) fn members_line(ctx: &egui::Context, body: Id) -> Option<RowLine> {
    layout_memory::measured(ctx, sub(body, "line")).map(|found| found.took).filter(|took| took.y > 0.0).map(|took| RowLine {
        body,
        centre: Some(took.x),
        height: Some(took.y),
        settled: true,
        places: true,
    })
}

/// The line the unwrapped `Row` body `body` sets its children on: its own (`centre`, `height`), or the line `shared`
/// high it shares with its container's inline header or end region, centred, when that centre is lower. Only its own
/// line, which this pass's measuring run recorded, is settled.
fn on_line(body: Id, own: Option<(f32, f32)>, shared: Option<f32>) -> RowLine {
    let (centre, height) = match (own, shared) {
        (Some((centre, height)), Some(shared)) => {
            let line = centre.max(shared / 2.0);
            (Some(line), Some((height + line - centre).max(shared)))
        }
        (Some((centre, height)), None) => (Some(centre), Some(height)),
        (None, Some(shared)) => (Some(shared / 2.0), Some(shared)),
        (None, None) => (None, None),
    };
    RowLine { body, centre, height, settled: own.is_some(), places: true }
}

/// Starts a sweep of a body's children in a measuring run: the body's first-line `target` and, for the unwrapped
/// `Row` body `row`, its line start afresh, `least` high at least (`ContainerProps::least_height`).
fn open_sweep(ctx: &egui::Context, target: Option<Id>, row: Option<Id>, least: f32) {
    if let Some(target) = target {
        open_first_lines(ctx, &[target]);
    }
    if let Some(row) = row {
        // The least height counts as a child of more than one line that high.
        let mut took = LineTook::default();
        took.add(None, least.max(0.0));
        pass_state::with::<RowLines, _>(ctx, |rows| rows.0.insert(row, took));
    }
}

/// Records the line of the unwrapped `Row` body `id` from what its children took in `ui`, the block they ran in,
/// with the line `shared` it shares with its container's regions; a real run records nothing. The last sweep of a
/// measuring run gives the line. The block takes the row's height once its children sit on the line, and the row
/// reports its line to `target`, the container its body reports a first line to.
fn record_line(ui: &mut Ui, sizing: bool, id: Id, shared: Option<f32>, target: Option<Id>) {
    if !sizing {
        return;
    }
    let ctx = ui.ctx().clone();
    let took = pass_state::with::<RowLines, _>(&ctx, |rows| rows.0.remove(&id)).unwrap_or_default();
    let own = (took.height() > 0.0).then(|| (took.centre.unwrap_or(0.0), took.height()));
    let line = on_line(id, own, shared);
    // What a child took without reporting its line still counts toward the row's height.
    let height = line.height.unwrap_or(0.0).max(ui.min_rect().height());
    let centre = line.centre.unwrap_or(0.0);
    // The record keeps the line it shared, which `row_line` matches.
    found(&ctx, sizing, sub(id, "line"), Vec2::new(0.0, shared.unwrap_or(0.0)), if height > 0.0 { Vec2::new(centre, height) } else { Vec2::ZERO });
    if height <= 0.0 {
        return;
    }
    let top = ui.max_rect().top();
    alloc::reach_down_to(ui, top + height);
    if let Some(target) = target {
        report_first_line(&ctx, target, took.centre.map(|_| top + centre));
    }
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
#[cfg(feature = "testing")]
pub(crate) fn run_body<R>(ui: &mut Ui, env: &Env, id: Id, layout: &BodyLayout, gap: Vec2, body: impl FnMut(&mut Ui) -> R) -> R {
    run_body_measured(ui, env, id, layout, gap, BodyLine::default(), body).0
}

/// What an unwrapped `Row` body's line starts from besides its children: the line `shared` with its container's inline
/// header or end region, which the children are centred on, and the `least` height of the line
/// (`ContainerProps::least_height`), which counts as a child of more than one line that high.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BodyLine {
    pub(crate) shared: Option<f32>,
    pub(crate) least: f32,
}

/// `run_body`, with the width the children take at the gap alone for a `Row` that places its block itself (`End`,
/// `Center`, `SpaceBetween`): the width the body asks for, where the room it is placed in spans wider. `line` is what
/// an unwrapped `Row` body's line starts from: the line it shares with its container's inline header or end region,
/// which it sets its children on, and its least height.
pub(crate) fn run_body_measured<R>(
    ui: &mut Ui,
    env: &Env,
    id: Id,
    layout: &BodyLayout,
    gap: Vec2,
    line: BodyLine,
    mut body: impl FnMut(&mut Ui) -> R,
) -> (R, Option<f32>) {
    let BodyLine { shared, least } = line;
    let rtl = env.direction.is_rtl();
    let ctx = ui.ctx().clone();
    let sizing = measure::active(&ctx);
    // Each sweep of a measuring run finds the body's first line and an unwrapped row's line afresh.
    let target = env.first_line.filter(|_| sizing);
    let row = (sizing && matches!(layout, BodyLayout::Row { wrap: false, .. })).then_some(id);
    // Each sweep and the real run start the body's flow, and a bracketed row's reports, afresh.
    let (after_rule, bracket) = (env.after_rule, env.bracket == Some(id));
    let body = |ui: &mut Ui| {
        open_sweep(&ctx, target, row, least);
        pass_state::with::<FlowStarts, _>(&ctx, |starts| starts.0.insert(id, after_rule));
        if bracket {
            pass_state::with::<Brackets, _>(&ctx, |brackets| brackets.0.insert(id, BracketTook::default()));
        }
        body(ui)
    };
    // Each layout runs in its own function, so a body nested in bodies keeps only its own layout's frame on the stack.
    let line = RowIn { id, gap, shared, sizing, rtl };
    match layout {
        BodyLayout::Stack => (run_stack(ui, env, &line, body), None),
        BodyLayout::Row { wrap: false, align: RowAlign::SpaceBetween } => run_spread_row(ui, env, &line, body),
        BodyLayout::Row { wrap: false, align: RowAlign::Start } => (run_start_row(ui, env, &line, body), None),
        BodyLayout::Row { wrap, align } => run_row(ui, env, &line, (*wrap, *align), body),
        BodyLayout::Grid { columns, fit, cells } => (run_grid(ui, env, id, (&columns[..], *fit, *cells), gap, body), None),
    }
}

/// What the layouts of `run_body_measured` read besides the env.
#[derive(Clone, Copy)]
struct RowIn {
    id: Id,
    gap: Vec2,
    /// The line the body shares with its container's inline header or end region.
    shared: Option<f32>,
    sizing: bool,
    rtl: bool,
}

/// A `Stack` body: each leaf shows at most the stack's room, never below its least.
fn run_stack<R>(ui: &mut Ui, env: &Env, at: &RowIn, body: impl FnMut(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let env = env.derive_local(|scoped| {
        scoped.shrink = Some(Shrink::Fit);
        scoped.flow_body = Some(at.id);
        scoped.stack_gap = Some((at.id, at.gap.y));
    });
    let installed = Env::install_for(ui, env.clone());
    let mut body = body;
    let body = move |ui: &mut Ui, _: &Env| body(ui);
    // A held child of a column whose own room is unbounded keeps the width it measured at.
    let inner = if env.content_column && flow_room(ui, &env).x.is_finite() {
        run_column(ui, &env, at.id, child::column_layout(at.rtl), body)
    } else {
        run_sharing(ui, &env, at.id, Axis::Y, child::column_layout(at.rtl), body)
    };
    Env::restore(&ctx, installed);
    inner
}

/// An unwrapped `Row` that spreads its children over its room.
fn run_spread_row<R>(ui: &mut Ui, env: &Env, at: &RowIn, mut body: impl FnMut(&mut Ui) -> R) -> (R, Option<f32>) {
    let ctx = ui.ctx().clone();
    let RowIn { id, gap, shared, sizing, rtl } = *at;
    let (inner, _) = child::block(ui, None, child::row_layout(rtl, false), |ui| {
        let line = on_line(id, row_line(&ctx, sizing, id, shared), shared);
        let row = env.derive_local(|scoped| {
            scoped.grid = Some(id);
            scoped.grid_top = false;
            scoped.row_line = Some(line);
            scoped.first_line = None;
            scoped.flow_body = Some(id);
        });
        let installed = Env::install_for(ui, row.clone());
        let inner = run_spread(ui, &row, id, gap.x, rtl, &mut body);
        Env::restore(&ctx, installed);
        record_line(ui, sizing, id, shared, env.first_line);
        inner
    });
    inner
}

/// An unwrapped `Row` placed from its start, whose children share and shrink to its room.
fn run_start_row<R>(ui: &mut Ui, env: &Env, at: &RowIn, mut body: impl FnMut(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let RowIn { id, shared, sizing, rtl, .. } = *at;
    let room = ui.available_rect_before_wrap();
    let line = on_line(id, row_line(&ctx, sizing, id, shared), shared);
    run_sharing(ui, env, id, Axis::X, child::row_layout(rtl, false), |ui, sweep| {
        let row = sweep.derive_local(|scoped| {
            scoped.row_room = Some(room.width());
            scoped.row_line = Some(line);
            scoped.first_line = None;
            scoped.flow_body = Some(id);
        });
        let installed = Env::install_for(ui, row.clone());
        let inner = run_shrinking(ui, &row, id, &mut body);
        Env::restore(&ctx, installed);
        record_line(ui, sizing, id, shared, env.first_line);
        inner
    })
}

/// A wrapped `Row`, or one that places its block at its center or end: the width its children take at the gap alone
/// is what the body asks for.
fn run_row<R>(ui: &mut Ui, env: &Env, at: &RowIn, (wrap, align): (bool, RowAlign), mut body: impl FnMut(&mut Ui) -> R) -> (R, Option<f32>) {
    let ctx = ui.ctx().clone();
    let RowIn { id, shared, sizing, rtl, .. } = *at;
    let room = ui.available_rect_before_wrap();
    let rect = match align {
        // A wrapping row wraps at the line length its container sets for paragraphs (a dialog's), as the paragraphs
        // beside it do: a dialog sized from its content stays as wide as its text, and a row of radios that would not
        // fit there takes a second line instead of widening the dialog. A room unbounded at its end edge (a measuring
        // run's) is cut from its start edge.
        RowAlign::Start | RowAlign::SpaceBetween if wrap => env
            .measure
            .filter(|measure| *measure < room.width() && (!rtl || room.right().is_finite()))
            .map(|measure| Flow::new(room, rtl).span(0.0, measure)),
        RowAlign::Start | RowAlign::SpaceBetween => None,
        RowAlign::Center | RowAlign::End => {
            let last = this_pass(&ctx, sizing, id);
            // In a body that scrolls along x the band is the width its view shows, not the scroll region.
            let band = if env.unbounded.0 { shrink_room(ui, env) } else { room.width() };
            // Without a measuring run, or in a band without an end, the block is placed as `Start`.
            last.filter(|last| band.is_finite() && last.x > 0.0 && last.x < band).map(|last| {
                let spare = band - last.x;
                let lead = if align == RowAlign::Center { spare / 2.0 } else { spare };
                Flow::new(room, rtl).take_start(lead).1.rect()
            })
        }
    };
    // A wrapped row has a line per wrap, whose height this pass's measuring run finds (`wrap_line`), and its first
    // child gives its first line.
    let line = (!wrap).then(|| on_line(id, row_line(&ctx, sizing, id, shared), shared));
    let lines = wrap && !env.least;
    // Where its least is asked, a wrapping row is as narrow as its widest child at its least: every child on a
    // line of its own.
    let block_layout = if wrap && env.least { child::column_layout(rtl) } else { child::row_layout(rtl, wrap) };
    let (inner, took) = child::block(ui, rect, block_layout, |ui| {
        let row = env.derive_local(|scoped| {
            scoped.row_room = Some(room.width());
            scoped.row_line = line;
            scoped.flow_body = Some(id);
            if line.is_some() {
                scoped.first_line = None;
            }
            // A leaf wider than a whole line takes the line, never below its least, as in a stack.
            if wrap {
                scoped.shrink = Some(Shrink::Fit);
            }
        });
        let installed = Env::install_for(ui, row.clone());
        if lines {
            open_wraps(&ctx, sizing, id);
        }
        let inner = if wrap { body(ui) } else { run_shrinking(ui, &row, id, &mut body) };
        if lines {
            close_wraps(&ctx, sizing, id, room.size());
        }
        Env::restore(&ctx, installed);
        if !wrap {
            record_line(ui, sizing, id, shared, env.first_line);
        }
        inner
    });
    if align != RowAlign::Start {
        found(&ctx, sizing, id, room.size(), took.size());
    }
    (inner, (align != RowAlign::Start).then(|| took.width()))
}
