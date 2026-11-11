//! `ColumnGroup`: rows that are not one `Grid` share the widest label per column key.

use crate::runtime::egui::memory::Service;
use crate::runtime::layout_memory;
use crate::runtime::memory::sub;
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

/// A row's report to the group `group`, as a measuring run hands it forward: a pass that takes a body's kept
/// transcript makes it again, so the group's widths are what the run would have found.
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
}

/// Adds `report` to the innermost open group `group`; `None` when that group is not open.
fn add(ctx: &Context, group: Id, report: Report) -> Option<f32> {
    Service::<Open>::run(ctx, |open| {
        let (widest, beyond) = open.0.iter_mut().rev().find_map(|frame| match frame {
            Frame::Group { id, widest, beyond, .. } if *id == group => Some((widest, beyond)),
            _ => None,
        })?;
        Some(match report {
            Report::Width(column, content) => {
                let widest = widest.entry(column).or_insert(0.0);
                *widest = widest.max(content);
                *widest
            }
            Report::Beyond(columns, past) => {
                let widest = beyond.entry(columns).or_insert(0.0);
                *widest = widest.max(past);
                *widest
            }
        })
    })
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
struct Open(Vec<Frame>);

enum Frame {
    /// A group: rows report their widths and read the widest this pass found. `beyond` is, per run of columns a row
    /// sits after (in order from the start edge), the widest the row reaches past them (see [`report_beyond`]).
    /// `depth`: the measuring runs open when the group opened (`layout_memory::depth`).
    Group { id: Id, widest: HashMap<ColumnKey, f32>, beyond: HashMap<Vec<ColumnKey>, f32>, room: Option<f32>, depth: usize },
    /// No group: rows inside read none, whatever group is open outside.
    Barrier,
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
        Self::scope_measured(ui, key, rows).0
    }

    /// `scope`, with the width the group's rows take in a measuring run once every row has reported: for each run of
    /// columns rows reported reaching past, the sum of those columns' widest widths and the most a row reaches past
    /// them. Only the end of the scope knows the widest columns, so a row that ran before the widest one measured
    /// against a narrower column than the real run gives it.
    pub(crate) fn scope_measured<R>(ui: &mut Ui, key: impl Hash + Debug, rows: impl FnOnce(&mut Ui, &mut ColumnGroup) -> R) -> (R, Option<f32>) {
        let ctx = ui.ctx().clone();
        let id = sub(ui.scope_id(), key);
        let depth = layout_memory::depth(&ctx);
        Service::<Open>::run(&ctx, |open| open.0.push(Frame::Group { id, widest: HashMap::new(), beyond: HashMap::new(), room: None, depth }));
        let mut group = ColumnGroup { id, ctx: ctx.clone() };
        let result = rows(ui, &mut group);
        let Some(Frame::Group { id, widest, beyond, .. }) = Service::<Open>::run(&ctx, |open| open.0.pop()) else { return (result, None) };
        if !crate::runtime::measure::active(&ctx) {
            return (result, None);
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
        (result, extent)
    }

    /// Runs `rows` with no group open: a labelled row inside reads no column, so only the rows that sit directly in a
    /// group's body share its widths. A component's own label steps and a nested container's body run in one.
    pub fn barrier<R>(ui: &mut Ui, rows: impl FnOnce(&mut Ui) -> R) -> R {
        let ctx = ui.ctx().clone();
        Service::<Open>::run(&ctx, |open| open.0.push(Frame::Barrier));
        let result = rows(ui);
        Service::<Open>::run(&ctx, |open| open.0.pop());
        result
    }

    /// Records the width the group's container was offered. A row that sits directly in the group decides whether
    /// its label stacks against that width, not against the width the container shrank to last pass, so a row that
    /// needs more than last pass's content widens it instead of stacking.
    pub(crate) fn offer(&mut self, width: f32) {
        let id = self.id;
        Service::<Open>::run(&self.ctx, |open| {
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

/// The width of `column` in the innermost open frame, `None` when no group is open or a barrier is. The row reports
/// `content`.
#[doc(hidden)]
pub fn ambient_width(ctx: &Context, column: &ColumnKey, content: f32) -> Option<f32> {
    let id = Service::<Open>::run(ctx, |open| match open.0.last() {
        Some(Frame::Group { id, .. }) => Some(*id),
        _ => None,
    })?;
    Some(shared_width(ctx, id, column, content))
}

/// The width the innermost open frame's container was offered, `None` when no group is open or a barrier is, or the
/// container recorded none.
pub(crate) fn ambient_room(ctx: &Context) -> Option<f32> {
    Service::<Open>::run(ctx, |open| match open.0.last() {
        Some(Frame::Group { room, .. }) => *room,
        _ => None,
    })
}

/// Reports that a row of the innermost open group, in a content-sized column, sits after `columns` (in order from the
/// start edge: a label column, or a menu row's mark and icon columns) and reaches `past` beyond the last of them: for
/// a labelled row the gap and its control. The group's extent sums the columns' widest widths before `past`. Nothing
/// happens where no group is open or a barrier is.
#[doc(hidden)]
pub fn report_beyond(ctx: &Context, columns: &[ColumnKey], past: f32) {
    let group = Service::<Open>::run(ctx, |open| match open.0.last() {
        Some(Frame::Group { id, .. }) => Some(*id),
        _ => None,
    });
    if let Some(group) = group {
        report(ctx, group, Report::Beyond(columns.to_vec(), past));
    }
}

fn shared_width(ctx: &Context, group: Id, column: &ColumnKey, content: f32) -> f32 {
    let so_far = report(ctx, group, Report::Width(column.clone(), content)).unwrap_or(content);
    let width = column_width(ctx, group, column).map_or(so_far, |found| found.max(so_far));
    // The rows' measures depend on the width as well as on their room: a sweep that has seen only some of the labels
    // gives a narrower one.
    let depth = Service::<Open>::run(ctx, |open| {
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

/// The widest width of `column` a measuring run of this pass recorded for `group`.
fn column_width(ctx: &Context, group: Id, column: &ColumnKey) -> Option<f32> {
    layout_memory::measured(ctx, sub(group, column)).map(|found| found.took.x)
}
