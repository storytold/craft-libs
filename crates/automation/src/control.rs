//! The UI-thread side of the control channel.
//!
//! A transport sends [`ControlRequest`]s; once per frame, between the input and the UI pass, the
//! app drains them through [`ControlChannel::next_request`], runs its own method table on each
//! (`engine.execute`, `ui.inspect`, `ui.set`, `ui.dialog.*`, `ui.screenshot`, …) and hands the
//! [`Reply`] to [`ControlChannel::settle`]. The channel replies at once, after the queued
//! injected input has been processed ([`Reply::AfterInput`], so a following `ui.inspect` sees
//! the effect), when a background job ends ([`Reply::AfterJob`]), or when a screenshot completes
//! ([`Reply::Screenshot`], through [`crate::screenshot::Captures`]).
//!
//! Deadlines: a request still queued when its [`REPLY_DEADLINE`] passes is dropped before dispatch
//! and the transport replies [`expired_error`] (safe to re-send); one that was dispatched and gave
//! no reply in time gets [`timeout_error`] (it may still apply). [`Dispatch`] settles which of the
//! two happened, once, between the UI thread and the transport.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use serde_json::{Value, json};

use crate::screenshot::{Captures, DocumentId, Finished, ScreenshotRequest, screenshot_reply};
use crate::synthetic::TARGET_CHANGED;

/// A reply: the JSON object the transport writes as one line.
pub type ControlResponse = Value;

/// A background job's id in `jobs.list`, `jobs.cancel` and [`Reply::AfterJob`].
pub type JobId = u64;

pub use crate::protocol::REPLY_DEADLINE;

/// The error for a dispatched request with no reply in [`REPLY_DEADLINE`]: the command may still
/// run and apply, so re-sending it blindly could apply an edit twice.
pub fn timeout_error() -> String {
    format!(
        "timeout: no reply in {}s — the command may still be running and may still apply, or the app was not drawing frames (display asleep, window fully occluded, or a long operation blocked the frame loop)",
        REPLY_DEADLINE.as_secs()
    )
}

/// The error for a request still queued when its deadline passed: dropped before dispatch, safe to
/// re-send. Same `timeout` prefix as [`timeout_error`], which machines match on.
pub fn expired_error() -> String {
    format!(
        "timeout: not run — the request was still queued when its {}s deadline passed, so it was dropped before dispatch and is safe to re-send; the app was not drawing frames (display asleep, window fully occluded, or a long operation blocked the frame loop)",
        REPLY_DEADLINE.as_secs()
    )
}

/// One request from a transport, with where its reply goes.
#[non_exhaustive]
pub struct ControlRequest {
    /// The method name, such as `ui.click`.
    pub method: String,
    /// The request parameters as JSON.
    pub params: Value,
    /// Where the answer goes; the transport reads the other end.
    pub reply: Sender<ControlResponse>,
    /// Requests still queued at this instant are rejected; dispatched work continues.
    pub deadline: Option<Instant>,
    /// Shared with the transport, so when it stops waiting it knows whether the command ran.
    pub dispatch: Dispatch,
}

impl ControlRequest {
    /// A request with no deadline and a fresh reply channel (tests, in-process clients).
    pub fn new(method: impl Into<String>, params: Value) -> (Self, Receiver<ControlResponse>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx, deadline: None, dispatch: Dispatch::default() }, rx)
    }

    /// `self` with its deadline [`REPLY_DEADLINE`] from now.
    pub fn with_deadline(mut self) -> Self {
        self.deadline = Some(motion::now() + REPLY_DEADLINE);
        self
    }
}

/// Whether a request was dispatched, settled once between the UI thread (about to run it) and the
/// transport (giving up on its reply).
#[derive(Clone, Debug, Default)]
pub struct Dispatch(std::sync::Arc<std::sync::atomic::AtomicU8>);

const QUEUED: u8 = 0;
const DISPATCHED: u8 = 1;
const ABANDONED: u8 = 2;

impl Dispatch {
    /// Moves a queued request to `to`; returns the state it ends in.
    fn settle(&self, to: u8) -> u8 {
        use std::sync::atomic::Ordering;
        self.0.compare_exchange(QUEUED, to, Ordering::AcqRel, Ordering::Acquire).map_or_else(|state| state, |_| to)
    }

