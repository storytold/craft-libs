//! The ctx-side view of a measuring run: the sizing pass the framework starts to measure a container's contents
//! before the real run. APIs that get only a `Context` ask here whether to skip an effect, so a measuring run
//! delivers nothing the real run then lacks.

use super::egui::memory as store;
use super::pass_state;
use egui::Context;
#[cfg(feature = "testing")]
use egui::Id;

/// The widths per column the items measured inside `cell_widths` report, innermost call last.
#[derive(Default)]
struct CellWidths(Vec<Vec<(craft_behavior::ColumnKey, f32)>>);

/// Runs `measure` (one item's measure) and returns what it returned with the widest content it reported for each
/// column (`MeasureCx::cell_width`).
pub(crate) fn cell_widths<R>(ctx: &Context, measure: impl FnOnce() -> R) -> (R, Vec<(craft_behavior::ColumnKey, f32)>) {
    store::with::<CellWidths, _>(ctx, |open| open.0.push(Vec::new()));
    let out = measure();
    let widths = store::with::<CellWidths, _>(ctx, |open| open.0.pop()).unwrap_or_default();
    (out, widths)
}

/// Records `width` for `column` in the innermost open `cell_widths`; outside one it does nothing.
pub(crate) fn report_cell_width(ctx: &Context, column: &craft_behavior::ColumnKey, width: f32) {
    store::with::<CellWidths, _>(ctx, |open| {
        let Some(widths) = open.0.last_mut() else { return };
        match widths.iter_mut().find(|(held, _)| held == column) {
            Some((_, held)) => *held = held.max(width),
            None => widths.push((column.clone(), width)),
        }
    });
}

/// How many measuring runs enclose the running code of a context.
#[derive(Default)]
pub(crate) struct Measuring(u32);

/// The effects skipped in measuring runs, with the pass that logged them.
#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
struct EffectLog {
    pass: u64,
    effects: Vec<&'static str>,
}

/// The bodies whose contents ran in a measuring run, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
struct BodyRuns {
    pass: u64,
    runs: Vec<(Id, u32)>,
}

/// The derived values built in a pass, by component id, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
pub(crate) struct DerivedBuilds {
    pass: u64,
    builds: Vec<(Id, u32)>,
    /// The frame `frame_builds` counts, and the builds of every pass of it.
    frame: u64,
    frame_builds: Vec<(Id, u32)>,
}

/// How many contributions the measuring runs of a pass made that stand, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
struct Contributions {
    pass: u64,
    count: i64,
}

/// The runs of each component type, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
pub(crate) struct TypeRuns {
    pass: u64,
    runs: Vec<(&'static str, u32)>,
}

/// The collections whose rows were built and measured for the largest, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
struct WidestRuns {
    pass: u64,
    runs: Vec<(Id, u32)>,
}

/// The auto rows of collections built and measured for their height, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
struct RowMeasures {
    pass: u64,
    rows: Vec<(Id, u32)>,
}

/// The collection rows whose item was built, by item id, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
struct RowBuilds {
    pass: u64,
    rows: Vec<(Id, u32)>,
}

/// The rows that asked their body units for their least widths, with the pass that counted them.
#[cfg(feature = "testing")]
#[derive(Default)]
struct LeastRuns {
    pass: u64,
    runs: Vec<(Id, u32)>,
}

/// Runs `f` as a measuring run, for code outside the framework that measures before it shows (an app that sizes a rect
/// by `Show::measure_in` and then shows the component with `show_at`). Nested calls are fine.
///
/// What `f` measures is its own: it joins no kept transcript of the body around it, in that body's measuring run or in
/// its real run alike. Otherwise the body's measuring run would keep it as one of the body's components while the
/// real run, measuring outside every transcript, never asks it, and the body would read as changed every frame.
pub fn run<R>(ctx: &Context, f: impl FnOnce() -> R) -> R {
    measuring(ctx, || super::layout_memory::boundary(ctx, true, f))
}

/// Runs `f` as a measuring run whose measures join the transcript of the body whose measuring run is open around it:
/// the framework's own measuring runs. Nested calls are fine.
pub(crate) fn measuring<R>(ctx: &Context, f: impl FnOnce() -> R) -> R {
    /// Restores the depth when the run ends, by a panic too: a context used after a caught panic is not measuring.
    struct Open<'a>(&'a Context, u32);
    impl Drop for Open<'_> {
        fn drop(&mut self) {
            pass_state::with::<Measuring, _>(self.0, |depth| depth.0 = self.1);
        }
    }
    let before = pass_state::with::<Measuring, _>(ctx, |depth| {
        depth.0 += 1;
        depth.0 - 1
    });
    let _open = Open(ctx, before);
    super::layout_memory::guard::measuring(f)
}

