//! Installing from catalogs and GitHub over an app's own HTTP client: [`Fetch`] is the one thing an
//! app implements (craft-update's `Http` does, with the `http` feature); the functions here apply
//! every check of [`crate::catalog`] and [`crate::github`] around it. Nothing here writes to disk:
//! the caller shows the bundle it gets back to the person ([`crate::bundle::open`] reads its
//! manifest) and installs it into the [`crate::store`] once they agree.

use crate::bundle::MAX_BUNDLE_BYTES;
use crate::catalog::{Catalog, Location, MAX_CATALOG_BYTES, Package, Release, check_url, parse_catalog, release_url_problem};
use crate::github::{MAX_API_BYTES, MAX_SUMS_BYTES, RepoDownload, RepoRef, parse_release, plan_download, verify_sha256};

/// An HTTPS client.
pub trait Fetch {
    /// GET `url`, following redirects, and return the body: `Ok(None)` when the server answers 404
    /// Not Found. A body of more than `max` bytes is an error, refused as it arrives (not read
    /// whole first). `url` is `https://` on GitHub's hosts or `https://api.github.com/`. A client
    /// that can tell where redirects ended should refuse an end that
    /// [`crate::catalog::check_redirect`] refuses; the SHA-256 check guards the bytes either way.
    fn get(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String>;
}

/// Read `location`: over `fetch`, or from the disk for a local catalog's files.
pub fn read_location(fetch: &mut dyn Fetch, location: &Location, max: usize) -> Result<Vec<u8>, String> {
    match location {
        Location::Https(url) => fetch.get(url, max)?.ok_or_else(|| format!("{url} wasn't found")),
        Location::File(path) => {
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

#[cfg(feature = "http")]
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
        self.get_bytes(url, max)
    }
}
