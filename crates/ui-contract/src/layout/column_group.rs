//! `ColumnGroup`: rows that are not one `Grid` share the widest label per column key.

use crate::runtime::layout_memory;
use crate::runtime::memory::sub;
use crate::runtime::pass_state;
use craft_behavior::ColumnKey;
use egui::{Context, Id, Ui, Vec2};
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

/// The column the label step puts side labels in.
pub(crate) const LABELS: ColumnKey = ColumnKey::new("labels");

/// The column the label step puts glyph labels (an icon or a short text such as "W") in.
pub(crate) const GLYPHS: ColumnKey = ColumnKey::new("glyphs");

/// What a row's report to an open group adds: its width in a column, or how far it reaches past a run of columns.
#[derive(Clone)]
enum Report {
    Width(ColumnKey, f32),
    Beyond(Vec<ColumnKey>, f32),
}

/// A row's report to the open frame `group` (a group, or a `Through` frame for a reach past columns), as a measuring
/// run hands it forward: a pass that takes a body's kept transcript makes it again, so the group's widths are what
/// the run would have found.
struct Reported {
    group: Id,
    report: Report,
}

impl layout_memory::Contribution for Reported {
    fn apply(&self, ctx: &Context) {
        add(ctx, self.group, self.report.clone());
    }

    fn report(&self) -> Option<(Id, f32)> {
        match &self.report {
            Report::Width(column, width) => Some((self.group.with(("width", column)), *width)),
            // Only a measuring run reports how far a row reaches past the columns: the group's extent is a measure.
            Report::Beyond(..) => None,
        }
    }

    fn undo(&self, ctx: &Context) -> layout_memory::TakeBack {
        let (group, report) = (self.group, self.report.clone());
        let was = with_frame(ctx, group, |widest, beyond| match &report {
            Report::Width(column, _) => widest.and_then(|widest| widest.get(column).copied()),
            Report::Beyond(columns, _) => beyond.get(columns).copied(),
        });
        Box::new(move |ctx| {
            let Some(was) = was else { return };
            with_frame(ctx, group, |widest, beyond| match (&report, was) {
                (Report::Width(column, _), Some(width)) => drop(widest.map(|widest| widest.insert(column.clone(), width))),
                (Report::Width(column, _), None) => drop(widest.map(|widest| widest.remove(column))),
                (Report::Beyond(columns, _), Some(past)) => drop(beyond.insert(columns.clone(), past)),
                (Report::Beyond(columns, _), None) => drop(beyond.remove(columns)),
            });
        })
    }
}

/// A frame's widest width per column: a group's; `None` for a `Through` frame, which keeps none.
type Widths<'a> = Option<&'a mut Columns>;

