//! IPP (RFC 8010/8011, PWG 5100.x) as a [`crate::backends::Backend`]. Cargo feature `ipp`, on by
//! default; not compiled for `wasm32`.
//!
//! [`IppBackend`] prints to network printers that speak the Internet Printing Protocol over plain
//! HTTP (`ipp://` and `http://` URIs, port 631 by default), over the numeric-local-address URL
//! policy and the blocking HTTP transport of `craft-wsd`. It registers as `ipp`, has
//! [`crate::BackendClass::Network`] (so the registry only hands it requests that carry a
//! [`crate::UserConfirmed`]), names its printers `ipp:<printer uri>` and its jobs
//! `ipp:<printer uri>#<job id>` (see [`IppBackend::job_id`]).
//!
//! # Nothing happens until the app asks
//!
//! * The backend **lists only the printers the app added** ([`IppBackend::add`],
//!   [`IppBackend::with_printers`]). It does no discovery and sends no multicast.
//! * Building a backend, adding a printer and listing printers open no connection. A connection is
//!   made by [`Backend::printer_details`](crate::backends::Backend::printer_details) (one
//!   `Get-Printer-Attributes`) and by a job.
//! * Printer URIs must be `ipp://` or `http://` with a numeric loopback, link-local or private
//!   address; host names, public addresses, redirects and proxies are refused, and so is `ipps://`
//!   ([`error::IppError::Tls`]): there is no TLS.
//!
//! # Layers
//!
//! * [`IppBackend`]: the registry-facing backend (`backend.rs`).
//! * [`client::Client`]: one method per operation (`Get-Printer-Attributes`, `Validate-Job`,
//!   `Print-Job`, `Create-Job`, `Send-Document`, `Get-Job-Attributes`, `Get-Jobs`, `Cancel-Job`) over
//!   any transport; [`plan`] maps the neutral job settings onto job attributes against the printer's
//!   published capabilities; [`model`] reads the printer and job attributes.
//! * [`codec`]: the RFC 8010 binary encoding, strict and bounded.
//! * `adapter.rs`: the only file that names `craft-wsd`.
//!
//! # Submit, watch, retry
//!
//! `submit` returns a [`crate::Submitted`] as soon as the printer has accepted the job. Following
//! the job is a separate, polled step (`job_status` is one `Get-Job-Attributes`; IPP event
//! subscriptions are not used). Failures are classified by [`IppError::classify`]: transient or not,
//! safe to repeat or not, and where they failed; the IPP status table is in `docs/ipp.md`. After an
//! outcome-unknown failure the registry's `find_job` looks for the job (by the idempotency tag in
//! `job-name`) before anything is resent.
//!
//! # Tests
//!
//! There is no fake printer in this crate. The unit tests here use static byte vectors and a scripted
//! transport; the integration suite against a real IPP server (the fake printer is fakeprint) lives
//! in the pdfcraft-fakeprint harness, outside this repository.

pub mod adapter;
pub mod client;
pub mod codec;
pub mod error;
pub mod model;
pub mod plan;
pub mod policy;
pub mod protocol;

mod backend;

#[cfg(test)]
mod tests;

pub use backend::{IppBackend, IppJob, JobFlow};
pub use client::{Client, IppVersion, JobRequest, Which};
pub use codec::{Attribute, CodecError, Group, GroupTag, Limits, Message, Value};
pub use error::{Class, IppError, KINDS, Op, Result};
pub use model::{JobAttrs, PrinterAttrs};
pub use policy::{NetworkLimits, PrinterUri};
pub use protocol::{Operation, StatusCode};
