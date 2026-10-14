//! The pages driven by an application that holds no documents.
//!
//! The point of these is the seam. A fake application answers every question
//! [`CloudApp`] asks and records what it was told to do, so what crosses the
//! boundary is visible without a spreadsheet, a service or a window.

use std::sync::{Arc, Mutex};

use craft_cloud::{Capability, Entry, EntryKind};

use super::*;

/// Everything the fake application was asked to do.
#[derive(Default)]
struct Did {
    opened: Vec<String>,
    closed: Vec<u64>,
    encoded: Vec<u64>,
    /// The name each encode was asked for, which is where the bytes go.
    encoded_as: Vec<String>,
    /// The content type beside it, where the provider gave one.
    encoded_mime: Vec<Option<String>>,
    marked_saved: Vec<u64>,
    dropped: Vec<u64>,
    said: Vec<String>,
    toasted: Vec<String>,
    urls: Vec<String>,
    went_to_save_as: usize,
}

const NOTES: DocumentKind =
    DocumentKind { extensions: &["note", "txt"], mimes: &["text/plain", "application/x-note"], noun: "note", default_extension: "note" };

struct Notebook {
    cloud: CloudFiles,
    share: crate::share::ShareDialog,
    presence: crate::presence::PresenceBar,
    /// The document in the window, and what it is called.
    open: Option<(u64, String)>,
    /// What the next `open_bytes` answers with.
    opens_as: Result<Option<u64>, String>,
    did: Arc<Mutex<Did>>,
}

impl Notebook {
    fn new() -> Notebook {
        Notebook {
            cloud: CloudFiles::new(),
            share: crate::share::ShareDialog::new(),
            presence: crate::presence::PresenceBar::new(),
            open: Some((1, "Untitled")).map(|(uid, name)| (uid, name.to_owned())),
            opens_as: Ok(Some(1)),
            did: Arc::new(Mutex::new(Did::default())),
        }
    }

    fn did(&self) -> std::sync::MutexGuard<'_, Did> {
        self.did.lock().unwrap()
    }
}

impl CloudApp for Notebook {
    fn cloud(&self) -> &CloudFiles {
        &self.cloud
    }

    fn cloud_mut(&mut self) -> &mut CloudFiles {
        &mut self.cloud
    }

    fn kind(&self) -> DocumentKind {
        NOTES
    }

    fn active(&self) -> Option<u64> {
        self.open.as_ref().map(|(uid, _)| *uid)
    }

    fn title(&self, _uid: u64) -> String {
        self.open.as_ref().map(|(_, name)| name.clone()).unwrap_or_default()
    }

    fn open_bytes(&mut self, name: &str, _bytes: Vec<u8>) -> Result<Option<u64>, String> {
        self.did().opened.push(name.to_owned());
        self.opens_as.clone()
    }

    fn encode(&mut self, uid: u64, into: Destination<'_>) -> Result<Vec<u8>, String> {
        self.did().encoded.push(uid);
        self.did().encoded_as.push(into.name.to_owned());
        self.did().encoded_mime.push(into.mime.map(str::to_owned));
        Ok(b"a note".to_vec())
    }

    fn close(&mut self, uid: u64) {
        self.did().closed.push(uid);
    }

    fn mark_saved(&mut self, uid: u64) {
        self.did().marked_saved.push(uid);
    }

    fn drop_pending(&mut self, uid: u64) {
        self.did().dropped.push(uid);
    }

    fn sharing(&mut self) -> &mut crate::share::ShareDialog {
        &mut self.share
    }

    fn presence(&mut self) -> &mut crate::presence::PresenceBar {
        &mut self.presence
    }

    fn paint_icon(&self, _painter: &egui::Painter, _rect: egui::Rect, _kind: EntryKind, _colour: egui::Color32) {}

    fn open_url(&self, url: &str) {
        self.did.lock().unwrap().urls.push(url.to_owned());
    }

    fn say(&mut self, text: String) {
        self.did().said.push(text);
    }

