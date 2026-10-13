//! The app's side of the [`crate::protocol`], independent of how requests travel: the browser
//! client (`web::Client`, feature `web`) implements [`Backend`], and tests use stand-ins.

use crate::protocol::{ErrorReply, LoginReply, Status};

/// Sends requests to `craft-updated`. Calls return at once; answers arrive later through
/// [`Backend::poll`], so a browser can wait for `fetch` without blocking a frame.
pub trait Backend {
    fn request_status(&mut self);
    fn request_login(&mut self, username: &str, password: &str);
    fn request_update(&mut self, token: &str);
    fn request_logout(&mut self, token: &str);
    /// The answers that arrived since the last call.
    fn poll(&mut self) -> Vec<Reply>;
    /// Reloads the page so it runs the newly deployed version.
    fn reload_page(&mut self) {}
}

/// An answer from `craft-updated`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Status(Result<Status, ApiError>),
    Login(Result<LoginReply, ApiError>),
    Update(Result<Status, ApiError>),
}

/// Why a request didn't succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// `401`: wrong credentials, or the session ended.
    Unauthorized,
    /// `429`: sign-in is locked for a while after failed attempts.
    LockedOut(String),
    /// `409`: nothing to update, or an update is already running.
    Refused(String),
    /// The service didn't answer, or answered something unexpected.
    Unavailable(String),
}

impl ApiError {
    /// Maps an unsuccessful HTTP answer.
    pub fn from_http(status: u16, body: &str) -> ApiError {
        let message = serde_json::from_str::<ErrorReply>(body).map(|e| e.error).unwrap_or_else(|_| format!("HTTP {status}"));
        match status {
            401 => ApiError::Unauthorized,
            429 => ApiError::LockedOut(message),
            409 => ApiError::Refused(message),
            _ => ApiError::Unavailable(message),
        }
    }
}

/// Reads a `200`/`202` JSON answer, or maps the error.
pub fn parse_reply<T: serde::de::DeserializeOwned>(status: u16, body: &str) -> Result<T, ApiError> {
    if (200..300).contains(&status) {
        serde_json::from_str(body).map_err(|e| ApiError::Unavailable(format!("unexpected answer: {e}")))
    } else {
        Err(ApiError::from_http(status, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_answers_map_to_replies() {
        assert_eq!(ApiError::from_http(401, r#"{"error":"sign in"}"#), ApiError::Unauthorized);
        assert_eq!(ApiError::from_http(429, r#"{"error":"try again in 15 minutes"}"#), ApiError::LockedOut("try again in 15 minutes".into()));
        assert_eq!(ApiError::from_http(409, r#"{"error":"already running"}"#), ApiError::Refused("already running".into()));
        assert_eq!(ApiError::from_http(502, "<html>Bad Gateway</html>"), ApiError::Unavailable("HTTP 502".into()));
        let ok: Status = parse_reply(200, r#"{"current":"0.4.0","latest":"0.6.1","update_available":true}"#).unwrap();
        assert!(ok.update_available);
        assert!(matches!(parse_reply::<Status>(200, "<html>"), Err(ApiError::Unavailable(_))));
        assert_eq!(parse_reply::<Status>(404, ""), Err(ApiError::Unavailable("HTTP 404".into())));
    }
}
