//! `BodyLayout` and the layout of a body: the physical egui layout, the measured placement of `Row` End and
//! Center and of each row child on the row's line, and the cursors of `Grid` and of a `Row` that spreads its children.

use crate::contract::InBar;
use crate::runtime::egui::{alloc, child};
use crate::runtime::env::{Env, RowLine, Shrink};
use crate::runtime::layout_memory;
use crate::runtime::measure;
use crate::runtime::memory::sub;
use crate::runtime::pass_state;
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

    /// Children row-major over `columns`: a `'static` slice, or columns built in the pass.
    pub fn grid(columns: impl Into<Cow<'static, [ColumnWidth]>>) -> Self {
        BodyLayout::Grid { columns: columns.into() }
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
pub(crate) struct Cursors(HashMap<Id, Cursor>);

/// The number of children that fill the main axis of each `Stack` or `Row` body measured this pass, by body id.
#[derive(Default)]
pub(crate) struct FillCounts(HashMap<Id, u32>);

/// What the first child a measuring sweep placed in flow in each open target reported (`Scoped::first_line`): the y
/// of its first line's centre, or `None` for a child with no line of its own. A container's measuring sweep opens and
/// closes its header's and body's targets; a body's sweep starts its own target afresh.
#[derive(Default)]
pub(crate) struct FirstLines(HashMap<Id, Option<f32>>);

/// What the children of each unwrapped `Row` body took in the sweep of a measuring run running now, by body id.
#[derive(Default)]
pub(crate) struct RowLines(HashMap<Id, LineTook>);

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
        pass_state::with::<FillCounts, _>(ui.ctx(), |counts| *counts.0.entry(body).or_insert(0) += 1);
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

/// What a unit of a shrinking row reported in the first sweep: its id, the width it took there, the width it asks for
/// in no room, the least at which its content shows whole and the least it shrinks to, and whether its content is
/// made to be cut (`Component::cut_first`). A body unit reports what it took as all three; the row asks it for its
/// least (`Shrink::Least`) only when the row is short.
struct Reported {
    leaf: Id,
    took: f32,
    widths: ShrinkUnit,
    whole: f32,
    body: bool,
    cut_first: bool,
}

/// The widths of a unit of a row that shrinks to fit: the width it asks for and the least it narrows to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShrinkUnit {
    /// The width the unit takes where the row has room.
    pub ideal: f32,
    /// The least width the unit narrows to.
    pub least: f32,
}

/// The units each `Row` body counting this pass reported, in order, by body id.
#[derive(Default)]
pub(crate) struct ShrinkReports(HashMap<Id, Vec<Reported>>);

/// The least widths the body units of each `Row` body asking for them this pass reported, by row id.
#[derive(Default)]
pub(crate) struct LeastReports(HashMap<Id, HashMap<Id, f32>>);

/// Reports the widths of the unit `leaf` of the row `row` that counts them: a leaf's whole and least widths, or `None`
/// for a body.
pub(crate) fn report_shrink(ctx: &egui::Context, row: Id, leaf: Id, took: f32, ideal: f32, whole_least: Option<(f32, f32)>, cut_first: bool) {
    let (whole, least) = whole_least.map_or((ideal, ideal), |(whole, least)| (whole.min(ideal), least.min(ideal)));
    let widths = ShrinkUnit { ideal, least: least.min(whole) };
    let leaf = Reported { leaf, took, widths, whole, body: whole_least.is_none(), cut_first };
    pass_state::with::<ShrinkReports, _>(ctx, |reports| reports.0.entry(row).or_default().push(leaf));
}

/// Reports the least width of the body unit `body` of the row `row` that asks for it.
pub(crate) fn report_least(ctx: &egui::Context, row: Id, body: Id, least: f32) {
    pass_state::with::<LeastReports, _>(ctx, |reports| {
        if let Some(leasts) = reports.0.get_mut(&row) {
            leasts.insert(body, least);
        }
    });
}

