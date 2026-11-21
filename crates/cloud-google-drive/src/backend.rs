//! [`GoogleDriveBackend`]: the shared pages' jobs, answered against Google Drive.
//!
//! Every job runs on its own thread and answers through its [`Post`]; `submit` returns at once.
//! One [`Drive`] client sits behind a lock, so jobs touching Drive take turns. A sign-in waits
//! for the browser without holding it, and a sign-out removes the stored token before it waits
//! for a transfer in flight.
//!
//! The pages browse by slash-separated path; Drive addresses folders by id. Paths come from
//! listings only: a folder's path is its parent's path and its name, with a `/` in the name shown
//! as `∕`, and same-named folders in one folder numbered by id (`Reports`, `Reports (2)`), so the
//! same folder always gets the same path and a path names one folder. The backend remembers the
//! id of every folder path it has handed out. A path it doesn't know (after a sign-out, or one a
//! host kept) is found by listing each parent the same way, never by searching for a name.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use craft_cloud::{Capability, Entry, EntryKind};
use craft_ui::cloud::{Account, CloudBackend, Event, Fault, Job, Post};

use crate::drive::{Drive, DriveFile, Listing, SHARED_QUERY, SaveTarget, folder_query};
use crate::http::Transport;
use crate::oauth::{Auth, ClientConfig, SCOPE_DRIVE, Store, auth_url, token_handle};
use crate::{Error, Result};

/// How long a sign-in waits for the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The most folder paths remembered. Past this the map starts again before the next listing;
/// a forgotten path is found again by listing its parents.
const MAX_FOLDERS: usize = 10_000;
/// Downloads of a file that keeps changing before Open gives up.
const READ_ATTEMPTS: usize = 3;
/// The most sign-ins remembered between their start and the page's poll.
const MAX_LINKS: usize = 16;

/// Google Drive behind the shared Open and Save pages.
///
/// Built from the application's [`ClientConfig`] (or why there is none, which every job then
/// says), connected with [`craft_ui::cloud::CloudFiles::connect`]. A changed registration is a
/// new backend, connected again.
pub struct GoogleDriveBackend {
    inner: Arc<Inner>,
}

struct Inner {
    /// The client, or why there is none.
    drive: Mutex<std::result::Result<Drive, String>>,
    /// The registration and its token's keychain entry, readable without waiting for a transfer.
    config: Option<ClientConfig>,
    store: Store,
    /// The session's permission to store its token ([`Auth::keeping`]), reachable without the
    /// Drive lock.
    keep: Option<Arc<Mutex<bool>>>,
    /// Path → folder id, for every folder path handed out. `""` is My Drive.
    folders: Mutex<HashMap<String, String>>,
    /// Sign-ins started by [`Job::LinkStart`], by the handle the page polls.
    links: Mutex<HashMap<String, Linking>>,
    /// The sign-in waiting for the browser, if one is: a new one, or a sign-out, stops it.
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// Named on the page the browser shows after signing in.
    app: String,
    timeout: Duration,
}

#[derive(Clone, Debug)]
enum Linking {
    Waiting,
    Done,
    Failed(Fault),
}

