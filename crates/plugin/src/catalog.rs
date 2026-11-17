//! Catalogs: an `index.json` that lists plug-in packages and pins each release to a GitHub release
//! asset by URL, SHA-256 and size. EffectCraft's Plug-in Library format, generalized with a
//! package `kind` and per-release `apps`:
//!
//! ```json
//! {
//!   "schema": 1,
//!   "packages": [{
//!     "id": "io.github.example.palette-tools",
//!     "name": "Palette tools",
//!     "author": "Example",
//!     "description": "Makes swatches and palettes from a list of colours.",
//!     "license": "MIT OR Apache-2.0",
//!     "repo": "https://github.com/example/palette-tools",
//!     "kind": "command",
//!     "tags": ["colour"],
//!     "versions": [{
//!       "version": "1.2.0",
//!       "apps": { "figcraft": { "api": 6 }, "designcraft": { "api": 6, "minApp": "0.30.0" } },
//!       "url": "https://github.com/example/palette-tools/releases/download/v1.2.0/palette-tools.craftplugin",
//!       "sha256": "…64 hex digits…",
//!       "size": 48213
//!     }]
//!   }]
//! }
//! ```
//!
//! A release without `apps` is EffectCraft's: its own `api` and `minApp` apply to whichever app
//! reads the catalog, when that app accepts legacy entries. Fields this crate doesn't know (an
//! app's own, such as EffectCraft's `effects` or a package's `preview`) are kept in `extra`.
//!
//! Hostile input: the catalog is size-capped, and a package or release that breaks a rule is left
//! out with a warning, so one bad entry doesn't hide the others. URL rules, for catalogs and the
//! files they name: `https://` on GitHub's hosts only, a release URL must be a release asset
//! (`https://github.com/<owner>/<repo>/releases/download/<tag>/<file>`) of the package's `repo`
//! when it names one; a catalog read from a `file://` URL (a plug-in author testing an entry) may
//! also name local files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::bundle::{MAX_BUNDLE_BYTES, compare_versions, parse_version};
use crate::github::{RepoRef, bad_url_char, dot_segment, encoded_separator, is_hex64, parse_repo_url, strip_prefix_ignore_case};
use crate::manifest::{AppTarget, KIND_COMMAND, check_apps};
use crate::target::{AppInfo, Target, check_target};

/// The catalog format this build reads.
pub const CATALOG_SCHEMA: u32 = 1;
/// Hosts catalogs and plug-ins may be downloaded from (release assets redirect to the last two).
pub const HOSTS: &[&str] = &["github.com", "raw.githubusercontent.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];
/// Largest catalog read.
pub const MAX_CATALOG_BYTES: usize = 4 << 20;
/// Most packages read from one catalog, and releases from one package.
pub const MAX_PACKAGES: usize = 10_000;
pub const MAX_RELEASES: usize = 256;
const MAX_TAGS: usize = 16;
const MAX_URL: usize = 1000;

/// A catalog (`index.json`), as read: only the packages and releases that passed the checks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    #[serde(default)]
    pub packages: Vec<Package>,
}

/// One plug-in: its description and its releases.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Package {
    /// The plug-in's id (the `id` in its manifest).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
    /// The source repository, `https://github.com/<owner>/<repo>`; its releases are the only
    /// downloads the package may name.
    #[serde(default)]
    pub repo: String,
    /// `command` (the default) or an app's own kind.
    #[serde(default = "command_kind")]
    pub kind: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub versions: Vec<Release>,
    /// Fields this crate doesn't read (an app's own), as they were.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One release of a package: one file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    /// Dotted numbers (`1.2.0`).
    pub version: String,
    /// The apps it was written for, as in a manifest.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub apps: BTreeMap<String, AppTarget>,
    /// EffectCraft's form, without `apps`: the plug-in API it implements…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<u32>,
    /// …and the oldest app version it runs in.
    #[serde(default, rename = "minApp", skip_serializing_if = "Option::is_none")]
    pub min_app: Option<String>,
    /// A GitHub release asset (or, in a local catalog, a `file://` URL).
    pub url: String,
    /// Lower-case hex SHA-256 of the file at `url`.
    pub sha256: String,
    /// Its size in bytes; the download is refused when it differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Fields this crate doesn't read (EffectCraft's `effects`…), as they were.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn command_kind() -> String {
    KIND_COMMAND.to_string()
}

/// Where a URL may be read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// An `https://` URL on one of [`HOSTS`].
    Https(String),
    /// A local file (only from a local catalog).
    File(PathBuf),
}

