//! The one place this backend names `craft-wsd`.
//!
//! Every other module of [`crate::wsd`] imports the WSD core through here, so a rename or a
//! reshuffle in `craft-wsd` is a change to this file only. Nothing in it opens a socket by itself:
//! the transports are values the backend is given (or builds, see [`default_transport`]).

use std::sync::Arc;

use crate::RetrySafety;

pub use craft_wsd::soap::{Attachment, Call, SoapClient};
pub use craft_wsd::transport::{Request, Response, Transport};
pub use craft_wsd::{DeviceUrl, NetworkLimits, xml};

/// WS-Eventing, the client side (subscribe, renew, unsubscribe, parse what the printer pushes).
pub use craft_wsd::eventing as wse;
pub use craft_wsd::eventing::{Expiry, Filter, Incoming, Notification, SubscribeRequest, Subscription};
pub use craft_wsd::ns::WSE;
pub use craft_wsd::{EndpointRef, ReferenceParameter};

/// The shared protocol error, as `craft-wsd` reports it.
pub type WireError = craft_wsd::Error;
/// Every value [`WireError::kind`] can return.
pub use craft_wsd::KINDS as WIRE_KINDS;
/// A SOAP fault as the generic layer parses it (the sub-code is classified by [`super::fault`]).
pub type WireFault = craft_wsd::Fault;
pub use craft_wsd::fault::{Role, parse_fault};
pub use craft_wsd::ns::{ANONYMOUS, FAULT_ACTION, SOAP, WSA, WSA_FINAL, XOP};
pub use craft_wsd::{Fault, Stage, SubCode, TransportKind};

/// What the shared layer says about repeating a request that failed with a typed transport error:
/// nothing sent is [`RetrySafety::Safe`], a request that may have arrived is
/// [`RetrySafety::Idempotent`] when `idempotent` and [`RetrySafety::OutcomeUnknown`] otherwise.
pub fn transport_safety(kind: TransportKind, stage: Stage, idempotent: bool) -> RetrySafety {
    wire_safety(WireError::transport_at(kind, stage, ""), idempotent)
}

/// The same for an HTTP status without a SOAP fault (429 and 503 are a decline: safe).
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

/// The loopback HTTP server and the media-type parameter reader of `craft-wsd`'s `testing` feature,
/// for [`super::testing`].
#[cfg(feature = "testing")]
pub use craft_wsd::mtom::param;
#[cfg(feature = "testing")]
pub use craft_wsd::testing::{HttpRequest, Server, raw_response};

/// A transport the backend can hold and share between threads.
pub type SharedTransport = Arc<dyn Transport + Send + Sync>;

/// The blocking plain-HTTP transport: numeric local addresses only, no proxy, no redirects, capped
/// replies and a timeout. It is built only when a backend is made, and connects only when a job or a
/// details request asks it to.
pub fn default_transport() -> SharedTransport {
    Arc::new(craft_wsd::http::HttpTransport::new())
}

/// A fresh random UUID (lowercase, no `urn:uuid:` prefix).
pub fn new_uuid() -> String {
    craft_wsd::envelope::new_uuid()
}

/// `xop:Include` for the part with this content id.
pub fn xop_include_ref(cid: &str) -> String {
    craft_wsd::mtom::xop_include_ref(cid)
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    craft_wsd::mtom::base64(bytes)
}

/// Parse and validate a printer URL; `noun` names it in the refusal ("printer").
pub fn parse_device_url(text: &str, noun: &str) -> Result<DeviceUrl, WireError> {
    DeviceUrl::parse_as(text, noun)
}