/// Runs `f` on the widths and the reaches past columns of the innermost open frame `frame`; `None` when that frame is
/// not open.
fn with_frame<R>(ctx: &Context, frame: Id, f: impl FnOnce(Widths<'_>, &mut HashMap<Vec<ColumnKey>, f32>) -> R) -> Option<R> {
    pass_state::with::<Open, _>(ctx, |open| {
        let (widest, beyond) = open.0.iter_mut().rev().find_map(|open| match open {
            Frame::Group { id, widest, beyond, .. } if *id == frame => Some((Some(widest), beyond)),
            Frame::Through { id, beyond, .. } if *id == frame => Some((None, beyond)),
            _ => None,
        })?;
        Some(f(widest, beyond))
    })
}

/// Adds `report` to the innermost open frame `frame`; `None` when that frame is not open, or it keeps no widths.
fn add(ctx: &Context, frame: Id, report: Report) -> Option<f32> {
    with_frame(ctx, frame, |widest, beyond| match report {
        Report::Width(column, content) => {
            let widest = widest?.entry(column).or_insert(0.0);
            *widest = widest.max(content);
            Some(*widest)
        }
        Report::Beyond(columns, past) => {
            let widest = beyond.entry(columns).or_insert(0.0);
            *widest = widest.max(past);
            Some(*widest)
        }
    })
    .flatten()
}

/// Makes `report` to `group` through layout memory, which hands it forward from a measuring run.
fn report(ctx: &Context, group: Id, report: Report) -> Option<f32> {
    layout_memory::contribute(ctx, std::sync::Arc::new(Reported { group, report: report.clone() }));
    // The contribution added it already; this reads what the group holds now.
    let read = match report {
        Report::Width(column, _) => Report::Width(column, 0.0),
        Report::Beyond(columns, _) => Report::Beyond(columns, 0.0),
    };
    add(ctx, group, read)
}

/// The groups open in this pass, innermost last.
#[derive(Default)]
pub(crate) struct Open(Vec<Frame>);

enum Frame {
    /// A group: rows report their widths and read the widest this pass found. `beyond` is, per run of columns a row
    /// sits after (in order from the start edge), the widest the row reaches past them (see [`report_beyond`]).
    /// `early`: per column, the narrowest width a row of a `sized` Through frame inside read in a measuring run.
    /// `depth`: the measuring runs open when the group opened (`layout_memory::depth`).
    Group { id: Id, widest: Columns, beyond: HashMap<Vec<ColumnKey>, f32>, early: Columns, room: Option<f32>, depth: usize },
    /// A plain container's body in the group `group`: its rows read and report that group's widths, as the rows
    /// directly in the group do. Their reaches past the columns stay here, and the container hands them on to the
    /// frame around it (`Reached`). `room`: the width the container was offered. `sized`: the container shows at the
    /// width it measured (content-sized and not held by a content column), so a measure of it at a column narrower
    /// than the group's final one would stand in the real run; a held body shows across the column whatever it
    /// measured, and a body given its width takes the room.
    Through { id: Id, group: Id, beyond: HashMap<Vec<ColumnKey>, f32>, room: f32, sized: bool },
    /// No group: rows inside read none, whatever group is open outside.
    Barrier,
}

impl Open {
    /// The group a row in the innermost open frame reads its widths from, and the frame its reaches past the columns
    /// go to; `None` when no group is open or a barrier is.
    fn innermost(&self) -> Option<(Id, Id)> {
        match self.0.last()? {
            Frame::Group { id, .. } => Some((*id, *id)),
            Frame::Through { id, group, .. } => Some((*group, *id)),
            Frame::Barrier => None,
        }
    }
}

/// A width per column key.
pub(crate) type Columns = HashMap<ColumnKey, f32>;

/// How far the rows of a plain container's body reached past the columns of the group around it, from a measuring
/// run of the body (`ColumnGroup::through`): per run of columns, the width those columns had when the body closed and
/// the most a row reached past them.
pub(crate) struct Reached(Vec<(Vec<ColumnKey>, f32, f32)>);

impl Reached {
    /// Hands the rows' reaches on to the frame open around the container, so the group measures its extent at its
    /// widest columns across the containers it holds. `past` gives, from the width a run of columns had and a row's
    /// reach past them, how far the container reaches past them.
    pub(crate) fn forward(&self, ctx: &Context, past: impl Fn(f32, f32) -> f32) {
        for (columns, width, reach) in &self.0 {
            report_beyond(ctx, columns, past(*width, *reach));
        }
    }
}

/// What a measuring run of a group's rows found (`ColumnGroup::scope_measured`).
#[derive(Default)]
pub(crate) struct Scoped {
    pub(crate) extent: Option<f32>,
    pub(crate) unsettled: Option<Columns>,
}

/// The widest content per column key of one scope, measured through the same `LayoutMemory` rule as a `Grid`
/// `Content` column.
pub struct ColumnGroup {
    id: Id,
    ctx: Context,
}

impl ColumnGroup {
    /// Runs `rows` with a group named `key` under `ui`. A measuring run records the widest width per column the rows
    /// reported, and a real run's rows read it.
    pub fn scope<R>(ui: &mut Ui, key: impl Hash + Debug, rows: impl FnOnce(&mut Ui, &mut ColumnGroup) -> R) -> R {
        Self::scope_measured(ui, key, None, rows).0
    }

    /// `scope`, with what a measuring run of the rows found once every row has reported. Only the end of the scope
    /// knows the widest columns, so a row that ran before the widest one measured against a narrower column than the
    /// real run gives it.
    /// - `extent`: for each run of columns rows reported reaching past, the sum of those columns' widest widths and the
    ///   most a row reaches past them.
    /// - `unsettled`: the group's widths, when a row of a body inside that shows at its measured width read a narrower
    ///   column: that body's measure is wrong at these widths, and the caller runs the rows again `seed`ed with them.
    pub(crate) fn scope_measured<R>(
        ui: &mut Ui,
        key: impl Hash + Debug,
        seed: Option<Columns>,
        rows: impl FnOnce(&mut Ui, &mut ColumnGroup) -> R,
    ) -> (R, Scoped) {
        let ctx = ui.ctx().clone();
        let id = sub(ui.scope_id(), key);
        let depth = layout_memory::depth(&ctx);
        let widest = seed.unwrap_or_default();
        pass_state::with::<Open, _>(&ctx, |open| {
            open.0.push(Frame::Group { id, widest, beyond: HashMap::new(), early: HashMap::new(), room: None, depth });
        });
        let mut group = ColumnGroup { id, ctx: ctx.clone() };
        let result = rows(ui, &mut group);
        let Some(Frame::Group { id, widest, beyond, early, .. }) = pass_state::with::<Open, _>(&ctx, |open| open.0.pop()) else {
            return (result, Scoped::default());
        };
        if !crate::runtime::measure::active(&ctx) {
            return (result, Scoped::default());
        }
        let mut widths = HashMap::new();
        for (column, width) in widest {
            // A row whose record a later sweep reused reports nothing there, so the widest of the pass stands.
            let width = column_width(&ctx, id, &column).map_or(width, |found| found.max(width));
            layout_memory::emit_record(&ctx, sub(id, &column), layout_memory::Measured::new(Vec2::ZERO, Vec2::new(width, 0.0)));
            widths.insert(column, width);
        }
        let extent = beyond
            .iter()
            .filter_map(|(columns, past)| columns.iter().map(|column| widths.get(column).copied()).sum::<Option<f32>>().map(|sum| sum + past))
            .reduce(f32::max);
        let early = early.iter().any(|(column, read)| widths.get(column).is_some_and(|width| width > read));
        (result, Scoped { extent, unsettled: early.then_some(widths) })
    }

    /// Runs `rows` with no group open: a labelled row inside reads no column, whatever group is open outside. A
    /// component's own label steps run in one, and the bodies that lay out apart from the group (`shared_labels`).
    pub fn barrier<R>(ui: &mut Ui, rows: impl FnOnce(&mut Ui) -> R) -> R {
        let ctx = ui.ctx().clone();
        pass_state::with::<Open, _>(&ctx, |open| open.0.push(Frame::Barrier));
        let result = rows(ui);
        pass_state::with::<Open, _>(&ctx, |open| open.0.pop());
        result
    }

    /// Runs the `rows` of a plain container's body, offered `room`: inside a group they read and report its widths, as
    /// the rows directly in it do, so the labelled rows of a section, a box or a row line up with the group's.
    /// Outside a group they read none. `sized`: the container shows at the width it measures (`Frame::Through`). A
    /// measuring run returns how far the rows reached past the columns.
    pub(crate) fn through<R>(ui: &mut Ui, id: Id, room: f32, sized: bool, rows: impl FnOnce(&mut Ui) -> R) -> (R, Option<Reached>) {
        let ctx = ui.ctx().clone();
        let Some((group, _)) = pass_state::with::<Open, _>(&ctx, |open| open.innermost()) else { return (Self::barrier(ui, rows), None) };
        pass_state::with::<Open, _>(&ctx, |open| open.0.push(Frame::Through { id, group, beyond: HashMap::new(), room, sized }));
        let result = rows(ui);
        let Some(Frame::Through { beyond, .. }) = pass_state::with::<Open, _>(&ctx, |open| open.0.pop()) else { return (result, None) };
        if !crate::runtime::measure::active(&ctx) || beyond.is_empty() {
            return (result, None);
        }
        let reached = beyond.into_iter().map(|(columns, past)| {
            let width = columns.iter().map(|column| current_width(&ctx, group, column)).sum();
            (columns, width, past)
        });
        (result, Some(Reached(reached.collect())))
    }

    /// Records the width the group's container was offered. A row that sits directly in the group decides whether
    /// its label stacks against that width, not against the width the container shrank to last pass, so a row that
    /// needs more than last pass's content widens it instead of stacking.
    pub(crate) fn offer(&mut self, width: f32) {
        let id = self.id;
        pass_state::with::<Open, _>(&self.ctx, |open| {
            if let Some(Frame::Group { room, .. }) =
                open.0.iter_mut().rev().find(|frame| matches!(frame, Frame::Group { id: group, .. } if *group == id))
            {
                *room = Some(width);
            }
        });
    }

    /// The width every row gives column `column`: the widest content this pass's measuring run found, or in that run
    /// the widest so far. `content` is this row's own width.
    pub fn width(&mut self, ui: &Ui, column: &ColumnKey, content: f32) -> f32 {
        debug_assert!(ui.ctx() == &self.ctx, "a column group is used in the context that opened it");
        shared_width(&self.ctx, self.id, column, content)
    }
}

/// The width of `column` in the group the innermost open frame reads, `None` when no group is open or a barrier is.
/// The row reports `content`.
#[doc(hidden)]
pub fn ambient_width(ctx: &Context, column: &ColumnKey, content: f32) -> Option<f32> {
    let (group, _) = pass_state::with::<Open, _>(ctx, |open| open.innermost())?;
    let width = shared_width(ctx, group, column, content);
    // A row of a body that shows at its measured width reads the column into that measure: the group notes how
    // narrow, so its scope can tell the measure from one at its final width.
    if crate::runtime::measure::active(ctx) {
        pass_state::with::<Open, _>(ctx, |open| {
            let mut sized = false;
            for frame in open.0.iter_mut().rev() {
                match frame {
                    Frame::Through { sized: through, .. } => sized |= *through,
                    Frame::Group { id, early, .. } if *id == group => {
                        if sized {
                            let read = early.entry(column.clone()).or_insert(width);
                            *read = read.min(width);
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });
    }
    Some(width)
}

/// The width the innermost open frame's container was offered, `None` when no group is open or a barrier is, or the
/// container recorded none.
pub(crate) fn ambient_room(ctx: &Context) -> Option<f32> {
    pass_state::with::<Open, _>(ctx, |open| match open.0.last() {
        Some(Frame::Group { room, .. }) => *room,
        Some(Frame::Through { room, .. }) => Some(*room),
        _ => None,
    })
}

/// Reports that a row of the innermost open frame, in a content-sized column, sits after `columns` (in order from the
/// start edge: a label column, or a menu row's mark and icon columns) and reaches `past` beyond the last of them: for
/// a labelled row the gap and its control. The group's extent sums the columns' widest widths before `past`. Nothing
/// happens where no group is open or a barrier is.
#[doc(hidden)]
pub fn report_beyond(ctx: &Context, columns: &[ColumnKey], past: f32) {
    if let Some((_, frame)) = pass_state::with::<Open, _>(ctx, |open| open.innermost()) {
        report(ctx, frame, Report::Beyond(columns.to_vec(), past));
    }
}

fn shared_width(ctx: &Context, group: Id, column: &ColumnKey, content: f32) -> f32 {
    let so_far = report(ctx, group, Report::Width(column.clone(), content)).unwrap_or(content);
    let width = column_width(ctx, group, column).map_or(so_far, |found| found.max(so_far));
    // The rows' measures depend on the width as well as on their room: a sweep that has seen only some of the labels
    // gives a narrower one.
    let depth = pass_state::with::<Open, _>(ctx, |open| {
        open.0.iter().rev().find_map(|frame| match frame {
            Frame::Group { id, depth, .. } if *id == group => Some(*depth),
            _ => None,
        })
    });
    match depth {
        Some(depth) => layout_memory::read_input(ctx, depth, sub(group, column), width),
        None => width,
    }
}

/// The width `column` of the open group `group` has now, without a report.
fn current_width(ctx: &Context, group: Id, column: &ColumnKey) -> f32 {
    let so_far = with_frame(ctx, group, |widest, _| widest.and_then(|widest| widest.get(column).copied())).flatten().unwrap_or(0.0);
    column_width(ctx, group, column).map_or(so_far, |found| found.max(so_far))
}

/// The widest width of `column` a measuring run of this pass recorded for `group`.
fn column_width(ctx: &Context, group: Id, column: &ColumnKey) -> Option<f32> {
    layout_memory::measured(ctx, sub(group, column)).map(|found| found.took.x)
}
