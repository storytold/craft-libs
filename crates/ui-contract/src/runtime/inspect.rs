//! Decision records and findings: what each pipeline step did, kept in frames a test fixture drives and while
//! `FrameInput.inspect` is on.

use crate::runtime::egui::memory::Service;
use crate::runtime::frame;
use egui::{Context, Id};
use std::collections::HashMap;

/// A rule a component broke that the pipeline reports instead of panicking.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Finding {
    /// The rule's name.
    pub rule: &'static str,
    /// The component's type.
    pub component: &'static str,
    /// What the finding is about, such as a part or an id.
    pub subject: String,
}

#[derive(Default)]
struct Log {
    steps: HashMap<Id, Vec<&'static str>>,
    findings: Vec<Finding>,
    canvas_controls: Vec<&'static str>,
}

/// Whether records are kept: in frames a test fixture drives, and in any build while the debug inspector is on.
pub(crate) fn recording(ctx: &Context) -> bool {
    fixture_frame() || frame::inspecting(ctx)
}

#[cfg(feature = "testing")]
use crate::runtime::egui::fixture::driving as fixture_frame;

#[cfg(not(feature = "testing"))]
fn fixture_frame() -> bool {
    false
}

/// Starts the step record of `id` for a new show.
pub(crate) fn begin(ctx: &Context, id: Id) {
    if recording(ctx) {
        Service::<Log>::run(ctx, |log| log.steps.insert(id, Vec::new()));
    }
}

/// Records that `step` ran for `id`.
pub(crate) fn step(ctx: &Context, id: Id, step: &'static str) {
    if recording(ctx) {
        Service::<Log>::run(ctx, |log| log.steps.entry(id).or_default().push(step));
    }
}

/// The steps the latest show of `id` ran, in order.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "read by trials and the debug inspector"))]
pub(crate) fn steps_of(ctx: &Context, id: Id) -> Vec<&'static str> {
    Service::<Log>::run(ctx, |log| log.steps.get(&id).cloned().unwrap_or_default())
}

/// Reports a finding.
pub(crate) fn finding(ctx: &Context, finding: Finding) {
    if !recording(ctx) {
        return;
    }
    Service::<Log>::run(ctx, |log| {
        if !log.findings.contains(&finding) {
            log.findings.push(finding);
        }
    });
}

/// Every finding reported on the context.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "read by trials and the debug inspector"))]
pub(crate) fn findings(ctx: &Context) -> Vec<Finding> {
    Service::<Log>::run(ctx, |log| log.findings.clone())
}

/// Records that the canvas control `type_name` showed in this pass, so the canvas-control cases can check that a
/// press away from its parts reaches the tool.
pub(crate) fn canvas_control(ctx: &Context, type_name: &'static str) {
    if !recording(ctx) {
        return;
    }
    Service::<Log>::run(ctx, |log| {
        if !log.canvas_controls.contains(&type_name) {
            log.canvas_controls.push(type_name);
        }
    });
}

/// The canvas-control types that have shown on the context.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "read by trials and the debug inspector"))]
pub(crate) fn canvas_controls(ctx: &Context) -> Vec<&'static str> {
    Service::<Log>::run(ctx, |log| log.canvas_controls.clone())
}
