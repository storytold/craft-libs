//! The UI-thread side of the control channel.
//!
//! A transport sends [`ControlRequest`]s; once per frame, between the input and the UI pass, the
//! app drains them through [`ControlChannel::next_request`], runs its own method table on each
//! (`engine.execute`, `ui.inspect`, `ui.set`, `ui.dialog.*`, `ui.screenshot`, …) and hands the
//! [`Reply`] to [`ControlChannel::settle`]. The channel replies at once, after the queued
//! injected input has been processed ([`Reply::AfterInput`], so a following `ui.inspect` sees
//! the effect), when a background job ends ([`Reply::AfterJob`]), when the app answers a dialog's
//! held OK ([`Reply::AfterHeldOk`]), or when a screenshot completes ([`Reply::Screenshot`], through
//! [`crate::screenshot::Captures`]).
//!
//! Deadlines: a request still queued when its [`REPLY_DEADLINE`] passes is dropped before dispatch
//! and the transport replies [`expired_error`] (safe to re-send); one that was dispatched and gave
//! no reply in time gets [`timeout_error`] (it may still apply). [`Dispatch`] settles which of the
//! two happened, once, between the UI thread and the transport.

use std::sync::mpsc::{Receiver, Sender};

use craft_ui::dialog::{DialogHost, DialogId, HeldOk};
use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use serde_json::{Value, json};
use web_time::Instant;

use crate::screenshot::{Captures, DocumentId, Finished, ScreenshotParams, ScreenshotRequest, screenshot_reply};
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
    /// puts the PNG, `None` inlines it as base64 in the reply; `region` is refused at admission
    /// when it lies outside the window.
    #[non_exhaustive]
    Screenshot {
        /// Identifies the capture; must differ from every pending token.
        token: u64,
        /// File to write the PNG to, or `None` to inline it.
        path: Option<String>,
        /// The part of the window to capture, in points; `None` captures the whole window.
        region: Option<egui::Rect>,
        /// The viewport to capture.
        viewport: egui::ViewportId,
    },
    /// Synthetic input was queued: reply once the app has processed all of it.
    AfterInput,
    /// A command started a background job and the caller waits: reply with its result when it
    /// has been applied, or with its error or `cancelled`.
    AfterJob(JobId),
    /// A `ui.accessTree`: hold the reply until the window's settled access tree can be read
    /// ([`ControlChannel::poll_access_trees`]).
    AccessTree,
    /// A `ui.dialog.confirm` that filed the held OK of a dialog for the app: hold the reply until the
    /// app answers it ([`ControlChannel::poll_held`]).
    #[non_exhaustive]
    AfterHeldOk {
        /// The dialog whose OK is held.
        dialog: DialogId,
        /// Which open of the dialog ([`DialogHost::open_serial`]): a later open of the same dialog
        /// is not the one the request confirmed.
        open: u64,
        /// The app's refusal of the dialog's previous OK, when this confirm filed OK again after it:
        /// the earlier confirms still held for this open are answered with it.
        refused: Option<String>,
    },
}

impl Reply {
    /// A `ui.accessTree` reply: held until the window's settled access tree is available.
    pub fn access_tree() -> Reply {
        Reply::AccessTree
    }

