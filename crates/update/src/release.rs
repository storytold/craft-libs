//! GitHub releases: reading the API's answer and `SHA256SUMS.txt`.
//!
//! Everything here is untrusted input. Sizes are bounded by the caller, download links must point
//! at the app's own release downloads, and checksums must look like checksums.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::version::{self, Version};

/// One downloadable file of a release.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    /// Bytes; 0 when unknown.
    pub size: u64,
    /// `https://github.com/<repo>/releases/download/<tag>/<name>`.
    pub url: String,
    /// Lowercase hex SHA-256, when GitHub or `SHA256SUMS.txt` told us.
    pub sha256: Option<String>,
}

/// A published release of one app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    pub version: Option<Version>,
    /// `PhotoCraft v0.5.0`
    pub name: String,
    /// Unix seconds; `None` when only the tag is known.
    pub published_at: Option<i64>,
    pub html_url: String,
    /// Release notes (Markdown), capped.
    pub notes: String,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
    /// The `owner/name` repository the release was read from, whose download links its assets
    /// must be (`install::check_download_link`). Usually the app's own; a feed that announces a
    /// renamed repository reads the release from the new one. Empty means the app's own (a
    /// release saved before this field, or one built by hand).
    #[serde(default)]
    pub repo: String,
}

impl Release {
    pub fn version_label(&self) -> String {
        self.version.as_ref().map_or_else(|| self.tag.clone(), ToString::to_string)
    }

    /// Fills in checksums from a `SHA256SUMS.txt` for assets that don't have one.
    pub fn apply_sums(&mut self, sums: &[(String, String)]) {
        for asset in &mut self.assets {
            if asset.sha256.is_none() {
                asset.sha256 = sums.iter().find(|(_, n)| *n == asset.name).map(|(h, _)| h.clone());
            }
        }
    }

    pub fn sums_asset(&self) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == "SHA256SUMS.txt")
    }
}

const MAX_NOTES: usize = 64 * 1024;

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `url` is safe to hand to the system browser from untrusted text (release notes):
/// `https` only, a host, no user-info, no spaces or control characters, and no shell metacharacters.
///
/// The system opener is not always a direct `open`: the pinned `webbrowser` crate's WSL fallback
/// passes the URL to `cmd.exe /c start` while escaping only `^` and `&`, so a character like `|`
/// in an otherwise valid URL becomes a command separator (CWE-78). Because a legitimate
/// `https://` web link never needs these characters unescaped, we refuse any URL that contains a
/// shell/cmd metacharacter rather than rely on the opener to neutralise it.
pub fn safe_web_link(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    url.len() <= 2000
        && !authority.is_empty()
        && !authority.contains(['@', '\\'])
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && !url.chars().any(is_shell_meta)
}

/// Characters a command interpreter (POSIX shells or `cmd.exe`) may treat specially. A real
/// `https` URL never needs any of them unescaped in a browser-opened link. `%` matters because
/// `cmd.exe` expands `%NAME%` references, and an expanded value could itself carry metacharacters.
pub fn is_shell_meta(c: char) -> bool {
    matches!(c, '|' | '&' | ';' | '<' | '>' | '(' | ')' | '$' | '`' | '"' | '\'' | '^' | '!' | '*' | '%' | '{' | '}' | '[' | ']')
}

/// Whether `url` is a valid release page URL strictly for `repo` on GitHub:
/// starts with `https://github.com/<repo>/releases`, contains no dot-segments (`.` or `..`),
/// no percent-encoded path separators (`%2f`, `%5c`), no query/fragment/backslash, and
/// satisfies [`safe_web_link`] (no shell metacharacters or control bytes, CWE-78).
pub fn safe_release_html_url(repo: &str, url: &str) -> Option<String> {
    if !safe_web_link(url) {
        return None;
    }
    let releases = format!("https://github.com/{repo}/releases");
    if url == releases {
        return Some(url.to_string());
    }
    let rest = url.strip_prefix(&releases)?;
    let path = rest.strip_prefix('/')?;
    if path.is_empty() || path.contains(['?', '#', '\\']) {
        return None;
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains("%2f") || seg.contains("%2F") || seg.contains("%5c") || seg.contains("%5C") {
            return None;
        }
    }
    Some(url.to_string())
}

/// The prefix every download of `repo` must start with.
pub fn download_prefix(repo: &str) -> String {
    format!("https://github.com/{repo}/releases/download/")
}

