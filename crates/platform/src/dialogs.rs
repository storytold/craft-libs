//! UI-thread picker creation with reserved waiting workers. No backend or application policy.
//!
//! `create` must call the native picker method eagerly on the event-loop thread, returning its
//! already-created future. Hosts retain the parent through transport AND their continuation.
//! A missing backend answer is indeterminate; only explicit backend evidence proves cancellation.
use crate::dialog_outcome::{DialogError, ErrorKind, Outcome};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    },
    thread::JoinHandle,
};
/// An already-created native picker future that a reserved worker can wait on.
pub type DialogFuture<T> = Pin<Box<dyn Future<Output = Outcome<T>> + Send>>;

/// Wait for a future without polling an idle native dialog.
pub fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::park();
    }
}
// Notification callbacks must not unwind a reserved worker after the UI observes readiness.
fn notify(wake: &dyn Fn()) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(wake));
}
fn worker_error(message: impl Into<String>) -> DialogError {
    DialogError::new(ErrorKind::Worker, message)
}
fn guarded<T>(f: impl FnOnce() -> Outcome<T>) -> Outcome<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Outcome::Failed(worker_error("The file-dialog backend failed")))
}
/// Reserve the worker before opening a native sheet. A failed reservation never calls `create`.
/// No timeout is imposed on a live picker. Backend constructor/poll panics become explicit failures.
pub fn wait_prepared<T: Send + 'static, F: Future<Output = Outcome<T>> + Send + 'static>(
    create: impl FnOnce() -> F,
    deliver: impl FnOnce(Outcome<T>) + Send + 'static,
) -> std::io::Result<JoinHandle<()>> {
    let (send, receive) = mpsc::channel::<F>();
    let worker = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
        deliver(guarded(|| match receive.recv() {
            Ok(future) => block_on(future),
            Err(_) => Outcome::Failed(worker_error("The file-dialog factory failed")),
        }));
    })?;
    if let Ok(future) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(create)) {
        send.send(future).map_err(|_| std::io::Error::other("The reserved file-dialog worker stopped"))?;
    }
    Ok(worker)
}