/// How a sign-in ended, as far as the page that started it is concerned.
enum SignedIn {
    Yes,
    /// Refused before the browser opened.
    Refused(Fault),
    /// Failed after the browser opened.
    Failed(Fault),
    /// A newer sign-in, or a sign-out, took over: that one answers the page.
    Superseded,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `job` on a thread of its own. When the system has no thread to give, the job never
/// answers, which the pages survive by contract.
fn spawn(job: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name("craft-google-drive".into()).spawn(job) {
        log::warn!("Google Drive: no worker thread: {e}");
    }
}

/// What the pages act on, from what went wrong.
pub(crate) fn fault(error: Error) -> Fault {
    match error {
        // No sign-in, or Google refused the stored one: the page offers to link again.
        Error::NotSignedIn => Fault::NotLinked,
        Error::Conflict { .. } => Fault::Conflict,
        Error::Network(detail) => Fault::Unreachable(detail),
        // Too many requests, or Google having a bad moment: worth trying again shortly.
        Error::Api { status, message } if status == 429 || status >= 500 => Fault::Unreachable(message),
        e => Fault::Refused(e.to_string()),
    }
}

/// A Drive file as the pages show it, at `path`.
pub(crate) fn entry(file: &DriveFile, path: String) -> Entry {
    let etag = file.content_version();
    Entry {
        id: file.id.clone(),
        // Drive ids survive a rename and a move.
        room: file.id.clone(),
        name: file.open_name(),
        path,
        kind: if file.is_folder() { EntryKind::Folder } else { EntryKind::File },
        size: file.size,
        modified: Some(file.modified_time.clone()).filter(|m| !m.is_empty()),
        mime: Some(file.open_mime().to_owned()).filter(|m| !m.is_empty()),
        etag: Some(etag.to_owned()).filter(|e| !e.is_empty()),
        writable: file.writable(),
        shared_by: file.sharing_user.as_ref().map(|u| u.display_name.trim().to_owned()).filter(|n| !n.is_empty()),
    }
}

/// One path segment for a Drive name: a `/` in it would read as a folder boundary.
pub(crate) fn segment(name: &str) -> String {
    name.replace('/', "\u{2215}")
}

/// The path of `name` inside the folder at `parent`.
fn child_path(parent: &str, name: &str) -> String {
    format!("{}/{}", parent.trim_end_matches('/'), segment(name))
}

/// The path of every entry of the folder at `parent`, in `files`' order. Same-named folders
/// are numbered in id order, and the numbers are handed out in (name, id) order, so the paths
/// don't depend on the order Drive answered in. A number that would collide with a real name
/// moves on to the next free one.
pub(crate) fn child_paths(parent: &str, files: &[DriveFile]) -> Vec<String> {
    let mut folders: Vec<(String, &str)> = files.iter().filter(|f| f.is_folder()).map(|f| (child_path(parent, &f.name), f.id.as_str())).collect();
    folders.sort_unstable();
    let mut taken: HashSet<String> = folders.iter().map(|(base, _)| base.clone()).collect();
    let mut by_id: HashMap<&str, String> = HashMap::new();
    let mut previous: Option<&str> = None;
    for (base, id) in &folders {
        if previous != Some(base.as_str()) {
            previous = Some(base.as_str());
            by_id.insert(id, base.clone());
            continue;
        }
        let mut n = 2;
        loop {
            let at = format!("{base} ({n})");
            if taken.insert(at.clone()) {
                by_id.insert(id, at);
                break;
            }
            n += 1;
        }
    }
    files
        .iter()
        .map(|f| match f.is_folder().then(|| by_id.get(f.id.as_str())).flatten() {
            Some(at) => at.clone(),
            None => child_path(parent, &f.name),
        })
        .collect()
}

impl GoogleDriveBackend {
    /// The backend over the system keychain and the network. `config` is the application's
    /// client, or why it has none (shown as the account's fault). `app` names the application on
    /// the page the browser shows after signing in, and to Google as the user agent.
    pub fn new(config: Result<ClientConfig>, app: &str) -> GoogleDriveBackend {
        let http: Arc<dyn Transport> = Arc::new(crate::http::Ureq::new(app));
        GoogleDriveBackend::with(config, http, Arc::new(craft_suite_account::Keychain), app)
    }

    /// The client from `<env_prefix>_GOOGLE_CLIENT_ID`, else `google-drive.json` in `profile`.
    pub fn from_profile(profile: Option<&Path>, env_prefix: &str, app: &str) -> GoogleDriveBackend {
        GoogleDriveBackend::new(ClientConfig::load(profile, env_prefix), app)
    }

    /// The backend over a given transport and token store (tests, or a host with its own).
    pub fn with(config: Result<ClientConfig>, http: Arc<dyn Transport>, store: Store, app: &str) -> GoogleDriveBackend {
        let kept = config.as_ref().ok().cloned();
        let drive = config.map(|cfg| Drive::new(Auth::new(cfg, http, store.clone()))).map_err(|e| e.to_string());
        let keep = drive.as_ref().ok().map(|d| d.auth.keeping());
        GoogleDriveBackend {
            inner: Arc::new(Inner {
                drive: Mutex::new(drive),
                config: kept,
                store,
                keep,
                folders: Mutex::new(HashMap::new()),
                links: Mutex::new(HashMap::new()),
                cancel: Mutex::new(None),
                app: app.to_owned(),
                timeout: SIGN_IN_TIMEOUT,
            }),
        }
    }

    /// How long a sign-in waits for the browser (five minutes unless changed; `Duration::MAX`
    /// waits until it is cancelled).
    pub fn sign_in_timeout(mut self, timeout: Duration) -> GoogleDriveBackend {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.timeout = timeout;
        }
        self
    }
}