/// The one link a GitHub release asset may have: `https://github.com/<repo>/releases/download/<tag>/<name>`,
/// built from the release's own tag and the asset's own name, so a link can't point at another
/// release or anywhere else (#764). `None` unless the tag is a [`valid_tag`] and the name a
/// [`valid_file_name`]: only ASCII letters, digits and `._+-`, which github.com can't re-interpret.
/// A `+` (semver build metadata) is written `%2B`, as GitHub writes it in `browser_download_url`
/// (`.../download/17.0.9%2B9/jdk-17.0.9%2B9.deb` for the tag `17.0.9+9`).
pub fn expected_download_url(repo: &str, tag: &str, name: &str) -> Option<String> {
    let encoded = |s: &str| s.replace('+', "%2B");
    (valid_tag(tag) && valid_file_name(name)).then(|| format!("{}{}/{}", download_prefix(repo), encoded(tag), encoded(name)))
}

/// Whether `url` is `expected`, an [`expected_download_url`], with each `%2B` in either read as
/// `+`. GitHub serves a `+` in a download path the same either way, and links built before
/// `%2B` (an older launcher's feed, or a release saved by an older version) write it plainly.
/// No other escape is read, so `%2b` or `%2e` still has to match as written.
pub fn same_download_url(url: &str, expected: &str) -> bool {
    url == expected || url.replace("%2B", "+") == expected.replace("%2B", "+")
}

/// Whether `url` is a download that truly belongs to `repo`.
///
/// A raw `starts_with(prefix)` check is not enough: a value such as
/// `https://github.com/storytold/craft-launcher/releases/download/../../../ATTACKER/REPO/releases/download/v1/x`
/// shares the prefix textually, yet its interpreted path escapes the repository. The link is
/// accepted only when the text after the prefix is exactly `<tag>/<name>` — two non-empty path
/// segments with no traversal (`.`/`..`), no additional slashes, no query or fragment, and no
/// backslash or percent-encoded separators that a client could later re-interpret.
pub fn download_url_under(url: &str, prefix: &str) -> bool {
    let Some(rest) = url.strip_prefix(prefix) else { return false };
    download_suffix_ok(rest)
}

/// Whether `url` sits under a developer mirror `base` (ending in `/`) without escaping it.
pub fn download_url_under_mirror(url: &str, base: &str) -> bool {
    let Some(rest) = url.strip_prefix(base) else { return false };
    // A mirror serves `<tag>/<name>` just like GitHub, but we only require that it can't climb out
    // of its own folder; its layout is otherwise the developer's business.
    !rest.is_empty() && !url.contains(['?', '#', '\\']) && !contains_traversal(rest) && !contains_encoded_separator(rest)
}

/// `<tag>/<name>`: exactly two non-empty segments, no traversal, no re-interpretable separators.
fn download_suffix_ok(rest: &str) -> bool {
    if rest.contains(['?', '#', '\\']) || contains_encoded_separator(rest) {
        return false;
    }
    let mut parts = rest.split('/');
    let (Some(tag), Some(name), None) = (parts.next(), parts.next(), parts.next()) else { return false };
    valid_tag(tag) && valid_file_name(name)
}

/// Any `.` or `..` path segment that could climb out of the repository.
fn contains_traversal(rest: &str) -> bool {
    rest.split('/').any(|seg| seg == "." || seg == "..")
}

/// Percent-encoded slash, backslash or dot (`%2f`, `%5c`, `%2e`): a client or github.com could
/// decode these back into separators or a `..` segment after the check (#764), so a confined URL
/// must not contain them.
fn contains_encoded_separator(rest: &str) -> bool {
    let bytes = rest.as_bytes();
    bytes.windows(3).any(|w| w[0] == b'%' && matches!(&w[1..].to_ascii_lowercase()[..], b"2f" | b"5c" | b"2e"))
}

/// Also accepts a repository-qualified release tag (`artcraft-v0.42.1`). The prefix must
/// match this repository exactly; an unrelated app's tag is never treated as a release version.
fn tag_version(tag: &str, repo: &str) -> Option<version::Version> {
    version::parse(tag).or_else(|| {
        let name = repo.rsplit('/').next()?;
        let suffix = tag.strip_prefix(name)?.strip_prefix('-')?;
        version::parse(suffix)
    })
}

/// Reads `GET /repos/<repo>/releases/latest` (or one element of `/releases`).
pub fn parse_github_release(json: &str, repo: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("unreadable answer from GitHub ({e})"))?;
    release_from_value(&v, repo)
}

