//! WS-Print (Web Services for Devices) as a [`crate::backends::Backend`]. Cargo feature `wsd`, on by
//! default; not compiled for `wasm32`.
//!
//! [`WsdBackend`] prints to network printers that speak the WS-Print service, over `craft-wsd`
//! (SOAP 1.2 / WS-Addressing, MTOM, the numeric-local-address URL policy and the plain-HTTP
//! transport). It registers as `wsd`, has [`crate::BackendClass::Network`] (so the registry only
//! hands it requests that carry a [`crate::UserConfirmed`]) and names its printers `wsd:<url>`.
//!
//! # Nothing happens until the app asks
//!
//! * The backend **lists only the printers the app added** ([`WsdBackend::add`],
//!   [`WsdBackend::with_printers`]). It does no discovery and sends no multicast; finding printers
//!   on the network is a separate decision for the app.
//! * Building a backend, adding a printer and listing printers open no connection. A connection is
//!   made by [`Backend::printer_details`](crate::backends::Backend::printer_details) (one
//!   `GetPrinterElements`) and by a job.
//! * Printer URLs must be `http://` with a numeric loopback, link-local or private address; host
//!   names, public addresses, redirects and proxies are refused.
//!
//! # Layers
//!
//! * [`WsdBackend`]: the registry-facing backend (`backend.rs`).
//! * [`client::Client`]: one method per WS-Print operation over any transport; [`job`] is the flow
//!   that negotiates a format, plans the [`ticket::PrintTicket`], creates the job, sends the
//!   document (MTOM or inline base64): [`job::submit_document`] returns when the printer has accepted
//!   it, [`print_document`] adds a blocking watch for callers that want one. [`encoder`] and [`pwg`] make
//!   the document bytes (pass-through, a chain of alternatives, or PWG raster from a renderer);
//!   [`pwg`] re-exports the neutral [`crate::pwg`] encoder. Apps that go through the registry do not
//!   need these layers: they register a [`crate::DocumentConverter`] and the registry converts to a
//!   format this printer lists.
//! * `adapter.rs`: the only file that names `craft-wsd`.
//!
//! # Printer firmware and the ticket
//!
//! Some printers' firmware does not answer tickets containing `JobProcessing` or
//! `DocumentProcessing`. [`ticket::TicketShape`] lists smaller tickets for them; the default is the
//! full ticket, and a smaller one is opt-in: [`WsdBackend::with_ticket_shape`]. Settings that a
//! smaller ticket cannot carry (copies, sides, ...) are refused before anything is sent, or dropped
//! with a warning in the result when [`WsdBackend::with_drop_ticket_settings`] is set.
//!
//! # Submit, watch, retry
//!
//! [`WsdBackend`]'s `submit` returns a [`crate::Submitted`] as soon as the printer has accepted the
//! document. Following the job is a separate, polled step (`job_status` is one `GetJobElements`),
//! and failures are classified by [`WsdError::classify`]: transient or not, safe to repeat or not
//! ([`Op`] says whether the failed request was a query or a send), and where it failed. After an
//! outcome-unknown failure, [`WsdBackend::reconcile`] looks for the job before anything is resent.
//!
//! # Push status (opt-in)
//!
//! [`WsdBackend::with_events`] starts an inbound listener and WS-Eventing subscriptions so a watch
//! polls sooner; nothing listens otherwise. Events are only a wake-up, status always comes from
//! `job_status`. See [`events`] and `docs/events.md`.
//!
//! # Tracked jobs
//!
//! The backend carries a [`crate::PrintJob`]'s idempotency tag as a suffix of the ticket's job name
//! (cut so that it fits the 255 characters), lists jobs (`GetActiveJobs`, `GetJobHistory`) for
//! [`crate::Registry::list_jobs`] and finds an earlier attempt's job through
//! [`crate::Registry::find_job`], by tag. [`WsdBackend::reconcile`] is the older, WS-Print-only form
//! (by title and user) and its [`Reconciled`] is not [`crate::Reconciled`].

pub mod adapter;
pub mod client;
pub mod encoder;
pub mod error;
pub mod events;
pub mod fault;
pub mod job;
pub mod model;
pub mod mtom;
pub mod ns;
pub mod ops;
pub mod policy;
pub mod pwg;
pub mod ticket;

mod backend;

#[cfg(test)]
mod testutil;

#[cfg(feature = "testing")]
pub mod testing;

pub use backend::{JobMatch, Reconciled, WsdBackend, WsdJob};
pub use client::Client;
pub use encoder::{ColorMode, DocumentEncoder, PassThrough, RasterEncoder};
pub use error::{Class, KINDS, Op, Result, WsdError};
pub use events::{EventStats, EventTuning, EventsStatus, NotifyConfig, SubscriptionInfo, Wake};
pub use fault::{Fault, FaultCode};
pub use job::{
    Accepted, JobOutcome, JobResult, PollPolicy, PrintOptions, print_document, print_document_with_progress, printer_info, submit_document,
};
pub use model::{Capabilities, JobState, PrinterElements};
pub use mtom::Transfer;
pub use policy::{NetworkLimits, PrinterUrl};
pub use ticket::TicketShape;
