//! One error type for the whole crate.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// The caller's transport failed. The message is whatever it reported.
    #[error("transport failed: {0}")]
    Transport(String),

    /// The server answered, and the status says no. `body` is truncated for logs.
    #[error("{method} {path} returned HTTP {status}: {body}")]
    Status { method: &'static str, path: String, status: u16, body: String },

    /// The OCS envelope parsed, and its own status code says no. Nextcloud reports application
    /// failures here while still answering HTTP 200, so this is a distinct case from
    /// [`Error::Status`].
    #[error("OCS refused the request (statuscode {code}): {message}")]
    Ocs { code: u16, message: String },

    /// The body was not the shape the protocol promises.
    #[error("malformed {what}: {detail}")]
    Malformed { what: &'static str, detail: String },

    /// A path or name could not be used safely. Carrying this as an error, never a panic, is the
    /// point.
    #[error("invalid {what}: {detail}")]
    Invalid { what: &'static str, detail: String },
}

impl Error {
    /// Build a [`Error::Status`] from a response, truncating the body so a server that answers
    /// with a whole HTML error page cannot flood a log line.
    pub(crate) fn status(method: crate::http::Method, path: &str, resp: &crate::http::Response) -> Error {
        const MAX: usize = 400;
        let text = resp.text();
        let body: String = text.chars().take(MAX).collect();
        Error::Status { method: method.as_str(), path: path.to_owned(), status: resp.status, body }
    }

    pub(crate) fn malformed(what: &'static str, detail: impl Into<String>) -> Error {
        Error::Malformed { what, detail: detail.into() }
    }

    pub(crate) fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
        Error::Invalid { what, detail: detail.into() }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
