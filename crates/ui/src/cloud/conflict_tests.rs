//! Two risks a tabbed host runs that a single-document test cannot see: a document other than
//! the one a conflict or a job is about can be active when the answer lands, and a session that
//! ended can still have a job outstanding.
//!
//! `Tabs` is a host with two open documents (a tabbed window) and `Held` is a backend that keeps
//! every job so the test decides when, and with what, the service answers.

use std::sync::{Arc, Mutex};

use craft_cloud::{Capability, Entry, EntryKind};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

const NOTES: DocumentKind = DocumentKind { extensions: &["note"], mimes: &["text/plain"], noun: "note", default_extension: "note" };

#[derive(Default)]
struct Log {
    read: Vec<String>,
    encoded: Vec<u64>,
    closed: Vec<u64>,
    said: Vec<String>,
}

struct Tabs {
    cloud: CloudFiles,
    share: crate::share::ShareDialog,
    presence: crate::presence::PresenceBar,
    active: u64,
    next_uid: u64,
    log: Arc<Mutex<Log>>,
}

impl CloudApp for Tabs {
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
        Some(self.active)
    }
    fn title(&self, uid: u64) -> String {
        format!("doc {uid}")
    }
    fn open_bytes(&mut self, name: &str, _bytes: Vec<u8>) -> Result<Option<u64>, String> {
        self.log.lock().unwrap().read.push(name.to_owned());
        self.next_uid += 1;
        Ok(Some(self.next_uid))
    }
    fn encode(&mut self, uid: u64, _into: Destination<'_>) -> Result<Vec<u8>, String> {
        self.log.lock().unwrap().encoded.push(uid);
        Ok(format!("bytes of {uid}").into_bytes())
    }
    fn close(&mut self, uid: u64) {
        self.log.lock().unwrap().closed.push(uid);
    }
    fn sharing(&mut self) -> &mut crate::share::ShareDialog {
        &mut self.share
    }
    fn presence(&mut self) -> &mut crate::presence::PresenceBar {
        &mut self.presence
    }
    fn paint_icon(&self, _p: &egui::Painter, _r: egui::Rect, _k: EntryKind, _c: egui::Color32) {}
    fn open_url(&self, _url: &str) {}
    fn say(&mut self, text: String) {
        self.log.lock().unwrap().said.push(text);
    }
    fn app_name(&self) -> &str {
        "Notes"
    }
}

#[derive(Clone, Default)]
struct Held(Arc<Mutex<Vec<(Job, Post)>>>);

impl CloudBackend for Held {
    fn submit(&self, job: Job, post: Post) {
        self.0.lock().unwrap().push((job, post));
    }
}

impl Held {
    fn take(&self, pick: impl Fn(&Job) -> bool) -> Option<(Job, Post)> {
        let mut held = self.0.lock().unwrap();
        let i = held.iter().position(|(j, _)| pick(j))?;
        Some(held.remove(i))
    }
}

fn entry(path: &str, etag: &str) -> Entry {
    Entry {
        id: path.to_owned(),
        room: format!("room:{path}"),
        name: path.trim_start_matches('/').to_owned(),
        path: path.to_owned(),
        kind: EntryKind::File,
        size: Some(10),
        modified: None,
        mime: None,
        etag: Some(etag.to_owned()),
        writable: true,
        shared_by: None,
    }
}

fn account(name: &str) -> Account {
    Account { display: name.to_owned(), capabilities: vec![Capability::Files, Capability::Etags], provider_linked: true }
}

