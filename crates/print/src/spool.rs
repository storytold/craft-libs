//! The print job layer: discovery of printers and submission of jobs as host jobs, per platform.

#[cfg(any(windows, test))]
mod isolated;

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::Job;

#[cfg(windows)]
pub use isolated::helper_main;

/// The print job layer, run as host jobs. Desktop backends call the existing
/// [`submit_cancellable`](crate::submit_cancellable) and [`printer_queues`](crate::printer_queues).
/// On wasm the backend prints through the browser, lists no printers and
/// [`chooses_printer`](Spool::chooses_printer) is false.
#[derive(Clone)]
pub struct Spool {
    stub: PhantomData<()>,
}

impl Spool {
    /// A spool for this platform that runs its work on `jobs`.
    pub fn new(jobs: Arc<dyn craft_host::Jobs>) -> Self {
        let _ = jobs;
        todo!("pf.C-PR1")
    }

    /// A spool over `backend`, for trials and tests that fake the platform.
    pub fn with_backend(jobs: Arc<dyn craft_host::Jobs>, backend: Arc<dyn SpoolBackend>) -> Self {
        let _ = (jobs, backend);
        todo!("pf.C-PR1")
    }

    /// Whether the user chooses a printer (false on wasm, where the browser's dialog does).
    pub fn chooses_printer(&self) -> bool {
        todo!("pf.C-PR1")
    }

    /// Lists the printers in a job; the list arrives on the receiver.
    pub fn discover(&self) -> Receiver<Vec<crate::Printer>> {
        todo!("pf.C-PR1")
    }

    /// The printer discovery the print dialog takes, mapping each printer to `ui::Printer`.
    #[cfg(feature = "ui")]
    pub fn discoverer(&self) -> crate::ui::Discover {
        todo!("pf.C-PR1")
    }

    /// Prints `pdf` with `job` in a job; the job's id or the failure arrives on the receiver.
    pub fn submit(&self, pdf: Vec<u8>, job: Job) -> Receiver<Result<String, String>> {
        let _ = (pdf, job);
        todo!("pf.C-PR1")
    }

    /// Cancels the latest submitted job; whether there was one to cancel.
    pub fn cancel_latest(&self) -> bool {
        todo!("pf.C-PR1")
    }

    /// Stops the spool's work.
    pub fn shutdown(&self) {
        todo!("pf.C-PR1")
    }
}

/// The platform seam of [`Spool`].
pub trait SpoolBackend: Send + Sync {
    /// The printers, or `None` when the platform lists none.
    fn printers(&self) -> Option<Vec<crate::Printer>>;

    /// Prepares the print of `pdf` with `job`: how to cancel it and how to run it.
    fn prepare(&self, pdf: Vec<u8>, job: Job) -> Pending;
}

/// A prepared print: run it on a job, cancel it from anywhere.
pub struct Pending {
    /// Cancels the print; whether it was still cancellable.
    pub cancel: Box<dyn Fn() -> bool + Send + Sync>,
    /// Sends the print and answers the job's id.
    pub run: Box<dyn FnOnce() -> Result<String, crate::Error> + Send>,
    /// The print still ends with an answer after a cancel, which the caller waits for.
    pub waits_after_cancel: bool,
}
