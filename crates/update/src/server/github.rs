//! Asking GitHub for the newest release over HTTPS.

use crate::Error;
use crate::release::{MAX_RESPONSE_BYTES, Release, latest_release_url, parse_latest};
use std::time::Duration;

/// Where the newest release comes from. `craft-updated` asks GitHub; tests use stand-ins.
pub trait ReleaseSource: Send + Sync {
    fn latest(&self) -> Result<Release, Error>;
}

/// GitHub's releases API, unauthenticated (60 requests an hour per address; a check every few
/// hours uses a handful a day).
pub struct GitHub {
    url: String,
    agent: ureq::Agent,
}

impl GitHub {
    pub fn new(repository: &str) -> Result<GitHub, Error> {
        let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into();
        Ok(GitHub { url: latest_release_url(repository)?, agent })
    }
}

impl ReleaseSource for GitHub {
    fn latest(&self) -> Result<Release, Error> {
        let response = self
            .agent
            .get(&self.url)
            .header("User-Agent", concat!("craft-updated/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call();
        let mut response = match response {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(404)) => return Err(Error::Release(format!("{} has no published release", self.url))),
            Err(ureq::Error::StatusCode(code @ (403 | 429))) => {
                return Err(Error::Release(format!("GitHub refused the check (HTTP {code}); its hourly limit was probably reached")));
            }
            Err(e) => return Err(Error::Release(format!("couldn't ask GitHub for the newest release: {e}"))),
        };
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES as u64)
            .read_to_string()
            .map_err(|e| Error::Release(format!("couldn't read GitHub's answer: {e}")))?;
        parse_latest(&body)
    }
}