impl Inner {
    /// Runs `f` with the signed-in client: a missing client is refused, and a stored sign-in is
    /// picked up first.
    fn drive<T>(&self, f: impl FnOnce(&mut Drive) -> Result<T>) -> std::result::Result<T, Fault> {
        let mut guard = lock(&self.drive);
        let drive = guard.as_mut().map_err(|why| Fault::Refused(why.clone()))?;
        if !drive.auth.restore().map_err(fault)? {
            return Err(Fault::NotLinked);
        }
        f(drive).map_err(fault)
    }

    fn account(&self) -> std::result::Result<Account, Fault> {
        let (display, full) = self.drive(|d| Ok((d.who()?, d.auth.config().scope == SCOPE_DRIVE)))?;
        let mut capabilities = vec![Capability::Files, Capability::Etags];
        // With the narrower scope Drive lists only files this application made: nothing shared.
        if full {
            capabilities.push(Capability::SharedWithMe);
        }
        Ok(Account { display, capabilities, provider_linked: true })
    }

    /// One folder's files and their paths, remembering the id of every subfolder's path. Called
    /// with the Drive lock held, so a sign-out can't clear the map in the middle.
    fn children(&self, drive: &mut Drive, path: &str, id: &str) -> Result<Vec<(String, DriveFile)>> {
        self.children_listed(drive, path, id).map(|(children, _)| children)
    }

    /// [`Self::children`], and whether the listing was complete.
    fn children_listed(&self, drive: &mut Drive, path: &str, id: &str) -> Result<(Vec<(String, DriveFile)>, bool)> {
        let Listing { files, complete } = drive.listing(&folder_query(id)?)?;
        let paths = child_paths(path, &files);
        let mut folders = lock(&self.folders);
        for (at, file) in paths.iter().zip(&files) {
            if file.is_folder() {
                folders.insert(at.clone(), file.id.clone());
            }
        }
        Ok((paths.into_iter().zip(files).collect(), complete))
    }