fn depth(ctx: &Context) -> u32 {
    pass_state::with::<Measuring, _>(ctx, |depth| depth.0)
}

/// Whether a measuring run is in progress.
pub fn active(ctx: &Context) -> bool {
    depth(ctx) > 0
}

/// Whether the innermost measuring run is the outermost one: no other measuring run encloses it.
pub(crate) fn outermost(ctx: &Context) -> bool {
    depth(ctx) == 1
}

/// Whether the effect `what` may happen: `false` in a measuring run, and the caller skips it. Under test the
/// skipped effects are logged for `take_effects`.
pub fn effect(ctx: &Context, what: &'static str) -> bool {
    if !active(ctx) {
        return true;
    }
    #[cfg(any(test, feature = "testing"))]
    let frame = ctx.cumulative_pass_nr();
    #[cfg(any(test, feature = "testing"))]
    store::with::<EffectLog, _>(ctx, |log| {
        if log.pass != frame {
            *log = EffectLog { pass: frame, effects: Vec::new() };
        }
        log.effects.push(what);
    });
    #[cfg(not(any(test, feature = "testing")))]
    let _ = what;
    false
}

/// Takes the entries of the one-shot `queue` that `wanted` accepts. A measuring run takes none and leaves the queue
/// whole, so the real run finds them. Every take of a one-shot queue that a body or a component's
/// steps can reach goes through here.
pub fn take_queued<T>(ctx: &Context, queue: &mut Vec<T>, wanted: impl Fn(&T) -> bool) -> Vec<T> {
    if active(ctx) {
        return Vec::new();
    }
    let mut mine = Vec::new();
    let mut kept = Vec::new();
    for item in std::mem::take(queue) {
        if wanted(&item) { mine.push(item) } else { kept.push(item) }
    }
    *queue = kept;
    mine
}

/// The effects skipped in measuring runs this frame, in order; empties the log.
#[cfg(feature = "testing")]
pub fn take_effects(ctx: &Context) -> Vec<&'static str> {
    let frame = ctx.cumulative_pass_nr();
    store::with::<EffectLog, _>(ctx, |log| if log.pass == frame { std::mem::take(&mut log.effects) } else { Vec::new() })
}

/// Whether the running code is a run of the replay audit (`runtime::audit`): that is the audit's work, not the pass's,
/// so the count trials count nothing in it.
#[cfg(feature = "testing")]
fn uncounted(ctx: &Context) -> bool {
    super::replay::in_audit_run(ctx)
}

/// Counts a measuring run of the contents of the body `id` in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_body(ctx: &Context, id: Id) {
    if uncounted(ctx) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    store::with::<BodyRuns, _>(ctx, |log| {
        if log.pass != pass {
            *log = BodyRuns { pass, runs: Vec::new() };
        }
        match log.runs.iter_mut().find(|(body, _)| *body == id) {
            Some((_, count)) => *count += 1,
            None => log.runs.push((id, 1)),
        }
    });
}

/// Counts a contribution a measuring run made in this pass (`delta` 1), or a draft took back (-1).
#[cfg(feature = "testing")]
pub(crate) fn count_contribution(ctx: &Context, delta: i64) {
    if uncounted(ctx) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    store::with::<Contributions, _>(ctx, |log| {
        if log.pass != pass {
            *log = Contributions { pass, count: 0 };
        }
        log.count += delta;
    });
}