    fn toast(&mut self, text: String) {
        self.did().toasted.push(text);
    }

    fn go_to_save_as(&mut self) {
        self.did().went_to_save_as += 1;
    }

    fn app_name(&self) -> &str {
        "Notes"
    }
}

/// A service that answers on the calling thread, so a job submitted during a
/// test is answered before the next line.
struct Service(Arc<Mutex<Vec<Job>>>);

impl CloudBackend for Service {
    fn submit(&self, job: Job, post: Post) {
        let answer = match &job {
            Job::Account => Some(Event::Account(Ok(Account {
                display: "Ada Lovelace".to_owned(),
                capabilities: vec![Capability::Files, Capability::Etags],
                provider_linked: true,
            }))),
            Job::List(path) => Some(Event::Listed { path: path.clone(), entries: Ok(vec![file("/Minutes.note"), folder("/Archive")]) }),
            Job::Read { entry, into } => Some(Event::Read { entry: entry.clone(), into: *into, bytes: Ok(b"a note".to_vec()) }),
            Job::Write { uid, entry, .. } => {
                let mut stored = entry.clone();
                stored.etag = Some("v2".to_owned());
                Some(Event::Saved { uid: *uid, entry: Ok(stored) })
            }
            _ => None,
        };
        self.0.lock().unwrap().push(job);
        if let Some(event) = answer {
            post.send(event);
        }
    }
}

fn file(path: &str) -> Entry {
    Entry {
        id: path.to_owned(),
        room: format!("room:{path}"),
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        path: path.to_owned(),
        kind: EntryKind::File,
        size: Some(2048),
        modified: None,
        mime: None,
        etag: Some("v1".to_owned()),
        writable: true,
        shared_by: None,
    }
}

fn folder(path: &str) -> Entry {
    Entry { kind: EntryKind::Folder, size: None, etag: None, ..file(path) }
}

/// One frame's worth of draining, with no window.
fn pumped(app: &mut Notebook, ctx: &egui::Context) {
    ctx.begin_pass(Default::default());
    pump(app, ctx);
    // Nothing is painted here, so the pass's textures go nowhere. egui insists
    // they are accounted for rather than dropped on the floor.
    ctx.end_pass().textures_delta.clear();
}

fn wired() -> (Notebook, Arc<Mutex<Vec<Job>>>, egui::Context) {
    let mut app = Notebook::new();
    let jobs = Arc::new(Mutex::new(Vec::new()));
    app.cloud.connect(Service(jobs.clone()));
    (app, jobs, egui::Context::default())
}

#[test]
fn nothing_is_offered_until_a_service_is_wired() {
    let app = Notebook::new();
    assert!(!available(&app), "a build with no service offers nothing cloud-shaped");
    assert_eq!(report(&app)["available"], false);
}

#[test]
fn the_account_is_asked_for_once_per_run() {
    let (mut app, jobs, ctx) = wired();
    start(&mut app);
    start(&mut app);
    let asked = jobs.lock().unwrap().iter().filter(|j| matches!(j, Job::Account)).count();
    assert_eq!(asked, 1, "a frame that asks again would ask on every frame");
    pumped(&mut app, &ctx);
    assert_eq!(report(&app)["account"]["display"], "Ada Lovelace");
}

#[test]
fn only_the_documents_this_application_opens_are_offered() {
    let (mut app, _jobs, ctx) = wired();
    start(&mut app);
    pumped(&mut app, &ctx);
    let listing = report(&app);
    assert_eq!(listing["entries"], serde_json::json!(["Minutes.note", "Archive"]));
    assert_eq!(listing["openable"], serde_json::json!(["Minutes.note"]), "a folder is not something to open");
}

#[test]
fn a_content_type_opens_a_document_with_no_extension() {
    let mut exported = file("/exported");
    exported.mime = Some("text/plain; charset=utf-8".to_owned());
    assert!(opens(&exported, NOTES), "a provider that gives no extension still says what the bytes are");
    let mut picture = file("/holiday");
    picture.mime = Some("image/png".to_owned());
    assert!(!opens(&picture, NOTES));
    assert!(!opens(&folder("/Archive"), NOTES), "a folder opens nothing");
}

