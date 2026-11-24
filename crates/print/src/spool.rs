//! The print job layer: discovery of printers and submission of jobs as host jobs, per platform.

#[cfg(any(windows, test))]
pub mod isolated;

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::Job;

/// The print job layer, run as host jobs. Desktop backends call the existing
/// [`submit_cancellable`](crate::submit_cancellable) and [`printer_queues`](crate::printer_queues).
/// On wasm the backend prints through the browser, lists no printers and
/// [`chooses_printer`](Spool::chooses_printer) is false.
#[derive(Clone)]
pub struct Spool {
    inner: Arc<Inner>,
}

struct Inner {
    jobs: Arc<dyn craft_host::Jobs>,
    backend: Arc<dyn SpoolBackend>,
    /// Every submitted job until it is done or shut down, oldest first: `Jobs::spawn` gives no
    /// handle, so shutdown reaches a job only through its entry.
    entries: Mutex<Vec<Entry>>,
}

/// Whether a job's work has ended. A job being sent is told apart from a waiting one only by
/// its cancel's answer, at the moment it is asked; it is never stored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Done,
}

type Done = Arc<(Mutex<Phase>, Condvar)>;

struct Entry {
    cancel: Arc<dyn Fn() -> bool + Send + Sync>,
    waits_after_cancel: bool,
    done: Done,
}

impl Entry {
    fn finished(&self) -> bool {
        *lock(&self.done.0) == Phase::Done
    }

