//! What the browser and `craft-updated` say to each other: JSON over HTTP, through the web
//! server that already serves the app (a reverse-proxied path, `craft-update/` by default).
//!
//! | Request | Who | Answer |
//! |---|---|---|
//! | `GET api/status` | anyone | `200` [`Status`] |
//! | `POST api/login` [`LoginRequest`] | anyone | `200` [`LoginReply`], `401` wrong credentials, `429` locked out |
//! | `POST api/update` | administrator (`Authorization: Bearer <token>`) | `202` [`Status`], `401`, `409` nothing to update or already running |
//! | `POST api/check` | administrator | `200` [`Status`] after asking GitHub now, `401` |
//! | `POST api/logout` | administrator | `204` |
//!
//! Errors come back as [`ErrorReply`]. Requests carry no versions, tags, commands or paths: the
//! update always goes to the newest release `craft-updated` itself found on GitHub, built the
//! way its own configuration says.

use serde::{Deserialize, Serialize};

pub const STATUS: &str = "api/status";
pub const LOGIN: &str = "api/login";
pub const UPDATE: &str = "api/update";
pub const CHECK: &str = "api/check";
pub const LOGOUT: &str = "api/logout";

/// What every visitor may know: which version the site runs, whether a newer one is out, and
/// whether an update is happening. Failure details go only to the administrator's email and the
/// server log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// The version the site serves now.
    pub current: String,
    /// The newest release found on GitHub, once a check has succeeded.
    #[serde(default)]
    pub latest: Option<String>,
    /// The newest release's page on GitHub.
    #[serde(default)]
    pub release_url: Option<String>,
    /// Whether `latest` is newer than `current`.
    #[serde(default)]
    pub update_available: bool,
    /// When GitHub was last asked successfully (Unix seconds).
    #[serde(default)]
    pub checked_at: Option<u64>,
    #[serde(default)]
    pub job: Job,
}

/// The update job, as everyone sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Job {
    #[default]
    Idle,
    Running {
        version: String,
        started_at: u64,
    },
    Succeeded {
        version: String,
        finished_at: u64,
    },
    Failed {
        version: String,
        finished_at: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// A signed-in administrator's session: an opaque bearer token, valid for `expires_in` seconds
/// and only in the `craft-updated` process that issued it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginReply {
    pub token: String,
    pub expires_in: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReply {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_and_tolerates_missing_fields() {
        let s = Status {
            current: "0.4.0".into(),
            latest: Some("0.6.1".into()),
            release_url: Some("https://github.com/storytold/gridcraft/releases/tag/v0.6.1".into()),
            update_available: true,
            checked_at: Some(1_760_000_000),
            job: Job::Running { version: "0.6.1".into(), started_at: 1_760_000_100 },
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""state":"running""#), "{json}");
        assert_eq!(serde_json::from_str::<Status>(&json).unwrap(), s);
        let minimal: Status = serde_json::from_str(r#"{"current":"0.4.0"}"#).unwrap();
        assert_eq!(minimal.job, Job::Idle);
        assert!(!minimal.update_available);
    }
}