#[test]
fn a_typed_name_becomes_a_file_name_of_this_applications_own_kind() {
    assert_eq!(document_name("  Minutes ", NOTES), "Minutes.note");
    assert_eq!(document_name("Minutes.txt", NOTES), "Minutes.txt", "an extension this application opens is kept");
    assert_eq!(document_name("Minutes.v2", NOTES), "Minutes.v2.note", "an unknown suffix is part of the name");
    assert_eq!(document_name("/a/b/Minutes", NOTES), "Minutes.note", "a pasted path cannot become a folder");
    assert_eq!(document_name("   ", NOTES), "");
}

#[test]
fn walking_up_from_the_root_has_nowhere_to_go() {
    assert_eq!(parent_of(""), None);
    assert_eq!(parent_of("/Archive"), Some(String::new()));
    assert_eq!(parent_of("/Archive/2026"), Some("/Archive".to_owned()));
}

#[test]
fn opening_hands_the_bytes_to_the_application_and_remembers_the_entry() {
    let (mut app, _jobs, ctx) = wired();
    start(&mut app);
    pumped(&mut app, &ctx);
    open_entry(&mut app, file("/Minutes.note"), None);
    pumped(&mut app, &ctx);
    assert_eq!(app.did().opened, vec!["Minutes.note".to_owned()], "the application alone turns bytes into a document");
    let origin = report(&app)["origin"].clone();
    assert_eq!(origin["id"], "/Minutes.note", "the opaque handle is kept for the save");
    assert_eq!(origin["etag"], "v1", "so is the version it was read at");
    assert_eq!(origin["room"], "room:/Minutes.note", "and the collaboration identity, which is not the handle");
}

#[test]
fn a_reload_closes_the_document_it_replaced_only_once_the_new_one_is_open() {
    let (mut app, _jobs, ctx) = wired();
    app.opens_as = Ok(Some(2));
    open_entry(&mut app, file("/Minutes.note"), Some(1));
    pumped(&mut app, &ctx);
    assert_eq!(app.did().closed, vec![1], "the one it replaced is gone");
    assert_eq!(app.did().dropped, vec![1], "and so is whatever was recorded about saving it");

    // A read the application could not use leaves the user's work where it was.
    let (mut app, _jobs, ctx) = wired();
    app.opens_as = Err("these bytes are not a note".to_owned());
    open_entry(&mut app, file("/Minutes.note"), Some(1));
    pumped(&mut app, &ctx);
    assert!(app.did().closed.is_empty(), "nothing was closed, because nothing opened");
    assert_eq!(app.did().said, vec!["these bytes are not a note".to_owned()]);
}

#[test]
fn a_document_the_application_took_elsewhere_is_not_announced() {
    let (mut app, _jobs, ctx) = wired();
    // What an application answers while it prompts for a password.
    app.opens_as = Ok(None);
    open_entry(&mut app, file("/Minutes.note"), None);
    pumped(&mut app, &ctx);
    assert_eq!(report(&app)["status"], serde_json::Value::Null);
    assert_eq!(report(&app)["origin"], serde_json::Value::Null, "nothing came from the cloud yet");
}

#[test]
fn a_save_goes_out_under_the_version_the_document_was_read_at() {
    let (mut app, jobs, ctx) = wired();
    open_entry(&mut app, file("/Minutes.note"), None);
    pumped(&mut app, &ctx);
    save(&mut app).unwrap();
    assert_eq!(app.did().encoded, vec![1], "the application alone turns a document into bytes");
    assert_eq!(app.did().encoded_as, vec!["Minutes.note".to_owned()], "the bytes are asked for under the name they are going to");
    let condition = jobs.lock().unwrap().iter().find_map(|j| match j {
        Job::Write { entry, .. } => Some(entry.etag.clone()),
        _ => None,
    });
    assert_eq!(condition, Some(Some("v1".to_owned())), "the etag it was read at is the condition");
    pumped(&mut app, &ctx);
    assert_eq!(app.did().marked_saved, vec![1], "the version that reached the service is the saved one");
    assert_eq!(report(&app)["origin"]["etag"], "v2", "and the document moves on to the version the service returned");
}

