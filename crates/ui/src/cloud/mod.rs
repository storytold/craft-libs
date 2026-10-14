//! Open and Save against cloud storage, the same pages in every Craft app.
//!
//! What this offers is what a person asks for when they say "open the one in
//! my cloud": a Backstage page with their own files on one side and what other
//! people shared with them on the other, folders to walk into, an account badge
//! that says whose storage this is, a Save As page that picks a folder and
//! names a file, and a save that refuses to overwrite somebody else's work.
//!
//! Three things shape the implementation.
//!
//! Nothing here opens a socket. Every call goes through [`CloudBackend`]. A
//! host writes that half. This module never learns what it reaches. That keeps
//! it buildable for the web and testable with no service running.
//!
//! Nothing here blocks. A job goes to a worker, the answer arrives on a
//! channel, and [`pump`] drains it on a later frame. A blocking read of a
//! twenty megabyte document inside a frame would freeze the window for seconds.
//!
//! Nothing here is assumed about the provider. Every side of the Open page is
//! drawn only where the service reported the capability behind it. An empty
//! list under a heading reads to a user as a broken window. A feature the
//! provider does not have is therefore absent, heading and all.
//!
//! The application supplies [`CloudApp`]. That is what it alone can answer.
//! What its documents are, how bytes become one and back, and the words it
//! shows them in.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use craft_cloud::{Capability, Entry, EntryKind};
use egui::{Align2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value as Json, json};

mod app;
mod backend;
mod pages;
#[cfg(test)]
mod tests;
mod title;

pub use app::{CloudApp, CloudStyle, CopyFormat, DocumentKind};
pub use backend::{Account, CloudBackend, Event, Fault, Job, Post};
pub use pages::{conflict_modal, open_page, progress, save_page};
pub use title::{destination_name, prefers_cloud, share_button, shares, who};

/// How long to wait between asking whether the user has finished linking an
/// account in their browser.
const LINK_POLL: f64 = 1.5;

/// What a command says when no cloud service is configured. Not a failure of
/// the service: there is no service to fail.
pub const UNAVAILABLE: &str = "no cloud service is configured";

// ---------------------------------------------------------------- state

/// One folder, as it is being browsed.
#[derive(Clone, Debug, Default)]
pub struct Folder {
    pub path: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub error: Option<Fault>,
    /// Answered at least once. An empty folder then reads as empty. Without
    /// this it would read as still loading.
    pub loaded: bool,
}

impl Folder {
    fn begin(&mut self, path: &str) {
        self.path = path.to_owned();
        self.entries.clear();
        self.loading = true;
        self.loaded = false;
        self.error = None;
    }

    fn fill(&mut self, answer: Result<Vec<Entry>, Fault>) {
        self.loading = false;
        self.loaded = true;
        match answer {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
            }
            Err(fault) => {
                self.entries.clear();
                self.error = Some(fault);
            }
        }
    }
}

/// Where the document in this window came from, and the version it came at.
///
/// `entry.id` is the opaque handle a save passes back. `entry.etag` is the
/// version the conditional write is made against. `entry.room` is neither of
/// those. It is the collaboration identity, and it survives a rename.
#[derive(Clone, Debug)]
pub struct Origin {
    pub entry: Entry,
}

/// What the pages are waiting for. Drawn as progress while they wait.
#[derive(Clone, Debug)]
pub enum Busy {
    Opening { name: String },
    Saving { name: String, bytes: usize },
}

impl Busy {
    pub fn text(&self) -> String {
        match self {
            Busy::Opening { name } => format!("Opening {name}…"),
            Busy::Saving { name, bytes } => format!("Saving {name} ({})…", size_text(Some(*bytes as u64))),
        }
    }
}

/// A save that was refused because the stored copy had moved on.
#[derive(Clone, Debug)]
pub struct Conflict {
    /// The document that could not be saved.
    pub uid: u64,
    pub entry: Entry,
}

