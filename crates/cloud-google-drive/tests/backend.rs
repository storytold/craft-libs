#![cfg(not(target_arch = "wasm32"))]

//! Drives [`GoogleDriveBackend`] the way an app does: through `craft_ui::cloud`'s own public
//! functions, against a fake Google that answers by URL. No test reaches the network or the
//! keychain; the one sign-in test uses the real loopback listener on `127.0.0.1`.

use std::io::{Read as _, Write as _};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_cloud::{Capability, EntryKind};
use craft_cloud_google_drive::http::{Method, Request, Response, Transport};
use craft_cloud_google_drive::oauth::{self, ClientConfig};
use craft_cloud_google_drive::{Error, GoogleDriveBackend};
use craft_suite_account::{Memory, Secret, TokenStore};
use craft_ui::cloud::{self, CloudApp, CloudFiles, Destination, DocumentKind, Fault, Job, Link};
use serde_json::{Value, json};

const SHEET: DocumentKind = DocumentKind {
    extensions: &["xlsx"],
    mimes: &["application/vnd.openxmlformats-officedocument.spreadsheetml"],
    noun: "workbook",
    default_extension: "xlsx",
};
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const GOOGLE_SHEET: &str = "application/vnd.google-apps.spreadsheet";
const FOLDER: &str = "application/vnd.google-apps.folder";

// ---------------------------------------------------------------- a fake Google

type Answer = Box<dyn Fn(&Request) -> Option<(u16, Vec<u8>)> + Send + Sync>;

/// Answers each request with the first route that claims it, and records every request.
#[derive(Default)]
struct Google {
    routes: Mutex<Vec<Answer>>,
    seen: Mutex<Vec<Request>>,
}