#[test]
fn a_read_only_share_is_never_saved_over() {
    let (mut app, _jobs, ctx) = wired();
    let mut theirs = file("/Team.note");
    theirs.writable = false;
    open_entry(&mut app, theirs, None);
    pumped(&mut app, &ctx);
    assert!(!has_target(&app), "Save has nowhere of its own to go");
    let refused = save(&mut app).unwrap_err();
    assert!(refused.contains("reading only"), "and saying why names the fix: {refused}");
}

#[test]
fn a_refused_save_is_put_to_the_user_and_never_retried() {
    let (mut app, jobs, ctx) = wired();
    open_entry(&mut app, file("/Minutes.note"), None);
    pumped(&mut app, &ctx);
    jobs.lock().unwrap().clear();
    // What a service answers when somebody else saved first.
    app.cloud.events.0.send(Event::Saved { uid: 1, entry: Err(Fault::Conflict) }).unwrap();
    pumped(&mut app, &ctx);
    assert_eq!(report(&app)["conflict"]["name"], "Minutes.note", "the user is told, and asked what to do");
    assert_eq!(app.did().dropped, vec![1], "the version that did not land is forgotten");
    assert!(jobs.lock().unwrap().is_empty(), "nothing went out again on its own");
}

#[test]
fn no_account_yet_is_an_offer_to_link_and_never_an_error() {
    let (mut app, _jobs, ctx) = wired();
    app.cloud.events.0.send(Event::Account(Err(Fault::NotLinked))).unwrap();
    pumped(&mut app, &ctx);
    assert_eq!(report(&app)["link"], "offered");
    assert_eq!(report(&app)["accountError"], serde_json::Value::Null, "not having linked an account yet is not a failure");
    assert!(app.did().said.is_empty());
}

#[test]
fn a_browser_round_trip_is_polled_until_the_user_is_done() {
    let (mut app, jobs, ctx) = wired();
    app.cloud.events.0.send(Event::Browser { url: "https://cloud.example/link/abc".to_owned(), handle: Some("abc".to_owned()) }).unwrap();
    pumped(&mut app, &ctx);
    assert_eq!(app.did().urls, vec!["https://cloud.example/link/abc".to_owned()], "the browser is opened for them");
    assert_eq!(report(&app)["link"]["waiting"], "https://cloud.example/link/abc");

    // The wait is not polled again until it is due.
    jobs.lock().unwrap().clear();
    pumped(&mut app, &ctx);
    assert!(jobs.lock().unwrap().iter().all(|j| !matches!(j, Job::LinkPoll(_))), "a poll every frame would be a poll every sixteen milliseconds");

    if let Some(Link::Waiting { next_poll, .. }) = &mut app.cloud.link {
        *next_poll = 0.0;
    }
    pumped(&mut app, &ctx);
    assert!(jobs.lock().unwrap().iter().any(|j| matches!(j, Job::LinkPoll(_))));
}

#[test]
fn what_a_person_reads_is_about_their_own_kind_of_document() {
    let app = Notebook::new();
    assert_eq!(Fault::Conflict.text(app.kind().noun), "Somebody else saved this note while you were editing it.");
    assert_eq!(Fault::Conflict.code(), "conflict", "and the control channel reads the same fact as a word");
}