/// Linking a cloud account, which takes a browser round trip.
#[derive(Clone, Debug)]
pub enum Link {
    /// Offered and not started. Never drawn as an error.
    Offered,
    Starting,
    /// The browser is open. `handle` is polled until the user is done.
    Waiting {
        url: String,
        handle: Option<String>,
        next_poll: f64,
    },
    Failed(Fault),
}

/// Which side of the Open page a selection came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Mine,
    Shared,
}

/// Everything these pages keep about cloud storage.
///
/// One per window. The application declares a field of this type, hands it
/// back through [`CloudApp::cloud_mut`], and connects a backend once one
/// exists.
pub struct CloudFiles {
    backend: Option<Box<dyn CloudBackend>>,
    events: (Sender<Event>, Receiver<Event>),
    /// The badge and its menu. Signing in is the only way into any of this.
    /// The badge is the only control that offers it. It lives here. An
    /// application declares no field for it.
    accounts: crate::account::AccountMenu,
    /// Frame time, in seconds, as of the last [`pump`]. Kept because a link
    /// poll is scheduled from an answer that arrives with no frame in hand.
    clock: f64,
    pub account: Option<Account>,
    pub account_fault: Option<Fault>,
    /// The account is asked for once per run. A frame never asks again.
    asked_account: bool,
    /// The folder being browsed. Shared by the Open and Save As pages, which
    /// are never on screen together.
    pub mine: Folder,
    pub shared: Folder,
    pub source: Source,
    pub busy: Option<Busy>,
    pub link: Option<Link>,
    pub conflict: Option<Conflict>,
    /// Save As: the file name being typed.
    pub save_name: String,
    /// Documents that came from the cloud, by document uid.
    pub origins: HashMap<u64, Origin>,
    /// The last thing that happened, for the line under the page title.
    pub status: Option<String>,
    /// Whether the transfer in flight was started by the share dialog, which
    /// is waiting to be told how it went.
    pub sharing_save: bool,
}

impl Default for CloudFiles {
    fn default() -> CloudFiles {
        CloudFiles::new()
    }
}

impl CloudFiles {
    pub fn new() -> CloudFiles {
        CloudFiles {
            backend: None,
            events: channel(),
            accounts: crate::account::AccountMenu::new(),
            clock: 0.0,
            account: None,
            account_fault: None,
            asked_account: false,
            mine: Folder::default(),
            shared: Folder::default(),
            source: Source::default(),
            busy: None,
            link: None,
            conflict: None,
            save_name: String::new(),
            origins: HashMap::new(),
            status: None,
            sharing_save: false,
        }
    }

    /// Attach the thing that does the waiting.
    ///
    /// Until this is called nothing cloud-shaped is offered anywhere. That is
    /// what a build with no service configured wants.
    pub fn connect(&mut self, backend: impl CloudBackend + 'static) {
        self.backend = Some(Box::new(backend));
    }

    /// Whether a service is wired at all.
    pub fn available(&self) -> bool {
        self.backend.is_some()
    }

    /// The account badge's own strings, for a localized application that
    /// would rather translate them itself than through
    /// [`CloudApp::tr`](crate::cloud::CloudApp::tr).
    pub fn account_menu_mut(&mut self) -> &mut crate::account::AccountMenu {
        &mut self.accounts
    }

    /// Whether the provider offers somewhere to talk about a document.
    pub fn can_converse(&self) -> bool {
        self.can(Capability::Conversation)
    }

    fn can(&self, capability: Capability) -> bool {
        self.account.as_ref().is_some_and(|a| a.can(capability))
    }
}

// ---------------------------------------------------------------- asking

/// Whether the host wired a cloud service. When it did not, nothing
/// cloud-shaped is offered anywhere in the interface.
pub fn available<A: CloudApp>(app: &A) -> bool {
    app.cloud().available()
}

/// Whether the provider offers somewhere to talk about a document.
pub fn can_converse<A: CloudApp>(app: &A) -> bool {
    app.cloud().can_converse()
}

/// Hands a job to the host's worker.
pub fn submit<A: CloudApp>(app: &A, job: Job) {
    let cloud = app.cloud();
    if let Some(backend) = &cloud.backend {
        backend.submit(job, Post::new(cloud.events.0.clone()));
    }
}