/// A package id that is also a safe folder name: 1–100 letters, digits, `.`, `-`, `_`; no
/// leading or trailing dot and no `..`.
pub fn valid_package_id(id: &str) -> bool {
    (1..=100).contains(&id.len())
        && !id.starts_with('.')
        && !id.ends_with('.')
        && !id.contains("..")
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// Trimmed text of at most `max` characters without control characters (`None`: it isn't).
fn plain_text(s: &str, max: usize) -> Option<String> {
    let s = s.trim();
    (s.chars().count() <= max && !s.chars().any(char::is_control)).then(|| s.to_string())
}

/// Read a catalog fetched from `catalog_url`: packages and releases that break a rule are left
/// out and described in the warnings, so one bad entry doesn't hide the others. An error only
/// when the whole catalog can't be used (too large, not JSON, another format).
pub fn parse_catalog(bytes: &[u8], catalog_url: &str) -> Result<(Catalog, Vec<String>), String> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(format!("the catalog is larger than {} MB", MAX_CATALOG_BYTES >> 20));
    }
    /// The catalog's outline: each package is read on its own, so one with a field of the wrong
    /// type is left out rather than failing the catalog.
    #[derive(Deserialize)]
    struct Raw {
        schema: u32,
        #[serde(default)]
        packages: Vec<Value>,
    }
    let raw: Raw = serde_json::from_slice(bytes).map_err(|e| format!("the catalog isn't valid: {e}"))?;
    if raw.schema != CATALOG_SCHEMA {
        return Err(format!("the catalog uses format {} and this app reads format {CATALOG_SCHEMA}: update the app", raw.schema));
    }
    let mut warnings = vec![];
    if raw.packages.len() > MAX_PACKAGES {
        warnings.push(format!("read only the first {MAX_PACKAGES} packages"));
    }
    let mut seen = BTreeSet::new();
    let mut packages = vec![];
    for v in raw.packages.into_iter().take(MAX_PACKAGES) {
        match read_package(v, &mut warnings).and_then(|p| check_package(p, catalog_url, &mut warnings)) {
            // Windows and macOS folders ignore case: `Org.X` and `org.x` are one plug-in.
            Ok(p) if !seen.insert(p.id.to_ascii_lowercase()) => warnings.push(format!("left out a second package `{}`", p.id)),
            Ok(p) => packages.push(p),
            Err(why) => warnings.push(why),
        }
    }
    Ok((Catalog { schema: CATALOG_SCHEMA, packages }, warnings))
}

/// A package entry as JSON, its releases read one by one (a malformed one is left out).
fn read_package(mut v: Value, warnings: &mut Vec<String>) -> Result<Package, String> {
    let id: String = v.get("id").and_then(Value::as_str).unwrap_or_default().chars().take(100).collect();
    let versions = match v.as_object_mut().and_then(|o| o.remove("versions")) {
        Some(Value::Array(list)) => list,
        None | Some(Value::Null) => Vec::new(),
        Some(_) => return Err(format!("left out {id}: `versions` isn't a list")),
    };
    let mut p: Package = serde_json::from_value(v).map_err(|e| format!("left out a package (`{id}`): {e}"))?;
    if versions.len() > MAX_RELEASES {
        warnings.push(format!("{id}: read only the first {MAX_RELEASES} releases"));
    }
    for r in versions.into_iter().take(MAX_RELEASES) {
        match serde_json::from_value::<Release>(r) {
            Ok(r) => p.versions.push(r),
            Err(e) => warnings.push(format!("{id}: left out a release: {e}")),
        }
    }
    Ok(p)
}

/// `p` with its texts cleaned and its bad releases left out, or why the whole package is.
fn check_package(mut p: Package, catalog_url: &str, warnings: &mut Vec<String>) -> Result<Package, String> {
    let id: String = p.id.chars().take(100).collect();
    if !valid_package_id(&p.id) {
        return Err(format!("left out a package with a bad id (`{id}`)"));
    }
    let texts = [(&mut p.name, 64), (&mut p.author, 64), (&mut p.description, 1000), (&mut p.license, 128), (&mut p.repo, 200)];
    for (text, max) in texts {
        *text = plain_text(text, max).ok_or_else(|| format!("left out {id}: a text is too long or has control characters"))?;
    }
    if p.name.is_empty() {
        return Err(format!("left out {id}: it has no name"));
    }
    if !p.repo.is_empty() && parse_repo_url(&p.repo).is_none() {
        return Err(format!("left out {id}: its `repo` isn't a GitHub repository URL (https://github.com/<owner>/<repo>)"));
    }
    if p.kind.is_empty() || p.kind.len() > 32 || !p.kind.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err(format!("left out {id}: bad kind"));
    }
    p.tags.retain(|t| plain_text(t, 32).is_some_and(|t| !t.is_empty()));
    p.tags.truncate(MAX_TAGS);
    let repo = p.repo.clone();
    p.versions.retain_mut(|r| match release_problem(r, &repo, catalog_url) {
        Some(why) => {
            warnings.push(format!("{id}: left out release `{}`: {why}", r.version.chars().take(32).collect::<String>()));
            false
        }
        None => {
            r.sha256.make_ascii_lowercase();
            true
        }
    });
    Ok(p)
}