/// How many contributions the measuring runs made that stand, in the last pass whose runs made any: a draft that was
/// taken back counts none.
#[cfg(feature = "testing")]
pub fn contributions(ctx: &Context) -> i64 {
    store::with::<Contributions, _>(ctx, |log| log.count)
}

/// The bodies whose contents ran in a measuring run in the last pass, and how many runs each; empty when that pass
/// ran none.
#[cfg(feature = "testing")]
pub fn last_pass_body_runs(ctx: &Context) -> Vec<(Id, u32)> {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    store::with::<BodyRuns, _>(ctx, |log| if log.pass == last { log.runs.clone() } else { Vec::new() })
}

/// The bodies whose contents ran in a measuring run in the last pass that ran one, and how many runs each.
#[cfg(feature = "testing")]
pub fn body_runs(ctx: &Context) -> Vec<(Id, u32)> {
    store::with::<BodyRuns, _>(ctx, |log| log.runs.clone())
}

/// Counts a run of a component of the type `name` in this pass, measuring or real. Counts nothing without `testing`.
pub(crate) fn count_run(ctx: &Context, name: &'static str) {
    #[cfg(feature = "testing")]
    if !uncounted(ctx) {
        let pass = ctx.cumulative_pass_nr();
        pass_state::with::<TypeRuns, _>(ctx, |log| {
            if log.pass != pass {
                *log = TypeRuns { pass, runs: Vec::new() };
            }
            match log.runs.iter_mut().find(|(ty, _)| *ty == name) {
                Some((_, count)) => *count += 1,
                None => log.runs.push((name, 1)),
            }
        });
    }
    #[cfg(not(feature = "testing"))]
    let _ = (ctx, name);
}

/// The component types that ran in the last pass, by type name, and how many runs each, measuring runs included;
/// empty when that pass ran none.
#[cfg(feature = "testing")]
pub fn last_pass_runs(ctx: &Context) -> Vec<(&'static str, u32)> {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    pass_state::with::<TypeRuns, _>(ctx, |log| if log.pass == last { log.runs.clone() } else { Vec::new() })
}

/// Counts a build and measure of the rows of the collection `id` for the largest in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_widest(ctx: &Context, id: Id) {
    if uncounted(ctx) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    store::with::<WidestRuns, _>(ctx, |log| {
        if log.pass != pass {
            *log = WidestRuns { pass, runs: Vec::new() };
        }
        match log.runs.iter_mut().find(|(collection, _)| *collection == id) {
            Some((_, count)) => *count += 1,
            None => log.runs.push((id, 1)),
        }
    });
}

/// The collections whose rows were built and measured for the largest in the last pass, and how many times each.
#[cfg(feature = "testing")]
pub fn widest_runs(ctx: &Context) -> Vec<(Id, u32)> {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    store::with::<WidestRuns, _>(ctx, |log| if log.pass == last { log.runs.clone() } else { Vec::new() })
}

/// Counts a build and measure of the height of a collection's auto row, kept under `id`, in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_row_measure(ctx: &Context, id: Id) {
    if uncounted(ctx) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    store::with::<RowMeasures, _>(ctx, |log| {
        if log.pass != pass {
            *log = RowMeasures { pass, rows: Vec::new() };
        }
        match log.rows.iter_mut().find(|(row, _)| *row == id) {
            Some((_, count)) => *count += 1,
            None => log.rows.push((id, 1)),
        }
    });
}

/// The auto rows of collections built and measured for their height in the last pass, and how many times each.
#[cfg(feature = "testing")]
pub fn row_measures(ctx: &Context) -> Vec<(Id, u32)> {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    store::with::<RowMeasures, _>(ctx, |log| if log.pass == last { log.rows.clone() } else { Vec::new() })
}

