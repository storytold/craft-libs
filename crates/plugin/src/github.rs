//! Installing straight from a GitHub repository: `owner/repo` or `owner/repo@tag` as a person
//! types it, the releases API URL to read, the `.craftplugin` asset of the release GitHub
//! answers with, and its SHA-256 from a `SHA256SUMS` asset of the same release (or GitHub's own
//! `digest`), checked against the downloaded bytes.
//!
//! Pure functions: nothing here touches the network. [`crate::fetch`] strings them together over an
//! app's HTTP client. Everything GitHub answers is untrusted: texts are bounded, and an asset's
//! download link must be exactly the one its repository, tag and name make, so an answer can't
//! point a download anywhere else.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::bundle::{BUNDLE_EXTENSION, MAX_BUNDLE_BYTES, sha256_hex};

/// Largest releases API answer read (a release with many assets is a few hundred KB).
pub const MAX_API_BYTES: usize = 4 << 20;
/// Largest `SHA256SUMS` asset read.
pub const MAX_SUMS_BYTES: usize = 1 << 20;
/// Names a checksum list may have in a release, the preferred first.
pub const SUMS_NAMES: [&str; 2] = ["SHA256SUMS", "SHA256SUMS.txt"];
const MAX_TAG: usize = 64;
const MAX_REPO_PART: usize = 100;
const MAX_FILE_NAME: usize = 200;

/// A repository, and the release a person asked for (`None`: the latest).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
    pub tag: Option<String>,
}

impl RepoRef {
    /// `owner/name`.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// `https://github.com/owner/name`.
    pub fn url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    /// The releases API resource to read: the release of [`RepoRef::tag`], else the latest one
    /// (GitHub's latest is never a draft or a pre-release).
    pub fn releases_api_url(&self) -> String {
        let base = format!("https://api.github.com/repos/{}/{}/releases", self.owner, self.name);
        match &self.tag {
            Some(tag) => format!("{base}/tags/{}", tag.replace('+', "%2B")),
            None => format!("{base}/latest"),
        }
    }

    /// Whether `other` is the same repository (GitHub ignores case in owner and repository names).
    pub fn same_repo(&self, owner: &str, name: &str) -> bool {
        self.owner.eq_ignore_ascii_case(owner) && self.name.eq_ignore_ascii_case(name)
    }
}

/// Read what a person typed: `owner/repo`, `owner/repo@tag`, or the repository's URL
/// (`https://github.com/owner/repo`, optionally ending in `/` or `.git`, then optionally `@tag`).
pub fn parse_repo_spec(spec: &str) -> Result<RepoRef, String> {
    let spec = spec.trim();
    let shown: String = spec.chars().take(80).collect();
    let bad = || format!("`{shown}` isn't a GitHub repository: type owner/repo, or owner/repo@tag for one release");
    if spec.len() > 400 {
        return Err(bad());
    }
    let (repo, tag) = match spec.rsplit_once('@') {
        Some((repo, tag)) => (repo, Some(tag)),
        None => (spec, None),
    };
    if let Some(tag) = tag
        && !valid_tag(tag)
    {
        return Err(format!("`{}` isn't a release tag (letters, digits, `.`, `_`, `+` or `-`)", tag.chars().take(80).collect::<String>()));
    }
    let (owner, name) = match strip_prefix_ignore_case(repo, "https://github.com/") {
        Some(_) => parse_repo_url(repo).filter(|_| !repo.contains("/tree/")).ok_or_else(bad)?,
        None => {
            let (owner, name) = repo.split_once('/').ok_or_else(bad)?;
            let name = name.strip_suffix(".git").unwrap_or(name);
            if !valid_owner(owner) || !valid_repo_name(name) {
                return Err(bad());
            }
            (owner.to_string(), name.to_string())
        }
    };
    Ok(RepoRef { owner, name, tag: tag.map(str::to_string) })
}

