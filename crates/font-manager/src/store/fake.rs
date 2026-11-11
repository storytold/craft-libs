//! The in-memory font store used by tests.

use super::*;

/// What a [`FakeStore`] was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Call {
    /// Asked to install the family.
    Install(String),
    /// The family and the file's path in the index.
    InstallFile(String, String),
    /// Asked to remove the family.
    Remove(String),
    /// Asked to download the family.
    Download(String),
}

/// A font store without a network: "downloads" come from a table of file bytes by index path,
/// and installs write them into a folder as the real store does, `<dir>/<family>/<file>`. It
/// refuses to download while downloads are off, like the real one.
pub struct FakeStore {
    dir: PathBuf,
    files: HashMap<String, Vec<u8>>,
    allowed: AtomicBool,
    calls: Mutex<Vec<Call>>,
    failure: Mutex<Option<String>>,
    /// A google/fonts checkout to read files missing from `files` from.
    folder: Option<PathBuf>,
    /// The built-in indexes; the bundled ones when not given.
    builtin: Option<Index>,
    /// Index files user sources "download", by URL.
    served_indexes: HashMap<String, Vec<u8>>,
    user: Mutex<Vec<UserSource>>,
    /// User sources downloaded so far, and those whose file changed since.
    fetched: Mutex<Vec<String>>,
    changed: Mutex<Vec<String>>,
    /// Downloads stop halfway while this is true.
    held: (Mutex<bool>, Condvar),
}

impl FakeStore {
    /// A store that installs into `dir` and serves downloads from `files`, keyed by index path.
    pub fn new(dir: impl Into<PathBuf>, files: HashMap<String, Vec<u8>>) -> FakeStore {
        FakeStore {
            dir: dir.into(),
            files,
            allowed: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
            folder: None,
            builtin: None,
            served_indexes: HashMap::new(),
            user: Mutex::new(Vec::new()),
            fetched: Mutex::new(Vec::new()),
            changed: Mutex::new(Vec::new()),
            held: (Mutex::new(false), Condvar::new()),
        }
    }

    /// Uses `index` as the built-in indexes.
    pub fn with_index(mut self, index: Index) -> FakeStore {
        self.builtin = Some(index);
        self
    }

    /// Has the index at `url` "download" as `bytes` once added as a source.
    pub fn serving_index(mut self, url: &str, bytes: Vec<u8>) -> FakeStore {
        self.served_indexes.insert(url.to_owned(), bytes);
        self
    }

    /// The index at `url` now differs from the one first downloaded.
    pub fn change_index(&self, url: &str) {
        if let Ok(mut changed) = self.changed.lock() {
            changed.push(url.to_owned());
        }
    }

    /// Also "downloads" files from a google/fonts checkout, by their path in the index.
    pub fn serving(mut self, checkout: impl Into<PathBuf>) -> FakeStore {
        self.folder = Some(checkout.into());
        self
    }

    /// The next download fails with `error`, as when the network is down.
    pub fn fail_next(&self, error: &str) {
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(error.to_owned());
        }
    }

    /// Downloads report half their bytes, then wait until [`FakeStore::release`] (with
    /// `Runner::Thread`).
    pub fn hold(&self) {
        if let Ok(mut held) = self.held.0.lock() {
            *held = true;
        }
    }

    /// Let held downloads finish; see [`FakeStore::hold`].
    pub fn release(&self) {
        if let Ok(mut held) = self.held.0.lock() {
            *held = false;
        }
        self.held.1.notify_all();
    }

    fn wait_while_held(&self) {
        let Ok(mut held) = self.held.0.lock() else { return };
        while *held {
            match self.held.1.wait(held) {
                Ok(h) => held = h,
                Err(_) => return,
            }
        }
    }

    /// Everything asked of it, downloads refused for lack of consent included.
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    fn record(&self, call: Call) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    fn fetch(&self, family: &Family, only: Option<&str>, progress: &mut dyn FnMut(Progress)) -> Result<Vec<Downloaded>, StoreError> {
        if !self.downloads_allowed() {
            return Err(StoreError("downloading fonts is turned off".into()));
        }
        if let Some(error) = self.failure.lock().ok().and_then(|mut f| f.take()) {
            return Err(StoreError(error));
        }
        let wanted: Vec<_> = family.files.iter().filter(|f| only.is_none_or(|p| p == f.path)).collect();
        let total = wanted.iter().map(|f| f.size).sum();
        progress(Progress { done: total / 2, total });
        self.wait_while_held();
        let mut done = 0u64;
        let mut out = Vec::new();
        for file in wanted {
            let from_folder = || self.folder.as_ref().and_then(|dir| std::fs::read(dir.join(&file.path)).ok());
            let bytes =
                self.files.get(&file.path).cloned().or_else(from_folder).ok_or_else(|| StoreError(format!("{} isn't available", file.path)))?;
            done = done.saturating_add(file.size);
            progress(Progress { done, total });
            out.push(Downloaded { path: file.path.clone(), bytes });
        }
        Ok(out)
    }

    fn write(&self, family: &Family, files: Vec<Downloaded>) -> Result<(), StoreError> {
        let dir = self.dir.join(&family.family);
        std::fs::create_dir_all(&dir).map_err(err)?;
        for file in files {
            let name = file.path.rsplit('/').next().unwrap_or(&file.path);
            std::fs::write(dir.join(name), &file.bytes).map_err(err)?;
        }
        Ok(())
    }

    fn in_store(&self, path: &Path) -> bool {
        path.starts_with(&self.dir)
    }
}

