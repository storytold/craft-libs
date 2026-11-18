//! Installing from catalogs and GitHub over an app's own HTTP client: [`Fetch`] is the one thing an
//! app implements (craft-update's `Http` does, with the `http` feature); the functions here apply
//! every check of [`crate::catalog`] and [`crate::github`] around it. Nothing here writes to disk:
//! the caller shows the bundle it gets back to the person ([`crate::bundle::open`] reads its
//! manifest) and installs it into the [`crate::store`] once they agree.
//!
//! A local (`file://`) catalog, and the files it names, are read through [`Fetch::read_local`]:
//! by default straight from the disk, or wherever the app confines file access (automation roots)
//! when it overrides that method.
//!
//! The functions here block until their requests finish. Call them on a worker thread (or a task
//! of the app's own) and hand the result back to the UI; to cancel, let the app's [`Fetch`]
//! return an error once the person cancels, and to show progress, report it from the app's
//! [`Fetch::get`].

use std::path::Path;

use crate::bundle::MAX_BUNDLE_BYTES;
use crate::catalog::{Catalog, Location, MAX_CATALOG_BYTES, Package, Release, check_redirect, check_url, parse_catalog, release_url_problem};
use crate::github::{MAX_API_BYTES, MAX_SUMS_BYTES, RepoDownload, RepoRef, parse_release, plan_download, verify_sha256};