/// Reads `GET /repos/<repo>/releases` and returns the newest stable (or any, with
/// `allow_prerelease`) published release.
pub fn parse_github_release_list(json: &str, repo: &str, allow_prerelease: bool) -> Result<Option<Release>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("unreadable answer from GitHub ({e})"))?;
    let list = v.as_array().ok_or("unexpected answer from GitHub")?;
    let mut best: Option<Release> = None;
    for item in list {
        if item["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let Ok(r) = release_from_value(item, repo) else { continue };
        let Some(ver) = r.version.as_ref() else { continue };
        if !allow_prerelease && (r.prerelease || !version::is_stable(ver)) {
            continue;
        }
        let newer = best.as_ref().and_then(|b| b.version.as_ref()).is_none_or(|b| version::is_newer(ver, b));
        if newer {
            best = Some(r);
        }
    }
    Ok(best)
}

/// One release object of the GitHub API, with every text capped and only the assets whose name is
/// a plain file name and whose link is exactly [`expected_download_url`]. An API error object
/// (`message`) becomes its text.
fn release_from_value(v: &serde_json::Value, repo: &str) -> Result<Release, String> {
    if let Some(msg) = v["message"].as_str() {
        return Err(if msg == "Not Found" { "no releases yet".into() } else { format!("GitHub says: {}", truncate(msg, 200)) });
    }
    let tag = v["tag_name"].as_str().filter(|t| valid_tag(t)).ok_or("no release found")?.to_string();
    let releases = format!("https://github.com/{repo}/releases");
    let html_url = v["html_url"].as_str().and_then(|u| safe_release_html_url(repo, u)).unwrap_or_else(|| releases.clone());
    let assets = v["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let name = a["name"].as_str().filter(|n| valid_file_name(n))?.to_string();
                    let url =
                        a["browser_download_url"].as_str().filter(|u| expected_download_url(repo, &tag, &name).as_deref() == Some(*u))?.to_string();
                    let sha256 = a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).filter(|h| is_hex64(h)).map(str::to_ascii_lowercase);
                    Some(Asset { name, size: a["size"].as_u64().unwrap_or(0), url, sha256 })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version: tag_version(&tag, repo),
        name: v["name"].as_str().map(|n| truncate(n, 120)).filter(|n| !n.is_empty()).unwrap_or_else(|| tag.clone()),
        tag,
        published_at: v["published_at"].as_str().and_then(crate::time::parse_rfc3339),
        html_url,
        notes: v["body"].as_str().map(|b| truncate(b, MAX_NOTES)).unwrap_or_default(),
        prerelease: v["prerelease"].as_bool().unwrap_or(false),
        assets,
        repo: repo.to_string(),
    })
}

/// A release known only from its tag and `SHA256SUMS.txt` (the quota-free route: the
/// `releases/latest` redirect names the tag, and the sums list every file). Each link is
/// [`expected_download_url`], so it's the one the check before the download accepts; a file
/// without one is left out.
pub fn release_from_sums(repo: &str, tag: &str, sums: &[(String, String)]) -> Release {
    let asset =
        |name: &str, sha256: Option<String>| expected_download_url(repo, tag, name).map(|url| Asset { name: name.to_string(), size: 0, url, sha256 });
    let assets: Vec<Asset> = sums.iter().filter_map(|(hash, name)| asset(name, Some(hash.clone()))).chain(asset("SHA256SUMS.txt", None)).collect();
    Release {
        tag: tag.to_string(),
        version: tag_version(tag, repo),
        name: tag.to_string(),
        published_at: None,
        html_url: format!("https://github.com/{repo}/releases/tag/{tag}"),
        notes: String::new(),
        prerelease: false,
        assets,
        repo: repo.to_string(),
    }
}

/// The tag in a `https://github.com/<repo>/releases/tag/<tag>` redirect target.
pub fn tag_from_redirect(location: &str, repo: &str) -> Option<String> {
    let rest = location.strip_prefix(&format!("https://github.com/{repo}/releases/tag/"))?;
    valid_tag(rest).then(|| rest.to_string())
}

/// A release tag we'd put in a URL, a file or the UI: up to 64 of `[A-Za-z0-9._+-]`, not
/// starting with a dot, so no separators, control or bidi characters and no `.`/`..`.
pub fn valid_tag(tag: &str) -> bool {
    tag.len() <= 64 && !tag.starts_with('.') && plain_name(tag)
}

/// Non-empty and only `[A-Za-z0-9._+-]`, the characters every Craft release tag and asset uses.
fn plain_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