/// Asks who is signed in, once per run, and lists the root folder.
pub fn start<A: CloudApp>(app: &mut A) {
    if !available(app) {
        return;
    }
    if !app.cloud().asked_account {
        app.cloud_mut().asked_account = true;
        submit(app, Job::Account);
    }
    if !app.cloud().mine.loaded && !app.cloud().mine.loading {
        browse(app, "");
    }
}

/// Begins signing in to the service.
///
/// A worker signs in before any job it is given, so asking who the account is
/// starts the browser flow. The badge says it is working, because the worker
/// then blocks until the user finishes in the browser and a button that looks
/// idle for a minute reads as broken.
pub fn sign_in<A: CloudApp>(app: &mut A) {
    if !available(app) {
        return;
    }
    let working = app.tr("Finish signing in, in your browser");
    let cloud = app.cloud_mut();
    cloud.accounts.working();
    cloud.account_fault = None;
    // Cleared first. A sign-in that failed leaves this set, and a second press
    // would then be swallowed by the guard that stops a frame asking twice. A
    // button that does nothing the second time reads as a broken button.
    cloud.asked_account = true;
    cloud.link = None;
    cloud.status = Some(working);
    submit(app, Job::Account);
}

/// Ends the session, here and at the service.
///
/// The saved token goes too. Forgetting the account on screen while leaving the
/// token on disk would sign the user back in on the next run.
pub fn sign_out<A: CloudApp>(app: &mut A) {
    let signed_out = app.tr("Signed out");
    let cloud = app.cloud_mut();
    cloud.account = None;
    cloud.account_fault = None;
    cloud.asked_account = false;
    cloud.link = None;
    cloud.mine = Folder::default();
    cloud.shared = Folder::default();
    cloud.origins.clear();
    cloud.status = Some(signed_out);
    submit(app, Job::SignOut);
}

/// Lists one folder. An empty path is the user's root.
pub fn browse<A: CloudApp>(app: &mut A, path: &str) {
    if !available(app) {
        return;
    }
    app.cloud_mut().mine.begin(path);
    submit(app, Job::List(path.to_owned()));
}

/// Lists what other people have shared with this user.
pub fn load_shared<A: CloudApp>(app: &mut A) {
    if !available(app) {
        return;
    }
    app.cloud_mut().shared.begin("");
    submit(app, Job::SharedWithMe);
}

/// Begins linking a cloud account.
pub fn link<A: CloudApp>(app: &mut A) {
    if !available(app) {
        return;
    }
    app.cloud_mut().link = Some(Link::Starting);
    submit(app, Job::LinkStart);
}

/// Asks where to work together on the open document, then opens it.
///
/// Only offered where the provider reports it. A document that has never been
/// in the cloud has no handle to key a room on, so there is nothing to ask
/// about either.
pub fn work_together<A: CloudApp>(app: &mut A) {
    if !available(app) {
        return;
    }
    let Some(id) = origin(app).map(|o| o.entry.id.clone()) else {
        let noun = app.kind().noun;
        app.cloud_mut().status = Some(app.tr(&format!("Save this {noun} to the cloud first")));
        return;
    };
    converse(app, id);
}

/// Asks the provider where to work together on one document.
fn converse<A: CloudApp>(app: &mut A, id: String) {
    let opening = app.tr("Opening a conversation");
    app.cloud_mut().status = Some(opening);
    submit(app, Job::Conversation(id));
}

/// The parent of a path. `None` at the root.
pub fn parent_of(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) | None => Some(String::new()),
        Some(cut) => Some(trimmed[..cut].to_owned()),
    }
}

/// Whether this application can open an entry, judged by its extension and by
/// what the provider says it is.
///
/// Both signals are read because neither is reliable alone. A WebDAV server
/// hands back `application/octet-stream` for a perfectly good spreadsheet. A
/// file exported by a web application often has a correct content type and no
/// extension at all.
pub fn opens(entry: &Entry, kind: DocumentKind) -> bool {
    if entry.kind == EntryKind::Folder {
        return false;
    }
    let extension = entry.name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let by_name = extension.is_some_and(|e| kind.extensions.contains(&e.as_str()));
    by_name || entry.mime.as_deref().is_some_and(|mime| mime_opens(mime, kind))
}

