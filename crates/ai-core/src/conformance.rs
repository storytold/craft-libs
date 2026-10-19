//! Conformance checks any [`Provider`] must pass, for use in each adapter's own tests.
//!
//! Each function drives a provider through one behaviour and returns `Err(description)` on the
//! first violation, so a failing adapter says what it got wrong. The mock passes all of them,
//! and a real adapter run against a test server should too.

use crate::error::ErrorKind;
use crate::ids::ModelId;
use crate::job::{Event, JobState};
use crate::provider::Provider;
use crate::request::Request;

/// Polls beyond this without a terminal state count as a hang.
pub const MAX_POLLS: u32 = 1000;

/// What a completed run produced.
#[derive(Debug)]
pub struct Completed {
    pub state: JobState,
    pub events: Vec<Event>,
}

/// Submit, poll to the end, and check the whole life cycle: every observed transition is legal,
/// the terminal state is stable on further polls, and later polls repeat no events.
///
/// Polls back to back, which suits a provider whose jobs advance on each poll (the mock). A
/// provider that works on another thread should use [`run_lifecycle_paced`] so the work has time
/// to happen between polls.
pub fn run_lifecycle(p: &dyn Provider, model: &ModelId, request: &Request) -> Result<Completed, String> {
    run_lifecycle_paced(p, model, request, &|| {})
}

/// As [`run_lifecycle`], calling `pause` after every poll that is not yet terminal. Pass a sleep of
/// a few milliseconds for a provider backed by real I/O. The model crate cannot sleep itself,
/// because it must compile for wasm.
pub fn run_lifecycle_paced(p: &dyn Provider, model: &ModelId, request: &Request, pause: &dyn Fn()) -> Result<Completed, String> {
    let job = p.submit(model, request).map_err(|e| format!("submit failed: {e}"))?;
    let mut prev = JobState::Queued;
    let mut events = Vec::new();
    for _ in 0..MAX_POLLS {
        let snap = p.poll(&job).map_err(|e| format!("poll failed: {e}"))?;
        if !prev.can_become(&snap.state) {
            return Err(format!("illegal transition {prev:?} -> {:?}", snap.state));
        }
        events.extend(snap.events);
        prev = snap.state;
        if prev.is_terminal() {
            for _ in 0..2 {
                let again = p.poll(&job).map_err(|e| format!("poll after finish failed: {e}"))?;
                if again.state != prev {
                    return Err(format!("terminal state changed from {prev:?} to {:?}", again.state));
                }
                if !again.events.is_empty() {
                    return Err("events were repeated after the job finished".into());
                }
            }
            return Ok(Completed { state: prev, events });
        }
        pause();
    }
    Err(format!("job did not finish within {MAX_POLLS} polls"))
}

/// Cancelling a running job ends in `Cancelled`; cancelling twice is not an error.
pub fn run_cancel(p: &dyn Provider, model: &ModelId, request: &Request) -> Result<(), String> {
    run_cancel_paced(p, model, request, &|| {})
}

/// As [`run_cancel`], calling `pause` between polls (see [`run_lifecycle_paced`]).
pub fn run_cancel_paced(p: &dyn Provider, model: &ModelId, request: &Request, pause: &dyn Fn()) -> Result<(), String> {
    let job = p.submit(model, request).map_err(|e| format!("submit failed: {e}"))?;
    let before = p.poll(&job).map_err(|e| format!("poll failed: {e}"))?;
    p.cancel(&job).map_err(|e| format!("cancel failed: {e}"))?;
    p.cancel(&job).map_err(|e| format!("second cancel failed: {e}"))?;
    let mut last = before.state.clone();
    for _ in 0..MAX_POLLS {
        let snap = p.poll(&job).map_err(|e| format!("poll failed: {e}"))?;
        if !last.can_become(&snap.state) {
            return Err(format!("illegal transition {last:?} -> {:?}", snap.state));
        }
        last = snap.state;
        if last.is_terminal() {
            break;
        }
        pause();
    }
    if before.state.is_terminal() || last == JobState::Cancelled { Ok(()) } else { Err(format!("job ended as {last:?} after cancel")) }
}

/// A request the model cannot handle must be refused at submit with the given kind.
pub fn run_rejects(p: &dyn Provider, model: &ModelId, request: &Request, kind: ErrorKind) -> Result<(), String> {
    match p.submit(model, request) {
        Ok(_) => Err("submit accepted a request the model cannot handle".into()),
        Err(e) if e.kind == kind => Ok(()),
        Err(e) => Err(format!("expected {kind:?}, got {:?}: {e}", e.kind)),
    }
}

/// Polling or cancelling a job that does not exist is an error, not a panic or a hang.
pub fn run_unknown_job(p: &dyn Provider) -> Result<(), String> {
    let ghost = crate::ids::JobId::new("no-such-job").map_err(|e| e.to_string())?;
    if p.poll(&ghost).is_ok() {
        return Err("poll of an unknown job succeeded".into());
    }
    if p.cancel(&ghost).is_ok() {
        return Err("cancel of an unknown job succeeded".into());
    }
    Ok(())
}
