//! Where release information comes from.
//!
//! [`GitHubSource`] asks the GitHub REST API (one request per app, revalidated with ETags) and,
//! when the API is out of quota, falls back to github.com's release pages: the
//! `releases/latest` redirect names the tag and `SHA256SUMS.txt` lists every file with its
//! checksum, which costs no API quota at all. A later [`ReleaseSource`] will ask the ArtCraft
//! catalog API for every app in one request.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use crate::protocol::{LatestRelease, Source};
use crate::release::{self, Release};
use serde::{Deserialize, Serialize};

use crate::http::{Fetched, Http};

/// One answer from a source.
#[derive(Clone, Debug)]
pub struct Answer {
    #[allow(missing_docs)]
    pub latest: LatestRelease,
    /// Where the answer came from.
    pub source: Source,
}

/// Looks up latest releases. Called from worker threads.
pub trait ReleaseSource: Send + Sync {
    /// The latest release of `repo` (`owner/name`), including prereleases when `prereleases` is set.
    fn latest(&self, repo: &str, prereleases: bool) -> Answer;
    /// The last answer saved for `repo`, for an instant first paint.
    fn cached(&self, repo: &str, prereleases: bool) -> Option<LatestRelease>;
    /// The tag of `repo`'s newest stable release, by a route that costs no API quota (GitHub's
    /// `releases/latest` redirect), or `None` when it can't be told. Quick: a check pressed by the
    /// user asks this for every app to see whether the feed has fallen behind.
    fn newest_tag(&self, repo: &str) -> Option<String> {
        let _ = repo;
        None
    }
    /// API refused until this time (Unix seconds).
    fn rate_limited_until(&self) -> Option<i64>;
    /// Persist anything worth keeping.
    fn save(&self);
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Entry {
    etag: Option<String>,
    /// `None`: the repo has no releases.
    release: Option<Release>,
    prereleases: bool,
    fetched_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    entries: BTreeMap<String, Entry>,
}

/// Release information from GitHub: the REST API, falling back to the release pages when the
/// API is out of quota. See the module docs.
pub struct GitHubSource {
    http: Http,
    cache: Mutex<CacheFile>,
    cache_path: std::path::PathBuf,
    limited_until: Mutex<Option<i64>>,
}

impl GitHubSource {
    /// A source that reads `cache_path` at start (a missing or unreadable file gives an empty
    /// cache) and saves there in [`save`](ReleaseSource::save).
    pub fn new(http: Http, cache_path: std::path::PathBuf) -> Self {
        let cache = std::fs::read(&cache_path).ok().and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok()).unwrap_or_default();
        Self { http, cache: Mutex::new(cache), cache_path, limited_until: Mutex::new(None) }
    }

    fn entry(&self, repo: &str, prereleases: bool) -> Option<Entry> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.entries.get(repo).filter(|e| e.prereleases == prereleases).cloned()
    }

    fn store(&self, repo: &str, entry: Entry) {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner).entries.insert(repo.to_string(), entry);
    }

    fn limited(&self) -> bool {
        let until = *self.limited_until.lock().unwrap_or_else(PoisonError::into_inner);
        until.is_some_and(|u| u > crate::now())
    }

    fn ask_api(&self, repo: &str, prereleases: bool) -> Result<Option<Answer>, String> {
        let url = if prereleases {
            format!("https://api.github.com/repos/{repo}/releases?per_page=30")
        } else {
            format!("https://api.github.com/repos/{repo}/releases/latest")
        };
        let saved = self.entry(repo, prereleases);
        let found =
            |release: Option<Release>| Answer { latest: release.map_or(LatestRelease::None, LatestRelease::Found), source: Source::GitHubApi };
        match self.http.get_api(&url, saved.as_ref().and_then(|e| e.etag.as_deref()))? {
            Fetched::Body { text, etag } => {
                let release = if prereleases {
                    release::parse_github_release_list(&text, repo, true)?
                } else {
                    Some(release::parse_github_release(&text, repo)?)
                };
                self.store(repo, Entry { etag, release: release.clone(), prereleases, fetched_at: crate::now() });
                Ok(Some(found(release)))
            }
            Fetched::NotModified => match saved {
                Some(e) => {
                    self.store(repo, Entry { fetched_at: crate::now(), ..e.clone() });
                    Ok(Some(found(e.release)))
                }
                None => Err("GitHub said nothing changed, but there's no saved answer".into()),
            },
            Fetched::NotFound => {
                self.store(repo, Entry { etag: None, release: None, prereleases, fetched_at: crate::now() });
                Ok(Some(found(None)))
            }
            Fetched::RateLimited { reset } => {
                *self.limited_until.lock().unwrap_or_else(PoisonError::into_inner) = Some(retry_at(reset, crate::now()));
                Ok(None)
            }
        }
    }

    /// The quota-free route through github.com (stable releases only).
    fn ask_pages(&self, repo: &str) -> Result<Answer, String> {
        let location = self.http.redirect_target(&format!("https://github.com/{repo}/releases/latest"))?;
        let Some(tag) = location.as_deref().and_then(|l| release::tag_from_redirect(l, repo)) else {
            // No published release: github.com sends us to the release list instead.
            return Ok(Answer { latest: LatestRelease::None, source: Source::GitHubPages });
        };
        if let Some(e) = self.entry(repo, false)
            && let Some(r) = e.release.as_ref().filter(|r| r.tag == tag)
        {
            return Ok(Answer { latest: LatestRelease::Found(r.clone()), source: Source::GitHubPages });
        }
        // The same link `release_from_sums` stores and the check before the download accepts.
        let sums_url = release::expected_download_url(repo, &tag, "SHA256SUMS.txt").ok_or("the latest release's tag isn't one we'd fetch")?;
        let sums = self.http.get_text(&sums_url)?.map(|t| release::parse_sha256sums(&t)).unwrap_or_default();
        let r = release::release_from_sums(repo, &tag, &sums);
        self.store(repo, Entry { etag: None, release: Some(r.clone()), prereleases: false, fetched_at: crate::now() });
        Ok(Answer { latest: LatestRelease::Found(r), source: Source::GitHubPages })
    }
}