fn mime_opens(mime: &str, kind: DocumentKind) -> bool {
    let mime = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    kind.mimes.iter().any(|known| mime.starts_with(known))
}

/// A typed name as a document file name. Any folder separators are dropped and
/// a missing or unknown extension becomes the application's own.
pub fn document_name(typed: &str, kind: DocumentKind) -> String {
    let base = typed.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if base.is_empty() {
        return String::new();
    }
    let known = base.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && kind.extensions.contains(&ext.to_ascii_lowercase().as_str()));
    if known { base.to_owned() } else { format!("{base}.{}", kind.default_extension) }
}

/// One sentence with a value in it, translated whole.
///
/// The template reaches [`CloudApp::tr`] with its `{}` intact. A translator
/// sees the whole sentence and can put the value where their own language
/// wants it. A sentence assembled from translated fragments cannot be made to
/// read correctly in a language that orders them differently.
fn fill<A: CloudApp>(app: &A, template: &str, value: &str) -> String {
    app.tr(template).replacen("{}", value, 1)
}

/// A file name without its extension.
fn stem_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

// ---------------------------------------------------------------- opening

/// Reads a document and opens it. `into` names an open document the bytes
/// replace, which is how a reload after a conflict works.
pub fn open_entry<A: CloudApp>(app: &mut A, entry: Entry, into: Option<u64>) {
    if !available(app) || app.cloud().busy.is_some() {
        return;
    }
    let cloud = app.cloud_mut();
    cloud.status = None;
    cloud.busy = Some(Busy::Opening { name: entry.name.clone() });
    submit(app, Job::Read { entry, into });
}

/// Reads the active document's stored copy again. Local edits are discarded.
pub fn reload_active<A: CloudApp>(app: &mut A) -> Result<Json, String> {
    let noun = app.kind().noun;
    let Some(uid) = app.active() else { return Err(format!("no {noun} open")) };
    let Some(origin) = app.cloud().origins.get(&uid).cloned() else { return Err(format!("this {noun} did not come from the cloud")) };
    app.cloud_mut().conflict = None;
    let name = origin.entry.name.clone();
    open_entry(app, origin.entry, Some(uid));
    Ok(json!({"reloading": name}))
}

// ---------------------------------------------------------------- saving

/// The cloud document the active one came from, when it came from one.
pub fn origin<A: CloudApp>(app: &A) -> Option<&Origin> {
    let uid = app.active()?;
    app.cloud().origins.get(&uid)
}

/// Whether Save has somewhere to go without asking. A Backstage view uses this
/// to choose between Save and the Save As page.
pub fn has_target<A: CloudApp>(app: &A) -> bool {
    origin(app).is_some_and(|o| o.entry.writable)
}

/// Save: writes back over the entry this document came from.
///
/// The etag the document was read at goes out as the condition. A server whose
/// copy has moved on refuses the write. That arrives as [`Fault::Conflict`] and
/// is put to the user. It is never retried.
pub fn save<A: CloudApp>(app: &mut A) -> Result<Json, String> {
    let noun = app.kind().noun;
    let uid = app.active().ok_or(format!("no {noun} open"))?;
    let origin = app.cloud().origins.get(&uid).cloned().ok_or(format!("this {noun} did not come from the cloud"))?;
    if !origin.entry.writable {
        return Err("this document was shared with you for reading only. Save As keeps a copy of your own.".to_owned());
    }
    if app.cloud().busy.is_some() {
        return Err("a cloud transfer is already running".to_owned());
    }
    let bytes = app.encode(uid)?;
    let size = bytes.len();
    let cloud = app.cloud_mut();
    cloud.status = None;
    cloud.busy = Some(Busy::Saving { name: origin.entry.name.clone(), bytes: size });
    let answer = json!({"saving": origin.entry.name, "bytes": size, "ifMatch": origin.entry.etag});
    submit(app, Job::Write { uid, entry: origin.entry, bytes });
    Ok(answer)
}

