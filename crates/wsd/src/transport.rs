//! The injectable HTTP layer.
//!
//! The client never opens a socket itself: it hands a [`Request`] to a [`Transport`]. A transport
//! only ever sees a validated [`DeviceUrl`], and must honour the policy numbers in the request:
//! no proxy, no redirects, a timeout, and a response body no larger than `max_response`.
//!
//! A transport is not tied to sockets: an in-process device, a record/replay double or a different
//! platform's HTTP stack implements the same trait. It reports failure with the typed
//! constructors [`Error::transport`] / [`Error::transport_at`] (so callers never parse text) and
//! passes the device's `Retry-After` on in [`Response::retry_after`].

use std::time::Duration;

use crate::error::Error;
use crate::policy::DeviceUrl;

/// One HTTP POST.
#[derive(Clone, Debug)]
pub struct Request {
    pub content_type: String,
    /// The SOAP action URI, to be sent as `SOAPAction`.
    pub soap_action: String,
    pub body: Vec<u8>,
    pub timeout: Duration,
    /// Read at most this many body bytes; more is an error ([`Error::TooLarge`]).
    pub max_response: usize,
}

/// The reply: status, `Content-Type`, the (capped) body and the device's `Retry-After`.
///
/// `non_exhaustive`: build one with [`Response::new`] so fields can be added without breaking
/// transports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Response {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    /// The wait the device asked for with `Retry-After`, already parsed and capped
    /// ([`crate::parse_retry_after`]); `None` when absent or unusable.
    pub retry_after: Option<Duration>,
}

impl Response {
    /// A reply without a `Retry-After`.
    pub fn new(status: u16, content_type: Option<String>, body: Vec<u8>) -> Self {
        Self { status, content_type, body, retry_after: None }
    }

    /// The same reply with the device's `Retry-After`.
    #[must_use]
    pub fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }
}

/// Something that can POST to a device.
pub trait Transport {
    /// Redirects must be refused (as [`crate::TransportKind::Redirect`]), proxies must not be used,
    /// and the exchange must time out. A non-success status is a reply, not an error.
    fn post(&self, url: &DeviceUrl, request: Request) -> Result<Response, Error>;
}

impl<T: Transport + ?Sized> Transport for &T {
    fn post(&self, url: &DeviceUrl, request: Request) -> Result<Response, Error> {
        (**self).post(url, request)
    }
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn post(&self, url: &DeviceUrl, request: Request) -> Result<Response, Error> {
        (**self).post(url, request)
    }
}

impl<T: Transport + ?Sized> Transport for std::sync::Arc<T> {
    fn post(&self, url: &DeviceUrl, request: Request) -> Result<Response, Error> {
        (**self).post(url, request)
    }
}