/// GitHub's quota refills within the hour, so a later reset is a bad answer.
const MAX_LIMIT_WAIT: i64 = 60 * 60;

/// When to ask the API again after a refusal.
fn retry_at(reset: Option<i64>, now: i64) -> i64 {
    reset.unwrap_or(now + 15 * 60).min(now + MAX_LIMIT_WAIT)
}

impl ReleaseSource for GitHubSource {
    fn latest(&self, repo: &str, prereleases: bool) -> Answer {
        let api_error = if self.limited() {
            None
        } else {
            match self.ask_api(repo, prereleases) {
                Ok(Some(answer)) => return answer,
                Ok(None) => None,
                Err(e) => Some(e),
            }
        };
        if !prereleases && api_error.is_none() {
            match self.ask_pages(repo) {
                Ok(answer) => return answer,
                Err(e) => log::warn!("{repo}: release pages: {e}"),
            }
        }
        match self.cached(repo, prereleases) {
            Some(latest) => Answer { latest, source: Source::Cache },
            None => Answer { latest: api_error.map_or(LatestRelease::RateLimited, LatestRelease::Failed), source: Source::Cache },
        }
    }

    fn cached(&self, repo: &str, prereleases: bool) -> Option<LatestRelease> {
        self.entry(repo, prereleases).map(|e| e.release.map_or(LatestRelease::None, LatestRelease::Found))
    }

    fn newest_tag(&self, repo: &str) -> Option<String> {
        match self.http.redirect_target(&format!("https://github.com/{repo}/releases/latest")) {
            Ok(location) => location.as_deref().and_then(|l| release::tag_from_redirect(l, repo)),
            Err(e) => {
                log::debug!("{repo}: couldn't read the latest-release redirect: {e}");
                None
            }
        }
    }

    fn rate_limited_until(&self) -> Option<i64> {
        let until = *self.limited_until.lock().unwrap_or_else(PoisonError::into_inner);
        until.filter(|u| *u > crate::now())
    }

    fn save(&self) {
        let json = {
            let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            serde_json::to_vec(&*cache)
        };
        match json {
            Ok(bytes) => {
                if let Err(e) = crate::paths::write_atomic(&self.cache_path, &bytes) {
                    log::warn!("release cache: {e}");
                }
            }
            Err(e) => log::warn!("release cache: {e}"),
        }
    }
}

/// Release source that defaults to the signed `getartcraft.com` catalog and falls back to
/// `fallback` (typically [`GitHubSource`]) when the catalog is unavailable, stale, or omits a repo.
pub struct CatalogSource<F: ReleaseSource = GitHubSource> {
    http: Http,
    fallback: F,
    cached: Mutex<Option<crate::catalog::SignedCatalog>>,
}

impl<F: ReleaseSource> CatalogSource<F> {
    /// Creates a release source that checks the signed `getartcraft.com` catalog first and falls
    /// back to `fallback` (typically [`GitHubSource`]).
    pub fn new(http: Http, fallback: F) -> Self {
        Self { http, fallback, cached: Mutex::new(None) }
    }

    fn catalog(&self) -> Option<crate::catalog::SignedCatalog> {
        let now = crate::now();
        {
            let guard = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(c) = guard.as_ref()
                && c.generated_at.is_some_and(|t| now.saturating_sub(t) <= crate::catalog::MAX_AGE)
            {
                return Some(c.clone());
            }
        }
        match crate::catalog::SignedCatalog::fetch(&self.http) {
            Ok(fresh) => {
                *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(fresh.clone());
                Some(fresh)
            }
            Err(why) => {
                log::debug!("signed catalog unavailable ({why}); falling back to GitHub");
                None
            }
        }
    }
}