/// Save As: writes a new document into `folder` under `name`.
pub fn save_as<A: CloudApp>(app: &mut A, folder: &str, name: &str) -> Result<Json, String> {
    let kind = app.kind();
    let uid = app.active().ok_or(format!("no {} open", kind.noun))?;
    let name = document_name(name, kind);
    if name.is_empty() {
        return Err("type a file name".to_owned());
    }
    if app.cloud().busy.is_some() {
        return Err("a cloud transfer is already running".to_owned());
    }
    let bytes = app.encode(uid)?;
    let size = bytes.len();
    let cloud = app.cloud_mut();
    cloud.status = None;
    cloud.busy = Some(Busy::Saving { name: name.clone(), bytes: size });
    let answer = json!({"saving": name, "folder": folder, "bytes": size});
    submit(app, Job::Create { uid, folder: folder.to_owned(), name, bytes });
    Ok(answer)
}

// ---------------------------------------------------------------- reading the answers

/// Drains the host's answers and keeps a link or a transfer moving. Called
/// once a frame, before anything is drawn.
pub fn pump<A: CloudApp>(app: &mut A, ctx: &egui::Context) {
    if !available(app) {
        return;
    }
    app.cloud_mut().clock = ctx.input(|i| i.time);
    // Taken in one pass, then applied. An answer that arrives because of a job
    // this frame sent belongs to the next frame. A worker on its own thread
    // would have delivered it that way anyway.
    let mut arrived = Vec::new();
    while let Ok(event) = app.cloud().events.1.try_recv() {
        arrived.push(event);
    }
    for event in arrived {
        apply(app, event);
    }
    poll_link(app);
    // A worker answering is not a local event. Without this the progress line
    // would sit frozen until the user moved the mouse.
    if app.cloud().busy.is_some() || matches!(app.cloud().link, Some(Link::Starting | Link::Waiting { .. })) {
        ctx.request_repaint_after(Duration::from_millis(120));
    }
}