/// `(owner, name)` of a `https://github.com/<owner>/<name>` repository URL (an optional trailing
/// `/` or `.git`) or of a folder in one, `https://github.com/<owner>/<name>/tree/<ref>/<path>`,
/// as written; `None` for anything else.
pub fn parse_repo_url(url: &str) -> Option<(String, String)> {
    let rest = strip_prefix_ignore_case(url.trim(), "https://github.com/")?;
    if rest.contains(bad_url_char) {
        return None;
    }
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    let (owner, name) = match parts.as_slice() {
        [owner, name] => (*owner, name.strip_suffix(".git").unwrap_or(name)),
        [owner, name, "tree", folder @ ..] if !folder.is_empty() && folder.iter().all(|p| !p.is_empty() && !dot_segment(p)) => (*owner, *name),
        _ => return None,
    };
    (valid_owner(owner) && valid_repo_name(name)).then(|| (owner.to_string(), name.to_string()))
}

/// A GitHub owner: letters, digits and `-`, not starting or ending with `-`.
fn valid_owner(p: &str) -> bool {
    !p.is_empty() && p.len() <= MAX_REPO_PART && !p.starts_with('-') && !p.ends_with('-') && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A repository name: letters, digits, `.`, `-` and `_`, and not `.` or `..`.
fn valid_repo_name(p: &str) -> bool {
    !p.is_empty() && p.len() <= MAX_REPO_PART && p != "." && p != ".." && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// A release tag that can go into a URL, a file or the UI: up to 64 of `[A-Za-z0-9._+-]`, not
/// starting with a dot (no separators, control or bidi characters, no `.` or `..`).
pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= MAX_TAG && !tag.starts_with('.') && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// An asset name that is a plain file name everywhere: up to 200 of `[A-Za-z0-9._+-]`, not
/// starting or ending with a dot, and not a Windows device name.
pub fn valid_file_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(stem.as_bytes(), [b'C', b'O', b'M', b'1'..=b'9'] | [b'L', b'P', b'T', b'1'..=b'9']);
    !name.is_empty()
        && name.len() <= MAX_FILE_NAME
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        && !device
}

/// The one link a release asset of `owner/name` may have:
/// `https://github.com/<owner>/<name>/releases/download/<tag>/<file>`, with `+` written `%2B`
/// as GitHub writes it. `None` unless the tag and the file name are plain.
pub fn expected_download_url(owner: &str, name: &str, tag: &str, file: &str) -> Option<String> {
    let enc = |s: &str| s.replace('+', "%2B");
    (valid_owner(owner) && valid_repo_name(name) && valid_tag(tag) && valid_file_name(file))
        .then(|| format!("https://github.com/{owner}/{name}/releases/download/{}/{}", enc(tag), enc(file)))
}

/// Whether `url` is the download link of asset `file` of release `tag` of `repo`: exactly
/// [`expected_download_url`], except that the owner and repository may differ in case (GitHub
/// writes them as they are named, a person may have typed them otherwise) and a `+` may be
/// written as is.
pub fn is_asset_url(url: &str, repo: &RepoRef, tag: &str, file: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://github.com/") else { return false };
    let mut parts = rest.splitn(3, '/');
    let (Some(owner), Some(name), Some(path)) = (parts.next(), parts.next(), parts.next()) else { return false };
    let want = format!("releases/download/{tag}/{file}");
    repo.same_repo(owner, name) && valid_tag(tag) && valid_file_name(file) && path.replace("%2B", "+") == want
}

/// One file of a release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    /// Bytes, as GitHub reports them (0 when it didn't).
    pub size: u64,
    /// Exactly [`expected_download_url`].
    pub url: String,
    /// Lower-case hex SHA-256 GitHub computed (`digest`), when it gave one.
    pub sha256: Option<String>,
}

/// A release as the releases API describes it, with only the assets that passed the checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    /// Its title, at most 120 characters (the tag when it has none).
    pub name: String,
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
}