    fn wait(&self) {
        let (phase, signal) = &*self.done;
        let mut phase = lock(phase);
        while *phase == Phase::Waiting {
            phase = signal.wait(phase).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Marks its job done when dropped: after the job ran, or when the host dropped it unrun (a
/// thread that could not start), so shutdown never waits on a job that will not run.
struct Finish(Done);

impl Drop for Finish {
    fn drop(&mut self) {
        *lock(&self.0.0) = Phase::Done;
        self.0.1.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Spool {
    /// A spool for this platform that runs its work on `jobs`.
    pub fn new(jobs: Arc<dyn craft_host::Jobs>) -> Self {
        Self::with_backend(jobs, Arc::new(SystemBackend))
    }

    /// A spool over `backend`, for trials and tests that fake the platform.
    pub fn with_backend(jobs: Arc<dyn craft_host::Jobs>, backend: Arc<dyn SpoolBackend>) -> Self {
        Self { inner: Arc::new(Inner { jobs, backend, entries: Mutex::new(Vec::new()) }) }
    }

    /// Whether the user chooses a printer (false on wasm, where the browser's dialog does).
    pub fn chooses_printer(&self) -> bool {
        self.inner.backend.chooses_printer()
    }

    /// Lists the printers in a job; the list arrives on the receiver.
    pub fn discover(&self) -> Receiver<Vec<crate::Printer>> {
        self.listed(|printer| printer)
    }

    /// The printer discovery the print dialog takes, mapping each printer to `ui::Printer`.
    #[cfg(feature = "ui")]
    pub fn discoverer(&self) -> crate::ui::Discover {
        let spool = self.clone();
        Box::new(move || {
            spool.listed(|printer| crate::ui::Printer { name: printer.name, label: printer.description, default: printer.default, own_dialog: None })
        })
    }

    /// Asks the backend for its printers in a job and sends them through `map`; a platform that
    /// lists none sends an empty list.
    fn listed<T: Send + 'static>(&self, map: impl Fn(crate::Printer) -> T + Send + 'static) -> Receiver<Vec<T>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let backend = self.inner.backend.clone();
        self.inner.jobs.spawn(
            "print-discovery",
            Box::new(move || {
                let printers = backend.printers().unwrap_or_default();
                let _ = sender.send(printers.into_iter().map(map).collect());
            }),
        );
        receiver
    }

    /// Prints `pdf` with `job` in a job; the job's id or the failure arrives on the receiver.
    /// The job's copies are clamped to 1..=999, as [`Job::copies`] documents.
    pub fn submit(&self, pdf: Vec<u8>, job: Job) -> Receiver<Result<String, String>> {
        let job = Job { copies: job.copies.clamp(1, 999), ..job };
        let pending = self.inner.backend.prepare(pdf, job);
        let (sender, receiver) = std::sync::mpsc::channel();
        let done: Done = Arc::new((Mutex::new(Phase::Waiting), Condvar::new()));
        {
            let mut entries = lock(&self.inner.entries);
            entries.retain(|entry| !entry.finished());
            entries.push(Entry { cancel: Arc::from(pending.cancel), waits_after_cancel: pending.waits_after_cancel, done: done.clone() });
        }
        let finish = Finish(done);
        let run = pending.run;
        // The entry is listed before the spawn: `InlineJobs` runs the job inside this call.
        self.inner.jobs.spawn(
            "print-submit",
            Box::new(move || {
                let _finish = finish;
                let sent = run().map_err(|error| error.to_string());
                let _ = sender.send(sent);
            }),
        );
        receiver
    }

    /// Cancels the latest submitted job; whether there was one to cancel. `true` also when no job
    /// was submitted; `false` when the latest job was already being sent (or has been sent), so
    /// its receipt still arrives.
    pub fn cancel_latest(&self) -> bool {
        // Called outside the lock: a backend's cancel may wait for an acknowledgement.
        let latest = lock(&self.inner.entries).last().map(|entry| entry.cancel.clone());
        latest.is_none_or(|cancel| cancel())
    }

    /// Stops the spool's work: every job is cancelled first, so a waiting job can't start sending
    /// while another is waited for; then it waits for the jobs already being sent (their cancel
    /// answered `false`) and for those whose backend still answers after a cancel, so the spooler
    /// has them before the process exits. A job still waiting (on a sleeping printer's driver, say)
    /// is never sent and not waited for.
    pub fn shutdown(&self) {
        let entries = std::mem::take(&mut *lock(&self.inner.entries));
        let finishing: Vec<Entry> = entries
            .into_iter()
            .filter(|entry| {
                let cancelled = (entry.cancel)();
                entry.waits_after_cancel || !cancelled
            })
            .collect();
        for entry in &finishing {
            entry.wait();
        }
    }
}

/// The platform seam of [`Spool`].
pub trait SpoolBackend: Send + Sync {
    /// The printers, or `None` when the platform lists none.
    fn printers(&self) -> Option<Vec<crate::Printer>>;

    /// Prepares the print of `pdf` with `job`: how to cancel it and how to run it.
    fn prepare(&self, pdf: Vec<u8>, job: Job) -> Pending;

    /// Whether the user chooses a printer for a job; false where the platform's own print dialog
    /// does (the browser). [`Spool::chooses_printer`] answers it.
    fn chooses_printer(&self) -> bool {
        true
    }
}

/// A prepared print: run it on a job, cancel it from anywhere.
pub struct Pending {
    /// Cancels the print. `true`: it will never be sent. `false`: it was already being sent,
    /// which completes as usual (or it has been sent).
    pub cancel: Box<dyn Fn() -> bool + Send + Sync>,
    /// Sends the print and answers the job's id.
    pub run: Box<dyn FnOnce() -> Result<String, crate::Error> + Send>,
    /// The print still ends with an answer after a cancel, which the caller waits for.
    pub waits_after_cancel: bool,
}

/// This platform's spooler: `lp` through [`submit_cancellable`](crate::submit_cancellable) on
/// macOS and Linux, a helper process per job on Windows, the browser's print dialog on wasm.
struct SystemBackend;

impl SpoolBackend for SystemBackend {
    fn printers(&self) -> Option<Vec<crate::Printer>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            // No driver is asked: a networked printer in power save can hold a capability query
            // for a long time, and the list must not wait on it.
            Some(crate::printer_queues())
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    fn chooses_printer(&self) -> bool {
        cfg!(not(target_arch = "wasm32"))
    }

    fn prepare(&self, pdf: Vec<u8>, job: Job) -> Pending {
        #[cfg(windows)]
        {
            // One process per job: a cancelled driver cannot keep another job's provider lock.
            let submission = isolated::Submission::new();
            let token = submission.clone();
            Pending {
                cancel: Box::new(move || submission.cancel()),
                run: Box::new(move || token.submit(&pdf, &job).map_err(crate::Error::Spool)),
                waits_after_cancel: true,
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Pending {
                // The browser's dialog takes the job at once; there is nothing left to cancel.
                cancel: Box::new(|| false),
                run: Box::new(move || {
                    let _ = job;
                    crate::browser::print_pdf(&pdf).map(|()| String::new()).map_err(crate::Error::Spool)
                }),
                waits_after_cancel: false,
            }
        }
        #[cfg(not(any(windows, target_arch = "wasm32")))]
        {
            let cancel = crate::Cancel::new();
            let token = cancel.clone();
            Pending {
                cancel: Box::new(move || cancel.cancel()),
                run: Box::new(move || crate::submit_cancellable(&pdf, &job, &mut |_, _| {}, &token)),
                waits_after_cancel: false,
            }
        }
    }
}
