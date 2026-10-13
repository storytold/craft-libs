//! The transport seam.
//!
//! This crate builds requests and parses responses. It never opens a socket. The caller supplies
//! the transport by implementing [`Http`], which keeps the crate free of an HTTP dependency and
//! lets the same code serve a native app and a WebAssembly build, where the only way out is the
//! browser's own fetch.
//!
//! The split follows the one the Craft apps already use for Zotero, where the wire format and the
//! socket live in separate modules.

use std::collections::BTreeMap;
use std::fmt;

/// An HTTP method. WebDAV and CardDAV need more than the browser verbs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Put,
    Post,
    Delete,
    Propfind,
    Report,
    Mkcol,
    Lock,
    Unlock,
}

impl Method {
    /// The token that goes on the request line.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Put => "PUT",
            Method::Post => "POST",
            Method::Delete => "DELETE",
            Method::Propfind => "PROPFIND",
            Method::Report => "REPORT",
            Method::Mkcol => "MKCOL",
            Method::Lock => "LOCK",
            Method::Unlock => "UNLOCK",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A request to send. `url` is absolute, so a transport needs no base of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    /// Header names are lowercase, which keeps lookups and test assertions predictable.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn new(method: Method, url: impl Into<String>) -> Request {
        Request { method, url: url.into(), headers: BTreeMap::new(), body: Vec::new() }
    }

    /// Set a header. The name is lowercased.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Request {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }

    #[must_use]
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Request {
        self.body = body.into();
        self
    }

    /// Read a header back, by any casing.
    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

/// A response as the transport saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Response {
        Response { status, headers: BTreeMap::new(), body: body.into() }
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Response {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }

    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    /// 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as text, with invalid UTF-8 replaced. Server messages are for humans and for logs,
    /// so a stray byte must not cost us the whole message.
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }
}

/// A transport. One method, because that is all this crate needs.
///
/// An implementation is expected to send `req` as given, follow no redirects, and hand back
/// whatever came off the wire. Do not raise an error for a non-2xx status. The parsers here read
/// status codes themselves, and Nextcloud reports real outcomes through them, 207 Multi-Status
/// above all.
pub trait Http {
    /// The transport's own failure type, carried through [`crate::Error::Transport`].
    type Error: fmt::Display;

    fn send(&self, req: Request) -> Result<Response, Self::Error>;
}