type Job<R, T> = (R, Sender<Outcome<T>>);
/// Clonable request endpoint. At most one queued, preparing or live request is accepted.
pub struct DialogClient<R, T> {
    sender: SyncSender<Job<R, T>>,
    busy: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl<R, T> Clone for DialogClient<R, T> {
    fn clone(&self) -> Self {
        Self { sender: self.sender.clone(), busy: self.busy.clone(), wake: self.wake.clone() }
    }
}
impl<R, T> DialogClient<R, T> {
    /// Queue a request without blocking and wake the host to pump it.
    /// A busy or unavailable host answers with a worker failure; no second picker is opened.
    pub fn request(&self, request: R) -> Receiver<Outcome<T>> {
        let (answer, receive) = mpsc::channel();
        if self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            let _ = answer.send(Outcome::Failed(worker_error("A file dialog is already pending")));
        } else if let Err(error) = self.sender.try_send((request, answer)) {
            self.busy.store(false, Ordering::SeqCst);
            let (_, answer) = match error {
                mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
            };
            let _ = answer.send(Outcome::Failed(worker_error("The file-dialog host is unavailable")));
        }
        notify(self.wake.as_ref());
        receive
    }
}
struct Preparing<R, T> {
    request: R,
    ready: Receiver<()>,
    send: Sender<DialogFuture<T>>,
    completion: Completion<T>,
}
struct Completion<T> {
    done: Receiver<Outcome<T>>,
    answer: Sender<Outcome<T>>,
}
impl<T> Completion<T> {
    fn poll(&self) -> Option<Outcome<T>> {
        match self.done.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Outcome::Failed(worker_error("The file-dialog worker stopped without an answer"))),
        }
    }
}
enum State<R, T> {
    Preparing(Preparing<R, T>),
    Waiting(Completion<T>),
}
/// Event-loop-owned transport. Dropping a reply does not release a live picker or its parent.
/// `pump` must run before consuming replies, and after app code that may enqueue a request.
/// The host must retain this service/window while `pending()` or its own continuation is pending.
pub struct NativeDialogs<R, T> {
    queue: Receiver<Job<R, T>>,
    busy: Arc<AtomicBool>,
    state: Option<State<R, T>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    check: Arc<dyn Fn() -> Result<(), DialogError> + Send + Sync>,
}
impl<R, T: Send + 'static> NativeDialogs<R, T> {
    /// Create a client and its event-loop-owned service without a backend preflight check.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> (DialogClient<R, T>, Self) {
        Self::with_preflight(wake, || Ok(()))
    }
    /// Optional host-owned advisory check, run on the reserved worker before any picker exists.
    /// Hosts must bound this check. Unknown capability must permit the actual picker; return an
    /// Unavailable error only when absence is proven. This service performs no filesystem probes.
    pub fn with_preflight(
        wake: impl Fn() + Send + Sync + 'static,
        check: impl Fn() -> Result<(), DialogError> + Send + Sync + 'static,
    ) -> (DialogClient<R, T>, Self) {
        let (sender, queue) = mpsc::sync_channel(1);
        let busy = Arc::new(AtomicBool::new(false));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        (DialogClient { sender, busy: busy.clone(), wake: wake.clone() }, Self { queue, busy, state: None, wake, check: Arc::new(check) })
    }
    /// Whether a queued, preparing or live picker still owns the transport reservation.
    pub fn pending(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
    /// Advance requests on the event-loop thread and publish completed replies.
    /// Call before consuming replies and after enqueuing work. `create` must eagerly open
    /// the native picker on this thread and return its already-created future.
    pub fn pump(&mut self, mut create: impl FnMut(R) -> DialogFuture<T>) {
        if let Some(state) = self.state.take() {
            match state {
                State::Waiting(completion) => {
                    if let Some(outcome) = completion.poll() {
                        // Only the UI pump publishes replies, after releasing the transport.
                        // A continuation can therefore immediately request its next picker.
                        self.busy.store(false, Ordering::SeqCst);
                        let _ = completion.answer.send(outcome);
                    } else {
                        self.state = Some(State::Waiting(completion));
                    }
                }
                State::Preparing(preparing) => {
                    if let Some(outcome) = preparing.completion.poll() {
                        self.busy.store(false, Ordering::SeqCst);
                        let _ = preparing.completion.answer.send(outcome);
                    } else if preparing.ready.try_recv().is_ok() {
                        let future = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| create(preparing.request)))
                            .unwrap_or_else(|_| Box::pin(async { Outcome::Failed(worker_error("The file-dialog factory failed")) }));
                        let _ = preparing.send.send(future);
                        self.state = Some(State::Waiting(preparing.completion));
                    } else {
                        self.state = Some(State::Preparing(preparing));
                    }
                }
            }
        }
        if self.state.is_none()
            && let Ok((request, answer)) = self.queue.try_recv()
        {
            let (ready_send, ready) = mpsc::channel();
            let (send, receive) = mpsc::channel::<DialogFuture<T>>();
            let (done_send, done) = mpsc::channel();
            let check = self.check.clone();
            let wake = self.wake.clone();
            let started = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
                let output = guarded(|| {
                    if let Err(error) = check() {
                        return Outcome::Failed(error);
                    }
                    if ready_send.send(()).is_err() {
                        return Outcome::Failed(worker_error("The file-dialog host stopped"));
                    }
                    notify(wake.as_ref());
                    match receive.recv() {
                        Ok(future) => block_on(future),
                        Err(_) => Outcome::Failed(worker_error("The file-dialog host stopped")),
                    }
                });
                let _ = done_send.send(output);
                notify(wake.as_ref());
            });
            match started {
                Ok(_) => self.state = Some(State::Preparing(Preparing { request, ready, send, completion: Completion { done, answer } })),
                Err(error) => {
                    self.busy.store(false, Ordering::SeqCst);
                    let _ = answer.send(Outcome::Failed(worker_error(error.to_string())));
                    notify(self.wake.as_ref());
                }
            }
        }
    }
}
/// Holds close intent while a picker/continuation owns the parent. A retry never authorizes
/// discarding documents: the caller must run its ordinary unsaved-work policy again.
#[cfg(feature = "egui")]
#[derive(Default, Debug)]
pub struct DeferredClose {
    held: bool,
}
#[cfg(feature = "egui")]
impl DeferredClose {
    /// Hold actual close requests while a picker or app continuation is pending.
    /// Return `true` once after settlement so the host retries its unsaved-work policy;
    /// this never grants permission to discard changes or closes the viewport itself.
    pub fn update(&mut self, ctx: &egui::Context, pending: bool) -> bool {
        let outgoing = ctx.viewport(|v| {
            v.commands
                .iter()
                .rev()
                .find_map(|c| match c {
                    egui::ViewportCommand::Close => Some(true),
                    egui::ViewportCommand::CancelClose => Some(false),
                    _ => None,
                })
                .unwrap_or(false)
        });
        if pending {
            let requested = outgoing || ctx.input(|i| i.viewport().close_requested());
            self.held |= requested;
            // Every viewport command requests a repaint. An idle native picker must sleep.
            if requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            false
        } else if std::mem::take(&mut self.held) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.request_repaint();
            true
        } else {
            false
        }
    }
}