#[test]
fn working_together_needs_somewhere_for_the_room_to_be_keyed_on() {
    let (mut app, jobs, ctx) = wired();
    app.cloud.account = Some(Account { display: "Ada".to_owned(), capabilities: vec![Capability::Conversation], provider_linked: true });
    assert!(can_converse(&app));
    // A document that has never been in the cloud has no handle.
    work_together(&mut app);
    assert_eq!(report(&app)["status"], "Save this note to the cloud first");
    assert!(jobs.lock().unwrap().iter().all(|j| !matches!(j, Job::Conversation(_))));

    open_entry(&mut app, file("/Minutes.note"), None);
    pumped(&mut app, &ctx);
    work_together(&mut app);
    let asked = jobs.lock().unwrap().iter().any(|j| matches!(j, Job::Conversation(id) if id == "/Minutes.note"));
    assert!(asked, "the provider is asked about the handle it issued");
}

#[test]
fn signing_out_forgets_everything_this_window_knew() {
    let (mut app, jobs, ctx) = wired();
    start(&mut app);
    pumped(&mut app, &ctx);
    open_entry(&mut app, file("/Minutes.note"), None);
    pumped(&mut app, &ctx);
    sign_out(&mut app);
    let state = report(&app);
    assert_eq!(state["account"], serde_json::Value::Null);
    assert_eq!(state["origin"], serde_json::Value::Null);
    assert_eq!(state["entries"], serde_json::json!([]));
    assert!(jobs.lock().unwrap().iter().any(|j| matches!(j, Job::SignOut)), "the service is told, so the saved token goes too");
}

#[test]
fn bytes_are_said_the_way_a_person_reads_them() {
    assert_eq!(size_text(None), "");
    assert_eq!(size_text(Some(512)), "512 bytes");
    assert_eq!(size_text(Some(2048)), "2.0 KB");
    assert_eq!(size_text(Some(5 * 1024 * 1024)), "5.0 MB");
}

#[test]
fn a_destination_is_named_after_the_account_it_belongs_to() {
    let mut app = Notebook::new();
    assert_eq!(destination_name(&app), "Cloud storage", "two entries reading the same tell nobody which account this is");
    app.cloud.account = Some(Account { display: "Ada Lovelace".to_owned(), capabilities: Vec::new(), provider_linked: true });
    assert_eq!(destination_name(&app), "Ada Lovelace");
}

#[test]
fn the_contracts_own_conflict_is_the_one_a_save_must_not_retry() {
    assert_eq!(Fault::from(craft_cloud::Error::Conflict("Minutes.note".to_owned())), Fault::Conflict);
    assert_eq!(Fault::from(craft_cloud::Error::Unauthenticated), Fault::Unauthenticated);
    assert!(matches!(Fault::from(craft_cloud::Error::Unsupported("sharing")), Fault::Refused(_)));
}

#[test]
fn a_save_asks_for_the_bytes_under_the_name_they_are_going_to() {
    // A document opened as one format and saved in the application's own would
    // hand the user a file no other application reads, under a name that says
    // it is readable. The application cannot avoid that without being told
    // where the bytes are going.
    let (mut app, _jobs, ctx) = wired();
    let mut sheet = file("/figures.csv");
    sheet.mime = Some("text/csv".to_owned());
    open_entry(&mut app, sheet, None);
    pumped(&mut app, &ctx);
    save(&mut app).unwrap();
    assert_eq!(app.did().encoded_as, vec!["figures.csv".to_owned()]);
    assert_eq!(app.did().encoded_mime, vec![Some("text/csv".to_owned())], "the provider's own word for it comes too");

    // And Save As, where the name is the one just typed rather than the one
    // the document arrived under.
    let (mut app, _jobs, ctx) = wired();
    open_entry(&mut app, file("/figures.csv"), None);
    pumped(&mut app, &ctx);
    save_as(&mut app, "", "a copy.note").unwrap();
    assert_eq!(app.did().encoded_as, vec!["a copy.note".to_owned()]);
    assert_eq!(app.did().encoded_mime, vec![None], "nothing is there yet, so nothing says what it is");
}

