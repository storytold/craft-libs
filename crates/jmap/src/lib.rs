#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! The JMAP protocol core (RFC 8620), without sockets and without browser APIs.
//!
//! The crate describes HTTP requests and judges the responses; the program performs each exchange through
//! an [`HttpExchange`] (the browser's `fetch`, an HTTP client where there are sockets, a test's script). It
//! has no I/O of its own, no thread and no clock, so it builds for wasm32 unchanged and its futures need
//! not be `Send`. It is written from the public specifications: RFC 3986 and the WHATWG URL Standard
//! (addresses), RFC 6570 (address templates), RFC 8620 (the session, requests and responses, ids), RFC 9110
//! (status codes, `Retry-After`), RFC 7617 and RFC 6750 (credentials), RFC 9457 (problem details) and
//! RFC 8259 (JSON).
//!
//! - [`address`], [`host`], [`discovery`], [`template`], [`percent`]: absolute `http(s)` addresses, the
//!   transport rule (`https`, or plain `http` to `localhost`, `127.0.0.0/8` or `[::1]` only), what a person
//!   types as a session address, and the download and upload templates.
//! - [`http`], [`classify`], [`retry`], [`credentials`]: request and response descriptions, Basic and
//!   Bearer credentials, `Retry-After`, and the judgement of a response into an [`Error`].
//! - [`session`], [`client`], [`request`], [`response`], [`id`]: the session object, the client, requests
//!   of method calls with result references, the results of each call, and ids.
//! - [`redact`]: what a server says, made into short, clean excerpts with every known secret taken out.
//!
//! What a program asks with it (`Mailbox/get`, `Email/query`…) and what it makes of the answers is the
//! program's: the method calls' arguments and results are JSON values here.
//!
//! Everything a server sends is hostile: sizes, counts, nesting and the length of every string are
//! bounded, ids are held to the JMAP alphabet ([`Id`]), and nothing indexes into what came over the wire.
//! The credentials, the account's address and the session's stay out of error text ([`Redactor`]), and
//! `Debug` output shows no password, token, user name, path or mail.

// The protocol core, under the lints it was written with: every public item documented, and outside the
// tests no indexing and no arithmetic that can overflow.
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod address;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod classify;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod client;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod credentials;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod discovery;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod error;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod host;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod http;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod id;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod percent;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod redact;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod request;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod response;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod retry;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod session;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod template;

#[cfg(test)]
mod hostile_tests;
// The executor of the tests, and a scripted exchange. Never in a build that is not a test's.
#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod test_support;

pub use address::{Address, AddressError, Origin, Scheme};
pub use classify::judge_response;
pub use client::{Client, fetch_session, read_session, session_request};
pub use credentials::{Credentials, CredentialsError};
pub use discovery::session_address;
pub use error::{BuildError, Error, LimitName, RequestProblem, Unusable};
pub use host::{Host, HostScope};
pub use http::{HttpExchange, HttpRequest, HttpResponse, Method, RequestKind, TransportError};
pub use id::{Id, IdError};
pub use redact::{Excerpt, Redactor};
pub use request::{ApiRequest, CallId, ResultReference};
pub use response::{ApiResponse, CallFailure, MethodError, MethodErrorKind, MethodResponse};
pub use retry::{HttpDate, MAX_RETRY_DELAY, RetryAfter, UnixTime};
pub use session::{Account, CAPABILITY_CORE, CAPABILITY_MAIL, CoreLimits, Session};
pub use template::{AddressTemplate, TemplateError};
