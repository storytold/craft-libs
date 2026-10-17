//! Shared Web Services for Devices (WSD) plumbing. Layer L4.
//!
//! The protocol-agnostic half of WSD clients: a WS-Print or WS-Scan client builds on it and adds
//! only its own operations, models and fault codes.
//!
//! * [`xml`]: bounded parsing (size, depth, node count, no DTD) and escaped writing.
//! * [`envelope`]: SOAP 1.2 / WS-Addressing request envelopes and checked reply reading.
//! * [`fault`]: SOAP faults as service-neutral values; the service maps the sub-code.
//! * [`mtom`]: MTOM/XOP packaging and multipart reply reading.
//! * [`policy`]: [`DeviceUrl`] (numeric private-network addresses only) and [`NetworkLimits`].
//! * [`transport`]: the [`Transport`] seam; [`http`] (cargo feature `http`, not built for
//!   `wasm32`) is a hardened blocking implementation on `ureq` with no proxy, no redirects, a
//!   timeout and a body cap; it only connects to addresses that [`DeviceUrl`] admits.
//! * [`error`]: the shared [`Error`], with typed transport failures ([`TransportKind`], [`Stage`]),
//!   HTTP status, `Retry-After` and fault accessors; [`retry`]: [`RetrySafety`] and `Retry-After` parsing.
//! * `testing` (feature `testing`): a loopback HTTP server for tests.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod addressing;
pub mod cancel;
pub mod discovery;
pub mod envelope;
pub mod error;
pub mod eventing;
pub mod fault;
#[cfg(not(target_arch = "wasm32"))]
pub mod http;
pub mod metadata;
pub mod mtom;
pub mod ns;
pub mod policy;
pub mod retry;
pub mod service;
pub mod soap;
pub mod transport;
pub mod xml;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use addressing::{EndpointRef, ReferenceParameter};
pub use cancel::CancelToken;
pub use error::{Error, KINDS, Result, Stage, TransportKind};
pub use fault::{Fault, Role, SubCode};
pub use metadata::DeviceMetadata;
pub use policy::{DeviceUrl, NetworkLimits};
pub use retry::{MAX_RETRY_AFTER, RetrySafety, parse_retry_after};
pub use service::{ServiceDescriptor, ServiceKind};
pub use soap::{Attachment, Call, SoapClient};
pub use transport::{Request, Response, Transport};
pub use xml::QName;