/// Why a release can't be used, if it can't.
fn release_problem(r: &Release, repo: &str, catalog_url: &str) -> Option<String> {
    if parse_version(&r.version).is_none() {
        return Some("its version isn't dotted numbers like 1.2.0".into());
    }
    if r.min_app.as_deref().is_some_and(|m| parse_version(m).is_none()) {
        return Some("bad minApp".into());
    }
    if r.apps.is_empty() && r.api.is_none_or(|a| a == 0) {
        return Some("it names neither `apps` nor an `api`".into());
    }
    if let Err(why) = check_apps(&r.apps) {
        return Some(why);
    }
    if !is_hex64(&r.sha256) {
        return Some("bad sha256".into());
    }
    if r.size.is_some_and(|n| n == 0 || n > MAX_BUNDLE_BYTES as u64) {
        return Some(format!("its size isn't 1 byte to {} MB", MAX_BUNDLE_BYTES >> 20));
    }
    release_url_problem(repo, &r.url, catalog_url)
}

/// Why `url` can't be a release's download, if it can't: an `https://` URL must be a GitHub release
/// asset, of `repo` when it is given; a `file://` URL only comes from a local catalog.
pub fn release_url_problem(repo: &str, url: &str, catalog_url: &str) -> Option<String> {
    if url.len() > MAX_URL {
        return Some("its url is too long".into());
    }
    match check_url(url, Some(catalog_url)) {
        Ok(Location::File(_)) => return None,
        Ok(Location::Https(_)) => {}
        Err(why) => return Some(why),
    }
    let shown: String = url.chars().take(120).collect();
    let not_asset = || Some(format!("`{shown}` isn't a release asset (https://github.com/<owner>/<repo>/releases/download/<tag>/<file>)"));
    let Some(rest) = strip_prefix_ignore_case(url, "https://github.com/") else { return not_asset() };
    if rest.contains(bad_url_char) || encoded_separator(&rest.replace("%2B", "+").replace("%2b", "+")) {
        return not_asset();
    }
    let parts: Vec<&str> = rest.split('/').collect();
    // At least six segments: a tag may contain `/` (`release/v1`). `.` and `..` (also
    // percent-encoded) would move the download to another path.
    if parts.len() < 6 || parts.iter().any(|p| p.is_empty() || dot_segment(p)) {
        return not_asset();
    }
    let (Some(owner), Some(name), Some(&"releases"), Some(&"download")) = (parts.first(), parts.get(1), parts.get(2), parts.get(3)) else {
        return not_asset();
    };
    if parse_repo_url(&format!("https://github.com/{owner}/{name}")).is_none() {
        return not_asset();
    }
    let repo = repo.trim();
    if repo.is_empty() {
        return None;
    }
    match parse_repo_url(repo) {
        Some((o, n)) if o.eq_ignore_ascii_case(owner) && n.eq_ignore_ascii_case(name) => None,
        Some(_) => Some(format!("`{shown}` isn't a release asset of the package's repository {repo}")),
        None => Some(format!("the package's `repo` {repo} isn't a GitHub repository URL")),
    }
}

/// Check `url` against the rules: `https://` on [`HOSTS`]; `file://` when the catalog (`catalog`,
/// or `url` itself when it is the catalog) is local.
pub fn check_url(url: &str, catalog: Option<&str>) -> Result<Location, String> {
    let shown: String = url.chars().take(120).collect();
    if url.len() > MAX_URL || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(format!("`{shown}` isn't a URL this app downloads"));
    }
    if let Some(rest) = strip_prefix_ignore_case(url, "https://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
        if host.contains(['@', ':', '\\']) || !HOSTS.contains(&host.as_str()) {
            return Err(format!("`{shown}` isn't on GitHub: plug-ins are only downloaded from {}", HOSTS.join(", ")));
        }
        return Ok(Location::Https(url.to_string()));
    }
    let local_catalog = catalog.is_none_or(|c| c.starts_with("file://"));
    if url.starts_with("file://") && local_catalog {
        return file_url_path(url).map(Location::File);
    }
    Err(format!("`{shown}` isn't an https:// URL"))
}