#[test]
fn a_destination_reads_its_own_extension() {
    assert_eq!(Destination::new("figures.csv").extension(), "csv");
    assert_eq!(Destination::new("FIGURES.CSV").extension(), "csv", "a shouted name is the same name");
    assert_eq!(Destination::new("report.2026.xlsx").extension(), "xlsx", "the last one wins");
    // A bare name, which is what a web application's export commonly has.
    // Everything an application can learn is in the content type instead.
    assert_eq!(Destination::new("exported").extension(), "");
    assert_eq!(Destination::new("trailing.").extension(), "");
}

// ---- somebody who followed a shared link ----------------------------------

/// One frame of the title bar's right-hand end, as the accessibility tree
/// reported it.
///
/// Read off the tree rather than out of the state, because the question is
/// what a recipient can reach. A field saying "no menu" proves nothing about
/// what was drawn.
fn title_bar(app: &mut Notebook) -> Vec<(egui::accesskit::Role, String)> {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 48.0))), ..Default::default() };
    let mut output = ctx.run_ui(raw, |ui| who(app, ui));
    output.textures_delta.clear();
    let Some(tree) = output.platform_output.accesskit_update.as_ref() else { return Vec::new() };
    tree.nodes
        .iter()
        .filter_map(|(_, node)| {
            // egui puts a plain label's text in the accessibility value and a
            // control's text in the label, so both are text on screen.
            node.label().or_else(|| node.value()).map(|text| (node.role(), text.to_owned()))
        })
        .collect()
}

fn pressable(seen: &[(egui::accesskit::Role, String)]) -> Vec<String> {
    seen.iter().filter(|(role, _)| *role == egui::accesskit::Role::Button).map(|(_, text)| text.clone()).collect()
}

fn reads(seen: &[(egui::accesskit::Role, String)], fragment: &str) -> bool {
    seen.iter().any(|(_, text)| text.contains(fragment))
}

#[test]
fn a_recipient_is_named_and_offered_nothing() {
    let mut app = Notebook::new();
    app.cloud.as_guest("outsider@example.org", false);
    let seen = title_bar(&mut app);
    assert!(reads(&seen, "outsider@example.org"), "a recipient has no account, so their own address is the identity to show: {seen:?}");
    // The whole point. Every item an account menu holds is a sign-out, a
    // switch or a storage connection, and a recipient has none of those.
    assert_eq!(pressable(&seen), Vec::<String>::new(), "nothing here may be pressed: {seen:?}");
    for offer in ["Sign in", "Sign out", "Account", "Connect storage", "Add an account"] {
        assert!(!reads(&seen, offer), "a recipient was offered {offer}");
    }
}

#[test]
fn a_read_only_share_says_so_before_anybody_tries_to_save() {
    let mut app = Notebook::new();
    app.cloud.as_guest("outsider@example.org", true);
    assert!(reads(&title_bar(&mut app), "Reading only"), "finding out at the moment of saving means doing the work twice");

    app.cloud.as_guest("outsider@example.org", false);
    assert!(!reads(&title_bar(&mut app), "Reading only"), "a share that allows an edit must not say otherwise");
}

#[test]
fn an_address_of_only_spaces_leaves_no_badge() {
    let mut app = Notebook::new();
    app.cloud.as_guest("   ", false);
    assert!(app.cloud.guest().is_none(), "a badge with nothing on it reads as a fault");
    assert_eq!(title_bar(&mut app), Vec::new());
}

#[test]
fn a_signed_in_window_still_gets_its_account_badge() {
    let (mut app, _jobs, ctx) = wired();
    start(&mut app);
    pumped(&mut app, &ctx);
    let seen = title_bar(&mut app);
    assert!(!pressable(&seen).is_empty(), "somebody with an account still has a menu to reach: {seen:?}");
    assert!(reads(&seen, "Ada Lovelace"));
}

#[test]
fn the_report_says_who_this_window_is_for() {
    let mut app = Notebook::new();
    assert_eq!(report(&app)["guest"], serde_json::Value::Null, "most windows belong to nobody in particular yet");
    app.cloud.as_guest("outsider@example.org", true);
    assert_eq!(report(&app)["guest"], serde_json::json!({"recipient": "outsider@example.org", "readOnly": true}));
}