/// Reads `sha256sum` output: `<hex>  <name>` (or `<hex> *<name>`) per line, one entry per name.
/// A line repeated as is counts once; a name listed with two different hashes is dropped,
/// whichever line comes first, so the list vouches for neither (a file nothing else vouches for
/// isn't installed).
pub fn parse_sha256sums(text: &str) -> Vec<(String, String)> {
    let mut sums: Vec<(String, String)> = Vec::new();
    // Where each name is in `sums`: a 4 MiB list holds some 60,000 names, too many to search.
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut conflicting: HashSet<&str> = HashSet::new();
    for line in text.lines() {
        let Some((hash, rest)) = line.trim().split_once(char::is_whitespace) else { continue };
        let name = rest.trim_start().trim_start_matches('*');
        if !is_hex64(hash) || !valid_file_name(name) {
            continue;
        }
        let hash = hash.to_ascii_lowercase();
        match index.get(name) {
            None => {
                index.insert(name, sums.len());
                sums.push((hash, name.to_string()));
            }
            Some(&i) => {
                if sums.get(i).is_some_and(|(h, _)| *h != hash) && conflicting.insert(name) {
                    log::warn!("SHA256SUMS.txt lists {name} with different checksums; ignoring it");
                }
            }
        }
    }
    sums.retain(|(_, n)| !conflicting.contains(n.as_str()));
    sums
}

/// A bare file name we'd be willing to save: only `[A-Za-z0-9._+-]` (so no path, escape, control
/// or bidi character), no leading or trailing dot, and not a name Windows reserves for a device
/// (`CON`, `nul.txt`, `COM1.msi`).
pub fn valid_file_name(name: &str) -> bool {
    name.len() <= 200 && !name.starts_with('.') && !name.ends_with('.') && plain_name(name) && !windows_device(name)
}

/// Whether `name`'s part before the first dot is a Windows device name, which opens the device
/// instead of a file in any folder.
fn windows_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(stem.as_bytes(), [b'C', b'O', b'M', b'1'..=b'9'] | [b'L', b'P', b'T', b'1'..=b'9'])
}

