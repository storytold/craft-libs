//! The step functions of the pipeline (1.6): named functions over plain data, in pipeline order. `run` calls
//! them for every shape.

use crate::runtime::inspect;
use egui::{Context, Id};

mod apply;
mod env;
mod finish;
mod id;
mod label;
mod measure;
mod paint;
mod parts;
mod place;
mod report;
mod scroll_owner;
mod sense;
mod settle;
mod state;
mod stops;
mod style;
mod update;
mod value;

pub(crate) use apply::{apply_values, rerun_after_edit};
pub(crate) use env::{env, resolved};
pub(crate) use finish::{before_finish, inert_response, record_size};
pub(super) use id::measuring;
pub(crate) use id::{id, keeping_ids};
pub(crate) use label::{activate_by_label, scrub_by_label};
pub(crate) use measure::{MeasureIn, guard, measure, measure_cx};
pub(crate) use paint::{paint_focus, paint_surface, surface_ink};
pub(crate) use parts::{PartsStep, check_parts, declares_none};
pub(crate) use place::place;
pub(crate) use report::{ReportIn, report, report_inert};
#[cfg(feature = "testing")]
pub(crate) use scroll_owner::queue_reveal;
pub(crate) use scroll_owner::{peek_reveal, peek_scroll_moves, take_reveal, take_scroll_moves};
pub(crate) use sense::{part_state, register_wheel};
pub(crate) use settle::settle;
pub(crate) use state::{LastState, register_reset, state};
pub(crate) use stops::{StopsIn, body_group, join_group, member_group, register_stops, takes_focus};
pub(crate) use style::{style, style_cx};
pub(crate) use update::{after_update, edit_intents, gesture_gate, key_activation, record_context, takes_activation, update_inputs};
pub(crate) use value::{peek_value, value};

/// The 17 steps of the pipeline, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Env,
    Style,
    Id,
    Value,
    Measure,
    Place,
    Parts,
    Sense,
    Update,
    Overlay,
    Apply,
    State,
    Paint,
    Content,
    Settle,
    Report,
    Finish,
}

impl Step {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Step::Env => "env",
            Step::Style => "style",
            Step::Id => "id",
            Step::Value => "value",
            Step::Measure => "measure",
            Step::Place => "place",
            Step::Parts => "parts",
            Step::Sense => "sense",
            Step::Update => "update",
            Step::Overlay => "overlay",
            Step::Apply => "apply",
            Step::State => "state",
            Step::Paint => "paint",
            Step::Content => "content",
            Step::Settle => "settle",
            Step::Report => "report",
            Step::Finish => "finish",
        }
    }
}

/// The shape a pipeline run is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShapeKind {
    Leaf,
    Body,
    Item,
}

/// Which shapes run a step.
#[derive(Clone, Copy)]
struct Runs {
    leaf: bool,
    body: bool,
    item: bool,
}

const ALL: Runs = Runs { leaf: true, body: true, item: true };

/// The one step table: which shapes run each step (1.6). It gates only a step a shape omits, which today is the
/// content step of a leaf. The order the steps run in is the order of the `Trace::mark` calls in `run`, not the order
/// of the rows.
const TABLE: [(Step, Runs); 17] = [
    (Step::Env, ALL),
    (Step::Style, ALL),
    (Step::Id, ALL),
    (Step::Value, ALL),
    (Step::Measure, ALL),
    (Step::Place, ALL),
    (Step::Parts, ALL),
    (Step::Sense, ALL),
    (Step::Update, ALL),
    (Step::Overlay, ALL),
    (Step::Apply, ALL),
    (Step::State, ALL),
    (Step::Paint, ALL),
    (Step::Content, Runs { leaf: false, body: true, item: true }),
    (Step::Settle, ALL),
    (Step::Report, ALL),
    (Step::Finish, ALL),
];

/// Whether `shape` runs `step`.
pub(crate) fn runs(step: Step, shape: ShapeKind) -> bool {
    TABLE.iter().find(|(s, _)| *s == step).is_some_and(|(_, runs)| match shape {
        ShapeKind::Leaf => runs.leaf,
        ShapeKind::Body => runs.body,
        ShapeKind::Item => runs.item,
    })
}

/// The step record of one show. `recording` is read once per run, so a release frame with the inspector off
/// takes no lock per step; a sizing pass records nothing.
#[derive(Clone, Copy)]
pub(crate) struct Trace<'a> {
    ctx: &'a Context,
    id: Id,
    shape: ShapeKind,
    recording: bool,
}

impl<'a> Trace<'a> {
    pub(crate) fn new(ctx: &'a Context, id: Id, shape: ShapeKind, recording: bool) -> Self {
        Trace { ctx, id, shape, recording }
    }

    pub(crate) fn recording(&self) -> bool {
        self.recording
    }

    /// Records that `step` runs, and returns whether the shape runs it.
    pub(crate) fn mark(&self, step: Step) -> bool {
        let run = runs(step, self.shape);
        if run && self.recording {
            inspect::step(self.ctx, self.id, step.name());
        }
        run
    }
}