impl<F: ReleaseSource> ReleaseSource for CatalogSource<F> {
    fn latest(&self, repo: &str, prereleases: bool) -> Answer {
        if let Some(catalog) = self.catalog()
            && let Some(release) = catalog.latest_for_repo(repo, prereleases, crate::now())
        {
            return Answer { latest: LatestRelease::Found(release), source: Source::Feed };
        }
        self.fallback.latest(repo, prereleases)
    }

    fn cached(&self, repo: &str, prereleases: bool) -> Option<LatestRelease> {
        if let Some(c) = self.cached.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
            && let Some(r) = c.latest_for_repo(repo, prereleases, crate::now())
        {
            return Some(LatestRelease::Found(r));
        }
        self.fallback.cached(repo, prereleases)
    }

    fn newest_tag(&self, repo: &str) -> Option<String> {
        self.fallback.newest_tag(repo)
    }

    fn rate_limited_until(&self) -> Option<i64> {
        self.fallback.rate_limited_until()
    }

    fn save(&self) {
        self.fallback.save();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;

    #[trial]
    fn a_rate_limited_source_with_nothing_saved_answers_rate_limited_not_english_text() {
        // Nothing is written (a rate-limited answer saves nothing), and a name only this run uses has no cache to read.
        let dir = std::env::temp_dir().join(format!("craft-update-source-rate-limited-{}-{}", std::process::id(), crate::now()));
        let http = Http::new("craft-update-test/0.0").expect("an HTTP client");
        let source = GitHubSource::new(http, dir.join("releases.json"));
        *source.limited_until.lock().expect("lock") = Some(crate::now() + 600);
        assert_eq!(source.latest("storytold/none", true).latest, LatestRelease::RateLimited);
    }

    #[trial]
    fn a_reset_past_the_quota_window_cannot_stop_the_api_for_good() {
        assert_eq!(retry_at(Some(i64::MAX), 1_000), 1_000 + 60 * 60);
        assert_eq!(retry_at(Some(99_999_999_999), 1_000), 1_000 + 60 * 60);
    }

    /// A fallback that answers "no release" and records nothing, so a test sees which source answered.
    struct Fallback;

    impl ReleaseSource for Fallback {
        fn latest(&self, _repo: &str, _prereleases: bool) -> Answer {
            Answer { latest: LatestRelease::None, source: Source::GitHubApi }
        }

        fn cached(&self, _repo: &str, _prereleases: bool) -> Option<LatestRelease> {
            Some(LatestRelease::RateLimited)
        }

        fn newest_tag(&self, _repo: &str) -> Option<String> {
            Some("v9.9.9".into())
        }

        fn rate_limited_until(&self) -> Option<i64> {
            Some(42)
        }

        fn save(&self) {}
    }

    fn catalog_source() -> CatalogSource<Fallback> {
        let source = CatalogSource::new(Http::new("craft-update-test/0.0").expect("an HTTP client"), Fallback);
        *source.cached.lock().expect("lock") = Some(crate::catalog::tests::fresh_signed_catalog());
        source
    }

    #[trial]
    fn the_catalog_answers_for_a_repo_it_lists_and_marks_the_answer_as_the_feed() {
        let answer = catalog_source().latest("storytold/photocraft", false);
        assert_eq!(answer.source, Source::Feed);
        assert!(matches!(&answer.latest, LatestRelease::Found(release) if release.tag == "v0.23.1"), "{:?}", answer.latest);
    }

    #[trial]
    fn a_repo_the_catalog_omits_falls_back_to_the_other_source() {
        let answer = catalog_source().latest("storytold/unlisted", false);
        assert_eq!(answer.source, Source::GitHubApi);
        assert_eq!(answer.latest, LatestRelease::None);
    }

    #[trial]
    fn the_saved_answer_comes_from_the_catalog_or_else_the_fallback() {
        let source = catalog_source();
        assert!(matches!(source.cached("storytold/photocraft", true), Some(LatestRelease::Found(release)) if release.tag == "v0.24.0-rc.1"));
        assert_eq!(source.cached("storytold/unlisted", false), Some(LatestRelease::RateLimited));
    }

    #[trial]
    fn tags_quota_and_saving_are_the_fallbacks_business() {
        let source = catalog_source();
        assert_eq!(source.newest_tag("storytold/photocraft").as_deref(), Some("v9.9.9"));
        assert_eq!(source.rate_limited_until(), Some(42));
        source.save();
    }

    #[trial]
    fn a_stale_cached_catalog_is_not_used() {
        let source = CatalogSource::new(Http::new("craft-update-test/0.0").expect("an HTTP client"), Fallback);
        let stale = crate::catalog::tests::sample_toml().replace("2026-11-26T21:00:00Z", "2020-01-01T00:00:00Z");
        *source.cached.lock().expect("lock") = Some(crate::catalog::tests::signed(&stale));
        assert_eq!(source.cached("storytold/photocraft", false), Some(LatestRelease::RateLimited));
    }
}