fn drain(app: &mut Tabs, ctx: &egui::Context) {
    ctx.begin_pass(Default::default());
    pump(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

/// Two cloud documents open: uid 1 = A.note, uid 2 = B.note (B has unsaved edits).
fn two_tabs() -> (Tabs, Held, Arc<Mutex<Log>>) {
    let held = Held::default();
    let log = Arc::new(Mutex::new(Log::default()));
    let mut app = Tabs {
        cloud: CloudFiles::new(),
        share: crate::share::ShareDialog::new(),
        presence: crate::presence::PresenceBar::new(),
        active: 1,
        next_uid: 10,
        log: log.clone(),
    };
    app.cloud.connect(held.clone());
    app.cloud.origins.insert(1, Origin { entry: entry("/A.note", "a1") });
    app.cloud.origins.insert(2, Origin { entry: entry("/B.note", "b1") });
    (app, held, log)
}

/// Save A, the user moves to tab B while the write is on the wire, the service refuses A's write
/// (Conflict). Returns the app with the conflict modal pending, active tab = B.
fn conflict_on_a_while_b_is_active() -> (Tabs, Held, Arc<Mutex<Log>>) {
    let (mut app, held, log) = two_tabs();
    save(&mut app).unwrap();
    app.active = 2;
    let (job, post) = held.take(|j| matches!(j, Job::Write { .. })).unwrap();
    let Job::Write { uid, .. } = job else { panic!("expected Job::Write") };
    post.send(Event::Saved { uid, entry: Err(Fault::Conflict) });
    let ctx = egui::Context::default();
    drain(&mut app, &ctx);
    assert_eq!(app.cloud.conflict.as_ref().map(|c| (c.uid, c.entry.name.clone())), Some((1, "A.note".to_owned())));
    (app, held, log)
}

fn click_in_conflict_modal(app: Tabs, label: &str) -> Tabs {
    let mut harness = Harness::new_ui_state(|ui, app: &mut Tabs| conflict_modal(app, &ui.ctx().clone()), app);
    harness.run();
    harness.get_by_label(label).click();
    harness.run();
    harness.into_state()
}

/// The modal says A.note was not saved; clicking "Reload from the cloud" must reload A and
/// replace uid 1, the conflicted document, never whichever tab happens to be active.
#[test]
fn conflict_reload_acts_on_the_conflicted_document_even_when_another_tab_is_active() {
    let (app, held, log) = conflict_on_a_while_b_is_active();
    let mut app = click_in_conflict_modal(app, "Reload from the cloud");
    let (job, post) = held.take(|j| matches!(j, Job::Read { .. })).expect("Reload submitted a read");
    let Job::Read { entry: read, into } = job else { panic!("expected Job::Read") };
    let read_name = read.name.clone();
    post.send(Event::Read { entry: read, into, bytes: Ok(b"stored".to_vec()) });
    drain(&mut app, &egui::Context::default());
    let log = log.lock().unwrap();
    assert!(
        read_name == "A.note" && into == Some(1) && !log.closed.contains(&2),
        "conflict was on uid 1 (A.note); Reload read {read_name:?} into {into:?}, then closed {:?}",
        log.closed
    );
}

/// "Save as a copy" from the conflict modal must encode the conflicted document, A, not
/// whichever tab happens to be active when the Save As page's own button is finally clicked.
#[test]
fn conflict_save_as_copy_encodes_the_conflicted_document_even_when_another_tab_is_active() {
    let (app, held, log) = conflict_on_a_while_b_is_active();
    let mut app = click_in_conflict_modal(app, "Save as a copy…");
    let name = app.cloud.save_name.clone();
    save_as(&mut app, "", &name).unwrap(); // what the Save As page does with that name
    let (job, _) = held.take(|j| matches!(j, Job::Create { .. })).unwrap();
    let Job::Create { uid, name, bytes, .. } = job else { panic!("expected Job::Create") };
    assert_eq!(
        (uid, String::from_utf8_lossy(&bytes).into_owned()),
        (1, "bytes of 1".to_owned()),
        "copy of the conflicted A.note is created as {name:?} from uid {uid} (encoded uids {:?})",
        log.lock().unwrap().encoded
    );
}

/// A read the backend never answers (the trait allows "or never") must not leave every transfer
/// refused forever. Signing out and back in recovers Save.
#[test]
fn sign_out_recovers_save_after_a_backend_job_never_replies() {
    let (mut app, held, _log) = two_tabs();
    open_entry(&mut app, entry("/C.note", "c1"), None);
    held.take(|j| matches!(j, Job::Read { .. })).unwrap(); // dropped by the worker
    sign_out(&mut app);
    sign_in(&mut app);
    let (_, post) = held.take(|j| matches!(j, Job::SignIn)).unwrap();
    post.send(Event::Account(Ok(account("Ada"))));
    drain(&mut app, &egui::Context::default());
    app.cloud.origins.insert(1, Origin { entry: entry("/A.note", "a1") });
    let saved = save(&mut app);
    assert!(saved.is_ok(), "after sign-out and sign-in, Save is still refused: {saved:?}; busy={:?}", app.cloud.busy.as_ref().map(Busy::text));
}

/// An account and a listing answer a job issued before sign-out. Landing after it, they must
/// not put the previous session's account and file names on screen with nobody signed in.
#[test]
fn a_late_reply_from_before_sign_out_is_dropped() {
    let (mut app, held, _log) = two_tabs();
    start(&mut app);
    let (_, account_post) = held.take(|j| matches!(j, Job::Account)).unwrap();
    let (_, list_post) = held.take(|j| matches!(j, Job::List(_))).unwrap();
    sign_out(&mut app);
    account_post.send(Event::Account(Ok(account("Previous Person"))));
    list_post.send(Event::Listed { path: String::new(), entries: Ok(vec![entry("/Payroll 2026.note", "p1")]) });
    drain(&mut app, &egui::Context::default());
    let r = report(&app);
    assert!(
        r["account"].is_null() && r["entries"].as_array().is_some_and(Vec::is_empty),
        "signed out, yet account={} entries={}",
        r["account"],
        r["entries"]
    );
}