    /// The UI thread claims the request to run it: false once the transport gave up on it.
    pub fn start(&self) -> bool {
        self.settle(DISPATCHED) == DISPATCHED
    }

    /// The transport stops waiting: [`expired_error`] when the command can no longer run,
    /// [`timeout_error`] when it was dispatched and may still apply.
    pub fn give_up(&self) -> String {
        if self.settle(ABANDONED) == DISPATCHED { timeout_error() } else { expired_error() }
    }
}

/// What the app's method table answers for one request.
#[non_exhaustive]
pub enum Reply {
    /// Reply with this JSON now.
    Done(Value),
    /// A `ui.screenshot`: admit `token` to the queue; `path` is where the app's write authority
    /// puts the PNG, `None` inlines it as base64 in the reply.
    #[non_exhaustive]
    Screenshot {
        /// Identifies the capture; must differ from every pending token.
        token: u64,
        /// File to write the PNG to, or `None` to inline it.
        path: Option<String>,
        /// The viewport to capture.
        viewport: egui::ViewportId,
    },
    /// Synthetic input was queued: reply once the app has processed all of it.
    AfterInput,
    /// Synthetic input was queued as the window host's request `request`
    /// ([`WindowHost::injected_last_request`] right after queuing it): reply once the app has
    /// processed all of it, with the refusals that request's own input caused
    /// ([`ControlChannel::input_refused`]).
    #[non_exhaustive]
    AfterInputRequest {
        /// The window host's number for the request.
        request: u64,
    },
    /// A command started a background job and the caller waits: reply with its result when it
    /// has been applied, or with its error or `cancelled`.
    AfterJob(JobId),
}

impl Reply {
    /// A `ui.screenshot` reply for `token` of `viewport`, writing the PNG to `path` or inlining it
    /// when `None`.
    pub fn screenshot(token: u64, path: Option<String>, viewport: egui::ViewportId) -> Self {
        Self::Screenshot { token, path, viewport }
    }

    /// [`Reply::AfterInputRequest`] for the window host's request `request`.
    pub fn after_input_request(request: u64) -> Self {
        Self::AfterInputRequest { request }
    }

    /// `Done` with the [`result_reply`] of a method's result.
    pub fn from_result(r: Result<Value, String>) -> Self {
        Self::Done(result_reply(r))
    }
}