fn apply<A: CloudApp>(app: &mut A, event: Event) {
    let noun = app.kind().noun;
    match event {
        Event::Account(Ok(account)) => {
            let linked = account.provider_linked;
            // Who is signed in has changed, so what the provider can do may
            // have changed with it. The dialog keeps its first answer, and on a
            // cold start that answer is "nobody is signed in".
            app.sharing().refresh();
            let cloud = app.cloud_mut();
            cloud.account = Some(account);
            cloud.account_fault = None;
            if linked {
                cloud.link = None;
            } else if cloud.link.is_none() {
                cloud.link = Some(Link::Offered);
            }
        }
        // No account linked yet is a state with an obvious next step. It is an
        // offer to link, and never an error message.
        Event::Account(Err(Fault::NotLinked)) => {
            let cloud = app.cloud_mut();
            cloud.account_fault = None;
            if cloud.link.is_none() {
                cloud.link = Some(Link::Offered);
            }
        }
        Event::Account(Err(fault)) => app.cloud_mut().account_fault = Some(fault),
        Event::Conversation(Ok(conversation)) => {
            let said = fill(app, "Opened in {}", &conversation.name);
            app.cloud_mut().status = Some(said);
            app.open_url(&conversation.url);
        }
        Event::Conversation(Err(fault)) => {
            let text = fault.text(noun);
            app.cloud_mut().status = Some(text);
        }
        Event::Browser { url, handle } => {
            app.open_url(&url);
            let next_poll = app.cloud().clock + LINK_POLL;
            app.cloud_mut().link = Some(Link::Waiting { url, handle, next_poll });
        }
        Event::Linked(Ok(true)) => {
            // Storage just arrived, so ask what it can do.
            app.sharing().refresh();
            let linked = app.tr("Cloud account linked.");
            let cloud = app.cloud_mut();
            cloud.link = None;
            cloud.status = Some(linked);
            cloud.asked_account = false;
            submit(app, Job::Account);
            browse(app, "");
            app.cloud_mut().shared.loaded = false;
        }
        // The ordinary answer while the user is still in the browser.
        Event::Linked(Ok(false)) | Event::Linked(Err(Fault::NotLinked)) => {}
        Event::Linked(Err(fault)) => app.cloud_mut().link = Some(Link::Failed(fault)),
        Event::Listed { path, entries } => {
            let cloud = app.cloud_mut();
            if path == cloud.mine.path {
                cloud.mine.fill(entries);
            }
        }
        Event::Shared(entries) => app.cloud_mut().shared.fill(entries),
        Event::Read { entry, into, bytes } => {
            app.cloud_mut().busy = None;
            match bytes {
                Ok(bytes) => opened(app, entry, into, bytes),
                Err(Fault::NotLinked) => app.cloud_mut().link = Some(Link::Offered),
                Err(fault) => {
                    let text = format!("{}. {}", fill(app, "{} could not be opened", &entry.name), fault.text(noun));
                    app.say(text);
                }
            }
        }
        Event::Saved { uid, entry } => {
            app.cloud_mut().busy = None;
            match entry {
                Ok(entry) => {
                    app.mark_saved(uid);
                    stored(app, uid, entry);
                }
                // The one failure that must never be retried and never forced.
                Err(Fault::Conflict) => {
                    app.drop_pending(uid);
                    match app.cloud().origins.get(&uid).cloned() {
                        Some(origin) => app.cloud_mut().conflict = Some(Conflict { uid, entry: origin.entry }),
                        None => app.say(Fault::Conflict.text(noun)),
                    }
                }
                Err(Fault::NotLinked) => {
                    app.drop_pending(uid);
                    app.cloud_mut().link = Some(Link::Offered);
                }
                Err(fault) => {
                    app.drop_pending(uid);
                    let text = format!("{} {}", app.tr(&format!("The {noun} was not saved.")), fault.text(noun));
                    app.say(text);
                }
            }
        }
    }
}

/// Bytes arrived. The application opens them. The document then remembers
/// where they came from.
fn opened<A: CloudApp>(app: &mut A, entry: Entry, into: Option<u64>, bytes: Vec<u8>) {
    let name = entry.name.clone();
    match app.open_bytes(&name, bytes) {
        Ok(now) => {
            if let Some(uid) = now {
                app.cloud_mut().origins.insert(uid, Origin { entry });
            }
            // A reload closes the document it replaced only once the new one is
            // open. A read that fails leaves the user's work where it was.
            if let Some(old) = into.filter(|old| Some(*old) != now) {
                let cloud = app.cloud_mut();
                cloud.origins.remove(&old);
                app.drop_pending(old);
                app.close(old);
            }
            if now.is_some() {
                let said = fill(app, "Opened {} from the cloud.", &name);
                app.cloud_mut().status = Some(said);
            }
        }
        Err(reason) => app.say(reason),
    }
}

/// A save landed. The document now belongs to the entry the server returned, at
/// the version it returned.
fn stored<A: CloudApp>(app: &mut A, uid: u64, entry: Entry) {
    let name = entry.name.clone();
    let line = fill(app, "Saved {} to the cloud.", &name);
    app.toast(fill(app, "Saved {} to the cloud", &name));
    let cloud = app.cloud_mut();
    cloud.conflict = None;
    cloud.status = Some(line);
    cloud.origins.insert(uid, Origin { entry });
}

/// Keeps asking whether the user has finished linking in their browser.
fn poll_link<A: CloudApp>(app: &mut A) {
    let now = app.cloud().clock;
    let handle = {
        let Some(Link::Waiting { handle: Some(handle), next_poll, .. }) = &mut app.cloud_mut().link else { return };
        if now < *next_poll {
            return;
        }
        *next_poll = now + LINK_POLL;
        handle.clone()
    };
    submit(app, Job::LinkPoll(handle));
}

// ---------------------------------------------------------------- reporting