impl Installer for FakeStore {
    fn downloads_allowed(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }

    fn allow_downloads(&self, allowed: bool) -> Result<(), StoreError> {
        self.allowed.store(allowed, Ordering::SeqCst);
        Ok(())
    }

    fn install(&self, _: &Index, family: &Family, progress: &mut dyn FnMut(Progress)) -> Result<(), StoreError> {
        self.record(Call::Install(family.family.clone()));
        let files = self.fetch(family, None, progress)?;
        self.write(family, files)
    }

    fn install_file(&self, _: &Index, family: &Family, path: &str, progress: &mut dyn FnMut(Progress)) -> Result<(), StoreError> {
        self.record(Call::InstallFile(family.family.clone(), path.to_owned()));
        let files = self.fetch(family, Some(path), progress)?;
        self.write(family, files)
    }

    fn remove(&self, request: &RemoveRequest) -> Result<(), StoreError> {
        self.record(Call::Remove(request.family.clone()));
        if let Some(outside) = request.files.iter().find(|f| !self.in_store(f)) {
            return Err(StoreError(format!("{} isn't in the font store", outside.display())));
        }
        for file in &request.files {
            std::fs::remove_file(file).map_err(err)?;
        }
        Ok(())
    }

    fn download(&self, _: &Index, family: &Family, progress: &mut dyn FnMut(Progress)) -> Result<Vec<Downloaded>, StoreError> {
        self.record(Call::Download(family.family.clone()));
        self.fetch(family, None, progress)
    }

    fn indexes(&self) -> Result<Index, StoreError> {
        let builtin = match &self.builtin {
            Some(index) => index.clone(),
            None => crate::index::bundled().map_err(err)?,
        };
        let fetched = self.fetched.lock().map(|f| f.clone()).unwrap_or_default();
        let changed = self.changed.lock().map(|c| c.clone()).unwrap_or_default();
        let mut users = Vec::new();
        let mut problems = Vec::new();
        for source in self.user_sources() {
            let problem = |error: String| craft_font_store::SourceProblem { source: source.url.clone(), name: source.url.clone(), error };
            if !fetched.contains(&source.url) {
                problems.push(problem("not downloaded yet".into()));
                continue;
            }
            match self.served_indexes.get(&source.url).map(|bytes| Index::from_user_bytes(bytes, &source.url)) {
                Some(Ok(mut index)) => {
                    if changed.contains(&source.url) {
                        index.mark_changed();
                    }
                    users.push(index);
                }
                Some(Err(e)) => problems.push(problem(e.to_string())),
                None => problems.push(problem("nothing at this address".into())),
            }
        }
        Ok(Index::merge(std::iter::once(builtin).chain(users)).with_problems(problems))
    }

    fn fetch_indexes(&self) -> Result<Index, StoreError> {
        if !self.downloads_allowed() {
            return Err(StoreError("downloading fonts is turned off".into()));
        }
        if let Ok(mut fetched) = self.fetched.lock() {
            *fetched = self.user_sources().into_iter().map(|s| s.url).collect();
        }
        self.indexes()
    }

    fn user_sources(&self) -> Vec<UserSource> {
        self.user.lock().map(|u| u.clone()).unwrap_or_default()
    }

    fn add_user_source(&self, url: &str) -> Result<(), StoreError> {
        if !url.starts_with("https://") {
            return Err(StoreError("an index address starts with https://".into()));
        }
        let mut user = self.user.lock().map_err(err)?;
        if !user.iter().any(|s| s.url == url) {
            user.push(UserSource { url: url.to_owned(), added_at: 0, last_sha256: None });
        }
        Ok(())
    }

    fn remove_user_source(&self, url: &str) -> Result<(), StoreError> {
        self.user.lock().map_err(err)?.retain(|s| s.url != url);
        Ok(())
    }
}