/// The reply for a method's result: `{"ok": true, "result": value}` for `Ok(value)`,
/// `{"ok": false, "error": e}` for `Err(e)`.
pub fn result_reply(r: Result<Value, String>) -> ControlResponse {
    match r {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// At most this many refusals are kept for one input request's reply; it says `"truncated": true`
/// when more were reported.
pub const MAX_INPUT_REFUSALS: usize = 64;

/// The reply for input that was processed with `refusals` (none: `{"ok": true, "result": null}`),
/// else `{"ok": false, "error": <first>, "errors": [...]}`, with `"truncated": true` when
/// [`MAX_INPUT_REFUSALS`] dropped some.
pub fn input_reply(refusals: &[String], truncated: bool) -> ControlResponse {
    match refusals.first() {
        None => json!({"ok": true, "result": null}),
        Some(first) if truncated => json!({"ok": false, "error": first, "errors": refusals, "truncated": true}),
        Some(first) => json!({"ok": false, "error": first, "errors": refusals}),
    }
}

/// A reply waiting on synthetic input, and the refusals its request's input caused.
struct InputWaiter {
    reply: Sender<ControlResponse>,
    /// The window host's number for its request; `None` for [`Reply::AfterInput`], which collects
    /// no refusals.
    request: Option<u64>,
    refusals: Vec<String>,
    truncated: bool,
    /// A step of its request was seen running.
    started: bool,
    /// Its request's input has run: it started and none of its steps is still queued.
    ran: bool,
}

impl InputWaiter {
    /// A waiter for `request`.
    fn new(reply: Sender<ControlResponse>, request: Option<u64>) -> Self {
        Self { reply, request, refusals: Vec::new(), truncated: false, started: false, ran: false }
    }

    /// Its reply once its input was processed.
    fn processed(&self) -> ControlResponse {
        input_reply(&self.refusals, self.truncated)
    }
}

/// The requests waiting for the app, and the replies waiting on input, jobs and screenshots.
///
/// Per frame: `while let Some(req) = channel.next_request() { let outcome = app.handle(&req); channel.settle(ctx, req, outcome, document); }`
/// in the logic phase; [`Self::input_processed`] after the pass that consumed the injected
/// input; [`Self::job_finished`] when a job ends; [`Self::captures`] driven by the app's UI and
/// logic phases as `screenshot` documents.
#[non_exhaustive]
pub struct ControlChannel {
    rx: Receiver<ControlRequest>,
    input_waiters: Vec<InputWaiter>,
    job_waiters: Vec<(JobId, Sender<ControlResponse>)>,
    /// The screenshot queue; the app calls its `issue` and `drain` from its UI and logic phases.
    pub captures: Captures,
}

impl ControlChannel {
    /// Attach a transport's receiver (TCP, stdin, tests).
    pub fn new(rx: Receiver<ControlRequest>) -> Self {
        Self { rx, input_waiters: Vec::new(), job_waiters: Vec::new(), captures: Captures::default() }
    }

    /// The next request to run, claimed through [`Dispatch::start`]. Expired or abandoned
    /// requests are replied with [`expired_error`] here and skipped. `None` when the queue is
    /// empty this frame.
    pub fn next_request(&mut self) -> Option<ControlRequest> {
        while let Ok(req) = self.rx.try_recv() {
            if req.deadline.is_some_and(|deadline| motion::now() >= deadline) || !req.dispatch.start() {
                let _ = req.reply.send(json!({"ok": false, "error": expired_error()}));
                continue;
            }
            return Some(req);
        }
        None
    }

    /// Reply now, or park the reply on input, a job or a screenshot. `settle_ms` is the
    /// screenshot settle delay ([`crate::screenshot::settle_ms`] of the animation time);
    /// `document` the identity a screenshot is attributed to.
    pub fn settle(&mut self, req: ControlRequest, outcome: Reply, document: Option<DocumentId>, settle_ms: f64) {
        let reply = req.reply;
        match outcome {
            Reply::Done(v) => {
                let _ = reply.send(v);
            }
            Reply::AfterInput => self.input_waiters.push(InputWaiter::new(reply, None)),
            Reply::AfterInputRequest { request } => self.input_waiters.push(InputWaiter::new(reply, Some(request))),
            Reply::AfterJob(job) => self.job_waiters.push((job, reply)),
            Reply::Screenshot { token, path, viewport } => {
                if let Err((reply, e)) = self.captures.admit(ScreenshotRequest { token, path, viewport, reply, document, settle_ms }, motion::now()) {
                    let _ = reply.send(json!({"ok": false, "error": e}));
                }
            }
        }
    }

    /// Something injected input asked for was refused (a disabled command, a dialog's refused
    /// OK) in a pass that carries request `request`'s step
    /// ([`WindowHost::injected_running_request`]): `error` goes to that request's
    /// [`Reply::AfterInputRequest`] reply. A blank error is ignored; past [`MAX_INPUT_REFUSALS`]
    /// the reply is marked truncated. True when a waiter took it.
    pub fn input_refused(&mut self, request: u64, error: &str) -> bool {
        self.note_started(request);
        let error = error.trim();
        let Some(waiter) = self.input_waiters.iter_mut().find(|waiter| waiter.request == Some(request)) else { return false };
        if error.is_empty() {
            return false;
        }
        if waiter.refusals.len() >= MAX_INPUT_REFUSALS {
            waiter.truncated = true;
        } else {
            waiter.refusals.push(error.to_owned());
        }
        true
    }

    /// A step of request `request` is running.
    fn note_started(&mut self, request: u64) {
        for waiter in self.input_waiters.iter_mut().filter(|waiter| waiter.request == Some(request)) {
            waiter.started = true;
        }
    }

    /// At the end of a pass: the waiters whose request started and has no step left queued have
    /// run. Read before a withdrawal or cancel drops queued steps, so a dropped request never
    /// counts as run.
    fn note_ran(&mut self, host: &WindowHost) {
        if let Some(request) = host.injected_running_request() {
            self.note_started(request);
        }
        for waiter in self.input_waiters.iter_mut().filter(|waiter| waiter.started && !waiter.ran) {
            waiter.ran = waiter.request.is_some_and(|request| !host.injected_request_queued(request));
        }
    }

    /// The pass that consumed the injected input ended: reply to every input waiter,
    /// `{"ok": true, "result": null}` or its refusals ([`input_reply`]). Call it only after a pass
    /// that reached the end of the UI with the injected queue empty; an early-returning pass keeps
    /// the waiters.
    pub fn input_processed(&mut self) {
        for waiter in self.input_waiters.drain(..) {
            let _ = waiter.reply.send(waiter.processed());
        }
    }

    /// The injected input was dropped (the window lost it, a modal took over): reply
    /// `{"ok": false, "error": "input cancelled: {reason}"}` to every input waiter, except a
    /// [`Reply::AfterInputRequest`] whose input had already run (as [`Self::poll_input`] saw at
    /// the end of a pass), which gets its own reply as [`Self::input_processed`] gives it.
    pub fn input_cancelled(&mut self, reason: &str) {
        for waiter in self.input_waiters.drain(..) {
            let reply = if waiter.ran { waiter.processed() } else { json!({"ok": false, "error": format!("input cancelled: {reason}")}) };
            let _ = waiter.reply.send(reply);
        }
    }

    /// The host withdrew queued input ([`TARGET_CHANGED`]): a [`Reply::AfterInputRequest`] whose
    /// request still has steps queued (input of a viewport the withdrawal did not cover) keeps
    /// waiting; the others are answered as [`Self::input_cancelled`] answers them.
    fn input_withdrawn(&mut self, host: &WindowHost) {
        let (waiting, answered): (Vec<_>, Vec<_>) = std::mem::take(&mut self.input_waiters)
            .into_iter()
            .partition(|waiter| !waiter.ran && waiter.request.is_some_and(|request| host.injected_request_queued(request)));
        self.input_waiters = answered;
        self.input_cancelled(TARGET_CHANGED);
        self.input_waiters = waiting;
    }

    /// Answers the [`Reply::AfterInput`] waiters from the window host's input state: with
    /// `input cancelled` when the host withdrew queued input ([`TARGET_CHANGED`]) (a request whose
    /// input ran, or whose input the withdrawal did not cover, is not cancelled), with success
    /// once nothing is queued or in flight. Call it once per pass, at the end of the pass: it also
    /// notes which requests' input has run.
    pub fn poll_input(&mut self, host: &WindowHost) {
        if host.take_injected_withdrawn() {
            self.input_withdrawn(host);
        }
        self.note_ran(host);
        if host.injected_idle() {
            self.input_processed();
        }
    }

    /// Drops the host's queued input and the step in flight, and fails every
    /// [`Reply::AfterInput`] waiter with `input cancelled: {reason}`, also when only waiters
    /// remain (their input was already consumed). True when anything was pending or waiting.
    pub fn cancel_input(&mut self, host: &WindowHost, reason: &str) -> bool {
        let pending = host.cancel_injected() || !self.input_waiters.is_empty();
        self.input_cancelled(reason);
        pending
    }

    /// Job `job` ended: send `reply` to its waiters. It is the reply the command would have given
    /// had it run in place, usually [`result_reply`] of the job's result (a cancelled job's error
    /// is `cancelled`); a job that ends with an error and partial results carries both.
    pub fn job_finished(&mut self, job: JobId, reply: &ControlResponse) {
        self.job_waiters.retain(|(waiting, tx)| {
            if *waiting == job {
                let _ = tx.send(reply.clone());
                false
            } else {
                true
            }
        });
    }

    /// Reply to finished screenshots with [`crate::screenshot::screenshot_reply`] (a capture
    /// error `e` as `{"ok": false, "error": e}`); `write` is the app's file authority for replies
    /// with a `path`.
    pub fn finish_captures(&mut self, finished: Vec<Finished>, write: &mut crate::screenshot::PngWriter<'_>) {
        for f in finished {
            let reply = match f.result {
                Ok(image) => screenshot_reply(f.token, image, f.path.as_deref(), write),
                Err(e) => json!({"ok": false, "error": e}),
            };
            if let Some(tx) = f.reply {
                let _ = tx.send(reply);
            }
        }
    }

    /// Waiters of every kind, for `ui.inspect`'s `pending` count and tests.
    pub fn waiting(&self) -> usize {
        self.input_waiters.len() + self.job_waiters.len() + self.captures.pending()
    }
}