    /// A `ui.screenshot` reply for `token` of `viewport` with what [`ScreenshotParams::from_params`] read: the PNG
    /// goes to `params.path` or is inlined when `None`, and shows `params.region` or the whole window.
    pub fn screenshot(token: u64, params: ScreenshotParams, viewport: egui::ViewportId) -> Self {
        Self::Screenshot { token, path: params.path, region: params.region, viewport }
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

/// The requests waiting for the app, and the replies waiting on input, jobs and screenshots.
///
/// Per frame: `while let Some(req) = channel.next_request() { let outcome = app.handle(&req); channel.settle(ctx, req, outcome, document); }`
/// in the logic phase; [`Self::input_processed`] after the pass that consumed the injected
/// input; [`Self::job_finished`] when a job ends; [`Self::captures`] driven by the app's UI and
/// logic phases as `screenshot` documents.
#[non_exhaustive]
pub struct ControlChannel {
    rx: Receiver<ControlRequest>,
    input_waiters: Vec<Sender<ControlResponse>>,
    job_waiters: Vec<(JobId, Sender<ControlResponse>)>,
    tree_waiters: Vec<Sender<ControlResponse>>,
    held_waiters: Vec<(DialogId, u64, Sender<ControlResponse>)>,
    /// The screenshot queue; the app calls its `issue` and `drain` from its UI and logic phases.
    pub captures: Captures,
}

impl ControlChannel {
    /// Attach a transport's receiver (TCP, stdin, tests).
    pub fn new(rx: Receiver<ControlRequest>) -> Self {
        Self {
            rx,
            input_waiters: Vec::new(),
            job_waiters: Vec::new(),
            tree_waiters: Vec::new(),
            held_waiters: Vec::new(),
            captures: Captures::default(),
        }
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
    /// `document` the identity a screenshot is attributed to. A screenshot is admitted at
    /// [`motion::now`]; [`Self::settle_at`] takes the time instead.
    pub fn settle(&mut self, req: ControlRequest, outcome: Reply, document: Option<DocumentId>, settle_ms: f64) {
        self.settle_at(req, outcome, document, settle_ms, motion::now());
    }

    /// [`Self::settle`] with a screenshot admitted at `now`: its settle delay and its
    /// [`crate::screenshot::DEADLINE`] count from `now`, on the clock the app passes to
    /// [`Captures::issue`] and [`Captures::drain`].
    pub fn settle_at(&mut self, req: ControlRequest, outcome: Reply, document: Option<DocumentId>, settle_ms: f64, now: Instant) {
        let reply = req.reply;
        match outcome {
            Reply::Done(v) => {
                let _ = reply.send(v);
            }
            Reply::AfterInput => self.input_waiters.push(reply),
            Reply::AfterJob(job) => self.job_waiters.push((job, reply)),
            Reply::AccessTree => self.tree_waiters.push(reply),
            Reply::AfterHeldOk { dialog, open, refused } => {
                if let Some(reason) = refused {
                    self.held_waiters.retain(|(earlier, earlier_open, tx)| {
                        let answered = (*earlier, *earlier_open) == (dialog, open);
                        if answered {
                            let _ = tx.send(json!({"ok": false, "error": reason}));
                        }
                        !answered
                    });
                }
                self.held_waiters.push((dialog, open, reply));
            }
            Reply::Screenshot { token, path, region, viewport } => {
                let request = ScreenshotRequest { token, path, region, viewport, reply, document, settle_ms };
                if let Err((reply, e)) = self.captures.admit(request, now) {
                    let _ = reply.send(json!({"ok": false, "error": e}));
                }
            }
        }
    }

    /// The pass that consumed the injected input ended: reply
    /// `{"ok": true, "result": null}` to every [`Reply::AfterInput`] waiter. Call it only after a
    /// pass that reached the end of the UI with the injected queue empty; an early-returning pass
    /// keeps the waiters.
    pub fn input_processed(&mut self) {
        for waiter in self.input_waiters.drain(..) {
            let _ = waiter.send(json!({"ok": true, "result": null}));
        }
    }

    /// The injected input was dropped (the window lost it, a modal took over): reply
    /// `{"ok": false, "error": "input cancelled: {reason}"}` to every [`Reply::AfterInput`]
    /// waiter.
    pub fn input_cancelled(&mut self, reason: &str) {
        for waiter in self.input_waiters.drain(..) {
            let _ = waiter.send(json!({"ok": false, "error": format!("input cancelled: {reason}")}));
        }
    }

    /// Answers the [`Reply::AfterInput`] waiters from the window host's input state: with
    /// `input cancelled` when the host withdrew queued input ([`TARGET_CHANGED`]), with success
    /// once nothing is queued or in flight. Call it once per pass, at the end of the pass (after its
    /// UI ran): the step that pass consumed counts as run, so the reply goes out with that pass
    /// ([`WindowHost::injected_idle_at_pass_end`]). Called between passes, it waits for the pass. It also has the host
    /// keep its access trees, which `control` targets and `ui.find` read.
    pub fn poll_input(&mut self, host: &WindowHost) {
        host.keep_access_trees();
        if host.take_injected_withdrawn() {
            self.input_cancelled(TARGET_CHANGED);
        } else if host.injected_idle_at_pass_end() {
            self.input_processed();
        }
    }

    /// Answers the held `ui.accessTree` requests from the window host's settled access tree; a
    /// request stays held while the host has none. Runs in the logic phase of every pass, as
    /// [`poll_input`](Self::poll_input) does.
    pub fn poll_access_trees(&mut self, ctx: &egui::Context) {
        if self.tree_waiters.is_empty() {
            return;
        }
        // `ui.accessTree` takes no window: it reads the main window's tree.
        let Some(tree) = WindowHost::of(ctx).access_tree(egui::ViewportId::ROOT) else { return };
        let update = tree.update();
        let nodes: Vec<Value> = update
            .nodes
            .iter()
            .map(|(id, node)| {
                let children: Vec<u64> = node.children().iter().map(|child| child.0).collect();
                let mut entry = json!({"id": id.0, "role": format!("{:?}", node.role()), "children": children});
                if let Some(name) = node.label() {
                    entry["name"] = json!(name);
                }
                if let Some(value) = node.value() {
                    entry["value"] = json!(value);
                }
                if let Some(selection) = node.text_selection() {
                    entry["textSelection"] = json!({
                        "anchor": {"node": selection.anchor.node.0, "character": selection.anchor.character_index},
                        "focus": {"node": selection.focus.node.0, "character": selection.focus.character_index},
                    });
                }
                entry
            })
            .collect();
        let root = update.tree.as_ref().map(|tree| tree.root.0);
        let reply = json!({"ok": true, "result": {"nodes": nodes, "focus": update.focus.0, "root": root}});
        for waiter in self.tree_waiters.drain(..) {
            let _ = waiter.send(reply.clone());
        }
    }

    /// Answers the held `ui.dialog.confirm` requests ([`Reply::AfterHeldOk`]) from `host`: success
    /// once the app accepted the dialog's held OK; once it refused it, an error whose text is the
    /// refusal's reason, English for a reason given as a `Word`; an error once the dialog closed
    /// while it waited, also when the same dialog opened again since. A request stays held while
    /// the app has not answered. Call it in every pass after the app answered the held OKs it
    /// answers in that pass and before the pass runs the host ([`DialogHost::run`]), which closes a
    /// dialog the app accepted; calling it more than once in a pass is harmless.
    pub fn poll_held(&mut self, host: &DialogHost) {
        self.held_waiters.retain(|(dialog, open, reply)| {
            let answer = match host.held_state(*dialog) {
                _ if host.open_serial(*dialog) != Some(*open) => {
                    json!({"ok": false, "error": "the dialog closed while its OK waited for the app's answer"})
                }
                Some(HeldOk::Waiting) => return true,
                Some(HeldOk::Accepted) => json!({"ok": true, "result": null}),
                Some(HeldOk::Refused(reason)) => json!({"ok": false, "error": reason.as_str()}),
                _ => json!({"ok": false, "error": "the app refused OK"}),
            };
            let _ = reply.send(answer);
            false
        });
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
                Ok(image) => screenshot_reply(f.token, image, f.region, f.geometry, f.path.as_deref(), write),
                Err(e) => json!({"ok": false, "error": e}),
            };
            if let Some(tx) = f.reply {
                let _ = tx.send(reply);
            }
        }
    }

    /// Waiters of every kind, for `ui.inspect`'s `pending` count and tests.
    pub fn waiting(&self) -> usize {
        self.input_waiters.len() + self.job_waiters.len() + self.tree_waiters.len() + self.held_waiters.len() + self.captures.pending()
    }
}
