#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Exchange Web Services: the SOAP layer, without sockets.
//!
//! - [`soap`]: the request envelope ([`envelope`]) with the schema version asked for ([`Version`]),
//!   and [`Element`], a builder of the operation's elements that escapes everything it is given, so
//!   nothing a caller hands in can end an element, open another or break a line of the protocol.
//! - [`answer`]: the answer envelope read with `craft-xml` ([`Answer::parse`]): a fault as the error
//!   it names, the response messages of an operation ([`Answer::messages`]) with their class and code
//!   ([`codes`]), and a busy server's wait.
//!
//! Written from Microsoft's public documentation: the EWS reference and the Open Specifications
//! [MS-OXWSCDATA] (types) and [MS-OXWSCORE] (operations and their messages).
//!
//! What stays with the caller: HTTP, Autodiscover, authentication, the operations themselves (which
//! elements to send, what to make of the elements that come back) and any sync state.

pub mod answer;
pub mod codes;
pub mod soap;

pub use answer::{Answer, Error, ResponseMessage};
pub use codes::{ResponseClass, ResponseCode};
pub use soap::{Element, Version, envelope};
