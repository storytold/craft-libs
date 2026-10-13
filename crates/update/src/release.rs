//! The newest release of an app, from GitHub's "latest release" API
//! (`GET https://api.github.com/repos/{owner}/{repo}/releases/latest`), which already leaves out
//! drafts and prereleases.
//!
//! The tag of a release becomes a `git checkout` argument on the server, so it is held to the
//! strictest shape this crate knows: `v` and a release version, nothing else
//! ([`Release::tag`]). A release with any other tag is refused rather than built.

use crate::{Error, Version};
use serde::Deserialize;

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The git tag, exactly `v{version}` or `{version}` (release versions only, no build
    /// metadata), so it can't be read as an option or name anything but that tag.
    pub tag: String,
    pub version: Version,
    /// The release page, when it is on `https://github.com/`.
    pub url: Option<String>,
    /// When the release was published (RFC 3339, as GitHub sends it), if given.
    pub published_at: Option<String>,
}

/// The most of a release response read; GitHub's are a few KiB plus the release notes.
pub const MAX_RESPONSE_BYTES: usize = 1 << 20;

#[derive(Deserialize)]
struct Raw {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Parses a "latest release" response.
pub fn parse_latest(json: &str) -> Result<Release, Error> {
    if json.len() > MAX_RESPONSE_BYTES {
        return Err(Error::Release("the release response is too large".into()));
    }
    let raw: Raw = serde_json::from_str(json).map_err(|e| Error::Release(format!("the release response isn't one GitHub sends: {e}")))?;
    if raw.draft || raw.prerelease {
        return Err(Error::Release(format!("{} is a draft or prerelease", raw.tag_name)));
    }
    let tag = raw.tag_name.trim();
    let version = Version::parse(tag).ok_or_else(|| Error::Release(format!("the release tag {tag:?} isn't a version")))?;
    if version.is_prerelease() || (tag != format!("v{version}") && tag != version.to_string()) {
        return Err(Error::Release(format!("the release tag {tag:?} isn't a plain release version")));
    }
    let url = raw.html_url.filter(|u| u.starts_with("https://github.com/") && u.len() < 512 && !u.chars().any(char::is_whitespace));
    let published_at = raw.published_at.filter(|p| p.len() < 64);
    Ok(Release { tag: tag.to_string(), version, url, published_at })
}

/// Checks an `owner/repo` name the way GitHub restricts them, so it can go into an API URL.
pub fn validate_repository(repository: &str) -> Result<(), Error> {
    let bad = || Error::Config(format!("{repository:?} isn't a GitHub repository written as owner/name"));
    let (owner, name) = repository.split_once('/').ok_or_else(bad)?;
    let part_ok = |p: &str| {
        !p.is_empty() && p.len() <= 100 && p != "." && p != ".." && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    if part_ok(owner) && part_ok(name) { Ok(()) } else { Err(bad()) }
}

/// The API address of a repository's latest release.
pub fn latest_release_url(repository: &str) -> Result<String, Error> {
    validate_repository(repository)?;
    Ok(format!("https://api.github.com/repos/{repository}/releases/latest"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(tag: &str) -> String {
        serde_json::json!({
            "url": "https://api.github.com/repos/storytold/gridcraft/releases/1",
            "html_url": format!("https://github.com/storytold/gridcraft/releases/tag/{tag}"),
            "tag_name": tag,
            "name": format!("GridCraft {tag}"),
            "draft": false,
            "prerelease": false,
            "published_at": "2026-10-12T17:07:30Z",
            "assets": [],
            "body": "Release notes"
        })
        .to_string()
    }

    #[test]
    fn a_github_latest_release_response_parses() {
        let r = parse_latest(&response("v0.6.1")).unwrap();
        assert_eq!(r.tag, "v0.6.1");
        assert_eq!(r.version, Version::parse("0.6.1").unwrap());
        assert_eq!(r.url.as_deref(), Some("https://github.com/storytold/gridcraft/releases/tag/v0.6.1"));
        assert_eq!(r.published_at.as_deref(), Some("2026-10-12T17:07:30Z"));
        assert!(r.version.is_newer_than(&Version::parse("0.4.0").unwrap()));
        assert_eq!(parse_latest(&response("0.6.1")).unwrap().tag, "0.6.1");
    }

    #[test]
    fn tags_that_are_not_plain_versions_are_refused() {
        for tag in
            ["--upload-pack=touch /tmp/x", "v0.6.1-rc.1", "v0.6.1+build", "release-0.6.1", "main", "v0.6", "refs/heads/main", "v0.06.1", "V0.6.1", ""]
        {
            assert!(parse_latest(&response(tag)).is_err(), "{tag:?}");
        }
        // Surrounding whitespace is not part of the tag that reaches git.
        assert_eq!(parse_latest(&response(" v0.6.1\n")).unwrap().tag, "v0.6.1");
    }

    #[test]
    fn drafts_prereleases_and_junk_are_refused() {
        let mut draft: serde_json::Value = serde_json::from_str(&response("v0.7.0")).unwrap();
        draft["draft"] = true.into();
        assert!(parse_latest(&draft.to_string()).is_err());
        draft["draft"] = false.into();
        draft["prerelease"] = true.into();
        assert!(parse_latest(&draft.to_string()).is_err());
        for junk in ["", "null", "[]", "{}", "{\"message\":\"Not Found\"}", "{\"tag_name\": 7}", "<html>rate limited</html>"] {
            assert!(parse_latest(junk).is_err(), "{junk:?}");
        }
        assert!(parse_latest(&" ".repeat(MAX_RESPONSE_BYTES + 1)).is_err());
    }

    #[test]
    fn release_pages_off_github_are_dropped() {
        let mut v: serde_json::Value = serde_json::from_str(&response("v0.6.1")).unwrap();
        v["html_url"] = "javascript:alert(1)".into();
        assert_eq!(parse_latest(&v.to_string()).unwrap().url, None);
        v["html_url"] = "https://evil.example/storytold".into();
        assert_eq!(parse_latest(&v.to_string()).unwrap().url, None);
    }

    #[test]
    fn repositories_are_validated_before_they_reach_a_url() {
        assert_eq!(latest_release_url("storytold/gridcraft").unwrap(), "https://api.github.com/repos/storytold/gridcraft/releases/latest");
        for bad in ["", "gridcraft", "storytold/", "/gridcraft", "a/b/c", "../x", "storytold/..", "story told/x", "storytold/grid?craft", "x/y#z"] {
            assert!(validate_repository(bad).is_err(), "{bad:?}");
        }
    }
}
