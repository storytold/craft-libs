//! The one place this backend names `craft-wsd`.
//!
//! Every other module of [`crate::ipp`] imports the shared HTTP layer through here: the numeric
//! local-address URL policy, the injectable POST transport and its typed failures. IPP is carried
//! as `POST application/ipp`, which is what the transport sends; the SOAP-specific `SOAPAction` and
//! `Accept` headers the shared HTTP client adds are ignored by IPP servers.

use std::sync::Arc;

use crate::RetrySafety;

pub use craft_wsd::transport::{Request, Response, Transport};
pub use craft_wsd::{DeviceUrl, NetworkLimits};
pub use craft_wsd::{Stage, TransportKind};

/// The shared protocol error, as `craft-wsd` reports it.
pub type WireError = craft_wsd::Error;

/// A transport the backend can hold and share between threads.
pub type SharedTransport = Arc<dyn Transport + Send + Sync>;

/// The blocking plain-HTTP transport: numeric local addresses only, no proxy, no redirects, capped
/// replies and a timeout. It connects only when a request is made.
pub fn default_transport() -> SharedTransport {
    Arc::new(craft_wsd::http::HttpTransport::new())
}

/// What the shared layer says about repeating a request that failed with a typed transport error.
pub fn transport_safety(kind: TransportKind, stage: Stage, idempotent: bool) -> RetrySafety {
    wire_safety(WireError::transport_at(kind, stage, ""), idempotent)
}

/// The same for an HTTP status without an IPP body (429 and 503 are a decline: safe).
pub fn http_safety(status: u16, idempotent: bool) -> RetrySafety {
    wire_safety(WireError::http(status), idempotent)
}

fn wire_safety(error: WireError, idempotent: bool) -> RetrySafety {
    match error.retry_safety(idempotent) {
        craft_wsd::RetrySafety::Safe => RetrySafety::Safe,
        craft_wsd::RetrySafety::Idempotent => RetrySafety::Idempotent,
        // The shared enum is non-exhaustive: anything new is read as "it may have arrived".
        _ if idempotent => RetrySafety::Idempotent,
        _ => RetrySafety::OutcomeUnknown,
    }
}

/// Parse and validate a device URL (`http://<numeric local address>[:port]/path`).
pub fn parse_device_url(text: &str, noun: &str) -> Result<DeviceUrl, WireError> {
    DeviceUrl::parse_as(text, noun)
}