/// `s` cut to at most `max` bytes (on a char boundary) with an ellipsis, for text from a
/// release or the feed shown in the UI.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}…", &s[..s.floor_char_boundary(max)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "storytold/photocraft";

    #[test]
    fn repository_prefixed_tags_are_ordered_without_changing_download_links() {
        let repo = "storytold/artcraft";
        let json = r#"[{"tag_name":"artcraft-v0.41.1","draft":false,"prerelease":false,"assets":[]},
            {"tag_name":"artcraft-v0.42.1","draft":false,"prerelease":false,"assets":[]},
            {"tag_name":"other-app-v99.0.0","draft":false,"prerelease":false,"assets":[]},
            {"tag_name":"artcraft-v99.0.0","draft":true,"prerelease":false,"assets":[]}]"#;
        let release = parse_github_release_list(json, repo, false).expect("valid releases").expect("published release");
        assert_eq!(release.tag, "artcraft-v0.42.1");
        assert_eq!(release.version, version::parse("0.42.1"));
        let hash = "a".repeat(64);
        let sums = release_from_sums(repo, &release.tag, &[(hash, "artcraft-0.42.1-linux-x86_64.flatpak".into())]);
        assert_eq!(sums.version, release.version);
        assert_eq!(
            sums.assets[0].url,
            "https://github.com/storytold/artcraft/releases/download/artcraft-v0.42.1/artcraft-0.42.1-linux-x86_64.flatpak"
        );
        assert!(parse_github_release(r#"{"tag_name":"other-v0.42.1","assets":[]}"#, repo).expect("valid JSON").version.is_none());
    }

    #[test]
    fn truncate_keeps_whole_characters() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdef", 3), "abc…");
        // `é` is two bytes: cutting at 2 would split it, so the cut backs off to 1.
        assert_eq!(truncate("aé", 2), "a…");
    }

    #[test]
    fn only_plain_https_links_pass() {
        assert!(safe_web_link("https://github.com/storytold/photocraft/releases/tag/v1"));
        for bad in [
            "http://example.com/",
            "javascript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "ms-msdt:/id",
            "https://",
            "https://user@evil.example/",
            "https://example.com/a b",
            "https://example.com/\nx",
            "https:///path",
            // Shell/cmd metacharacters: the WSL opener fallback shells out, so these must be
            // refused even inside an otherwise valid https URL (CWE-78).
            "https://example.com/a|calc",
            "https://example.com/p?x=1&y=2",
            "https://example.com/`id`",
            "https://example.com/$(whoami)",
            "https://example.com/a;b",
            "https://example.com/a>b",
            "https://example.com/%COMSPEC%",
            "https://example.com/%PATH%",
        ] {
            assert!(!safe_web_link(bad), "{bad:?}");
        }
    }
    const HASH: &str = "8a5edab282632443219e051e4ade2d1d5bbc671c781051bf1437897cbdfea0f1";

    fn api_json(url: &str) -> String {
        format!(
            r#"{{"tag_name":"v0.5.0","name":"PhotoCraft v0.5.0","html_url":"https://github.com/storytold/photocraft/releases/tag/v0.5.0",
            "published_at":"2026-10-08T14:52:50Z","prerelease":false,"body":"Notes",
            "assets":[{{"name":"photocraft-0.5.0-macos-universal.dmg","size":123,"browser_download_url":"{url}","digest":"sha256:{HASH}"}}]}}"#
        )
    }

    #[test]
    fn reads_the_latest_release() {
        let r = parse_github_release(
            &api_json("https://github.com/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-macos-universal.dmg"),
            REPO,
        )
        .unwrap();
        assert_eq!(r.version, Some(Version::new(0, 5, 0)));
        assert_eq!(r.name, "PhotoCraft v0.5.0");
        assert_eq!(r.published_at, Some(1_791_471_170));
        assert_eq!(r.assets.len(), 1);
        assert_eq!(r.assets[0].sha256.as_deref(), Some(HASH));
        assert_eq!(r.assets[0].size, 123);
    }

    /// #764: an API asset's link must be exactly this release's `<tag>/<name>`.
    #[test]
    fn a_download_link_must_be_its_releases_own() {
        for url in [
            // Another release of the same app (a rollback), or another file name.
            "https://github.com/storytold/photocraft/releases/download/v0.4.0/photocraft-0.5.0-macos-universal.dmg",
            "https://github.com/storytold/photocraft/releases/download/v0.5.0/photocraft-0.4.0-macos-universal.dmg",
            "https://github.com/storytold/photocraft/releases/download/%2e%2e/%2E%2E/%2e%2e/%2e%2e/other/repo/releases/download/v1/photocraft-0.5.0-macos-universal.dmg",
            // The right path on another host, over plain http, or behind a look-alike host.
            "https://evil.example/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-macos-universal.dmg",
            "http://github.com/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-macos-universal.dmg",
            "https://github.com.evil.example/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-macos-universal.dmg",
            "https://github.com@evil.example/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-macos-universal.dmg",
        ] {
            assert!(parse_github_release(&api_json(url), REPO).unwrap().assets.is_empty(), "{url}");
        }
        assert_eq!(
            expected_download_url(REPO, "v0.6.0-rc.1", "photocraft-0.6.0-rc.1-macos-universal.dmg").as_deref(),
            Some("https://github.com/storytold/photocraft/releases/download/v0.6.0-rc.1/photocraft-0.6.0-rc.1-macos-universal.dmg")
        );
        for (tag, name) in [("", "x.dmg"), ("..", "x.dmg"), ("v1", ".."), ("v1", "a/b.dmg"), ("v1", "a%2fb.dmg"), ("v 1", "x.dmg"), ("v1", ".x.dmg")]
        {
            assert_eq!(expected_download_url(REPO, tag, name), None, "{tag:?} {name:?}");
        }
    }

    /// Two entries for one asset name, one with the release's own link and one elsewhere: only the
    /// release's own survives, in either order, so a later lookup by name can't pick the other.
    /// Built from `SHA256SUMS.txt`, every entry for a repeated name gets that same single link.
    #[test]
    fn two_links_for_one_asset_keep_only_the_releases_own() {
        let name = "photocraft-0.5.0-macos-universal.dmg";
        let own = format!("https://github.com/storytold/photocraft/releases/download/v0.5.0/{name}");
        let asset = |url: &str| format!(r#"{{"name":"{name}","size":123,"browser_download_url":"{url}","digest":"sha256:{HASH}"}}"#);
        let elsewhere = "https://evil.example/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-macos-universal.dmg";
        for (first, second) in [(elsewhere, own.as_str()), (own.as_str(), elsewhere)] {
            let json = format!(
                r#"{{"tag_name":"v0.5.0","name":"PhotoCraft v0.5.0","html_url":"https://github.com/storytold/photocraft/releases/tag/v0.5.0",
                "published_at":"2026-10-08T14:52:50Z","prerelease":false,"body":"Notes","assets":[{},{}]}}"#,
                asset(first),
                asset(second)
            );
            let r = parse_github_release(&json, REPO).unwrap();
            assert_eq!(r.assets.len(), 1, "{first} {second}");
            assert_eq!(r.assets[0].url, own);
        }

        let r = release_from_sums(REPO, "v0.5.0", &[("a".repeat(64), name.to_string()), ("b".repeat(64), name.to_string())]);
        let links: Vec<&str> = r.assets.iter().filter(|a| a.name == name).map(|a| a.url.as_str()).collect();
        assert!(!links.is_empty() && links.iter().all(|url| *url == own), "{links:?}");
    }

    /// A `+` in the tag or name (semver build metadata) is `%2B` in GitHub's download links, but a
    /// plain `+` in the asset's name and in the `releases/latest` redirect, as on bell-sw/Liberica's
    /// releases. The API's assets, and those built from `SHA256SUMS.txt`, get the same link.
    #[test]
    fn a_plus_in_the_tag_is_percent_encoded_in_the_link() {
        let name = "photocraft-1.0.0+build.1-linux-x86_64.AppImage";
        let github = "https://github.com/storytold/photocraft/releases/download/v1.0.0%2Bbuild.1/photocraft-1.0.0%2Bbuild.1-linux-x86_64.AppImage";
        let api = |url: &str| format!(r#"{{"tag_name":"v1.0.0+build.1","assets":[{{"name":"{name}","size":1,"browser_download_url":"{url}"}}]}}"#);
        let r = parse_github_release(&api(github), REPO).unwrap();
        assert_eq!((r.assets[0].name.as_str(), r.assets[0].url.as_str()), (name, github));
        // GitHub never writes the `+` itself into the link.
        assert!(parse_github_release(&api(&github.replace("%2B", "+")), REPO).unwrap().assets.is_empty());

        let tag = tag_from_redirect("https://github.com/storytold/photocraft/releases/tag/v1.0.0+build.1", REPO).unwrap();
        let r = release_from_sums(REPO, &tag, &[(HASH.to_string(), name.to_string())]);
        assert_eq!(r.assets[0].url, github);
        assert_eq!(r.sums_asset().unwrap().url, "https://github.com/storytold/photocraft/releases/download/v1.0.0%2Bbuild.1/SHA256SUMS.txt");
    }

    /// Links to other hosts or repositories, or that climb out with `..` or encoded separators,
    /// are dropped.
    #[test]
    fn drops_downloads_that_point_elsewhere() {
        for url in ["https://evil.example/photocraft.dmg", "https://github.com/storytold/other/releases/download/v1/x.dmg", "javascript:alert(1)"] {
            let r = parse_github_release(&api_json(url), REPO).unwrap();
            assert!(r.assets.is_empty(), "{url}");
        }
        assert_eq!(parse_github_release(r#"{"message":"Not Found"}"#, REPO).unwrap_err(), "no releases yet");
        assert!(parse_github_release("<html>", REPO).is_err());
    }

    #[test]
    fn release_html_url_rejects_traversal_and_metacharacters() {
        let repo = "storytold/photocraft";
        assert_eq!(
            safe_release_html_url(repo, "https://github.com/storytold/photocraft/releases/tag/v1.0.0"),
            Some("https://github.com/storytold/photocraft/releases/tag/v1.0.0".to_string())
        );
        assert_eq!(
            safe_release_html_url(repo, "https://github.com/storytold/photocraft/releases"),
            Some("https://github.com/storytold/photocraft/releases".to_string())
        );
        for bad in [
            "https://github.com/storytold/photocraft/releases/../../attacker/repo/releases/tag/v1",
            "https://github.com/storytold/photocraft/releases/tag/v1/../../../attacker/repo",
            "https://github.com/storytold/photocraft/releases/tag/v1|calc.exe",
            "https://github.com/storytold/photocraft/releases/tag/v1%2f..%2f..",
            "https://github.com/storytold/photocraft/releases/tag/v1?x=1",
            "https://github.com/storytold/photocraft/releases/tag/v1#hash",
            "https://github.com/storytold/other/releases/tag/v1.0.0",
            "http://github.com/storytold/photocraft/releases/tag/v1",
        ] {
            assert!(safe_release_html_url(repo, bad).is_none(), "should reject: {bad}");
        }
    }

    /// A link that shares the download prefix as text but climbs into another repository with `..`
    /// or an encoded separator or dot isn't under it.
    #[test]
    fn traversal_cannot_escape_the_repository() {
        // Raw-prefix matches but the interpreted path climbs into another repository (CWE-180).
        for url in [
            "https://github.com/storytold/photocraft/releases/download/../../../../attacker/repo/releases/download/v99/photocraft-99.0.0-linux-x86_64.AppImage",
            "https://github.com/storytold/photocraft/releases/download/v1/../../../attacker/repo/x.dmg",
            "https://github.com/storytold/photocraft/releases/download/./v1/x.dmg",
            "https://github.com/storytold/photocraft/releases/download/v1/sub/x.dmg",
            "https://github.com/storytold/photocraft/releases/download/v1%2f..%2f..%2fx.dmg",
            "https://github.com/storytold/photocraft/releases/download/v1/x%2fy.dmg",
            "https://github.com/storytold/photocraft/releases/download/%2e%2e/x.dmg",
            "https://github.com/storytold/photocraft/releases/download/%2E%2E/x.dmg",
            "https://github.com/storytold/photocraft/releases/download/v1/%2e%2e",
        ] {
            let prefix = download_prefix(REPO);
            assert!(!download_url_under(url, &prefix), "{url}");
            let r = parse_github_release(&api_json(url), REPO).unwrap();
            assert!(r.assets.is_empty(), "{url}");
        }
        // A plain, confined download still passes.
        let prefix = download_prefix(REPO);
        assert!(download_url_under("https://github.com/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-macos-universal.dmg", &prefix));
    }

    #[test]
    fn list_picks_the_newest_stable_release() {
        let json = r#"[
            {"tag_name":"v0.6.0-rc.1","prerelease":false,"draft":false,"assets":[]},
            {"tag_name":"v0.5.1","prerelease":true,"draft":false,"assets":[]},
            {"tag_name":"v0.7.0","draft":true,"assets":[]},
            {"tag_name":"v0.5.0","prerelease":false,"draft":false,"assets":[]},
            {"tag_name":"v0.4.0","prerelease":false,"draft":false,"assets":[]}]"#;
        assert_eq!(parse_github_release_list(json, REPO, false).unwrap().unwrap().tag, "v0.5.0");
        assert_eq!(parse_github_release_list(json, REPO, true).unwrap().unwrap().tag, "v0.6.0-rc.1");
        assert_eq!(parse_github_release_list("[]", REPO, false).unwrap(), None);
    }

    /// `SHA256SUMS.txt` lines, the tag in a `releases/latest` redirect, and the release built from
    /// both; malformed lines, traversing names and `.`/`..` tags are dropped.
    #[test]
    fn reads_checksums_and_redirects() {
        let sums = parse_sha256sums(&format!("{HASH}  photocraft-0.5.0-macos-universal.dmg\n{HASH} *b.zip\nnot a line\n{HASH}  ../escape\n"));
        assert_eq!(sums, [(HASH.to_string(), "photocraft-0.5.0-macos-universal.dmg".to_string()), (HASH.to_string(), "b.zip".to_string())]);
        assert_eq!(tag_from_redirect("https://github.com/storytold/photocraft/releases/tag/v0.5.0", REPO).as_deref(), Some("v0.5.0"));
        assert_eq!(tag_from_redirect("https://github.com/storytold/photocraft/releases", REPO), None);
        assert_eq!(tag_from_redirect("https://evil/storytold/photocraft/releases/tag/v1", REPO), None);
        for dots in [".", ".."] {
            assert_eq!(tag_from_redirect(&format!("https://github.com/storytold/photocraft/releases/tag/{dots}"), REPO), None);
        }
        let r = release_from_sums(REPO, "v0.5.0", &sums);
        assert_eq!(r.assets[0].url, "https://github.com/storytold/photocraft/releases/download/v0.5.0/photocraft-0.5.0-macos-universal.dmg");
        assert_eq!(r.version, Some(Version::new(0, 5, 0)));
    }

    #[test]
    fn a_name_listed_twice_gives_one_asset_or_none() {
        let other = "b".repeat(64);
        // The same line twice (and the same hash in upper case) is one entry.
        let sums = parse_sha256sums(&format!("{HASH}  a.zip\n{HASH} *a.zip\n{}  a.zip\n", HASH.to_ascii_uppercase()));
        assert_eq!(sums, [(HASH.to_string(), "a.zip".to_string())]);
        let r = release_from_sums(REPO, "v0.5.0", &sums);
        assert_eq!(r.assets.iter().filter(|a| a.name == "a.zip").count(), 1);
        // Two hashes for one name: the name is dropped in either line order, and the other
        // files keep theirs.
        for text in [format!("{HASH}  a.zip\n{other}  a.zip\n{HASH}  b.zip\n"), format!("{other}  a.zip\n{HASH}  b.zip\n{HASH}  a.zip\n")] {
            let sums = parse_sha256sums(&text);
            assert_eq!(sums, [(HASH.to_string(), "b.zip".to_string())], "{text}");
            let r = release_from_sums(REPO, "v0.5.0", &sums);
            assert!(r.assets.iter().all(|a| a.name != "a.zip"), "{text}");
        }
        // A third, matching line doesn't bring the name back.
        assert!(parse_sha256sums(&format!("{HASH}  a.zip\n{other}  a.zip\n{HASH}  a.zip\n")).is_empty());
    }

    #[test]
    fn a_full_size_list_keeps_its_order_and_finds_a_conflict_across_it() {
        // About as many names as fit in the 4 MiB a list may be, and a conflict between the
        // first and last lines.
        let other = "b".repeat(64);
        let mut text: String = (0..57_000).map(|i| format!("{HASH}  f{i}\n")).collect();
        text.push_str(&format!("{other}  f0\n"));
        assert!(text.len() <= 4 << 20);
        let sums = parse_sha256sums(&text);
        assert_eq!(sums.len(), 56_999);
        assert!(sums.iter().enumerate().all(|(i, (h, n))| *h == HASH && *n == format!("f{}", i + 1)));
    }

    /// A name with a path, a hidden or reserved form, a control character or a percent-encoded
    /// separator isn't a file name we'd save.
    #[test]
    fn file_names_cannot_escape() {
        for bad in [
            "",
            "..",
            "a/b",
            "a\\b",
            ".hidden",
            "c:x",
            "x\u{0}",
            "%2e%2e%2fother",
            "a%2Fb",
            "a%5cb",
            "x%2E%2E",
            "a b.dmg",
            "a#b",
            "evil\u{202e}gpj.exe",
            "x.msi.",
            "CON",
            "nul.txt",
            "COM1.msi",
            "lpt9",
            "prn",
            "aux.log",
            "a".repeat(201).as_str(),
        ] {
            assert!(!valid_file_name(bad), "{bad:?}");
        }
        for good in [
            "photocraft-0.5.0-linux-x86_64.AppImage",
            "lightcraft_0.9.0_amd64.deb",
            "x-1.0+git.tar.gz",
            "console.zip",
            "COM10.txt",
            "COM0.msi",
            "lpt0",
        ] {
            assert!(valid_file_name(good), "{good:?}");
        }
    }

    /// The tag rule the launcher's feed shares (craft-launcher#1024): plain characters, no
    /// leading dot, at most 64 bytes, for the API's `tag_name`, a redirect and a download link.
    #[test]
    fn tags_are_plain() {
        for good in ["v0.6.0", "v1.0.0-beta.2", "nightly-2026-10-29", "v1+build.7"] {
            assert!(valid_tag(good), "{good:?}");
        }
        for bad in ["", ".", "..", ".v1", "v1/x", "v1\\x", "v1\nx", "v1\u{0}x", "v1\u{202e}", "v 1", "v1?x", "v".repeat(65).as_str()] {
            assert!(!valid_tag(bad), "{bad:?}");
        }
        let api = r#"{"tag_name": "v1\u202egpj", "html_url": "", "assets": []}"#;
        assert!(parse_github_release(api, REPO).is_err());
        let prefix = download_prefix(REPO);
        assert!(download_url_under(&format!("{prefix}v1/a.dmg"), &prefix));
        assert!(!download_url_under(&format!("{prefix}v1\u{202e}/a.dmg"), &prefix), "an odd tag in a download link");
        assert!(!download_url_under(&format!("{prefix}v1/a\u{202e}.dmg"), &prefix), "an odd name in a download link");
        assert_eq!(tag_from_redirect(&format!("https://github.com/{REPO}/releases/tag/v1\u{202e}"), REPO), None);
    }

    /// A `+` matches written plainly or as `%2B`, either side; no other escape is read.
    #[test]
    fn a_plus_link_matches_either_way_and_nothing_else_does() {
        let expected = expected_download_url(REPO, "v1.0.0+b.1", "photocraft-1.0.0+b.1-linux-x86_64.AppImage").unwrap();
        assert!(same_download_url(&expected, &expected));
        assert!(same_download_url(&expected.replace("%2B", "+"), &expected));
        assert!(!same_download_url(&expected.replace("%2B", "%2b"), &expected));
        assert!(!same_download_url(&expected.replace("v1.0.0", "v1.0.1"), &expected));
    }

    /// Both converters record the repository they read the release from.
    #[test]
    fn a_release_knows_its_repository() {
        assert_eq!(release_from_sums(REPO, "v0.5.0", &[]).repo, REPO);
        let api = r#"{"tag_name": "v0.5.0", "assets": []}"#;
        assert_eq!(parse_github_release(api, REPO).unwrap().repo, REPO);
    }

    /// A `SHA256SUMS.txt` line whose name hides a traversal is dropped, so `release_from_sums`
    /// never builds a link from it; the ordinary line beside it is kept.
    #[test]
    fn checksum_names_with_encoded_separators_make_no_download() {
        let hash = "a".repeat(64);
        let sums = parse_sha256sums(&format!("{hash}  %2e%2e%2f%2e%2e%2fother%2frepo%2fx.dmg\n{hash}  photocraft-0.5.0-macos-universal.dmg\n"));
        assert_eq!(sums.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["photocraft-0.5.0-macos-universal.dmg"]);
    }
}