/// Counts a build of the item `id` of a collection row in this pass: for a measure, or to show it.
#[cfg(feature = "testing")]
pub(crate) fn count_row_build(ctx: &Context, id: Id) {
    if uncounted(ctx) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    store::with::<RowBuilds, _>(ctx, |log| {
        if log.pass != pass {
            *log = RowBuilds { pass, rows: Vec::new() };
        }
        match log.rows.iter_mut().find(|(row, _)| *row == id) {
            Some((_, count)) => *count += 1,
            None => log.rows.push((id, 1)),
        }
    });
}

/// The collection rows whose item was built in the last pass, by item id, and how many times each.
#[cfg(feature = "testing")]
pub fn row_builds(ctx: &Context) -> Vec<(Id, u32)> {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    store::with::<RowBuilds, _>(ctx, |log| if log.pass == last { log.rows.clone() } else { Vec::new() })
}

/// Counts a chord text an access report read for a node in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_chord_text(ctx: &Context) {
    totals(ctx, |log| log.chord_texts += 1);
}

/// The chord texts the access reports of the last pass read: one per command-bound node that exists.
#[cfg(feature = "testing")]
pub fn chord_texts(ctx: &Context) -> u32 {
    last_totals(ctx, |log| log.chord_texts, 0)
}

/// Counts a component's `Access` built for its report in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_access_build(ctx: &Context) {
    totals(ctx, |log| log.access_builds += 1);
}

/// The `Access` values the reports of the last pass built: none for a component whose report has no reader of them.
#[cfg(feature = "testing")]
pub fn last_pass_access_builds(ctx: &Context) -> u32 {
    last_totals(ctx, |log| log.access_builds, 0)
}

/// Counts a tooltip text built in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_tip_text(ctx: &Context) {
    totals(ctx, |log| log.tip_texts += 1);
}

/// The tooltip texts built in the last pass: only for a tooltip the pointer or the keyboard wants, or one still shown.
#[cfg(feature = "testing")]
pub fn last_pass_tip_texts(ctx: &Context) -> u32 {
    last_totals(ctx, |log| log.tip_texts, 0)
}

/// What a command resolution was for.
#[cfg(feature = "testing")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Resolve {
    /// A resolution nothing kept: a reader's first, or one for a key press or a fire.
    New,
    /// A kept resolution made again because an input of it changed.
    Again,
    /// A kept resolution compared with a full one by the testing check every 100th pass.
    Checked,
}

/// Counts a command resolution in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_resolve(ctx: &Context, what: Resolve) {
    totals(ctx, |log| log.resolves[what as usize] += 1);
}

/// The command resolutions of the last pass of kind `what`.
#[cfg(feature = "testing")]
pub fn last_pass_resolves(ctx: &Context, what: Resolve) -> u32 {
    last_totals(ctx, |log| log.resolves[what as usize], 0)
}

/// Counts a kept resolution a wake walked to compare its key.
#[cfg(feature = "testing")]
pub(crate) fn count_wake_walk(ctx: &Context) {
    totals(ctx, |log| log.wake_walks += 1);
}

/// The kept resolutions the wakes of the last pass walked.
#[cfg(feature = "testing")]
pub fn last_pass_wake_walks(ctx: &Context) -> u32 {
    last_totals(ctx, |log| log.wake_walks, 0)
}

/// Counts a sweep of the row `id` that asks its body units for their least widths in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_least(ctx: &Context, id: Id) {
    if uncounted(ctx) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    store::with::<LeastRuns, _>(ctx, |log| {
        if log.pass != pass {
            *log = LeastRuns { pass, runs: Vec::new() };
        }
        match log.runs.iter_mut().find(|(row, _)| *row == id) {
            Some((_, count)) => *count += 1,
            None => log.runs.push((id, 1)),
        }
    });
}

/// The rows that asked their body units for their least widths in the last pass that did so, and how many times each.
#[cfg(feature = "testing")]
pub fn least_runs(ctx: &Context) -> Vec<(Id, u32)> {
    store::with::<LeastRuns, _>(ctx, |log| log.runs.clone())
}