    /// The id of the folder at `path`, listing each parent not yet known.
    fn folder_id(&self, drive: &mut Drive, path: &str) -> Result<String> {
        let path = path.trim_end_matches('/');
        if path.is_empty() {
            return Ok("root".to_owned());
        }
        if let Some(id) = lock(&self.folders).get(path) {
            return Ok(id.clone());
        }
        let mut here = String::new();
        let mut id = "root".to_owned();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let next = format!("{here}/{part}");
            let parent = std::mem::replace(&mut here, next);
            let known = lock(&self.folders).get(&here).cloned();
            id = match known {
                Some(known) => known,
                None => {
                    let (_, complete) = self.children_listed(drive, &parent, &id)?;
                    let found = lock(&self.folders).get(&here).cloned();
                    match found {
                        Some(found) => found,
                        // Not in the part that was read: maybe further down, maybe not there.
                        None if !complete => {
                            return Err(Error::Invalid(format!(
                                "{here} isn't among the first entries Google Drive listed; open its folder to find it"
                            )));
                        }
                        None => return Err(Error::Invalid(format!("there is no folder {here} in Google Drive"))),
                    }
                }
            };
        }
        Ok(id)
    }

    fn list(&self, path: &str) -> std::result::Result<Vec<Entry>, Fault> {
        let path = path.trim_end_matches('/').to_owned();
        {
            let mut folders = lock(&self.folders);
            if folders.len() >= MAX_FOLDERS {
                folders.clear();
            }
        }
        let children = self.drive(|d| {
            let id = self.folder_id(d, &path)?;
            self.children(d, &path, &id)
        })?;
        Ok(children.into_iter().map(|(at, file)| entry(&file, at)).collect())
    }

    /// Files other people shared, in Drive's order. Folders are left out: the pages browse
    /// folders by path, and a shared folder has none here.
    fn shared(&self) -> std::result::Result<Vec<Entry>, Fault> {
        let files = self.drive(|d| d.search(SHARED_QUERY))?;
        Ok(files.iter().filter(|f| !f.is_folder()).map(|f| entry(f, format!("/{}", segment(&f.name)))).collect())
    }

    /// The bytes, and the entry at the version they are: a save then checks against what was
    /// read, not what was listed.
    fn read(&self, wanted: &Entry) -> std::result::Result<(Entry, Vec<u8>), Fault> {
        self.drive(|d| {
            // The revision is read again after the download: the bytes are only said to be that
            // revision when it didn't move during the download.
            let mut file = d.metadata(&wanted.id)?;
            for _ in 0..READ_ATTEMPTS {
                let bytes = d.download(&file)?;
                let after = d.metadata(&wanted.id)?;
                if after.content_version() == file.content_version() {
                    return Ok((entry(&after, wanted.path.clone()), bytes));
                }
                file = after;
            }
            Err(Error::Invalid(format!("{} kept changing in Google Drive while it was being opened; try again", wanted.name)))
        })
    }

    fn write(&self, over: &Entry, bytes: &[u8]) -> std::result::Result<Entry, Fault> {
        // A save with no version to check against could overwrite somebody else's work.
        let Some(version) = over.etag.clone() else {
            return Err(Fault::Refused(format!("{} has no Google Drive version to check against; save a copy instead", over.name)));
        };
        let target = SaveTarget::Existing { id: over.id.clone(), name: over.name.clone(), version };
        self.drive(|d| d.save(&target, bytes)).map(|file| entry(&file, over.path.clone()))
    }

    /// A new file in the folder at `folder`. A name already there is refused: Drive would make a
    /// second file of the same name, and a later Open could pick either.
    fn create(&self, folder: &str, name: &str, bytes: &[u8]) -> std::result::Result<Entry, Fault> {
        let folder = folder.trim_end_matches('/').to_owned();
        let file = self.drive(|d| {
            let parent = self.folder_id(d, &folder)?;
            if d.child_named(&parent, name.trim())?.is_some() {
                return Err(Error::Invalid(format!("{} already exists", name.trim())));
            }
            let parent = (parent != "root").then_some(parent);
            d.save(&SaveTarget::New { name: name.to_owned(), parent }, bytes)
        })?;
        Ok(entry(&file, child_path(&folder, &file.name)))
    }

    /// Opens the consent page and waits for Google's redirect, off the lock. `on_url` is told
    /// the address to open once the listener is up.
    fn sign_in(&self, on_url: impl FnOnce(String)) -> SignedIn {
        let Some(cfg) = self.config.clone() else {
            let why = lock(&self.drive).as_ref().err().cloned().unwrap_or_default();
            return SignedIn::Refused(Fault::Refused(why));
        };
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(old) = lock(&self.cancel).replace(cancel.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        let begun = (|| Ok((crate::pkce::Pkce::generate()?, crate::pkce::random_token(24)?, crate::loopback::Listener::bind()?)))();
        let (pkce, state, listener) = match begun {
            Ok(begun) => begun,
            Err(e) => return SignedIn::Refused(fault(e)),
        };
        let redirect = listener.redirect_uri();
        on_url(auth_url(&cfg, &redirect, &pkce.challenge, &state));
        let code = listener.wait(&state, &self.app, &cancel, self.timeout);
        let superseded = || cancel.load(Ordering::Relaxed);
        let code = match code {
            Err(_) if superseded() => return SignedIn::Superseded,
            Err(e) => return SignedIn::Failed(fault(e)),
            Ok(code) => code,
        };
        let mut guard = lock(&self.drive);
        // A sign-out sets the flag before it closes the keep switch, so checking the flag while
        // holding the switch orders the two: a sign-out after this point closes it again, and
        // the exchange below then keeps nothing.
        {
            let mut keep = self.keep.as_ref().map(|k| lock(k));
            if superseded() {
                return SignedIn::Superseded;
            }
            if let Some(k) = keep.as_deref_mut() {
                *k = true;
            }
        }
        let Ok(drive) = guard.as_mut() else { return SignedIn::Superseded };
        match drive.auth.exchange(&code, &pkce.verifier, &redirect, Instant::now()) {
            Ok(()) => {
                // A new sign-in may be another account: no folder ids carry over.
                lock(&self.folders).clear();
                SignedIn::Yes
            }
            Err(e) => SignedIn::Failed(fault(e)),
        }
    }

    fn sign_out(&self) {
        if let Some(waiting) = lock(&self.cancel).take() {
            waiting.store(true, Ordering::Relaxed);
        }
        lock(&self.links).clear();
        // The stored token goes first, so quitting during a long transfer doesn't leave the
        // person signed in on the next launch. It is kept in memory to be revoked below.
        let stored = self.config.as_ref().and_then(|cfg| {
            // Closed first and held while the token is removed: a refresh finishing during the
            // transfer can't store a rotated token behind this sign-out.
            let mut keep = self.keep.as_ref().map(|k| lock(k));
            if let Some(k) = keep.as_deref_mut() {
                *k = false;
            }
            let handle = token_handle(cfg);
            let stored = self.store.get(&handle).ok().flatten();
            if let Err(e) = self.store.remove(&handle) {
                log::warn!("Google Drive: sign-out didn't remove the stored token: {e}");
            }
            stored
        });
        let mut guard = lock(&self.drive);
        lock(&self.folders).clear();
        if let Ok(drive) = guard.as_mut() {
            // Revokes at Google: the session's own token, or the stored one when this run never
            // used it.
            if drive.auth.signed_in() {
                if let Err(e) = drive.auth.sign_out() {
                    log::warn!("Google Drive: sign-out: {e}");
                }
            } else if let Some(token) = stored {
                drive.auth.revoke(token.expose());
            }
        }
    }

    /// Records a link's outcome for the page's next poll.
    fn finish_link(&self, handle: &str, done: Linking) {
        if let Some(slot) = lock(&self.links).get_mut(handle) {
            *slot = done;
        }
    }
}

