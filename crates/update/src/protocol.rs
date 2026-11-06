//! The vocabulary installers and release lookups report in: what a job is doing, how it ended,
//! where new installs go and what a release lookup found.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::release::Release;

/// Parent folders for new installs. Existing installations keep their recorded location.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallLocations {
    pub root: Option<String>,
    pub per_app: BTreeMap<String, String>,
}

impl InstallLocations {
    pub fn folder(&self, app: &str) -> Option<&str> {
        self.per_app.get(app).map(String::as_str).or(self.root.as_deref())
    }
}

/// The answer to "what's the latest release of this app?".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LatestRelease {
    Found(Release),
    /// The repo exists but has no published release.
    None,
    /// Couldn't find out (offline, rate-limited, unreadable answer).
    Failed(String),
}

/// What a running job is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPhase {
    Downloading {
        done: u64,
        total: u64,
    },
    /// The connection dropped; trying again shortly, from where it stopped.
    Reconnecting {
        done: u64,
        total: u64,
    },
    Verifying,
    /// The system's installer is busy with another app; this one goes next.
    Waiting,
    /// Waiting on the OS installer (MSI) or copying files.
    Installing,
    Removing,
}

/// The `JobFinished` error a paused job ends with (not a failure).
pub const PAUSED: &str = "paused";
/// The `JobFinished` error a cancelled job ends with (not a failure).
pub const CANCELLED: &str = "cancelled";
/// The `JobFinished` error of an install whose system prompt (password, Windows Installer) the
/// user dismissed after the download finished (not a failure).
pub const INSTALL_CANCELLED: &str = "install cancelled";

/// Where release information came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// ArtCraft's release feed (storyteller-web), which polls GitHub for everyone.
    Feed,
    /// The GitHub REST API.
    GitHubApi,
    /// github.com release pages (no API quota; used when the API is rate-limited).
    GitHubPages,
    /// Saved answers from an earlier check.
    Cache,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Feed => "ArtCraft",
            Self::GitHubApi => "GitHub",
            Self::GitHubPages => "GitHub (release pages)",
            Self::Cache => "saved results",
        }
    }
}