/// How a memoized view, or a scrolled `Lazy` child, took part in a pass.
#[cfg(feature = "testing")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoRun {
    /// It ran.
    Ran,
    /// Its last run was replayed in place of running.
    Replayed,
    /// It is out of view and took its last extent without running.
    OutOfView,
}

/// How each memoized view took part in one pass (real runs only), and in every pass of one frame (every call, the
/// calls of sizing and measuring runs included).
#[cfg(feature = "testing")]
#[derive(Default)]
pub(crate) struct MemoLog {
    pass: u64,
    runs: Vec<(&'static str, Id, MemoRun)>,
    frame: u64,
    frame_runs: Vec<(&'static str, Id, MemoRun)>,
}

/// Counts how the view `name` (`id`) took part in a call of this pass; `real` for a call in a real run. Fed by
/// `contract::Lazy`'s one runner, which memoized views share.
#[cfg(feature = "testing")]
pub(crate) fn count_memo_run(ctx: &Context, name: &'static str, id: Id, run: MemoRun, real: bool) {
    if uncounted(ctx) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    let frame = super::egui::pass::frame_nr(ctx, super::egui::pass::current_viewport(ctx));
    pass_state::with::<MemoLog, _>(ctx, |log| {
        if log.frame != frame {
            log.frame = frame;
            log.frame_runs.clear();
        }
        log.frame_runs.push((name, id, run));
        if real {
            if log.pass != pass {
                log.pass = pass;
                log.runs.clear();
            }
            log.runs.push((name, id, run));
        }
    });
}

/// How the memoized views (by name, `"Lazy"` for a `Lazy` child) took part in the real run of the last pass, in the
/// order they were decided; empty when that pass decided none.
#[cfg(feature = "testing")]
pub fn last_pass_memo_runs(ctx: &Context) -> Vec<(&'static str, Id, MemoRun)> {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    pass_state::with::<MemoLog, _>(ctx, |log| if log.pass == last { log.runs.clone() } else { Vec::new() })
}

/// How the memoized views took part in every pass of the last frame, in order: every call, those of sizing and
/// measuring runs included (a `Ran` there is a measuring run, which keeps no record). Empty when that frame decided
/// none.
#[cfg(feature = "testing")]
pub fn last_frame_memo_runs(ctx: &Context) -> Vec<(&'static str, Id, MemoRun)> {
    let viewport = super::egui::pass::current_viewport(ctx);
    let last = super::egui::pass::frame_nr(ctx, viewport).saturating_sub(1);
    pass_state::with::<MemoLog, _>(ctx, |log| if log.frame == last { log.frame_runs.clone() } else { Vec::new() })
}

/// Bytes fed into memo keys in one pass, and the layouts, measure asks, chord texts, `Access` values and tooltip texts
/// of that pass.
#[cfg(feature = "testing")]
#[derive(Default)]
pub(crate) struct Totals {
    pass: u64,
    key_bytes: usize,
    text_layouts: u32,
    chord_texts: u32,
    access_builds: u32,
    tip_texts: u32,
    resolves: [u32; 3],
    wake_walks: u32,
    asks: Vec<(&'static str, u32)>,
}

/// The totals of the running pass and of the one before it, which a pass's own counting does not overwrite: a pass
/// that counts in its begin hook keeps the last pass readable.
#[cfg(feature = "testing")]
#[derive(Default)]
pub(crate) struct PassTotals {
    current: Totals,
    previous: Totals,
}

#[cfg(feature = "testing")]
fn totals<R>(ctx: &Context, f: impl FnOnce(&mut Totals) -> R) -> R {
    let pass = ctx.cumulative_pass_nr();
    pass_state::with::<PassTotals, _>(ctx, |log| {
        if log.current.pass != pass {
            let current = std::mem::replace(&mut log.current, Totals { pass, ..Totals::default() });
            log.previous = current;
        }
        f(&mut log.current)
    })
}

#[cfg(feature = "testing")]
fn last_totals<R>(ctx: &Context, f: impl FnOnce(&Totals) -> R, none: R) -> R {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    pass_state::with::<PassTotals, _>(ctx, |log| {
        if log.current.pass == last {
            f(&log.current)
        } else if log.previous.pass == last {
            f(&log.previous)
        } else {
            none
        }
    })
}

/// Counts `bytes` fed into a memo key in this pass: a memoized view's key, before its run and after it.
#[cfg(feature = "testing")]
pub(crate) fn count_memo_key_bytes(ctx: &Context, bytes: usize) {
    if uncounted(ctx) {
        return;
    }
    totals(ctx, |log| log.key_bytes += bytes);
}

/// The bytes fed into memo keys in the last pass.
#[cfg(feature = "testing")]
pub fn last_pass_memo_key_bytes(ctx: &Context) -> usize {
    last_totals(ctx, |log| log.key_bytes, 0)
}

/// Counts a build of a derived value of the component `id` in this pass (`runtime::derived`).
#[cfg(feature = "testing")]
pub(crate) fn count_derived(ctx: &Context, id: Id) {
    if uncounted(ctx) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    let frame = super::egui::pass::frame_nr(ctx, super::egui::pass::current_viewport(ctx));
    pass_state::with::<DerivedBuilds, _>(ctx, |log| {
        if log.frame != frame {
            log.frame = frame;
            log.frame_builds.clear();
        }
        if log.pass != pass {
            log.pass = pass;
            log.builds.clear();
        }
        for builds in [&mut log.builds, &mut log.frame_builds] {
            match builds.iter_mut().find(|(of, _)| *of == id) {
                Some((_, count)) => *count += 1,
                None => builds.push((id, 1)),
            }
        }
    });
}

/// The components that built a derived value in any pass of the last frame, and how many builds each.
#[cfg(feature = "testing")]
pub fn last_frame_derived_builds(ctx: &Context) -> Vec<(Id, u32)> {
    let last = super::egui::pass::frame_nr(ctx, super::egui::pass::current_viewport(ctx)).saturating_sub(1);
    pass_state::with::<DerivedBuilds, _>(ctx, |log| if log.frame == last { log.frame_builds.clone() } else { Vec::new() })
}

/// The components that built a derived value in the last pass, and how many builds each; empty when that pass built
/// none.
#[cfg(feature = "testing")]
pub fn last_pass_derived_builds(ctx: &Context) -> Vec<(Id, u32)> {
    let last = ctx.cumulative_pass_nr().saturating_sub(1);
    pass_state::with::<DerivedBuilds, _>(ctx, |log| if log.pass == last { log.builds.clone() } else { Vec::new() })
}

/// Counts a text layout handed to egui (`text_layout::layout`, `layout_with`) in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_text_layout(ctx: &Context) {
    if uncounted(ctx) {
        return;
    }
    totals(ctx, |log| log.text_layouts += 1);
}

/// The text layouts handed to egui in the last pass.
#[cfg(feature = "testing")]
pub fn last_pass_text_layouts(ctx: &Context) -> u32 {
    last_totals(ctx, |log| log.text_layouts, 0)
}

/// Counts a measure call of a leaf component of the type `name` in this pass.
#[cfg(feature = "testing")]
pub(crate) fn count_measure_ask(ctx: &Context, name: &'static str) {
    if uncounted(ctx) {
        return;
    }
    totals(ctx, |log| match log.asks.iter_mut().find(|(ty, _)| *ty == name) {
        Some((_, count)) => *count += 1,
        None => log.asks.push((name, 1)),
    });
}

/// The leaf types that were asked to measure in the last pass, and how many times each.
#[cfg(feature = "testing")]
pub fn last_pass_measure_asks(ctx: &Context) -> Vec<(&'static str, u32)> {
    last_totals(ctx, |log| log.asks.clone(), Vec::new())
}