impl CloudBackend for GoogleDriveBackend {
    fn submit(&self, job: Job, post: Post) {
        let inner = self.inner.clone();
        match job {
            Job::Account => spawn(move || post.send(Event::Account(inner.account()))),
            // The pages' own sign-in, from the account badge: the browser opens, and the answer
            // is who signed in, then "linked", which lists My Drive again in place of the
            // "not linked" a cold start left there.
            Job::SignIn => spawn(move || {
                let browser = post.dup();
                match inner.sign_in(|url| browser.send(Event::Browser { url, handle: None })) {
                    SignedIn::Yes => {
                        post.send(Event::Account(inner.account()));
                        post.send(Event::Linked(Ok(true)));
                    }
                    SignedIn::Refused(f) => post.send(Event::Account(Err(f))),
                    // The page is waiting on the browser: this ends that wait with Try again.
                    SignedIn::Failed(f) => post.send(Event::Linked(Err(f))),
                    SignedIn::Superseded => {}
                }
            }),
            // Linking: the same sign-in, polled by the page through the handle.
            Job::LinkStart => spawn(move || {
                let handle = match crate::pkce::random_token(16) {
                    Ok(handle) => handle,
                    Err(e) => return post.send(Event::Linked(Err(fault(e)))),
                };
                {
                    let mut links = lock(&inner.links);
                    if links.len() >= MAX_LINKS {
                        links.clear();
                    }
                    links.insert(handle.clone(), Linking::Waiting);
                }
                let browser = post.dup();
                let browser_handle = handle.clone();
                match inner.sign_in(|url| browser.send(Event::Browser { url, handle: Some(browser_handle) })) {
                    SignedIn::Yes => inner.finish_link(&handle, Linking::Done),
                    SignedIn::Failed(f) => inner.finish_link(&handle, Linking::Failed(f)),
                    // Refused before the browser opened: the page has no handle to poll yet.
                    SignedIn::Refused(f) => {
                        lock(&inner.links).remove(&handle);
                        post.send(Event::Linked(Err(f)));
                    }
                    SignedIn::Superseded => {
                        lock(&inner.links).remove(&handle);
                    }
                }
            }),
            Job::LinkPoll(handle) => spawn(move || {
                let mut links = lock(&inner.links);
                let answer = match links.get(&handle).cloned() {
                    Some(Linking::Waiting) => Ok(false),
                    Some(Linking::Done) => {
                        links.remove(&handle);
                        Ok(true)
                    }
                    Some(Linking::Failed(f)) => {
                        links.remove(&handle);
                        Err(f)
                    }
                    None => Err(Fault::Refused("that Google sign-in is over; sign in again".into())),
                };
                drop(links);
                post.send(Event::Linked(answer));
            }),
            Job::List(path) => spawn(move || post.send(Event::Listed { entries: inner.list(&path), path })),
            Job::SharedWithMe => spawn(move || post.send(Event::Shared(inner.shared()))),
            Job::Read { entry, into } => spawn(move || match inner.read(&entry) {
                Ok((entry, bytes)) => post.send(Event::Read { entry, into, bytes: Ok(bytes) }),
                Err(f) => post.send(Event::Read { entry, into, bytes: Err(f) }),
            }),
            Job::Write { uid, entry, bytes } => spawn(move || post.send(Event::Saved { uid, entry: inner.write(&entry, &bytes) })),
            Job::Create { uid, folder, name, bytes } => spawn(move || post.send(Event::Saved { uid, entry: inner.create(&folder, &name, &bytes) })),
            // The pages forget their own state whether or not anything answers.
            Job::SignOut => spawn(move || inner.sign_out()),
            Job::Conversation(_) => {
                spawn(move || post.send(Event::Conversation(Err(Fault::Refused("Google Drive has no conversation for a document here".into())))))
            }
        }
    }
}