/// The whole of the cloud state, for a control channel and for tests.
pub fn report<A: CloudApp>(app: &A) -> Json {
    let kind = app.kind();
    let cloud = app.cloud();
    let account = cloud.account.as_ref().map(|a| {
        json!({
            "display": a.display,
            "providerLinked": a.provider_linked,
            "capabilities": a.capabilities.iter().map(|c| serde_json::to_value(c).unwrap_or_default()).collect::<Vec<_>>(),
        })
    });
    let came_from = origin(app)
        .map(|o| json!({"id": o.entry.id, "room": o.entry.room, "name": o.entry.name, "etag": o.entry.etag, "writable": o.entry.writable}));
    json!({
        "available": cloud.available(),
        "account": account,
        "accountError": cloud.account_fault.as_ref().map(Fault::code),
        "folder": cloud.mine.path,
        "entries": cloud.mine.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>(),
        "openable": cloud.mine.entries.iter().filter(|e| opens(e, kind)).map(|e| e.name.clone()).collect::<Vec<_>>(),
        "shared": cloud.shared.entries.iter().map(|e| json!({"name": e.name, "sharedBy": e.shared_by, "writable": e.writable, "openable": opens(e, kind)})).collect::<Vec<_>>(),
        "busy": cloud.busy.as_ref().map(Busy::text),
        "link": match &cloud.link {
            None => Json::Null,
            Some(Link::Offered) => json!("offered"),
            Some(Link::Starting) => json!("starting"),
            Some(Link::Waiting { url, .. }) => json!({"waiting": url}),
            Some(Link::Failed(fault)) => json!({"failed": fault.code()}),
        },
        "conflict": cloud.conflict.as_ref().map(|c| json!({"name": c.entry.name})),
        "origin": came_from,
        "status": cloud.status,
    })
}

// ---------------------------------------------------------------- shared drawing

/// Bytes as a person reads them.
pub fn size_text(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else { return String::new() };
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    match UNITS.get(unit) {
        Some(name) if unit > 0 => format!("{size:.1} {name}"),
        _ => format!("{bytes} bytes"),
    }
}

fn dim(ui: &mut Ui, style: &CloudStyle, text: &str) {
    ui.label(RichText::new(text).font(egui::FontId::new(12.5, style.family.clone())).color(style.dim));
}

/// A bordered column with a heading. This is how the two Open sources sit side
/// by side.
fn column(ui: &mut Ui, style: &CloudStyle, heading: &str, body: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE.fill(style.surface).stroke(Stroke::new(1.0, style.border)).corner_radius(6.0).inner_margin(10).show(ui, |ui| {
        ui.label(RichText::new(heading).font(egui::FontId::new(13.0, style.bold.clone())).color(style.text));
        ui.add_space(6.0);
        egui::ScrollArea::vertical().id_salt(heading.to_owned()).max_height(300.0).auto_shrink([false, false]).show(ui, body);
    });
}

/// One row in a file list, with a second line for the detail that matters on
/// that side of the page (a size for your own files, who shared it for
/// somebody else's).
fn entry_row<A: CloudApp>(app: &A, ui: &mut Ui, entry: &Entry, detail: &str, reserve: f32) -> egui::Response {
    let style = app.style(ui);
    let width = (ui.available_width() - reserve).max(60.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
    let label = entry.name.clone();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 4.0, style.hover);
    }
    app.paint_icon(p, Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(18.0, 18.0)), entry.kind, style.text);
    let (name_y, detail_y) = if detail.is_empty() { (rect.center().y, rect.center().y) } else { (rect.center().y - 8.0, rect.center().y + 9.0) };
    p.text(pos2(rect.left() + 38.0, name_y), Align2::LEFT_CENTER, &entry.name, egui::FontId::new(13.0, style.family.clone()), style.text);
    if !detail.is_empty() {
        p.text(pos2(rect.left() + 38.0, detail_y), Align2::LEFT_CENTER, detail, egui::FontId::new(11.0, style.family.clone()), style.dim);
    }
    resp
}