impl Google {
    fn new() -> Arc<Google> {
        let google = Arc::new(Google::default());
        // Every refresh succeeds unless a test says otherwise.
        google.on(|r| (r.url == oauth::TOKEN_ENDPOINT).then(|| json_reply(200, json!({"access_token": "access-1", "expires_in": 3599}))));
        google.on(|r| (r.url == oauth::REVOKE_ENDPOINT).then(|| json_reply(200, json!({}))));
        google.on(|r| r.url.contains("/about?").then(|| json_reply(200, json!({"user": {"displayName": "Test Person"}}))));
        google
    }
    fn on(&self, route: impl Fn(&Request) -> Option<(u16, Vec<u8>)> + Send + Sync + 'static) {
        self.routes.lock().unwrap().push(Box::new(route));
    }
    fn seen(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
    fn query(&self, needle: &str) -> Vec<String> {
        self.seen().into_iter().map(|r| decode(&r.url)).filter(|u| u.contains(needle)).collect()
    }
}

impl Transport for Google {
    fn send(&self, req: Request) -> craft_cloud_google_drive::Result<Response> {
        self.seen.lock().unwrap().push(req.clone());
        let routes = self.routes.lock().unwrap();
        // Later routes win, so a test can override a default.
        for route in routes.iter().rev() {
            if let Some((status, body)) = route(&req) {
                return Ok(Response { status, body });
            }
        }
        Err(Error::Network(format!("no route for {}", req.url)))
    }
}

fn json_reply(status: u16, v: Value) -> (u16, Vec<u8>) {
    (status, v.to_string().into_bytes())
}

fn decode(url: &str) -> String {
    craft_cloud_google_drive::encode::unpct(url)
}

fn file(id: &str, name: &str, mime: &str, rev: &str) -> Value {
    json!({"id": id, "name": name, "mimeType": mime, "version": "1", "headRevisionId": rev, "size": "4", "capabilities": {"canEdit": true}})
}

fn client() -> ClientConfig {
    ClientConfig::new("123-abc.apps.googleusercontent.com", Some("not-really-secret"), None).unwrap()
}

fn signed_in_store() -> Memory {
    let store = Memory::new();
    store.put(&oauth::token_handle(&client()), &Secret::new("refresh-1")).unwrap();
    store
}

// ---------------------------------------------------------------- the least an app can be

/// What the harness was told to open: each document's name and bytes.
type Opened = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

struct Harness {
    cloud: CloudFiles,
    share: craft_ui::share::ShareDialog,
    presence: craft_ui::presence::PresenceBar,
    opened: Opened,
    said: Arc<Mutex<Vec<String>>>,
    urls: Arc<Mutex<Vec<String>>>,
}

impl CloudApp for Harness {
    fn cloud(&self) -> &CloudFiles {
        &self.cloud
    }
    fn cloud_mut(&mut self) -> &mut CloudFiles {
        &mut self.cloud
    }
    fn kind(&self) -> DocumentKind {
        SHEET
    }
    fn active(&self) -> Option<u64> {
        Some(1)
    }
    fn title(&self, _uid: u64) -> String {
        "Budget".into()
    }
    fn open_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String> {
        self.opened.lock().unwrap().push((name.to_owned(), bytes));
        Ok(Some(1))
    }
    fn encode(&mut self, _uid: u64, _into: Destination<'_>) -> Result<Vec<u8>, String> {
        Ok(b"PKxl".to_vec())
    }
    fn close(&mut self, _uid: u64) {}
    fn sharing(&mut self) -> &mut craft_ui::share::ShareDialog {
        &mut self.share
    }
    fn presence(&mut self) -> &mut craft_ui::presence::PresenceBar {
        &mut self.presence
    }
    fn paint_icon(&self, _painter: &egui::Painter, _rect: egui::Rect, _kind: EntryKind, _colour: egui::Color32) {}
    fn open_url(&self, url: &str) {
        self.urls.lock().unwrap().push(url.to_owned());
    }
    fn say(&mut self, text: String) {
        self.said.lock().unwrap().push(text);
    }
    fn app_name(&self) -> &str {
        "Harness"
    }
}

fn wired(config: craft_cloud_google_drive::Result<ClientConfig>, google: &Arc<Google>, store: Memory) -> (Harness, egui::Context) {
    let mut app = Harness {
        cloud: CloudFiles::new(),
        share: craft_ui::share::ShareDialog::new(),
        presence: craft_ui::presence::PresenceBar::new(),
        opened: Arc::default(),
        said: Arc::default(),
        urls: Arc::default(),
    };
    let http: Arc<dyn Transport> = google.clone();
    app.cloud.connect(GoogleDriveBackend::with(config, http, Arc::new(store), "Harness").sign_in_timeout(Duration::from_secs(10)));
    (app, egui::Context::default())
}

/// Pumps the pages until `done`, or fails after ten seconds.
fn settle(app: &mut Harness, ctx: &egui::Context, mut done: impl FnMut(&Harness) -> bool) {
    let start = Instant::now();
    loop {
        ctx.begin_pass(Default::default());
        cloud::pump(app, ctx);
        ctx.end_pass().textures_delta.clear();
        if done(app) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for the backend");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// My Drive holds a folder and a Google Sheet; the folder holds an `.xlsx`.
fn drive_with_files(google: &Arc<Google>) {
    google.on(|r| {
        let url = decode(&r.url);
        (r.method == Method::Get && url.contains("'root' in parents")).then(|| {
            json_reply(
                200,
                json!({"files": [
                    {"id": "fold1", "name": "Reports", "mimeType": FOLDER},
                    {"id": "fold2", "name": "Reports", "mimeType": FOLDER},
                    file("sheet1", "Budget", GOOGLE_SHEET, ""),
                ]}),
            )
        })
    });
    google.on(|r| {
        let url = decode(&r.url);
        (r.method == Method::Get && url.contains("'fold1' in parents"))
            .then(|| json_reply(200, json!({"files": [file("x1", "Q3.xlsx", XLSX, "rev-1")]})))
    });
    google.on(|r| (r.method == Method::Get && decode(&r.url).contains("'fold2' in parents")).then(|| json_reply(200, json!({"files": []}))));
}

// ---------------------------------------------------------------- tests

#[test]
fn without_a_client_the_account_says_how_to_set_one_up() {
    let google = Google::new();
    let (mut app, ctx) = wired(Err(Error::NotConfigured("add google-drive.json".into())), &google, Memory::new());
    cloud::start(&mut app);
    settle(&mut app, &ctx, |a| a.cloud().account_fault.is_some());
    assert!(matches!(&app.cloud().account_fault, Some(Fault::Refused(m)) if m.contains("google-drive.json")));
    assert!(google.seen().is_empty(), "nothing goes to Google without a client");
}

#[test]
fn a_client_without_a_sign_in_offers_to_link() {
    let google = Google::new();
    let (mut app, ctx) = wired(Ok(client()), &google, Memory::new());
    cloud::start(&mut app);
    settle(&mut app, &ctx, |a| matches!(a.cloud().link, Some(Link::Offered)));
    assert!(app.cloud().account.is_none());
    assert!(google.seen().is_empty(), "nothing goes to Google before a sign-in");
}

#[test]
fn a_stored_sign_in_names_the_account_and_what_it_can_do() {
    let google = Google::new();
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::submit(&app, Job::Account);
    settle(&mut app, &ctx, |a| a.cloud().account.is_some());
    let account = app.cloud().account.clone().unwrap();
    assert_eq!(account.display, "Test Person");
    assert!(account.provider_linked);
    assert_eq!(account.capabilities, vec![Capability::Files, Capability::Etags, Capability::SharedWithMe]);
}

#[test]
fn folders_are_browsed_by_path_and_same_named_ones_stay_apart() {
    let google = Google::new();
    drive_with_files(&google);
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());

    cloud::browse(&mut app, "");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    let paths: Vec<_> = app.cloud().mine.entries.iter().map(|e| (e.path.clone(), e.name.clone())).collect();
    assert_eq!(
        paths,
        [("/Reports".into(), "Reports".into()), ("/Reports (2)".into(), "Reports".into()), ("/Budget".into(), "Budget.xlsx".into())],
        "two folders named alike get two paths; a Google Sheet opens as .xlsx"
    );
    let sheet = &app.cloud().mine.entries[2];
    assert_eq!(sheet.mime.as_deref(), Some(XLSX));
    assert!(!sheet.writable, "a Google Sheet is saved as a copy, never overwritten");

    cloud::browse(&mut app, "/Reports");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded && a.cloud().mine.path == "/Reports");
    let inside = &app.cloud().mine.entries;
    assert_eq!(inside.len(), 1);
    assert_eq!(inside[0].path, "/Reports/Q3.xlsx");
    assert_eq!(inside[0].etag.as_deref(), Some("rev-1"));
    assert!(google.query("name = ").is_empty(), "a folder already listed needs no lookup by name");
}

#[test]
fn a_path_never_listed_is_found_by_listing_its_parents() {
    let google = Google::new();
    drive_with_files(&google);
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    // The second of two folders named Reports, asked for before anything was listed.
    cloud::browse(&mut app, "/Reports (2)");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    assert!(app.cloud().mine.error.is_none(), "{:?}", app.cloud().mine.error);
    assert_eq!(google.query("'fold2' in parents").len(), 1, "the second Reports, found by listing My Drive");
    assert!(google.query("name = ").is_empty(), "never searched for by name");
}

#[test]
fn a_missing_folder_is_a_fault_not_my_drive() {
    let google = Google::new();
    drive_with_files(&google);
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::browse(&mut app, "/Nowhere");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    assert!(matches!(&app.cloud().mine.error, Some(Fault::Refused(m)) if m.contains("/Nowhere")));
    assert!(app.cloud().mine.entries.is_empty(), "My Drive wasn't shown in its place");
}

#[test]
fn save_as_over_a_name_already_there_is_refused() {
    let google = Google::new();
    drive_with_files(&google);
    google.on(|r| decode(&r.url).contains("name = 'Q3.xlsx'").then(|| json_reply(200, json!({"files": [{"id": "x1"}]}))));
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::browse(&mut app, "/Reports");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    cloud::save_as(&mut app, "/Reports", "Q3.xlsx").unwrap();
    settle(&mut app, &ctx, |a| a.cloud().busy.is_none());
    assert!(app.said.lock().unwrap().iter().any(|m| m.contains("already exists")), "{:?}", app.said.lock().unwrap());
    assert!(google.seen().iter().all(|r| r.method != Method::Post || r.url == oauth::TOKEN_ENDPOINT), "no upload");
}

#[test]
fn opening_a_google_sheet_exports_it_and_remembers_where_it_came_from() {
    let google = Google::new();
    drive_with_files(&google);
    google.on(|r| r.url.contains("/files/sheet1?fields").then(|| json_reply(200, file("sheet1", "Budget", GOOGLE_SHEET, ""))));
    google.on(|r| r.url.contains("/files/sheet1/export?").then(|| (200, b"PKxl".to_vec())));
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::browse(&mut app, "");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    let entry = app.cloud().mine.entries[2].clone();

    cloud::submit(&app, Job::Read { entry, into: None });
    settle(&mut app, &ctx, |a| !a.opened.lock().unwrap().is_empty());
    assert_eq!(app.opened.lock().unwrap()[0], ("Budget.xlsx".to_owned(), b"PKxl".to_vec()));
    let origin = app.cloud().origins.get(&1).cloned().unwrap();
    assert_eq!(origin.entry.id, "sheet1");
    assert!(cloud::save(&mut app).is_err(), "Save refuses and points to Save As");
}

#[test]
fn a_save_over_a_changed_file_is_refused_and_nothing_is_uploaded() {
    let google = Google::new();
    drive_with_files(&google);
    // Somebody else saved: the content revision moved from rev-1 to rev-2.
    google.on(|r| r.url.contains("/files/x1?fields").then(|| json_reply(200, file("x1", "Q3.xlsx", XLSX, "rev-2"))));
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::browse(&mut app, "/Reports");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    let entry = app.cloud().mine.entries[0].clone();
    assert_eq!(entry.etag.as_deref(), Some("rev-1"));

    cloud::submit(&app, Job::Write { uid: 1, entry: entry.clone(), bytes: b"PKmine".to_vec() });
    // The pages show the conflict for a document they know came from the cloud.
    app.cloud_mut().origins.insert(1, cloud::Origin { entry });
    settle(&mut app, &ctx, |a| a.cloud().conflict.is_some());
    assert!(google.seen().iter().all(|r| r.method != Method::Patch), "the refused save sent no bytes");
}

#[test]
fn an_unchanged_file_is_saved_back_at_its_new_version() {
    let google = Google::new();
    google.on(|r| (r.method == Method::Get && r.url.contains("/files/x1?fields")).then(|| json_reply(200, file("x1", "Q3.xlsx", XLSX, "rev-1"))));
    google.on(|r| {
        (r.method == Method::Patch && r.url.contains("/files/x1?uploadType=media")).then(|| json_reply(200, file("x1", "Q3.xlsx", XLSX, "rev-2")))
    });
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    let entry = craft_cloud::Entry {
        id: "x1".into(),
        room: "x1".into(),
        name: "Q3.xlsx".into(),
        path: "/Reports/Q3.xlsx".into(),
        kind: EntryKind::File,
        size: None,
        modified: None,
        mime: Some(XLSX.into()),
        etag: Some("rev-1".into()),
        writable: true,
        shared_by: None,
    };
    app.cloud_mut().origins.insert(1, cloud::Origin { entry });
    cloud::save(&mut app).unwrap();
    settle(&mut app, &ctx, |a| a.cloud().busy.is_none() && a.cloud().origins.get(&1).is_some_and(|o| o.entry.etag.as_deref() == Some("rev-2")));
    let up = google.seen().into_iter().find(|r| r.method == Method::Patch).unwrap();
    assert_eq!(&*up.body, &b"PKxl"[..]);
    assert_eq!(app.cloud().origins.get(&1).unwrap().entry.path, "/Reports/Q3.xlsx", "the path survives the save");
}

#[test]
fn save_as_goes_into_the_folder_being_browsed() {
    let google = Google::new();
    drive_with_files(&google);
    google.on(|r| {
        (r.method == Method::Post && r.url.contains("uploadType=multipart")).then(|| json_reply(200, file("n1", "Plan.xlsx", XLSX, "rev-1")))
    });
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::browse(&mut app, "");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    cloud::browse(&mut app, "/Reports (2)");
    settle(&mut app, &ctx, |a| a.cloud().mine.path == "/Reports (2)" && !a.cloud().mine.loading);

    cloud::save_as(&mut app, "/Reports (2)", "Plan").unwrap();
    settle(&mut app, &ctx, |a| a.cloud().origins.contains_key(&1));
    let up = google.seen().into_iter().find(|r| r.method == Method::Post && r.url.contains("uploadType=multipart")).unwrap();
    let body = String::from_utf8_lossy(&up.body);
    assert!(body.contains(r#""parents":["fold2"]"#), "the second Reports, not the first: {body}");
    assert!(body.contains(r#""name":"Plan.xlsx""#), "{body}");
    assert_eq!(app.cloud().origins.get(&1).unwrap().entry.path, "/Reports (2)/Plan.xlsx");
}

#[test]
fn shared_files_say_who_shared_them_and_folders_are_left_out() {
    let google = Google::new();
    google.on(|r| {
        decode(&r.url).contains("sharedWithMe = true").then(|| {
            let mut shared = file("s1", "Team.xlsx", XLSX, "rev-1");
            shared["sharingUser"] = json!({"displayName": "A Colleague"});
            shared["capabilities"] = json!({"canEdit": false});
            json_reply(200, json!({"files": [shared, {"id": "sf", "name": "Shared folder", "mimeType": FOLDER}]}))
        })
    });
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::load_shared(&mut app);
    settle(&mut app, &ctx, |a| a.cloud().shared.loaded);
    let shared = &app.cloud().shared.entries;
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].shared_by.as_deref(), Some("A Colleague"));
    assert!(!shared[0].writable);
}

#[test]
fn sign_out_revokes_at_google_and_forgets_the_token() {
    let google = Google::new();
    let store = signed_in_store();
    let (mut app, ctx) = wired(Ok(client()), &google, store.clone());
    cloud::sign_out(&mut app);
    let start = Instant::now();
    while store.get(&oauth::token_handle(&client())).unwrap().is_some() {
        assert!(start.elapsed() < Duration::from_secs(10), "the token was never removed");
        std::thread::sleep(Duration::from_millis(5));
    }
    // The token goes first; the revoke follows on the worker.
    let start = Instant::now();
    while !google.seen().iter().any(|r| r.url == oauth::REVOKE_ENDPOINT) {
        assert!(start.elapsed() < Duration::from_secs(10), "never revoked at Google");
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = ctx;
    let revoke = google.seen().into_iter().find(|r| r.url == oauth::REVOKE_ENDPOINT).unwrap();
    assert!(String::from_utf8_lossy(&revoke.body).contains("token=refresh-1"));
}

#[test]
fn signing_in_opens_the_browser_and_ends_with_the_account() {
    let google = Google::new();
    google.on(|r| {
        (r.url == oauth::TOKEN_ENDPOINT && String::from_utf8_lossy(&r.body).contains("grant_type=authorization_code"))
            .then(|| json_reply(200, json!({"access_token": "access-1", "refresh_token": "refresh-new", "expires_in": 3599})))
    });
    drive_with_files(&google);
    let store = Memory::new();
    let (mut app, ctx) = wired(Ok(client()), &google, store.clone());
    // A cold start with no sign-in: My Drive answers "not linked".
    cloud::start(&mut app);
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    assert_eq!(app.cloud().mine.error, Some(Fault::NotLinked));
    cloud::sign_in(&mut app);
    settle(&mut app, &ctx, |a| !a.urls.lock().unwrap().is_empty());

    // The browser: Google sends it back to the loopback address with a code and the state.
    let url = app.urls.lock().unwrap()[0].clone();
    assert!(url.starts_with(oauth::AUTH_ENDPOINT));
    let query = craft_cloud_google_drive::encode::parse_query(url.split_once('?').unwrap().1, 32);
    let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
    let addr = get("redirect_uri").trim_start_matches("http://").to_owned();
    let mut browser = std::net::TcpStream::connect(&addr).unwrap();
    write!(browser, "GET /?state={}&code=the-code HTTP/1.1\r\nHost: {addr}\r\n\r\n", get("state")).unwrap();
    let mut page = String::new();
    let _ = browser.read_to_string(&mut page);
    assert!(page.contains("go back to Harness"), "the page names the app: {page}");

    settle(&mut app, &ctx, |a| a.cloud().account.is_some());
    assert_eq!(app.cloud().account.as_ref().unwrap().display, "Test Person");
    let kept = store.get(&oauth::token_handle(&client())).unwrap().unwrap();
    assert_eq!(kept.expose(), "refresh-new", "the refresh token went to the keychain");
    // The badge's sign-in lists My Drive again, in place of the cold start's "not linked".
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded && a.cloud().mine.error.is_none() && !a.cloud().mine.entries.is_empty());
    assert!(app.cloud().link.is_none());
}

#[test]
fn a_badge_sign_in_denied_in_the_browser_ends_the_wait_with_try_again() {
    let google = Google::new();
    let (mut app, ctx) = wired(Ok(client()), &google, Memory::new());
    cloud::sign_in(&mut app);
    settle(&mut app, &ctx, |a| !a.urls.lock().unwrap().is_empty());
    let url = app.urls.lock().unwrap()[0].clone();
    let query = craft_cloud_google_drive::encode::parse_query(url.split_once('?').unwrap().1, 32);
    let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
    let addr = get("redirect_uri").trim_start_matches("http://").to_owned();
    let mut browser = std::net::TcpStream::connect(&addr).unwrap();
    write!(browser, "GET /?state={}&error=access_denied HTTP/1.1\r\n\r\n", get("state")).unwrap();
    let mut page = String::new();
    let _ = browser.read_to_string(&mut page);
    settle(&mut app, &ctx, |a| matches!(a.cloud().link, Some(Link::Failed(_))));
}

#[test]
fn signing_out_during_a_long_transfer_removes_the_token_at_once() {
    let google = Google::new();
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    google.on(move |r| {
        r.url.contains("/files/big?fields").then(|| {
            // A download that takes until the test lets it finish.
            let _ = wait.lock().unwrap().recv_timeout(Duration::from_secs(10));
            json_reply(200, file("big", "Big.xlsx", XLSX, "rev-1"))
        })
    });
    let store = signed_in_store();
    let (mut app, ctx) = wired(Ok(client()), &google, store.clone());
    let entry = craft_cloud::Entry {
        id: "big".into(),
        room: "big".into(),
        name: "Big.xlsx".into(),
        path: "/Big.xlsx".into(),
        kind: EntryKind::File,
        size: None,
        modified: None,
        mime: None,
        etag: None,
        writable: true,
        shared_by: None,
    };
    cloud::submit(&app, Job::Read { entry, into: None });
    let start = Instant::now();
    while !google.seen().iter().any(|r| r.url.contains("/files/big?fields")) {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    cloud::sign_out(&mut app);
    let start = Instant::now();
    while store.get(&oauth::token_handle(&client())).unwrap().is_some() {
        assert!(start.elapsed() < Duration::from_secs(5), "the token waited for the transfer");
        std::thread::sleep(Duration::from_millis(5));
    }
    release.send(()).unwrap();
    let _ = ctx;
}

#[test]
fn a_link_refused_before_the_browser_opens_says_why() {
    let google = Google::new();
    let (mut app, ctx) = wired(Err(Error::NotConfigured("no client".into())), &google, Memory::new());
    cloud::link(&mut app);
    settle(&mut app, &ctx, |a| matches!(a.cloud().link, Some(Link::Failed(_))));
}

#[test]
fn a_file_changed_during_its_download_opens_at_the_revision_it_has() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let google = Google::new();
    drive_with_files(&google);
    // The first download lands while somebody saves rev-2; the second is stable.
    let downloads = Arc::new(AtomicUsize::new(0));
    let seen = downloads.clone();
    google.on(move |r| {
        r.url.contains("/files/x1?fields").then(|| {
            let rev = if seen.load(Ordering::SeqCst) == 0 { "rev-1" } else { "rev-2" };
            json_reply(200, file("x1", "Q3.xlsx", XLSX, rev))
        })
    });
    let count = downloads.clone();
    google.on(move |r| {
        r.url.contains("/files/x1?alt=media").then(|| {
            let n = count.fetch_add(1, Ordering::SeqCst);
            (200, if n == 0 { b"PKold".to_vec() } else { b"PKnew".to_vec() })
        })
    });
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::browse(&mut app, "/Reports");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    let entry = app.cloud().mine.entries[0].clone();
    cloud::submit(&app, Job::Read { entry, into: None });
    settle(&mut app, &ctx, |a| !a.opened.lock().unwrap().is_empty());
    assert_eq!(app.opened.lock().unwrap()[0].1, b"PKnew".to_vec(), "the bytes of the revision it is said to be");
    assert_eq!(app.cloud().origins.get(&1).and_then(|o| o.entry.etag.clone()).as_deref(), Some("rev-2"));
    assert_eq!(downloads.load(Ordering::SeqCst), 2);
}

#[test]
fn a_folder_past_the_listed_part_isnt_called_missing() {
    let google = Google::new();
    google.on(|r| {
        decode(&r.url)
            .contains("'root' in parents")
            .then(|| json_reply(200, json!({"files": [{"id": "fold1", "name": "A", "mimeType": FOLDER}], "nextPageToken": "more"})))
    });
    let (mut app, ctx) = wired(Ok(client()), &google, signed_in_store());
    cloud::browse(&mut app, "/Zed");
    settle(&mut app, &ctx, |a| a.cloud().mine.loaded);
    assert!(matches!(&app.cloud().mine.error, Some(Fault::Refused(m)) if m.contains("first entries")), "{:?}", app.cloud().mine.error);
}
