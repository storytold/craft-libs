//! The font store's consent setting, user-added sources and the cached and fetched indexes.

use super::{
    Caller, Cancel, Error, FontStore, INDEX_CACHE, Index, IndexError, IndexPin, MAX_INDEX_BYTES, MAX_SETTINGS_BYTES, MAX_SOURCES_BYTES,
    MAX_USER_SOURCES, Network, PREVIEW_SERVICE_ENV, PathBuf, PreviewServiceSource, SETTINGS, SOURCES_CACHE, Settings, SourceEntry, SourceProblem,
    SourcesFile, SystemTime, UserSource, VerifyError, check_preview_service, io, is_source_url, read_limited, resolve_preview_source, sha256_hex,
    slug,
};

impl FontStore {
    // ----- consent

    /// The user's saved choice, else the host's default ([`FontStore::with_default_network`]),
    /// else [`Network::Off`]. A missing `settings.json` is no choice; one that can't be read or
    /// parsed is [`Network::Off`].
    pub fn network(&self) -> Network {
        self.settings().network.or(self.default_network).unwrap_or_default()
    }

    /// The saved settings: the defaults without a file, [`Settings::unreadable`] for a file that
    /// can't be read, is too large or doesn't parse.
    fn settings(&self) -> Settings {
        let path = self.dir.join(SETTINGS);
        if std::fs::metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
            return Settings::default();
        }
        read_limited(&path, MAX_SETTINGS_BYTES).and_then(|b| serde_json::from_slice::<Settings>(&b).ok()).unwrap_or_else(Settings::unreadable)
    }

    /// Turns downloading on: call it only from the user's own action (a setting, an Install
    /// click). [`Error::NotAllowed`] for an agent.
    pub fn enable_network(&self) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        self.change_settings(|s| s.network = Some(Network::On))
    }

    /// The preview service's base URL, or `None` while there is none: [`PREVIEW_SERVICE_ENV`]
    /// environment variable when set and not empty, else the `preview_service` setting, else the
    /// host's default ([`FontStore::with_default_preview_service`]); no host default while
    /// `settings.json` can't be read or parsed. Installs of curated families
    /// ([`crate::Family::is_curated`]) try its mirror (`<service>/v1/files/<sha256>`,
    /// `<service>/v1/licences/<sha256>`) before the index's URLs. A value that is not a usable URL
    /// counts as unset.
    pub fn preview_service(&self) -> Option<String> {
        self.preview_service_and_source().0
    }

    /// Where [`FontStore::preview_service`] gets its value: the environment, the saved setting,
    /// the host's default, or nowhere. An unusable value counts as unset, as there.
    pub fn preview_service_source(&self) -> PreviewServiceSource {
        self.preview_service_and_source().1
    }

    fn preview_service_and_source(&self) -> (Option<String>, PreviewServiceSource) {
        #[cfg(test)]
        let env = self.preview_env.clone().unwrap_or_else(|| std::env::var(PREVIEW_SERVICE_ENV).ok());
        #[cfg(not(test))]
        let env = std::env::var(PREVIEW_SERVICE_ENV).ok();
        let settings = self.settings();
        let default = if settings.unreadable { None } else { self.default_preview_service.as_deref() };
        resolve_preview_source(env.as_deref(), settings.preview_service.as_deref(), default)
    }

    /// Sets (or with `None` clears) the preview service in `settings.json`, from the user's own
    /// action. Only https URLs are accepted, and http for a loopback host. The environment
    /// variable still takes precedence; while the setting is clear, the host's default applies.
    /// Installs and [`FontStore::legal_text`] ask the service only for curated families
    /// ([`crate::Family::is_curated`]), never with the hashes of files from the user's sources.
    /// [`Error::NotAllowed`] for an agent, [`Error::BadServiceUrl`] for an unusable URL.
    pub fn set_preview_service(&self, url: Option<&str>) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        let url = match url {
            Some(u) => Some(check_preview_service(u)?),
            None => None,
        };
        self.change_settings(|s| s.preview_service = url)
    }

    /// Turns downloading off. Anyone may.
    pub fn disable_network(&self) -> Result<(), Error> {
        self.change_settings(|s| s.network = Some(Network::Off))
    }

    /// Whether the app may fetch font previews from the preview service. On unless the user
    /// turned it off. It matters only while a service is set; it doesn't affect installs.
    pub fn previews_enabled(&self) -> bool {
        self.settings().previews
    }

    /// Turns previews from the preview service on or off, from the user's own action.
    /// [`Error::NotAllowed`] for an agent.
    pub fn set_previews_enabled(&self, enabled: bool) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        self.change_settings(|s| s.previews = enabled)
    }

    /// Whether the user keeps store fonts in Craft apps only by default. Off by default: the font
    /// manager then copies each family it installs into the OS user font folder
    /// ([`FontStore::make_available`]), so other apps can use it, unless the user chose otherwise
    /// for that family ([`crate::InstalledFamily::available_to_all`]).
    pub fn craft_apps_only(&self) -> bool {
        self.settings().craft_apps_only
    }

    /// Sets [`FontStore::craft_apps_only`], from the user's own action. It only records the choice;
    /// installed families keep theirs. [`Error::NotAllowed`] for an agent.
    pub fn set_craft_apps_only(&self, on: bool) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        self.change_settings(|s| s.craft_apps_only = on)
    }

    /// Records the user's choice for the installed family ([`crate::InstalledFamily::available_to_all`]).
    /// It only records it: the caller copies or withdraws the family. [`Error::NotAllowed`] for an
    /// agent, [`Error::NotInstalled`] for a family that isn't installed.
    pub fn set_available_to_all_choice(&self, source: &str, name: &str, on: bool) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        let _lock = self.lock()?;
        let mut manifest = self.read_manifest();
        let entry = manifest.families.iter_mut().find(|f| f.is(source, name)).ok_or_else(|| Error::NotInstalled(name.to_string()))?;
        entry.available_to_all = Some(on);
        self.write_manifest(&manifest)
    }

    fn change_settings(&self, change: impl FnOnce(&mut Settings)) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut settings = self.settings();
        change(&mut settings);
        let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| io("saving the setting")(std::io::Error::other(e)))?;
        self.write_file(SETTINGS, &bytes)
    }

    // ----- sources the user added

    /// The sources the user added, in the order added.
    pub fn user_sources(&self) -> Vec<UserSource> {
        self.settings().user_sources
    }

    /// Adds an index file at `url` as a source, from the user's own action; the next
    /// [`FontStore::fetch_indexes`] downloads it. Its families are never curated.
    /// [`Error::NotAllowed`] for an agent.
    pub fn add_user_source(&self, url: &str) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        let url = url.trim();
        if !is_source_url(url) {
            return Err(Error::BadUrl(url.chars().take(200).collect()));
        }
        let added_at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.change_settings(|s| {
            if !s.user_sources.iter().any(|u| u.url == url) && s.user_sources.len() < MAX_USER_SOURCES {
                s.user_sources.push(UserSource { url: url.to_string(), added_at, last_sha256: None });
            }
        })
    }

    /// Removes a source the user added, and its cached index. Families installed from it stay
    /// installed.
    pub fn remove_user_source(&self, url: &str) -> Result<(), Error> {
        self.change_settings(|s| s.user_sources.retain(|u| u.url != url))?;
        let _ = std::fs::remove_file(self.user_cache(url));
        Ok(())
    }

    fn user_cache(&self, url: &str) -> PathBuf {
        let key: String = sha256_hex(url.as_bytes()).chars().take(16).collect();
        self.dir.join(INDEX_CACHE).join(format!("user-{key}"))
    }

    pub(crate) fn require_network(&self) -> Result<(), Error> {
        if self.network() == Network::On { Ok(()) } else { Err(Error::NetworkOff) }
    }

    // ----- index

    /// The indexes saved by the last [`FontStore::fetch_indexes`], when its `sources.json` is
    /// `pin`'s and every built-in index matches it, with the sources the user added.
    pub fn cached_indexes(&self, pin: &IndexPin) -> Option<Index> {
        let cache = self.dir.join(INDEX_CACHE);
        let bytes = read_limited(&cache.join(SOURCES_CACHE), MAX_SOURCES_BYTES as u64)?;
        if sha256_hex(&bytes) != pin.sha256 {
            return None;
        }
        let sources = SourcesFile::from_json(&bytes).ok()?;
        let mut parts = Vec::new();
        for entry in &sources.indexes {
            let bytes = read_limited(&cache.join(builtin_cache_name(entry)), MAX_INDEX_BYTES as u64)?;
            parts.push(entry.read(&bytes, entry.gz().is_some()).ok()?);
        }
        let (users, problems) = self.cached_user_indexes();
        Some(Index::merge(parts.into_iter().chain(users)).with_problems(problems))
    }

    /// The indexes without a request: the built-in ones compiled in, with the sources the user
    /// added as last fetched.
    #[cfg(feature = "bundled-index")]
    pub fn offline_indexes(&self) -> Index {
        let (users, mut problems) = self.cached_user_indexes();
        let builtin = Index::bundled().unwrap_or_else(|e| {
            problems.insert(0, SourceProblem { source: "sources.json".into(), name: "Built-in sources".into(), error: e.to_string() });
            Index::default()
        });
        Index::merge(std::iter::once(builtin).chain(users)).with_problems(problems)
    }

    fn cached_user_indexes(&self) -> (Vec<Index>, Vec<SourceProblem>) {
        let mut indexes = Vec::new();
        let mut problems = Vec::new();
        for user in self.user_sources() {
            let problem = |error: String| SourceProblem { source: user.url.clone(), name: user.url.clone(), error };
            let Some(bytes) = read_limited(&self.user_cache(&user.url), MAX_INDEX_BYTES as u64) else {
                problems.push(problem("not downloaded yet".into()));
                continue;
            };
            match user_index(&bytes, &user) {
                Ok(index) => indexes.push(index),
                Err(e) => problems.push(problem(e.to_string())),
            }
        }
        (indexes, problems)
    }

    /// Downloads `sources.json` as `pin` names it, every index it lists and every source the user
    /// added, checks them and saves them for [`FontStore::cached_indexes`]. Needs the network on.
    ///
    /// An index that fails leaves the others usable: it is reported in [`Index::problems`]. Only
    /// a `sources.json` that can't be had fails the whole call.
    pub fn fetch_indexes(&self, pin: &IndexPin, cancel: &Cancel) -> Result<Index, Error> {
        self.require_network()?;
        let check = |bytes: &[u8]| if sha256_hex(bytes) == pin.sha256 { Ok(()) } else { Err(VerifyError::Sha256) };
        let bytes = self.fetch(&pin.urls(), 0, MAX_SOURCES_BYTES as u64, "sources.json", cancel, &mut |_| {}, &check)?;
        let sources = SourcesFile::from_json(&bytes)?;
        let cache = self.dir.join(INDEX_CACHE);
        std::fs::create_dir_all(&cache).map_err(io("creating the index folder"))?;
        let mut parts = Vec::new();
        let mut problems = Vec::new();
        for entry in &sources.indexes {
            match self.fetch_builtin(pin, entry, cancel) {
                Ok(index) => parts.push(index),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => problems.push(SourceProblem { source: entry.source.clone(), name: entry.name.clone(), error: e.to_string() }),
            }
        }
        self.write_file(&format!("{INDEX_CACHE}/{SOURCES_CACHE}"), &bytes)?;
        for user in self.user_sources() {
            match self.fetch_user(&user, cancel) {
                Ok(index) => parts.push(index),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => problems.push(SourceProblem { source: user.url.clone(), name: user.url.clone(), error: e.to_string() }),
            }
        }
        Ok(Index::merge(parts).with_problems(problems))
    }

    fn fetch_builtin(&self, pin: &IndexPin, entry: &SourceEntry, cancel: &Cancel) -> Result<Index, Error> {
        let (path, size, sha256) = entry.gz().unwrap_or((&entry.path, entry.size, &entry.sha256));
        let check = |bytes: &[u8]| {
            if bytes.len() as u64 != size {
                Err(VerifyError::Size { expected: size, actual: bytes.len() as u64 })
            } else if sha256_hex(bytes) != sha256 {
                Err(VerifyError::Sha256)
            } else {
                Ok(())
            }
        };
        let bytes = self.fetch(&pin.file_urls(path), 0, size, &entry.name, cancel, &mut |_| {}, &check)?;
        let index = entry.read(&bytes, entry.gz().is_some())?;
        self.write_file(&format!("{INDEX_CACHE}/{}", builtin_cache_name(entry)), &bytes)?;
        Ok(index)
    }

    fn fetch_user(&self, user: &UserSource, cancel: &Cancel) -> Result<Index, Error> {
        let bytes = self.fetch(std::slice::from_ref(&user.url), 0, MAX_INDEX_BYTES as u64, &user.url, cancel, &mut |_| {}, &|_| Ok(()))?;
        let index = user_index(&bytes, user)?;
        if let Some(name) = self.user_cache(&user.url).file_name().and_then(|n| n.to_str()) {
            self.write_file(&format!("{INDEX_CACHE}/{name}"), &bytes)?;
        }
        if user.last_sha256.is_none() {
            let sha = sha256_hex(&bytes);
            self.change_settings(|s| {
                if let Some(u) = s.user_sources.iter_mut().find(|u| u.url == user.url && u.last_sha256.is_none()) {
                    u.last_sha256 = Some(sha);
                }
            })?;
        }
        Ok(index)
    }
}

fn builtin_cache_name(entry: &SourceEntry) -> String {
    format!("{}{}", slug(&entry.source), if entry.gz().is_some() { ".json.gz" } else { ".json" })
}

/// A user-added source's index from the bytes fetched from it.
fn user_index(bytes: &[u8], user: &UserSource) -> Result<Index, IndexError> {
    let mut index = Index::from_user_bytes(bytes, &user.url)?;
    if user.last_sha256.as_ref().is_some_and(|h| *h != sha256_hex(bytes)) {
        index.mark_changed();
    }
    Ok(index)
}