/// An HTTPS client.
pub trait Fetch {
    /// GET `url`, following redirects, and return the body: `Ok(None)` when the server answers 404
    /// Not Found. A body of more than `max` bytes is an error, refused as it arrives (not read
    /// whole first). `url` is `https://` on GitHub's hosts or `https://api.github.com/`. Each
    /// redirect should stay where [`crate::catalog::check_redirect`] allows: a client that can
    /// look at a redirect before following it walks them with [`follow_redirects`] (as the `http`
    /// feature's client does); the SHA-256 check guards a bundle's bytes either way.
    fn get(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String>;

    /// Read a local file a `file://` catalog URL names (the catalog, or a release it lists), of at
    /// most `max` bytes. `path` is what [`crate::catalog::file_url_path`] made of the URL: absolute,
    /// percent-decoded, never a network share. The default reads the disk ([`read_local_file`]);
    /// an app that confines file access (MCP automation roots) overrides it to check `path` first
    /// and then call [`read_local_file`], or to refuse local catalogs altogether.
    fn read_local(&mut self, path: &Path, max: usize) -> Result<Vec<u8>, String> {
        read_local_file(path, max)
    }
}

/// Read at most `max` bytes of the local file `path` (what [`Fetch::read_local`] does unless the
/// app overrides it). A web build has no files and gets an error.
pub fn read_local_file(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let shown = path.display();
    let len = std::fs::metadata(path).map_err(|e| format!("cannot read {shown}: {e}"))?.len();
    if len > max as u64 {
        return Err(format!("{shown} is larger than {} KB", max / 1024));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
    if bytes.len() > max {
        return Err(format!("{shown} is larger than {} KB", max / 1024));
    }
    Ok(bytes)
}

/// Most redirects [`follow_redirects`] follows.
pub const MAX_REDIRECTS: usize = 10;

/// Walk `url`'s redirects with `probe` (where a URL redirects to, without following it: `None`
/// when it doesn't, or when that can't be told) and return where they end. Every hop, `url`
/// included, must pass [`check_redirect`] (`https://` on [`crate::catalog::HOSTS`]); a relative
/// `Location` (`/path`, `//host/path`) is resolved against the hop before it. More than
/// [`MAX_REDIRECTS`] hops is an error. The caller then GETs the end it returns.
pub fn follow_redirects(url: &str, probe: &mut dyn FnMut(&str) -> Option<String>) -> Result<String, String> {
    check_redirect(url)?;
    let mut here = url.to_string();
    for _ in 0..MAX_REDIRECTS {
        let Some(next) = probe(&here) else { return Ok(here) };
        let next = resolve_location(&here, next.trim())?;
        check_redirect(&next)?;
        here = next;
    }
    Err(format!("the download was redirected more than {MAX_REDIRECTS} times"))
}

/// A redirect's `Location` as an absolute URL, resolved against `base` (an `https://` URL).
fn resolve_location(base: &str, location: &str) -> Result<String, String> {
    let shown: String = location.chars().take(120).collect();
    if location.contains("://") {
        return Ok(location.to_string());
    }
    if let Some(rest) = location.strip_prefix("//") {
        return Ok(format!("https://{rest}"));
    }
    if location.starts_with('/') {
        let after = base.get("https://".len()..).unwrap_or_default();
        let host = after.split(['/', '?', '#']).next().unwrap_or_default();
        return Ok(format!("https://{host}{location}"));
    }
    Err(format!("the download was redirected to `{shown}`, which isn't a URL this app follows"))
}

/// Read `location`: over [`Fetch::get`], or through [`Fetch::read_local`] for a local catalog's
/// files.
pub fn read_location(fetch: &mut dyn Fetch, location: &Location, max: usize) -> Result<Vec<u8>, String> {
    match location {
        Location::Https(url) => fetch.get(url, max)?.ok_or_else(|| format!("{url} wasn't found")),
        Location::File(path) => fetch.read_local(path, max),
    }
}

/// Fetch and read the catalog at `url` (`https://` on GitHub's hosts, or `file://`); the warnings
/// list what was left out.
pub fn fetch_catalog(fetch: &mut dyn Fetch, url: &str) -> Result<(Catalog, Vec<String>), String> {
    let location = check_url(url, None)?;
    let bytes = read_location(fetch, &location, MAX_CATALOG_BYTES)?;
    parse_catalog(&bytes, url)
}

/// Download `release` of `package` from the catalog at `catalog_url`, checked against the size and
/// the SHA-256 the catalog pins.
pub fn download_release(fetch: &mut dyn Fetch, catalog_url: &str, package: &Package, release: &Release) -> Result<Vec<u8>, String> {
    if let Some(why) = release_url_problem(&package.repo, &release.url, catalog_url) {
        return Err(why);
    }
    let location = check_url(&release.url, Some(catalog_url))?;
    let max = release.size.and_then(|n| usize::try_from(n).ok()).unwrap_or(MAX_BUNDLE_BYTES).min(MAX_BUNDLE_BYTES);
    let bytes = read_location(fetch, &location, max)?;
    if release.size.is_some_and(|n| n != bytes.len() as u64) {
        return Err(format!("the download is {} bytes, not the {} the catalog lists", bytes.len(), release.size.unwrap_or_default()));
    }
    verify_sha256(&bytes, &release.sha256)?;
    Ok(bytes)
}

/// Read `repo`'s release (its latest, or its tag) through the releases API, and its `SHA256SUMS`:
/// what [`download_repo`] will download and the SHA-256 it must have. Downloads only the API
/// answer and the checksum list.
pub fn resolve_repo(fetch: &mut dyn Fetch, repo: &RepoRef) -> Result<RepoDownload, String> {
    let api = repo.releases_api_url();
    let json = fetch.get(&api, MAX_API_BYTES)?.ok_or_else(|| format!("{} has no such release (or isn't a public repository)", repo.slug()))?;
    let json = String::from_utf8(json).map_err(|_| "GitHub's answer isn't UTF-8 text".to_string())?;
    let release = parse_release(&json, repo)?;
    let sums = match release.sums_asset() {
        Some(a) => {
            let bytes = fetch.get(&a.url, MAX_SUMS_BYTES)?.ok_or_else(|| format!("{} of release {} wasn't found", a.name, release.tag))?;
            Some(String::from_utf8(bytes).map_err(|_| format!("{} isn't text", a.name))?)
        }
        None => None,
    };
    plan_download(repo, &release, sums.as_deref())
}

/// Download what [`resolve_repo`] found, checked against its size (when GitHub gave one) and
/// SHA-256.
pub fn download_repo(fetch: &mut dyn Fetch, plan: &RepoDownload) -> Result<Vec<u8>, String> {
    check_url(&plan.url, None)?;
    let max = match usize::try_from(plan.size) {
        Ok(0) | Err(_) => MAX_BUNDLE_BYTES,
        Ok(n) => n.min(MAX_BUNDLE_BYTES),
    };
    let bytes = fetch.get(&plan.url, max)?.ok_or_else(|| format!("{} wasn't found", plan.url))?;
    if plan.size != 0 && plan.size != bytes.len() as u64 {
        return Err(format!("the download is {} bytes, not the {} GitHub lists", bytes.len(), plan.size));
    }
    verify_sha256(&bytes, &plan.sha256)?;
    Ok(bytes)
}

/// craft-update's client. API requests (`https://api.github.com/`) go through `get_api`, which
/// follows GitHub's redirects itself (a renamed repository) without re-checking them: its answer
/// is only read as release JSON, whose asset URLs are checked like any other, and the GitHub token
/// is never sent on a redirect (ureq's default). Every other request first walks its redirects
/// with `redirect_target` (a HEAD request per hop) through [`follow_redirects`], so a hop off
/// GitHub's hosts is refused before anything is downloaded, then GETs where they end. Not
/// checked: a server on GitHub's hosts that answers that GET with another redirect than it gave
/// the HEAD request (followed, on HTTPS only, as ureq is told). Bundles are pinned by SHA-256 and
/// size either way; a catalog's entries are checked whatever served it.
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
impl Fetch for craft_update::Http {
    fn get(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
        if url.starts_with("https://api.github.com/") {
            return match self.get_api(url, None)? {
                craft_update::http::Fetched::Body { text, .. } if text.len() > max => Err(format!("GitHub's answer is larger than {max} bytes")),
                craft_update::http::Fetched::Body { text, .. } => Ok(Some(text.into_bytes())),
                craft_update::http::Fetched::NotFound => Ok(None),
                craft_update::http::Fetched::RateLimited { .. } => Err("GitHub's API limit for this hour is used up: try again later".into()),
                craft_update::http::Fetched::NotModified => Err("GitHub answered 304 Not Modified to a request without a cache".into()),
            };
        }
        // A HEAD that fails (no redirect, or a signed URL that only answers GET) ends the walk.
        let end = follow_redirects(url, &mut |u| self.redirect_target(u).ok().flatten())?;
        self.get_bytes(&end, max)
    }
}