/// Read the releases API's answer for `repo` (one release object). An API error object
/// (`message`) becomes its text; drafts are refused; assets whose name isn't plain or whose link
/// isn't exactly [`expected_download_url`] are left out.
pub fn parse_release(json: &str, repo: &RepoRef) -> Result<Release, String> {
    if json.len() > MAX_API_BYTES {
        return Err(format!("GitHub's answer is larger than {} MB", MAX_API_BYTES >> 20));
    }
    let v: Value = serde_json::from_str(json).map_err(|e| format!("unreadable answer from GitHub ({e})"))?;
    if let Some(msg) = v.get("message").and_then(Value::as_str) {
        return Err(if msg == "Not Found" {
            format!("{} has no such release (or isn't a public repository)", repo.slug())
        } else {
            format!("GitHub says: {}", msg.chars().filter(|c| !c.is_control()).take(200).collect::<String>())
        });
    }
    let tag = v.get("tag_name").and_then(Value::as_str).filter(|t| valid_tag(t)).ok_or("GitHub's answer has no usable release tag")?;
    if repo.tag.as_deref().is_some_and(|want| want != tag) {
        return Err(format!("GitHub answered with release {tag}, not {}", repo.tag.as_deref().unwrap_or_default()));
    }
    if v.get("draft").and_then(Value::as_bool).unwrap_or(false) {
        return Err(format!("release {tag} is a draft"));
    }
    let assets = v
        .get("assets")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let name = a.get("name").and_then(Value::as_str).filter(|n| valid_file_name(n))?;
                    let url = a.get("browser_download_url").and_then(Value::as_str).filter(|u| is_asset_url(u, repo, tag, name))?;
                    let sha256 = a
                        .get("digest")
                        .and_then(Value::as_str)
                        .and_then(|d| d.strip_prefix("sha256:"))
                        .filter(|h| is_hex64(h))
                        .map(str::to_ascii_lowercase);
                    let size = a.get("size").and_then(Value::as_u64).unwrap_or(0);
                    Some(ReleaseAsset { name: name.to_string(), size, url: url.to_string(), sha256 })
                })
                .collect()
        })
        .unwrap_or_default();
    let name = v.get("name").and_then(Value::as_str).map(|n| n.chars().filter(|c| !c.is_control()).take(120).collect::<String>());
    Ok(Release {
        tag: tag.to_string(),
        name: name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| tag.to_string()),
        prerelease: v.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
        assets,
    })
}

impl Release {
    /// The release's plug-in bundle: its one `.craftplugin` asset. A release with none, or with
    /// several (which one would be meant?), can't be installed from its repository.
    pub fn bundle_asset(&self) -> Result<&ReleaseAsset, String> {
        let suffix = format!(".{BUNDLE_EXTENSION}");
        let mut bundles = self.assets.iter().filter(|a| a.name.to_ascii_lowercase().ends_with(&suffix));
        match (bundles.next(), bundles.next()) {
            (Some(a), None) => Ok(a),
            (None, _) => Err(format!("release {} has no .{BUNDLE_EXTENSION} file", self.tag)),
            (Some(_), Some(_)) => Err(format!("release {} has several .{BUNDLE_EXTENSION} files: install it from a catalog instead", self.tag)),
        }
    }

    /// The release's checksum list (`SHA256SUMS`, else `SHA256SUMS.txt`).
    pub fn sums_asset(&self) -> Option<&ReleaseAsset> {
        SUMS_NAMES.iter().find_map(|n| self.assets.iter().find(|a| a.name == *n))
    }
}

/// 64 hex digits.
pub(crate) fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Read `sha256sum` output: `<hex>  <name>` (or `<hex> *<name>`) per line, as `(hash, name)`
/// with the hash in lower case. Lines that aren't such a pair are skipped; a name listed twice
/// with the same hash counts once, and a name listed with two different hashes is left out, so
/// the list vouches for neither.
pub fn parse_sha256sums(text: &str) -> Vec<(String, String)> {
    let mut sums: Vec<(String, String)> = Vec::new();
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
                if sums.get(i).is_some_and(|(h, _)| *h != hash) {
                    conflicting.insert(name);
                }
            }
        }
    }
    sums.retain(|(_, n)| !conflicting.contains(n.as_str()));
    sums
}