/// Check where a download ended after following redirects: still `https://` on [`HOSTS`].
pub fn check_redirect(final_url: &str) -> Result<(), String> {
    match check_url(final_url, None) {
        Ok(Location::Https(_)) => Ok(()),
        _ => Err(format!("the download was redirected to `{}`, which isn't on GitHub", final_url.chars().take(120).collect::<String>())),
    }
}

/// `file:///C:/x` or `file:///home/x` as a path (percent-decoded; no remote hosts). A path that
/// starts with two separators or holds a `\` (raw or percent-encoded), or a percent-encoded `/`,
/// is refused: on Windows such a path can name a network share, and reading it contacts that host.
pub fn file_url_path(url: &str) -> Result<PathBuf, String> {
    let shown: String = url.chars().take(120).collect();
    let bad = |why: &str| format!("bad file URL `{shown}`: {why}");
    let rest = url.strip_prefix("file://").ok_or_else(|| bad("it doesn't start with file://"))?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return Err(bad("only local files can be read (file:///path)"));
    }
    let b = rest.as_bytes();
    // `/C:/…` on Windows.
    let rest = if b.get(1).is_some_and(u8::is_ascii_alphabetic) && b.get(2) == Some(&b':') {
        rest.get(1..).ok_or_else(|| bad("bad drive letter"))?
    } else {
        rest
    };
    let mut out = Vec::with_capacity(rest.len());
    let mut bytes = rest.bytes();
    while let Some(c) = bytes.next() {
        if c != b'%' {
            out.push(c);
            continue;
        }
        let byte = match [bytes.next(), bytes.next()] {
            [Some(hi), Some(lo)] => std::str::from_utf8(&[hi, lo]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()),
            _ => None,
        }
        .ok_or_else(|| bad("a % isn't followed by two hex digits"))?;
        if matches!(byte, b'/' | b'\\') {
            return Err(bad("a percent-encoded / or \\ isn't allowed"));
        }
        out.push(byte);
    }
    let path = String::from_utf8(out).map_err(|_| bad("the path isn't UTF-8"))?;
    if path.contains('\\') {
        return Err(bad("use / between folders, not \\"));
    }
    if path.starts_with("//") {
        return Err(bad("network paths can't be read, only local files"));
    }
    if path.chars().any(char::is_control) {
        return Err(bad("the path has control characters"));
    }
    Ok(PathBuf::from(path))
}

impl Release {
    /// Whether `app` can run this release of a `kind` package (see [`crate::Manifest::target`]).
    pub fn target(&self, kind: &str, app: &AppInfo<'_>) -> Result<Target, String> {
        check_target(kind, &self.apps, self.api.map(|api| (api, self.min_app.as_deref())), app)
    }
}

impl Package {
    /// The newest release `app` can run (kind, `apps`, API and minimum app version).
    pub fn latest_for(&self, app: &AppInfo<'_>) -> Option<&Release> {
        self.versions.iter().filter(|r| r.target(&self.kind, app).is_ok()).fold(None, |best: Option<&Release>, r| match best {
            Some(b) if compare_versions(&r.version, &b.version).is_none_or(|o| o.is_le()) => Some(b),
            _ => Some(r),
        })
    }

    /// The release `version` (compared as numbers: `1.2` is `1.2.0`).
    pub fn release(&self, version: &str) -> Option<&Release> {
        self.versions.iter().find(|r| compare_versions(&r.version, version).is_some_and(|o| o.is_eq()))
    }

    /// Check a downloaded bundle's manifest against the catalog's entry: the same id, kind and
    /// version as `release`. Run it after [`crate::bundle::open`], before the install confirmation.
    pub fn check_manifest(&self, release: &Release, manifest: &crate::Manifest) -> Result<(), String> {
        if !manifest.id.eq_ignore_ascii_case(&self.id) {
            return Err(format!("the catalog lists {} but the download is {}", self.id, manifest.id));
        }
        if manifest.kind != self.kind {
            return Err(format!("the catalog lists a `{}` plug-in but the download is a `{}` one", self.kind, manifest.kind));
        }
        if compare_versions(&manifest.version, &release.version).is_none_or(|o| o.is_ne()) {
            return Err(format!("the catalog lists version {} but the download is version {}", release.version, manifest.version));
        }
        Ok(())
    }

    /// The package's repository, when it names one.
    pub fn repo_ref(&self) -> Option<RepoRef> {
        parse_repo_url(&self.repo).map(|(owner, name)| RepoRef { owner, name, tag: None })
    }
}

impl Catalog {
    /// The package `id` (ignoring case, as the store does).
    pub fn package(&self, id: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.id.eq_ignore_ascii_case(id))
    }
}
