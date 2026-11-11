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
    /// File name.
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
    /// The git tag.
    pub tag: String,
    /// The version parsed from the tag; `None` when the tag is not a version.
    pub version: Option<Version>,
    /// `PhotoCraft v0.5.0`
    pub name: String,
    /// Unix seconds; `None` when only the tag is known.
    pub published_at: Option<i64>,
    /// Web page of the release.
    pub html_url: String,
    /// Release notes (Markdown), capped.
    pub notes: String,
    #[allow(missing_docs)]
    pub prerelease: bool,
    #[allow(missing_docs)]
    pub assets: Vec<Asset>,
    /// The `owner/name` repository the release was read from, whose download links its assets
    /// must be (`install::check_download_link`). Usually the app's own; a feed that announces a
    /// renamed repository reads the release from the new one. Empty means the app's own (a
    /// release saved before this field, or one built by hand).
    #[serde(default)]
    pub repo: String,
}

impl Release {
    /// The version as text, or the tag when it is not a version.
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

    /// The `SHA256SUMS.txt` asset, if the release has one.
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
    use craft_ui::testing::trial;

    const REPO: &str = "storytold/photocraft";

    #[trial]
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

    #[trial]
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

    /// #764: an API asset's link must be exactly this release's `<tag>/<name>`.
    #[trial]
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

    /// A link that shares the download prefix as text but climbs into another repository with `..`
    /// or an encoded separator or dot isn't under it.
    #[trial]
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

    /// A name with a path, a hidden or reserved form, a control character or a percent-encoded
    /// separator isn't a file name we'd save.
    #[trial]
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
}