/// The hash `sums` lists for `name`.
pub fn sum_for<'a>(sums: &'a [(String, String)], name: &str) -> Option<&'a str> {
    sums.iter().find(|(_, n)| n == name).map(|(h, _)| h.as_str())
}

/// Check `bytes` against `expected` (hex, any case); their SHA-256, lower-case hex.
pub fn verify_sha256(bytes: &[u8], expected: &str) -> Result<String, String> {
    let got = sha256_hex(bytes);
    if !is_hex64(expected) || !got.eq_ignore_ascii_case(expected) {
        return Err(format!("the download's SHA-256 is {got}, not the published {}", expected.chars().take(64).collect::<String>()));
    }
    Ok(got)
}

/// What to download to install a repository's release, once its checksum is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoDownload {
    /// The repository, with the tag of the release that was read.
    pub repo: RepoRef,
    /// The release's title.
    pub release_name: String,
    /// The `.craftplugin` asset's name and link.
    pub asset: String,
    pub url: String,
    /// Its size as GitHub reported it (0: unknown; the download is still capped).
    pub size: u64,
    /// Lower-case hex SHA-256 the bytes must have.
    pub sha256: String,
}

/// Decide what to download from `release` of `repo`: its bundle asset, and the SHA-256 that
/// `sums` (the text of its `SHA256SUMS` asset, when it has one) and GitHub's `digest` give. When
/// both give one they must agree; when neither does, nothing vouches for the file and it isn't
/// installed.
pub fn plan_download(repo: &RepoRef, release: &Release, sums: Option<&str>) -> Result<RepoDownload, String> {
    let asset = release.bundle_asset()?;
    if asset.size > MAX_BUNDLE_BYTES as u64 {
        return Err(format!("{} is larger than {} MB", asset.name, MAX_BUNDLE_BYTES >> 20));
    }
    let listed = sums.map(parse_sha256sums).and_then(|s| sum_for(&s, &asset.name).map(str::to_string));
    let sha256 = match (listed, asset.sha256.clone()) {
        (Some(a), Some(b)) if a != b => {
            return Err(format!("{} lists a different SHA-256 for {} than GitHub computed", SUMS_NAMES[0], asset.name));
        }
        (Some(h), _) | (None, Some(h)) => h,
        (None, None) => {
            return Err(format!("release {} publishes no SHA-256 for {} (add a {} file to the release)", release.tag, asset.name, SUMS_NAMES[0]));
        }
    };
    Ok(RepoDownload {
        repo: RepoRef { tag: Some(release.tag.clone()), ..repo.clone() },
        release_name: release.name.clone(),
        asset: asset.name.clone(),
        url: asset.url.clone(),
        size: asset.size,
        sha256,
    })
}

/// The rest of `s` after `prefix`, ignoring ASCII case.
pub(crate) fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| s.get(prefix.len()..)).flatten()
}

/// A `.` or `..` path segment, also percent-encoded.
pub(crate) fn dot_segment(p: &str) -> bool {
    let p = p.to_ascii_lowercase().replace("%2e", ".");
    p == "." || p == ".."
}

/// A character a release or repository URL may not contain: `?`, `#`, `\`, white space (Unicode's
/// too), a control character or U+FEFF.
pub(crate) fn bad_url_char(c: char) -> bool {
    matches!(c, '?' | '#' | '\\' | '\u{feff}') || c.is_whitespace() || c.is_control()
}

/// A percent-encoded `/`, `\` or `.` (`%2f`, `%5c`, `%2e`), which a client could decode back into a
/// separator or a `..` segment after the URL was checked.
pub(crate) fn encoded_separator(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.contains("%2f") || lower.contains("%5c") || lower.contains("%2e")
}