/// The width the shrink step of the row `row` gave its leaf `leaf` in this pass's measuring run. A leaf given no
/// width has dropped out of the row (`shrink_or_drop`): it shows nothing and takes no place.
pub(crate) fn shrink_cap(ctx: &egui::Context, row: Id, leaf: Id) -> Option<f32> {
    layout_memory::measured(ctx, shrink_key(row, leaf)).map(|found| found.took.x)
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
fn shrink_or_drop(
    units: &[ShrinkUnit],
    whole: &[f32],
    first: &[bool],
    (fixed, gap): (f32, f32),
    room: f32,
    order: ShrinkOrder,
    drops: bool,
) -> Vec<f32> {
    let mut kept = vec![true; units.len()];
    let mut fixed = fixed;
    // Half a point: less than a pixel shows at any scale the box runs.
    let fits =
        |kept: &[bool], fixed: f32| fixed + units.iter().zip(kept).filter(|(_, kept)| **kept).map(|(unit, _)| unit.least).sum::<f32>() - room < 0.5;
    while drops && !fits(&kept, fixed) {
        let Some(at) = (0..units.len()).rev().find(|at| kept[*at] && first[*at]) else { break };
        kept[at] = false;
        fixed -= gap;
    }
    let pick = |all: &[f32]| -> Vec<f32> { all.iter().zip(&kept).filter(|(_, kept)| **kept).map(|(value, _)| *value).collect() };
    let shown: Vec<ShrinkUnit> = units.iter().zip(&kept).filter(|(_, kept)| **kept).map(|(unit, _)| *unit).collect();
    let shown_first: Vec<bool> = first.iter().zip(&kept).filter(|(_, kept)| **kept).map(|(first, _)| *first).collect();
    let mut widths = shrink_in_turns(&shown, &pick(whole), &shown_first, fixed, room, order).into_iter();
    kept.iter().map(|kept| if *kept { widths.next().unwrap_or_default() } else { 0.0 }).collect()
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
/// and the records a measuring run leaves are the ones the real run's children read and check.
fn run_shrinking<R>(ui: &mut Ui, id: Id, mut body: impl FnMut(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let layout = *ui.layout();
    let sweep = |ui: &mut Ui, shrink: Shrink, body: &mut dyn FnMut(&mut Ui) -> R| {
        let installed = Env::install_for(
            ui,
            Env::of(ui).derive_local(|scoped| {
                scoped.shrink = Some(shrink);
                // The least of a body is what it takes in no room.
                if matches!(shrink, Shrink::Least(_)) {
                    scoped.unbounded.0 = true;
                    scoped.least = true;
                }
            }),
        );
        let inner = body(ui);
        Env::restore(&ctx, installed);
        inner
    };
    if !measure::active(&ctx) {
        return child::block(ui, None, layout, |ui| sweep(ui, Shrink::Apply(id), &mut body)).0;
    }
    let room = shrink_room(ui);
    let order = if Env::of(ui).values.get::<InBar>().0 { ShrinkOrder::FromEnd } else { ShrinkOrder::Even };
    let (drops, gap) = (view_room(ui).is_none(), ui.spacing().item_spacing.x);
    // Records the width each unit gets, and returns whether one took another width in the first sweep.
    let settle = |leaves: &[Reported], fixed: f32| {
        let units: Vec<ShrinkUnit> = leaves.iter().map(|leaf| leaf.widths).collect();
        let whole: Vec<f32> = leaves.iter().map(|leaf| leaf.whole).collect();
        let first: Vec<bool> = leaves.iter().map(|leaf| leaf.cut_first).collect();
        let widths = if room.is_finite() {
            shrink_or_drop(&units, &whole, &first, (fixed, gap), room, order, drops)
        } else {
            units.iter().map(|unit| unit.ideal).collect()
        };
        let mut again = false;
        for (leaf, width) in leaves.iter().zip(widths) {
            layout_memory::emit_record(&ctx, shrink_key(id, leaf.leaf), layout_memory::Measured::new(Vec2::new(room, 0.0), Vec2::new(width, 0.0)));
            // Half a point: less than a pixel shows at any scale the box runs.
            again |= (leaf.took - width).abs() >= 0.5;
        }
        again
    };
    let (inner, rect, mut leaves, fixed, probe, again) = crate::contract::keeping_ids(&ctx, || {
        let mut first = ui.new_child(egui::UiBuilder::new().layout(layout));
        pass_state::with::<ShrinkReports, _>(&ctx, |reports| reports.0.insert(id, Vec::new()));
        let inner = sweep(&mut first, Shrink::Count(id), &mut body);
        let leaves = pass_state::with::<ShrinkReports, _>(&ctx, |reports| reports.0.remove(&id)).unwrap_or_default();
        let rect = first.min_rect();
        let fixed = rect.width() - leaves.iter().map(|leaf| leaf.took).sum::<f32>();
        let short = fixed + leaves.iter().map(|leaf| leaf.widths.ideal).sum::<f32>() - room >= 0.5;
        let probe = room.is_finite() && short && leaves.iter().any(|leaf| leaf.body);
        let again = !probe && settle(&leaves, fixed);
        ((inner, rect, leaves, fixed, probe, again), probe || again)
    });
    let again = again || probe;
    if probe {
        #[cfg(feature = "testing")]
        measure::count_least(&ctx, id);
        pass_state::with::<LeastReports, _>(&ctx, |reports| reports.0.insert(id, HashMap::new()));
        crate::contract::keeping_ids(&ctx, || {
            layout_memory::probing(&ctx, || {
                let mut least = ui.new_child(egui::UiBuilder::new().layout(layout));
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
    if again {
        child::block(ui, None, layout, |ui| sweep(ui, Shrink::Apply(id), &mut body)).0
    } else {
        ui.advance_cursor_after_rect(rect);
        inner
    }
}

/// In a body that scrolls along x, what its view shows from the start of the rest of `ui`'s layout: the width a row
/// there shrinks to fit and a child that fills its width fills. `None` outside such a body.
fn view_room(ui: &Ui) -> Option<f32> {
    let env = Env::of(ui);
    env.lazy.filter(|view| env.unbounded.0 && view.axes.0).map(|view| (view.rect.width() - view.start_of(ui.available_rect_before_wrap()).x).max(0.0))
}

/// The width the leaves of a row in `ui` shrink to fit: the row's flow room, except in a body that scrolls along x,
/// where it is what the view shows from the row's start. There the leaves narrow to their least before the body
/// scrolls, and when even their least does not fit they keep their natural widths and the body scrolls.
fn shrink_room(ui: &Ui) -> f32 {
    view_room(ui).unwrap_or_else(|| flow_room(ui).x)
}

/// Runs `body` as the content of the `Stack` (along y) or unwrapped `Row` (along x) body `id` in `ui`, the block
/// it is placed in, when the body is given that axis.
///
/// A measuring run sweeps the children once, the children that fill the main axis counted at no extent, and records
/// what the others and the gaps took with the number that fill. With a child that fills, it sweeps them again: the
/// others reuse what the first sweep recorded, and each child that fills is measured once, at its equal share of the
/// rest. The real run gives each the same share from that record.
fn run_sharing<R>(ui: &mut Ui, env: &Env, id: Id, axis: Axis, layout: egui::Layout, mut body: impl FnMut(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let given = if axis == Axis::Y { !env.content.1 } else { !env.content.0 };
    if !given {
        return child::block(ui, None, layout, body).0;
    }
    let key = sub(id, "share");
    let along = |size: Vec2| if axis == Axis::Y { size.y } else { size.x };
    // Along x in a body that scrolls along x, the children that fill share what the view shows.
    let room_of = |ui: &Ui| match axis {
        Axis::X => view_room(ui).unwrap_or_else(|| ui.available_rect_before_wrap().width()),
        _ => along(ui.available_rect_before_wrap().size()),
    };
    let sweep = |ui: &mut Ui, share: Option<f32>, body: &mut dyn FnMut(&mut Ui) -> R| {
        let installed = Env::install_for(
            ui,
            env.derive_local(|scoped| {
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
            let room = room_of(ui);
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
        let room = room_of(&first);
        pass_state::with::<FillCounts, _>(&ctx, |counts| counts.0.insert(id, 0));
        let inner = sweep(&mut first, None, &mut body);
        let fills = pass_state::with::<FillCounts, _>(&ctx, |counts| counts.0.remove(&id)).unwrap_or(0);
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
    /// How far below the child's top the centre of its first line lies: half a control's height, a container's
    /// first child's first line, a `Row` body's line. `None` for a child with no line of its own: a collection whose
    /// rows run along Y, or a container whose first child is one. The child is one line when that centre lies at
    /// half its height (`one_line`).
    pub first: Option<f32>,
}

/// How far a one-line child's first-line centre may lie from half its height: half a pixel at the scales the box runs.
const ONE_LINE_SLACK: f32 = 0.5;

/// The centre of the line of a child `height` tall whose first line's centre lies `first` below its top, when the
/// child is one line: a control, a row of one-line children, a container around one line (a field with its label
/// beside its control). A child of more than one line (a column, a field with its label above or a help line below)
/// has its first line above its middle.
fn one_line(first: Option<f32>, height: f32) -> Option<f32> {
    first.filter(|first| (2.0 * first - height).abs() <= ONE_LINE_SLACK)
}

/// The rect a child of measured `size` takes in flow in `ui`: the next cell of the `Grid` or spread `Row` body that
/// encloses it directly, else the next allocation of `ui`'s layout. A direct child of an unwrapped `Row` that is one
/// line sits with its centre on the row's line; a child of more than one line (a column, a field with a help line)
/// sits at the row's top. In a `Grid` cell a child that `fill`s takes the
/// cell's whole height from the cell's top, so the cards of one row share their top and bottom edges; any other child
/// sits at the vertical centre of its cell.
#[doc(hidden)]
pub fn place_in_flow(ui: &mut Ui, size: Vec2) -> Rect {
    place_in_cell(ui, size, InFlow { fill: false, first: Some(size.y / 2.0) })
}

/// `place_in_flow` for a child that is `child`. In a measuring run the child reports its line to the row around it
/// and its first line to the container whose header or body it starts.
pub(crate) fn place_in_cell(ui: &mut Ui, size: Vec2, child: InFlow) -> Rect {
    let env = Env::of(ui);
    let (line, target) = (env.row_line, env.first_line);
    let child = InFlow { first: one_line(child.first, size.y), ..child };
    let above = line.and_then(|line| line.centre).zip(child.first).map_or(0.0, |(centre, first)| (centre - first).max(0.0));
    let slot = Vec2::new(size.x, line.and_then(|line| line.height).map_or(above + size.y, |height| height.max(above + size.y)));
    let (rect, in_cell) = take_in(ui, env.grid, slot);
    let placed = if child.fill && in_cell {
        rect
    } else {
        let (above, below) = match line {
            Some(_) => (above, (rect.height() - above - size.y).max(0.0)),
            None => {
                let spare = (rect.height() - size.y).max(0.0);
                (spare / 2.0, spare / 2.0)
            }
        };
        Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + above), egui::pos2(rect.max.x, rect.max.y - below))
    };
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

/// Takes `size` in flow in `ui`: the next cell of the `Grid` or spread `Row` body that encloses it directly, else the
/// next allocation of `ui`'s layout. Returns the rect taken and whether it is a cell.
pub(crate) fn take_in_flow(ui: &mut Ui, size: Vec2) -> (Rect, bool) {
    take_in(ui, Env::of(ui).grid, size)
}

/// `take_in_flow`, with `grid` the body of `ui`'s env that places its children itself.
fn take_in(ui: &mut Ui, grid: Option<Id>, size: Vec2) -> (Rect, bool) {
    let cell = grid.and_then(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get_mut(&body).map(|cursor| cursor.cell(size))));
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
    let cell = Env::of(ui).grid.and_then(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get(&body).map(Cursor::region)));
    cell.unwrap_or_else(|| ui.available_rect_before_wrap())
}

/// The width a child of the `Row` body that encloses `ui` directly takes alone in it: the room the row is offered,
/// whatever its earlier children took, unbounded in a body that scrolls along x. `None` outside such a row.
pub(crate) fn row_alone_room(ui: &Ui) -> Option<f32> {
    let env = Env::of(ui);
    env.row_room.map(|room| if env.unbounded.0 { f32::INFINITY } else { room })
}

/// The room a child shown in flow in `ui` measures and sizes in: the room of the next cell of the `Grid` or spread
/// `Row` body that encloses `ui` directly, else the rest of `ui`'s layout.
pub(crate) fn flow_room(ui: &Ui) -> Vec2 {
    let cell = Env::of(ui).grid.and_then(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get(&body).map(Cursor::room)));
    cell.unwrap_or_else(|| open_room(ui))
}

/// The rest of `ui`'s layout (the room offered to the container, less the earlier siblings and gaps; a whole line in a row
/// that wraps); unbounded along the axes of the scrolling body that encloses it, whose region is only as large as egui
/// can make it.
fn open_room(ui: &Ui) -> Vec2 {
    let (x, y) = Env::of(ui).unbounded;
    let mut room = ui.available_size_before_wrap();
    // In a row that wraps, a child that does not fit the rest of the line goes on the next one: its room is a line.
    let layout = ui.layout();
    if layout.main_wrap && layout.is_horizontal() {
        room.x = ui.max_rect().width();
    }
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
    let cell = Env::of(ui).grid.and_then(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get(&body).map(Cursor::fill_room)));
    cell.unwrap_or_else(|| open_room(ui))
}

/// `flow_fill_room` for a leaf that fills its width: along x in a body that scrolls along x it is what the view shows
/// from the leaf's start, so the leaf fills the view and what is past it scrolls.
pub(crate) fn leaf_fill_room(ui: &Ui) -> Vec2 {
    let room = flow_fill_room(ui);
    match view_room(ui).filter(|_| room.x.is_infinite()) {
        Some(view) => Vec2::new(view, room.y),
        None => room,
    }
}

/// Whether the next child shown in flow in `ui` goes in a `Content` column of the `Grid` body that encloses `ui`
/// directly: like a content-sized column, a child that fills its width is placed at its measured width there.
pub(crate) fn in_grid_content_column(ui: &Ui) -> bool {
    Env::of(ui)
        .grid
        .is_some_and(|body| pass_state::with::<Cursors, _>(ui.ctx(), |cursors| cursors.0.get(&body).is_some_and(Cursor::in_content_column)))
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

/// Runs `body`, the cells of the `Grid` body `id` over `columns`, in `ui`.
///
/// A measuring run sweeps the cells once, each `Content` column in the room the other columns leave at their least
/// (`GridCursor::each_alone`). When the columns do not fit beside each other (`GridCursor::short`), it sweeps them again
/// as a probe, every leaf at its least and no column bounded, which gives each column its least, and then once more:
/// each column keeps at least its least, a `Content` column measuring in the room the others leave at theirs. When
/// the leasts do not fit beside each other, that last sweep lays the cells out one under another across the room
/// instead, so no column is cut. The real run places the cells from the widths, leasts and layout the measuring run
/// recorded.
fn run_grid<R>(ui: &mut Ui, env: &Env, id: Id, columns: &[ColumnWidth], gap: Vec2, mut body: impl FnMut(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let rtl = env.direction.is_rtl();
    let layout = child::column_layout(rtl);
    // One sweep of the cells in `ui` with the cursor `make` gives the flow of `ui`'s layout; returns what the cursor
    // measured.
    let mut sweep = |ui: &mut Ui, scoped: &Env, make: &dyn Fn(Flow) -> GridCursor| {
        let top = ui.available_rect_before_wrap().top();
        let cursor = make(Flow::new(ui.available_rect_before_wrap(), rtl));
        pass_state::with::<Cursors, _>(&ctx, |cursors| cursors.0.insert(id, Cursor::Grid(cursor)));
        let installed = Env::install_for(ui, scoped.derive_local(|scoped| scoped.grid = Some(id)));
        let inner = body(ui);
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
        (inner, measured, short)
    };
    if !measure::active(&ctx) {
        let stacked = this_pass(&ctx, false, sub(id, "stacked")).is_some_and(|stacked| stacked.x > 0.0);
        let columns = if stacked { STACKED } else { columns };
        let last = measured_tracks(&ctx, false, id, columns.len());
        return child::block(ui, None, layout, |ui| sweep(ui, env, &|flow| GridCursor::new(flow, columns, gap, &last)).0).0;
    }
    // A measuring run under a body sized from its content gives each fill column the width its cells take, as a
    // content column: the room it would share is not the body's to claim.
    let content_run = env.content.0;
    let cursor = |columns: &[ColumnWidth], last: &GridMeasure, flow: Flow| match content_run {
        true => GridCursor::sized_by_content(flow, columns, gap, last),
        false => GridCursor::new(flow, columns, gap, last),
    };
    let none = GridMeasure::default();
    let (inner, measured, rect, short) = crate::contract::keeping_ids(&ctx, || {
        let mut first = ui.new_child(egui::UiBuilder::new().layout(layout));
        let (inner, measured, short) = sweep(&mut first, env, &|flow| cursor(columns, &none, flow).each_alone());
        ((inner, measured, first.min_rect(), short), short)
    });
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
            let mut least = ui.new_child(egui::UiBuilder::new().layout(layout));
            sweep(&mut least, &probe, &|flow| GridCursor::unbounded(flow, columns, gap)).1.columns
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
    let (inner, measured, _) = child::block(ui, None, layout, |ui| sweep(ui, env, &|flow| cursor(columns, &last, flow))).0;
    record(&measured, &last.least, stacked, ui.available_rect_before_wrap().size());
    inner
}

/// What this pass's measuring run found under `id`. A measuring run itself reads nothing.
fn this_pass(ctx: &egui::Context, sizing: bool, id: Id) -> Option<Vec2> {
    if sizing { None } else { layout_memory::measured(ctx, id).map(|found| found.took) }
}

/// The line of the unwrapped `Row` body `id` in this pass's measuring run, with the line it shares already in it: how
/// far below its top the line's centre lies, and the row's height once its children sit on it.
fn row_line(ctx: &egui::Context, sizing: bool, id: Id) -> Option<(f32, f32)> {
    this_pass(ctx, sizing, sub(id, "line")).map(|took| (took.x, took.y)).filter(|(_, height)| *height > 0.0)
}

/// The line the unwrapped `Row` body `body` sets its children on: its own (`centre`, `height`), or the line `shared`
/// high it shares with its container's inline header or end region, centred, when that centre is lower.
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
    RowLine { body, centre, height }
}

/// Starts a sweep of a body's children in a measuring run: the body's first-line `target` and, for the unwrapped
/// `Row` body `row`, its line start afresh.
fn open_sweep(ctx: &egui::Context, target: Option<Id>, row: Option<Id>) {
    if let Some(target) = target {
        open_first_lines(ctx, &[target]);
    }
    if let Some(row) = row {
        pass_state::with::<RowLines, _>(ctx, |rows| rows.0.insert(row, LineTook::default()));
    }
}

/// Records the line of the unwrapped `Row` body `id` from what its children took in `ui`, the block they ran in,
/// with the line `shared` it shares with its container's regions; a real run records nothing. The last sweep of a
/// measuring run gives the line. The block takes the row's height once its children sit on the line, and the row
/// reports its line to the target its body reports a first line to.
fn record_line(ui: &mut Ui, sizing: bool, id: Id, shared: Option<f32>) {
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
    found(&ctx, sizing, sub(id, "line"), Vec2::ZERO, if height > 0.0 { Vec2::new(centre, height) } else { Vec2::ZERO });
    if height <= 0.0 {
        return;
    }
    let top = ui.max_rect().top();
    alloc::reach_down_to(ui, top + height);
    if let Some(target) = Env::of(ui).first_line {
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
pub(crate) fn run_body<R>(ui: &mut Ui, env: &Env, id: Id, layout: &BodyLayout, gap: Vec2, body: impl FnMut(&mut Ui) -> R) -> R {
    run_body_measured(ui, env, id, layout, gap, None, body).0
}

/// `run_body`, with the width the children take at the gap alone for a `Row` that places its block itself (`End`,
/// `Center`, `SpaceBetween`): the width the body asks for, where the room it is placed in spans wider. `shared` is the
/// line the body shares with its container's inline header or end region: an unwrapped `Row` sets its children on it.
pub(crate) fn run_body_measured<R>(
    ui: &mut Ui,
    env: &Env,
    id: Id,
    layout: &BodyLayout,
    gap: Vec2,
    shared: Option<f32>,
    mut body: impl FnMut(&mut Ui) -> R,
) -> (R, Option<f32>) {
    let rtl = env.direction.is_rtl();
    let ctx = ui.ctx().clone();
    let sizing = measure::active(&ctx);
    // Each sweep of a measuring run finds the body's first line and an unwrapped row's line afresh.
    let target = env.first_line.filter(|_| sizing);
    let row = (sizing && matches!(layout, BodyLayout::Row { wrap: false, .. })).then_some(id);
    let mut body = |ui: &mut Ui| {
        open_sweep(&ctx, target, row);
        body(ui)
    };
    match layout {
        BodyLayout::Stack => {
            // Each leaf shows at most the stack's room, never below its least.
            let env = env.derive_local(|scoped| scoped.shrink = Some(Shrink::Fit));
            let installed = Env::install_for(ui, env.clone());
            let inner = run_sharing(ui, &env, id, Axis::Y, child::column_layout(rtl), body);
            Env::restore(&ctx, installed);
            (inner, None)
        }
        BodyLayout::Row { wrap: false, align: RowAlign::SpaceBetween } => {
            let (inner, _) = child::block(ui, None, child::row_layout(rtl, false), |ui| {
                let flow = Flow::new(ui.available_rect_before_wrap(), rtl);
                let spread = Spread::new(flow, gap.x, this_pass(&ctx, sizing, id));
                pass_state::with::<Cursors, _>(&ctx, |cursors| cursors.0.insert(id, Cursor::Spread(spread)));
                let line = on_line(id, row_line(&ctx, sizing, id), shared);
                let installed = Env::install_for(
                    ui,
                    env.derive_local(|scoped| {
                        scoped.grid = Some(id);
                        scoped.row_line = Some(line);
                        scoped.first_line = None;
                    }),
                );
                let inner = body(ui);
                Env::restore(&ctx, installed);
                record_line(ui, sizing, id, shared);
                inner
            });
            let measured = match pass_state::with::<Cursors, _>(&ctx, |cursors| cursors.0.remove(&id)) {
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
            let line = on_line(id, row_line(&ctx, sizing, id), shared);
            let inner = run_sharing(ui, env, id, Axis::X, child::row_layout(rtl, false), |ui| {
                let installed = Env::install_for(
                    ui,
                    Env::of(ui).derive_local(|scoped| {
                        scoped.row_room = Some(room.width());
                        scoped.row_line = Some(line);
                        scoped.first_line = None;
                    }),
                );
                let inner = run_shrinking(ui, id, &mut body);
                Env::restore(&ctx, installed);
                record_line(ui, sizing, id, shared);
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
                    // In a body that scrolls along x the band is the width its view shows, not the scroll region.
                    let band = if Env::of(ui).unbounded.0 { shrink_room(ui) } else { room.width() };
                    // Without a measuring run, or in a band without an end, the block is placed as `Start`.
                    last.filter(|last| band.is_finite() && last.x > 0.0 && last.x < band).map(|last| {
                        let spare = band - last.x;
                        let lead = if *align == RowAlign::Center { spare / 2.0 } else { spare };
                        Flow::new(room, rtl).take_start(lead).1.rect()
                    })
                }
            };
            // A wrapped row has a line per wrap, so its children stay on the top of each, and its first child gives its
            // first line.
            let line = (!*wrap).then(|| on_line(id, row_line(&ctx, sizing, id), shared));
            let (inner, took) = child::block(ui, rect, child::row_layout(rtl, *wrap), |ui| {
                let installed = Env::install_for(
                    ui,
                    env.derive_local(|scoped| {
                        scoped.row_room = Some(room.width());
                        scoped.row_line = line;
                        if line.is_some() {
                            scoped.first_line = None;
                        }
                    }),
                );
                let inner = if *wrap { body(ui) } else { run_shrinking(ui, id, &mut body) };
                Env::restore(&ctx, installed);
                if !*wrap {
                    record_line(ui, sizing, id, shared);
                }
                inner
            });
            if *align != RowAlign::Start {
                found(&ctx, sizing, id, room.size(), took.size());
            }
            (inner, (*align != RowAlign::Start).then(|| took.width()))
        }
        BodyLayout::Grid { columns } => (run_grid(ui, env, id, columns, gap, body), None),
    }
}
