use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_suite_account::{Memory, Secret, TokenStore};
use proptest::prelude::*;
use serde_json::json;

use crate::Error;
use crate::drive::{self, Drive, DriveFile, SaveTarget};
use crate::encode;
use crate::http::{Method, Request, Response, Transport};
use crate::oauth::{self, Auth, ClientConfig};
use crate::pkce::{self, Pkce};

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const ODT: &str = "application/vnd.oasis.opendocument.text";
const GOOGLE_DOC: &str = "application/vnd.google-apps.document";
const GOOGLE_SHEET: &str = "application/vnd.google-apps.spreadsheet";
/// An environment prefix nobody has set: fresh for each call, so a variable on the machine running
/// the tests can't stand in for the file under test.
fn unset_env() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("CRAFT_GDRIVE_TEST_{}_{nanos}_{n}", std::process::id())
}

/// The client file in `dir`, parsed as it is (no environment involved).
fn read_back(dir: &std::path::Path) -> ClientConfig {
    ClientConfig::from_json(&std::fs::read(dir.join(oauth::CONFIG_FILE)).unwrap()).unwrap()
}

/// Answers scripted responses in order and records every request.
#[derive(Default)]
struct Fake {
    replies: Mutex<VecDeque<Response>>,
    seen: Mutex<Vec<Request>>,
}

impl Fake {
    fn with(replies: Vec<(u16, serde_json::Value)>) -> Arc<Self> {
        let replies = replies.into_iter().map(|(status, v)| Response { status, body: v.to_string().into_bytes() }).collect();
        Arc::new(Self { replies: Mutex::new(replies), seen: Mutex::default() })
    }
    fn push_raw(&self, status: u16, body: &[u8]) {
        self.replies.lock().unwrap().push_back(Response { status, body: body.to_vec() });
    }
    fn seen(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

impl Transport for Fake {
    fn send(&self, req: Request) -> crate::Result<Response> {
        self.seen.lock().unwrap().push(req);
        self.replies.lock().unwrap().pop_front().ok_or_else(|| Error::Network("no scripted reply".into()))
    }
}

fn cfg() -> ClientConfig {
    ClientConfig::new("123-abc.apps.googleusercontent.com", Some("not-really-secret"), None).unwrap()
}

fn handle() -> String {
    oauth::token_handle(&cfg())
}

fn signed_in(fake: &Arc<Fake>) -> Auth {
    let store = Memory::new();
    store.put(&handle(), &Secret::new("refresh-1")).unwrap();
    let mut auth = Auth::new(cfg(), fake.clone(), Arc::new(store));
    assert!(auth.restore().unwrap());
    auth
}

fn token_reply() -> (u16, serde_json::Value) {
    (200, json!({"access_token": "access-1", "expires_in": 3599, "token_type": "Bearer"}))
}

fn file_json(id: &str, name: &str, mime: &str, version: &str) -> serde_json::Value {
    json!({"id": id, "name": name, "mimeType": mime, "modifiedTime": "2026-11-09T10:00:00.000Z", "version": version, "size": "1234", "capabilities": {"canEdit": true}})
}

fn header<'a>(r: &'a Request, k: &str) -> Option<&'a str> {
    r.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
}

// ---- PKCE and encoding

#[test]
fn pkce_matches_rfc7636_appendix_b() {
    let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
    assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
}

#[test]
fn generated_verifiers_are_43_unreserved_chars_and_differ() {
    let a = Pkce::generate().unwrap();
    let b = Pkce::generate().unwrap();
    assert_eq!(a.verifier.len(), 43);
    assert!(a.verifier.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    assert_ne!(a.verifier, b.verifier);
    assert!(!format!("{a:?}").contains(&a.verifier), "the verifier must not appear in Debug output");
}

#[test]
fn percent_encoding_round_trips() {
    assert_eq!(encode::pct("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    assert_eq!(encode::unpct("a%20b+c%2"), "a b c%2");
    assert_eq!(encode::drive_literal(r"it's a\b"), r"'it\'s a\\b'");
}

proptest! {
    #[test]
    fn pct_then_unpct_is_identity(s in ".*") {
        prop_assert_eq!(encode::unpct(&encode::pct(&s)), s);
    }

    #[test]
    fn hostile_callbacks_never_panic(target in ".{0,300}", state in "[a-z]{0,10}") {
        let _ = oauth::parse_callback(&target, &state);
        let _ = crate::loopback::request_target(target.as_bytes());
        let _ = encode::parse_query(&target, 32);
    }

    #[test]
    fn hostile_ids_never_reach_a_url(id in ".{0,300}") {
        if let Ok(ok) = drive::check_id(&id) {
            prop_assert!(ok.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        }
    }
}

// ---- configuration

#[test]
fn config_accepts_our_file_and_googles_download() {
    let ours = ClientConfig::from_json(br#"{"client_id":"x.apps.googleusercontent.com","scope":"file"}"#).unwrap();
    assert_eq!(ours.scope, oauth::SCOPE_DRIVE_FILE);
    assert_eq!(ours.client_secret, None);
    let google = ClientConfig::from_json(br#"{"installed":{"client_id":"y.apps.googleusercontent.com","client_secret":"s"}}"#).unwrap();
    assert_eq!(google.client_id, "y.apps.googleusercontent.com");
    assert_eq!(google.scope, oauth::SCOPE_DRIVE);
    assert!(!format!("{google:?}").contains("\"s\""), "the secret must not appear in Debug output");
}

#[test]
fn config_rejects_junk() {
    for bad in [&b"not json"[..], br#"{}"#, br#"{"client_id":""}"#, br#"{"client_id":"a b"}"#, br#"{"client_id":"a","scope":"all"}"#] {
        assert!(matches!(ClientConfig::from_json(bad), Err(Error::NotConfigured(_))), "{}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn missing_config_file_is_not_configured() {
    let dir = tempfile::tempdir().unwrap();
    let env = unset_env();
    assert!(std::env::var(oauth::env_names(&env).0).is_err());
    assert!(matches!(ClientConfig::load(Some(dir.path()), &env), Err(Error::NotConfigured(_))));
}

// ---- consent URL and redirect

#[test]
fn auth_url_carries_pkce_state_and_offline_access() {
    let url = oauth::auth_url(&cfg(), "http://127.0.0.1:5555", "CHAL", "STATE");
    assert!(url.starts_with(oauth::AUTH_ENDPOINT));
    let q = encode::parse_query(url.split_once('?').unwrap().1, 32);
    let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    assert_eq!(get("redirect_uri"), Some("http://127.0.0.1:5555"));
    assert_eq!(get("code_challenge"), Some("CHAL"));
    assert_eq!(get("code_challenge_method"), Some("S256"));
    assert_eq!(get("state"), Some("STATE"));
    assert_eq!(get("access_type"), Some("offline"));
    assert_eq!(get("scope"), Some(oauth::SCOPE_DRIVE));
    assert!(!url.contains("not-really-secret"), "the client secret never goes to the browser");
}

#[test]
fn callback_checks_state_and_reports_denial() {
    assert_eq!(oauth::parse_callback("/?state=s1&code=4%2Fabc", "s1").unwrap(), Some("4/abc".into()));
    assert_eq!(oauth::parse_callback("/favicon.ico", "s1").unwrap(), None);
    // Without this sign-in's state, nothing ends it: not a code, not an error.
    assert_eq!(oauth::parse_callback("/?state=other&code=x", "s1").unwrap(), None);
    assert_eq!(oauth::parse_callback("/?code=x", "s1").unwrap(), None);
    assert_eq!(oauth::parse_callback("/?error=Visit%20example.com", "s1").unwrap(), None);
    assert!(matches!(oauth::parse_callback("/?error=access_denied&state=s1", "s1"), Err(Error::SignIn(m)) if m.contains("denied")));
    // Google's error text isn't echoed.
    assert!(matches!(oauth::parse_callback("/?error=Visit%20example.com&state=s1", "s1"), Err(Error::SignIn(m)) if !m.contains("example")));
    assert!(matches!(oauth::parse_callback("/?state=s1", "s1"), Err(Error::SignIn(_))));
}

#[test]
fn request_line_parsing() {
    assert_eq!(crate::loopback::request_target(b"GET /?code=1 HTTP/1.1\r\nHost: x\r\n\r\n").as_deref(), Some("/?code=1"));
    assert_eq!(crate::loopback::request_target(b"POST / HTTP/1.1\r\n"), None);
    assert_eq!(crate::loopback::request_target(b"\xff\xfe"), None);
}

#[test]
fn loopback_receives_the_redirect() {
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicBool;
    let listener = crate::loopback::Listener::bind().unwrap();
    let uri = listener.redirect_uri();
    let addr = uri.trim_start_matches("http://").to_owned();
    let browser = std::thread::spawn(move || {
        // A favicon request first, as browsers do, then the redirect.
        for target in ["/favicon.ico", "/?state=st&code=the-code"] {
            let mut s = std::net::TcpStream::connect(&addr).unwrap();
            write!(s, "GET {target} HTTP/1.1\r\nHost: {addr}\r\n\r\n").unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
        }
    });
    let cancel = AtomicBool::new(false);
    assert_eq!(listener.wait("st", "Craft", &cancel, Duration::from_secs(10)).unwrap(), "the-code");
    browser.join().unwrap();
}

#[test]
fn loopback_honours_cancel() {
    use std::sync::atomic::AtomicBool;
    let listener = crate::loopback::Listener::bind().unwrap();
    let cancel = AtomicBool::new(true);
    assert!(matches!(listener.wait("st", "Craft", &cancel, Duration::from_secs(10)), Err(Error::SignIn(m)) if m == "cancelled"));
}

// ---- tokens

#[test]
fn access_token_refreshes_only_when_expiring() {
    let fake = Fake::with(vec![token_reply()]);
    let mut auth = signed_in(&fake);
    let now = Instant::now();
    assert_eq!(auth.access_token(now).unwrap(), "access-1");
    assert_eq!(auth.access_token(now + Duration::from_secs(60)).unwrap(), "access-1", "no second refresh");
    assert_eq!(fake.seen().len(), 1);
    let body = String::from_utf8(fake.seen()[0].body.to_vec()).unwrap();
    assert!(body.contains("grant_type=refresh_token") && body.contains("refresh_token=refresh-1"));
}

#[test]
fn revoked_refresh_token_signs_out() {
    let fake = Fake::with(vec![(400, json!({"error": "invalid_grant", "error_description": "Token has been expired or revoked."}))]);
    let mut auth = signed_in(&fake);
    assert_eq!(auth.access_token(Instant::now()), Err(Error::NotSignedIn));
    assert!(!auth.signed_in());
    assert!(!auth.restore().unwrap(), "the stored token was removed");
}

#[test]
fn network_failure_during_refresh_keeps_the_account() {
    let fake = Fake::with(vec![]);
    let mut auth = signed_in(&fake);
    assert!(matches!(auth.access_token(Instant::now()), Err(Error::Network(_))));
    assert!(auth.signed_in(), "a flaky network must not sign the person out");
}

#[test]
fn sign_out_revokes_and_forgets() {
    let fake = Fake::with(vec![(200, json!({}))]);
    let mut auth = signed_in(&fake);
    auth.sign_out().unwrap();
    assert!(!auth.signed_in());
    assert_eq!(fake.seen()[0].url, oauth::REVOKE_ENDPOINT);
    assert!(!auth.restore().unwrap());
}

#[test]
fn malformed_token_responses_are_errors() {
    let now = Instant::now();
    assert!(oauth::parse_tokens(&json!({}), now).is_err());
    assert!(oauth::parse_tokens(&json!({"access_token": ""}), now).is_err());
    let t = oauth::parse_tokens(&json!({"access_token": "a", "expires_in": u64::MAX}), now).unwrap();
    assert!(t.expires_at <= now + Duration::from_secs(86_400));
}

// ---- Drive calls

#[test]
fn save_refuses_when_drive_changed_since_open() {
    let fake = Fake::with(vec![token_reply(), (200, file_json("f1", "Essay.docx", DOCX, "9"))]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "f1".into(), name: "Essay.docx".into(), version: "7".into() };
    let err = d.save(&target, b"PK").unwrap_err();
    assert_eq!(err, Error::Conflict { name: "Essay.docx".into(), opened: "7".into(), remote: "9".into() });
    assert!(fake.seen().iter().all(|r| r.method != Method::Patch), "nothing was uploaded");
}

/// A rename or a new share bumps Drive's `version` but not the content revision: the save goes
/// ahead. A new content revision is a conflict.
#[test]
fn only_a_content_change_is_a_conflict() {
    let with_rev = |version: &str, rev: &str| {
        let mut v = file_json("f1", "Essay.docx", DOCX, version);
        v["headRevisionId"] = rev.into();
        (200, v)
    };
    let target = SaveTarget::Existing { id: "f1".into(), name: "Essay.docx".into(), version: "r1".into() };
    let fake = Fake::with(vec![token_reply(), with_rev("9", "r1"), with_rev("10", "r2")]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.save(&target, b"PK").unwrap().content_version(), "r2", "renamed or shared only: saved");
    let fake = Fake::with(vec![token_reply(), with_rev("9", "r5")]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.save(&target, b"PK").unwrap_err(), Error::Conflict { name: "Essay.docx".into(), opened: "r1".into(), remote: "r5".into() });
}

#[test]
fn save_updates_when_unchanged() {
    let fake = Fake::with(vec![token_reply(), (200, file_json("f1", "Essay.docx", DOCX, "7")), (200, file_json("f1", "Essay.docx", DOCX, "8"))]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "f1".into(), name: "Essay.docx".into(), version: "7".into() };
    assert_eq!(d.save(&target, b"PKdata").unwrap().version, "8");
    let up = &fake.seen()[2];
    assert_eq!(up.method, Method::Patch);
    assert!(up.url.starts_with(&format!("{}/files/f1?uploadType=media", drive::UPLOAD_API)));
    assert_eq!(header(up, "Content-Type"), Some(DOCX));
    assert_eq!(&*up.body, &b"PKdata"[..]);
}

#[test]
fn save_new_sends_multipart_with_parent() {
    let fake = Fake::with(vec![token_reply(), (200, file_json("n1", "New.odt", ODT, "1"))]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::New { name: "New.odt".into(), parent: Some("folder_1".into()) };
    assert_eq!(d.save(&target, b"ODTBYTES").unwrap().id, "n1");
    let up = &fake.seen()[1];
    assert_eq!(up.method, Method::Post);
    assert!(up.url.contains("uploadType=multipart"));
    let ct = header(up, "Content-Type").unwrap();
    let boundary = ct.strip_prefix("multipart/related; boundary=").unwrap();
    let body = String::from_utf8_lossy(&up.body);
    assert!(body.starts_with(&format!("--{boundary}\r\n")));
    assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    assert!(body.contains(r#""parents":["folder_1"]"#) && body.contains(ODT) && body.contains("ODTBYTES"));
}

#[test]
fn save_rejects_bad_names_ids_and_huge_documents() {
    let fake = Fake::with(vec![]);
    let mut d = Drive::new(signed_in(&fake));
    for t in [
        SaveTarget::New { name: "  ".into(), parent: None },
        SaveTarget::New { name: "a\nb.docx".into(), parent: None },
        SaveTarget::New { name: "ok.docx".into(), parent: Some("x/y".into()) },
        SaveTarget::Existing { id: "a?b".into(), name: "x".into(), version: "1".into() },
    ] {
        assert!(matches!(d.save(&t, b"PK"), Err(Error::Invalid(_))), "{t:?}");
    }
    assert!(fake.seen().is_empty(), "nothing was sent for invalid input");
}

#[test]
fn api_errors_carry_googles_message() {
    let fake = Fake::with(vec![token_reply(), (404, json!({"error": {"code": 404, "message": "File not found: f1."}}))]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.metadata("f1"), Err(Error::Api { status: 404, message: "File not found: f1.".into() }));
}

#[test]
fn client_files_are_read_only_when_small_regular_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("folder")).unwrap();
    assert!(matches!(oauth::import_client(dir.path(), &dir.path().join("folder")), Err(Error::NotConfigured(_))), "a folder is refused");
    let big = dir.path().join("client_secret_big.json");
    std::fs::write(&big, vec![b' '; 65 * 1024]).unwrap();
    assert!(matches!(oauth::import_client(dir.path(), &big), Err(Error::NotConfigured(m)) if m.contains("too large")));
    #[cfg(unix)]
    {
        let fifo = dir.path().join("client_secret_fifo.json");
        if std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
            assert!(matches!(oauth::import_client(dir.path(), &fifo), Err(Error::NotConfigured(_))), "a pipe is refused without blocking");
        }
    }
    assert!(!dir.path().join(oauth::CONFIG_FILE).exists(), "nothing written");
}

#[test]
fn re_entering_only_the_client_id_keeps_its_secret() {
    let dir = tempfile::tempdir().unwrap();
    oauth::write_client(dir.path(), "123-abc.apps.googleusercontent.com", Some("s3cret")).unwrap();
    oauth::write_client(dir.path(), "123-abc.apps.googleusercontent.com", None).unwrap();
    assert_eq!(read_back(dir.path()).client_secret.as_deref(), Some("s3cret"), "same client: secret kept");
    oauth::write_client(dir.path(), "456-def.apps.googleusercontent.com", None).unwrap();
    assert_eq!(read_back(dir.path()).client_secret, None, "another client: not kept");
}

#[test]
fn write_client_checks_the_id_keeps_the_scope_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(oauth::write_client(dir.path(), "  ", None), Err(Error::NotConfigured(_))), "an empty ID is refused");
    assert!(!dir.path().join(oauth::CONFIG_FILE).exists(), "nothing written for a bad ID");
    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(dir.path().join(oauth::CONFIG_FILE), br#"{"client_id": "old", "scope": "file"}"#).unwrap();
    oauth::write_client(dir.path(), " 123-abc.apps.googleusercontent.com ", Some("s3cret")).unwrap();
    let cfg = read_back(dir.path());
    assert_eq!(cfg.client_id, "123-abc.apps.googleusercontent.com");
    assert_eq!(cfg.client_secret.as_deref(), Some("s3cret"));
    assert_eq!(cfg.scope, oauth::SCOPE_DRIVE_FILE, "the file's scope is kept");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(oauth::CONFIG_FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temporary file left behind");
}

#[test]
fn a_downloaded_client_file_is_found_and_imported() {
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    let profile = root.path().join("profile");
    std::fs::create_dir_all(&downloads).unwrap();
    assert_eq!(oauth::newest_client_file(&downloads), None, "nothing to import");
    std::fs::write(downloads.join("notes.json"), b"{}").unwrap();
    let older = downloads.join("client_secret_1-old.apps.googleusercontent.com.json");
    std::fs::write(&older, br#"{"installed": {"client_id": "1-old.apps.googleusercontent.com", "client_secret": "a"}}"#).unwrap();
    let newer = downloads.join("client_secret_2-new.apps.googleusercontent.com.json");
    std::fs::write(&newer, br#"{"installed": {"client_id": "2-new.apps.googleusercontent.com", "client_secret": "b"}}"#).unwrap();
    // Times set apart explicitly: some file systems keep them to two seconds.
    let base = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options().write(true).open(&older).unwrap().set_modified(base).unwrap();
    std::fs::File::options().write(true).open(&newer).unwrap().set_modified(base + Duration::from_secs(60)).unwrap();
    assert_eq!(oauth::newest_client_file(&downloads).as_deref(), Some(newer.as_path()), "the newest client file, never other JSON");

    let cfg = oauth::import_client(&profile, &newer).unwrap();
    assert_eq!(cfg.client_id, "2-new.apps.googleusercontent.com");
    assert_eq!(read_back(&profile).client_secret.as_deref(), Some("b"));

    // A file that isn't a client changes nothing.
    std::fs::write(downloads.join("client_secret_bad.json"), b"{\"web\": 1}").unwrap();
    assert!(oauth::import_client(&profile, &downloads.join("client_secret_bad.json")).is_err());
    assert_eq!(read_back(&profile).client_id, "2-new.apps.googleusercontent.com");
}

// ---- the parts the shared pages need

#[test]
fn not_signed_in_sends_nothing() {
    let fake = Fake::with(vec![]);
    let mut d = Drive::new(Auth::new(cfg(), fake.clone(), Arc::new(Memory::new())));
    assert_eq!(d.search(drive::SHARED_QUERY), Err(Error::NotSignedIn));
    assert!(fake.seen().is_empty());
}

#[test]
fn exchange_sends_verifier_and_stores_refresh_token() {
    let fake = Fake::with(vec![(200, json!({"access_token": "a", "refresh_token": "r", "expires_in": 3599}))]);
    let store = Memory::new();
    let mut auth = Auth::new(cfg(), fake.clone(), Arc::new(store.clone()));
    auth.exchange("CODE", "VERIFIER", "http://127.0.0.1:1", Instant::now()).unwrap();
    assert_eq!(store.get(&handle()).unwrap().map(|s| s.expose().to_owned()).as_deref(), Some("r"));
    let seen = fake.seen();
    let body = String::from_utf8(seen[0].body.to_vec()).unwrap();
    assert_eq!(seen[0].url, oauth::TOKEN_ENDPOINT);
    for part in ["grant_type=authorization_code", "code=CODE", "code_verifier=VERIFIER", "client_secret=not-really-secret"] {
        assert!(body.contains(part), "{part} missing from {body}");
    }
    assert!(!format!("{:?}", seen[0]).contains("VERIFIER"), "request Debug output leaks the body");
}

#[test]
fn each_client_has_its_own_keychain_entry() {
    let c = |id: &str, scope: Option<&str>| ClientConfig::new(id, None, scope).unwrap();
    assert_eq!(
        oauth::token_handle(&c(" 123-abc.apps.googleusercontent.com ", None)),
        "google-drive.refresh-token.123-abc.apps.googleusercontent.com"
    );
    assert_ne!(oauth::token_handle(&c("a", None)), oauth::token_handle(&c("b", None)));
    assert_ne!(
        oauth::token_handle(&c("a", Some("drive"))),
        oauth::token_handle(&c("a", Some("file"))),
        "a narrower scope never reuses the wider grant"
    );
}

#[test]
fn a_keychain_that_refuses_is_an_error_not_a_sign_in() {
    let auth_store = Memory::refusing("locked");
    let mut auth = Auth::new(cfg(), Fake::with(vec![]), Arc::new(auth_store));
    assert!(matches!(auth.restore(), Err(Error::Keychain(m)) if m == "locked"));
}

#[test]
fn a_folder_listing_reads_every_page_and_skips_bad_ids() {
    let fake = Fake::with(vec![
        token_reply(),
        (200, json!({"nextPageToken": "p2", "files": [file_json("abc_1-2", "Budget.xlsx", XLSX, "7"), file_json("../etc", "Evil.xlsx", XLSX, "1")]})),
        (200, json!({"files": [{"id": "f9", "name": "Archive", "mimeType": drive::MIME_FOLDER}]})),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    let files = d.search(&drive::folder_query("root").unwrap()).unwrap();
    assert_eq!(files.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["abc_1-2", "f9"]);
    assert!(files[1].is_folder());
    let seen = fake.seen();
    let q = encode::parse_query(seen[1].url.split_once('?').unwrap().1, 32);
    let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(get("q").as_deref(), Some("'root' in parents and trashed = false"));
    assert!(seen[2].url.contains("pageToken=p2"), "the second page was asked for");
    assert_eq!(header(&seen[1], "Authorization"), Some("Bearer access-1"));
}

#[test]
fn a_huge_folder_stops_after_the_last_page_it_reads() {
    let mut replies = vec![token_reply()];
    for n in 0..drive::MAX_PAGES + 2 {
        replies.push((200, json!({"nextPageToken": format!("p{n}"), "files": []})));
    }
    let fake = Fake::with(replies);
    let mut d = Drive::new(signed_in(&fake));
    d.search(drive::SHARED_QUERY).unwrap();
    assert_eq!(fake.seen().len(), 1 + drive::MAX_PAGES, "bounded");
}

#[test]
fn queries_quote_what_they_are_given() {
    assert!(drive::folder_query("x' or '1").is_err(), "a folder id is checked, never quoted into a query");
    let q = drive::child_query("root", "it's").unwrap();
    assert!(q.contains(r"name = 'it\'s'"), "{q}");
    assert!(drive::child_query("../x", "a").is_err());
}

#[test]
fn native_google_files_open_as_office_files_and_stay_read_only() {
    let sheet: DriveFile =
        serde_json::from_value(json!({"id": "s1", "name": "Budget", "mimeType": GOOGLE_SHEET, "version": "3", "capabilities": {"canEdit": true}}))
            .unwrap();
    assert_eq!(sheet.open_name(), "Budget.xlsx");
    assert_eq!(sheet.open_mime(), XLSX);
    assert!(!sheet.writable(), "a Google Sheet is never overwritten by a file");
    let doc: DriveFile = serde_json::from_value(json!({"id": "d1", "name": "Notes.docx", "mimeType": GOOGLE_DOC})).unwrap();
    assert_eq!(doc.open_name(), "Notes.docx", "no second extension");
    let form: DriveFile = serde_json::from_value(json!({"id": "q1", "name": "Survey", "mimeType": "application/vnd.google-apps.form"})).unwrap();
    assert_eq!(form.export(), None);
    let read_only: DriveFile =
        serde_json::from_value(json!({"id": "r1", "name": "a.xlsx", "mimeType": XLSX, "capabilities": {"canEdit": false}})).unwrap();
    assert!(!read_only.writable());
}

#[test]
fn native_files_are_exported_and_others_refused() {
    let fake = Fake::with(vec![token_reply()]);
    fake.push_raw(200, b"PK\x03\x04xlsx");
    let mut d = Drive::new(signed_in(&fake));
    let sheet = DriveFile { id: "s1".into(), name: "Budget".into(), mime_type: GOOGLE_SHEET.into(), ..DriveFile::default() };
    assert_eq!(d.download(&sheet).unwrap(), b"PK\x03\x04xlsx");
    let req = &fake.seen()[1];
    assert!(req.url.contains("/files/s1/export?mimeType="), "{}", req.url);
    assert_eq!(req.limit, drive::MAX_DOCUMENT);
    let form = DriveFile { id: "q1".into(), name: "Survey".into(), mime_type: "application/vnd.google-apps.form".into(), ..DriveFile::default() };
    assert!(matches!(d.download(&form), Err(Error::Invalid(_))));
    assert_eq!(fake.seen().len(), 2, "nothing sent for a form");
}

#[test]
fn save_never_overwrites_a_native_file() {
    let fake = Fake::with(vec![token_reply(), (200, json!({"id": "s1", "name": "Budget", "mimeType": GOOGLE_SHEET, "version": "3"}))]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "s1".into(), name: "Budget.xlsx".into(), version: "3".into() };
    assert!(matches!(d.save(&target, b"PK"), Err(Error::Invalid(m)) if m.contains("save a copy")));
    assert!(fake.seen().iter().all(|r| r.method != Method::Patch));
}

#[test]
fn who_is_the_name_or_the_address() {
    let fake = Fake::with(vec![token_reply(), (200, json!({"user": {"displayName": " ", "emailAddress": "someone@example.com"}}))]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.who().unwrap(), "someone@example.com");
}

#[test]
fn a_401_refreshes_the_access_token_and_retries_once() {
    let fake = Fake::with(vec![
        token_reply(),
        (401, json!({"error": {"code": 401, "message": "Invalid Credentials"}})),
        (200, json!({"access_token": "access-2", "expires_in": 3599, "token_type": "Bearer"})),
        (200, json!({"files": []})),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.search(drive::SHARED_QUERY), Ok(vec![]));
    let seen = fake.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(header(&seen[3], "Authorization"), Some("Bearer access-2"), "retried with the refreshed token");
}

#[test]
fn a_401_then_a_refused_refresh_signs_out() {
    let fake = Fake::with(vec![
        token_reply(),
        (401, json!({"error": {"code": 401, "message": "Invalid Credentials"}})),
        (400, json!({"error": "invalid_grant", "error_description": "Token has been expired or revoked."})),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.search(drive::SHARED_QUERY), Err(Error::NotSignedIn));
    assert_eq!(d.auth.access_token(Instant::now()), Err(Error::NotSignedIn), "signed out, nothing more sent");
    assert_eq!(fake.seen().len(), 3);
}

#[test]
fn a_401_with_a_fresh_token_is_drives_error_not_a_sign_out() {
    let denied = || (401, json!({"error": {"code": 401, "message": "Invalid Credentials"}}));
    let fake =
        Fake::with(vec![token_reply(), denied(), (200, json!({"access_token": "access-2", "expires_in": 3599, "token_type": "Bearer"})), denied()]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.search(drive::SHARED_QUERY), Err(Error::Api { status: 401, message: "Invalid Credentials".into() }));
    assert_eq!(fake.seen().len(), 4, "one retry only");
}

#[test]
fn mime_for_names() {
    for (name, mime) in
        [("a.DOCX", DOCX), ("a.xlsx", XLSX), ("a.odt", ODT), ("a.pdf", "application/pdf"), ("a.csv", "text/csv"), ("a", "application/octet-stream")]
    {
        assert_eq!(drive::mime_for(name), mime, "{name}");
    }
    let _ = pkce::random_token(0).unwrap();
}

#[test]
fn faults_say_what_the_pages_act_on() {
    use craft_ui::cloud::Fault;
    assert_eq!(crate::backend::fault(Error::NotSignedIn), Fault::NotLinked);
    assert_eq!(crate::backend::fault(Error::Conflict { name: "a".into(), opened: "1".into(), remote: "2".into() }), Fault::Conflict);
    assert!(matches!(crate::backend::fault(Error::Network("down".into())), Fault::Unreachable(_)));
    assert!(matches!(crate::backend::fault(Error::Api { status: 403, message: "no".into() }), Fault::Refused(_)));
}

#[test]
fn entries_carry_the_content_version_and_a_one_folder_path() {
    let mut file: DriveFile = serde_json::from_value(file_json("f1", "Q3/Q4.xlsx", XLSX, "9")).unwrap();
    file.head_revision_id = "rev-2".into();
    let e = crate::backend::entry(&file, format!("/Reports/{}", crate::backend::segment(&file.name)));
    assert_eq!(e.etag.as_deref(), Some("rev-2"));
    assert_eq!(e.room, "f1");
    assert_eq!(e.path, "/Reports/Q3\u{2215}Q4.xlsx", "a slash in a name isn't a folder boundary");
    assert!(e.writable);
}

#[test]
fn a_save_into_the_trash_is_refused() {
    let mut trashed = file_json("f1", "Q3.xlsx", XLSX, "7");
    trashed["trashed"] = json!(true);
    let fake = Fake::with(vec![token_reply(), (200, trashed)]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "f1".into(), name: "Q3.xlsx".into(), version: "7".into() };
    assert!(matches!(d.save(&target, b"PK"), Err(Error::Invalid(m)) if m.contains("trash")));
    assert!(fake.seen().iter().all(|r| r.method != Method::Patch), "nothing uploaded");
}

#[test]
fn no_keychain_means_nobody_is_signed_in_and_a_sign_in_lasts_the_session() {
    let fake = Fake::with(vec![(200, json!({"access_token": "a", "refresh_token": "r", "expires_in": 3599}))]);
    let mut auth = Auth::new(cfg(), fake, Arc::new(Memory::missing("no Secret Service")));
    assert_eq!(auth.restore(), Ok(false), "not an error that stops the page offering a sign-in");
    auth.exchange("CODE", "V", "http://127.0.0.1:1", Instant::now()).unwrap();
    assert_eq!(auth.restore(), Ok(true), "signed in for this session");
}

#[test]
fn busy_or_failing_google_is_unreachable_not_a_refusal() {
    use craft_ui::cloud::Fault;
    for status in [429, 500, 503] {
        assert!(matches!(crate::backend::fault(Error::Api { status, message: "later".into() }), Fault::Unreachable(_)), "{status}");
    }
}

#[test]
fn responses_never_print_their_body() {
    let r = Response { status: 200, body: br#"{"refresh_token":"secret-1"}"#.to_vec() };
    assert!(!format!("{r:?}").contains("secret-1"));
}

#[test]
fn a_timeout_too_long_to_add_waits_until_cancelled() {
    use std::sync::atomic::AtomicBool;
    let listener = crate::loopback::Listener::bind().unwrap();
    let cancel = AtomicBool::new(true);
    assert!(matches!(listener.wait("st", "Craft", &cancel, Duration::MAX), Err(Error::SignIn(m)) if m == "cancelled"));
}

#[test]
fn a_trickling_connection_doesnt_hold_up_the_redirect() {
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicBool;
    let listener = crate::loopback::Listener::bind().unwrap();
    let addr = listener.redirect_uri().trim_start_matches("http://").to_owned();
    let browser = std::thread::spawn(move || {
        // Connects and says nothing, then the real redirect.
        let _idle = std::net::TcpStream::connect(&addr).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let mut s = std::net::TcpStream::connect(&addr).unwrap();
        write!(s, "GET /?state=st&code=c2 HTTP/1.1\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
    });
    let started = Instant::now();
    assert_eq!(listener.wait("st", "Craft", &AtomicBool::new(false), Duration::from_secs(20)).unwrap(), "c2");
    assert!(started.elapsed() < Duration::from_secs(8), "one silent connection costs at most its own limit");
    browser.join().unwrap();
}

#[test]
fn same_named_folders_are_numbered_by_id_whatever_order_drive_answers_in() {
    let folder = |id: &str, name: &str| DriveFile { id: id.into(), name: name.into(), mime_type: drive::MIME_FOLDER.into(), ..DriveFile::default() };
    let a = vec![folder("b2", "Reports"), folder("a1", "Reports"), folder("c3", "Reports (2)")];
    let mut b = a.clone();
    b.reverse();
    let paths = |files: &[DriveFile]| -> Vec<(String, String)> {
        let mut v: Vec<_> = files.iter().map(|f| f.id.clone()).zip(crate::backend::child_paths("/Top", files)).collect();
        v.sort();
        v
    };
    assert_eq!(paths(&a), paths(&b), "stable across answer order");
    assert_eq!(
        paths(&a),
        [("a1".into(), "/Top/Reports".into()), ("b2".into(), "/Top/Reports (3)".into()), ("c3".into(), "/Top/Reports (2)".into())],
        "a real folder named Reports (2) keeps its name; the duplicate moves on"
    );
}

// ---- review round 2

#[test]
fn size_is_written_back_the_way_drive_sends_it() {
    for f in [file_json("f1", "a.docx", DOCX, "1"), json!({"id": "f2", "name": "Doc", "mimeType": GOOGLE_DOC})] {
        let read: DriveFile = serde_json::from_value(f).unwrap();
        let again: DriveFile = serde_json::from_value(serde_json::to_value(&read).unwrap()).unwrap();
        assert_eq!(read, again);
    }
    let sized: DriveFile = serde_json::from_value(file_json("f1", "a.docx", DOCX, "1")).unwrap();
    assert_eq!(serde_json::to_value(&sized).unwrap()["size"], json!("1234"));
}

#[test]
fn a_save_with_no_version_is_refused_before_anything_is_sent() {
    let fake = Fake::with(vec![]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "f1".into(), name: "Essay.docx".into(), version: " ".into() };
    assert!(matches!(d.save(&target, b"PK"), Err(Error::Invalid(_))));
    assert!(fake.seen().is_empty());
}

#[test]
fn a_retry_after_a_401_checks_the_revision_again() {
    let fake = Fake::with(vec![
        token_reply(),
        (200, file_json("f1", "Essay.docx", DOCX, "7")),
        (401, json!({"error": {"code": 401, "message": "Invalid Credentials"}})),
        (200, json!({"access_token": "access-2", "expires_in": 3599})),
        // Somebody saved while the token was refreshed.
        (200, file_json("f1", "Essay.docx", DOCX, "8")),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "f1".into(), name: "Essay.docx".into(), version: "7".into() };
    assert!(matches!(d.save(&target, b"PK"), Err(Error::Conflict { .. })));
    assert_eq!(fake.seen().iter().filter(|r| r.method == Method::Patch).count(), 1, "no second upload over the newer revision");
}

#[test]
fn a_name_lookup_follows_empty_pages_and_never_guesses_absence() {
    let fake = Fake::with(vec![token_reply(), (200, json!({"files": [], "nextPageToken": "t1"})), (200, json!({"files": [{"id": "x1"}]}))]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.child_named("root", "Budget.xlsx"), Ok(Some("x1".into())));
    assert!(decode_url(&fake.seen()[2].url).contains("pageToken=t1"));

    let mut endless = vec![token_reply()];
    endless.extend((0..drive::MAX_PAGES).map(|i| (200, json!({"files": [], "nextPageToken": format!("t{i}")}))));
    let fake = Fake::with(endless);
    let mut d = Drive::new(signed_in(&fake));
    assert!(d.child_named("root", "Budget.xlsx").is_err(), "an unfinished lookup isn't \"not there\"");
}

#[test]
fn a_listing_cut_off_at_the_page_limit_says_so() {
    let mut pages = vec![token_reply()];
    pages.extend(
        (0..drive::MAX_PAGES).map(|i| (200, json!({"files": [file_json(&format!("f{i}"), "a.docx", DOCX, "1")], "nextPageToken": format!("t{i}")}))),
    );
    let fake = Fake::with(pages);
    let mut d = Drive::new(signed_in(&fake));
    let listing = d.listing(&drive::folder_query("root").unwrap()).unwrap();
    assert_eq!(listing.files.len(), drive::MAX_PAGES);
    assert!(!listing.complete);
}

#[test]
fn a_wrong_client_secret_keeps_the_sign_in() {
    let fake = Fake::with(vec![(401, json!({"error": "invalid_client", "error_description": "Unauthorized"}))]);
    let mut auth = signed_in(&fake);
    assert!(matches!(auth.access_token(Instant::now()), Err(Error::Api { status: 401, .. })));
    assert!(auth.signed_in(), "fixing the client restores the session");
    let fake = Fake::with(vec![]);
    let mut again = Auth::new(
        cfg(),
        fake,
        Arc::new({
            let store = Memory::new();
            store.put(&handle(), &Secret::new("refresh-1")).unwrap();
            store
        }),
    );
    assert!(again.restore().unwrap());
}

#[test]
fn a_renewed_refresh_token_is_kept() {
    let fake = Fake::with(vec![(200, json!({"access_token": "a2", "refresh_token": "refresh-2", "expires_in": 3599}))]);
    let store = Memory::new();
    store.put(&handle(), &Secret::new("refresh-1")).unwrap();
    let mut auth = Auth::new(cfg(), fake, Arc::new(store.clone()));
    assert!(auth.restore().unwrap());
    assert_eq!(auth.access_token(Instant::now()).unwrap(), "a2");
    assert_eq!(store.get(&handle()).unwrap().map(|s| s.expose().to_owned()).as_deref(), Some("refresh-2"));
}

#[test]
fn a_renewed_refresh_token_arriving_after_sign_out_started_isnt_kept() {
    let fake = Fake::with(vec![(200, json!({"access_token": "a2", "refresh_token": "refresh-2", "expires_in": 3599}))]);
    let store = Memory::new();
    store.put(&handle(), &Secret::new("refresh-1")).unwrap();
    let mut auth = Auth::new(cfg(), fake, Arc::new(store.clone()));
    assert!(auth.restore().unwrap());
    // What the backend's sign-out does before it waits for the transfer.
    *auth.keeping().lock().unwrap() = false;
    store.remove(&handle()).unwrap();
    assert_eq!(auth.access_token(Instant::now()).unwrap(), "a2", "the transfer carries on");
    assert_eq!(store.get(&handle()).unwrap(), None, "nothing to restore on the next launch");
}

/// Answers the token request only after a sign-out has run: the session's keep switch closed
/// and the stored token removed, as the backend's sign-out does while Google is answering.
struct SignOutDuring {
    store: Memory,
    keep: std::sync::OnceLock<Arc<Mutex<bool>>>,
}

impl Transport for SignOutDuring {
    fn send(&self, _req: Request) -> crate::Result<Response> {
        if let Some(keep) = self.keep.get() {
            *keep.lock().unwrap() = false;
            self.store.remove(&handle()).unwrap();
        }
        let v = json!({"access_token": "a1", "refresh_token": "refresh-new", "expires_in": 3599});
        Ok(Response { status: 200, body: v.to_string().into_bytes() })
    }
}

#[test]
fn a_sign_in_finishing_after_sign_out_started_isnt_kept() {
    let store = Memory::new();
    let http = Arc::new(SignOutDuring { store: store.clone(), keep: std::sync::OnceLock::new() });
    let mut auth = Auth::new(cfg(), http.clone(), Arc::new(store.clone()));
    http.keep.set(auth.keeping()).unwrap();
    auth.exchange("code", "verifier", "http://127.0.0.1:1/", Instant::now()).unwrap();
    assert_eq!(store.get(&handle()).unwrap(), None, "nothing to restore on the next launch");
    assert!(auth.signed_in(), "held in memory, for the waiting sign-out to revoke");
}

#[test]
fn a_save_refused_after_a_refresh_is_googles_error_not_a_sign_out() {
    let denied = || (401, json!({"error": {"code": 401, "message": "Invalid Credentials"}}));
    let fake = Fake::with(vec![
        token_reply(),
        (200, file_json("f1", "Essay.docx", DOCX, "7")),
        denied(),
        (200, json!({"access_token": "access-2", "expires_in": 3599})),
        (200, file_json("f1", "Essay.docx", DOCX, "7")),
        denied(),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "f1".into(), name: "Essay.docx".into(), version: "7".into() };
    assert_eq!(d.save(&target, b"PK"), Err(Error::Api { status: 401, message: "Invalid Credentials".into() }));
    assert!(d.auth.signed_in(), "still signed in");
}

#[test]
fn a_session_only_sign_in_removes_the_earlier_one() {
    // Another account's sign-in is stored; this one comes back with no refresh token.
    let fake = Fake::with(vec![(200, json!({"access_token": "a", "expires_in": 3599}))]);
    let store = Memory::new();
    store.put(&handle(), &Secret::new("refresh-other-account")).unwrap();
    let mut auth = Auth::new(cfg(), fake, Arc::new(store.clone()));
    auth.exchange("CODE", "V", "http://127.0.0.1:1", Instant::now()).unwrap();
    assert!(auth.signed_in());
    assert_eq!(store.get(&handle()).unwrap(), None, "the next launch doesn't sign the other account back in");
}

#[test]
fn an_imported_client_keeps_the_scope_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("client.json");
    std::fs::write(&src, br#"{"client_id": "1-a.apps.googleusercontent.com", "client_secret": "s", "scope": "file"}"#).unwrap();
    let profile = dir.path().join("profile");
    let cfg = oauth::import_client(&profile, &src).unwrap();
    assert_eq!(cfg.scope, oauth::SCOPE_DRIVE_FILE);
    assert_eq!(read_back(&profile), cfg, "what a later load reads is what was returned");
}

#[test]
fn a_body_exactly_at_the_limit_is_accepted() {
    assert_eq!(crate::http::bounded(vec![7; 10], 10).map(|b| b.len()), Ok(10));
    assert!(matches!(crate::http::bounded(vec![7; 11], 10), Err(Error::Protocol(_))));
}

#[test]
fn a_refused_revoke_still_signs_out_here() {
    let fake = Fake::with(vec![(400, json!({"error": "invalid_token"}))]);
    let store = Memory::new();
    store.put(&handle(), &Secret::new("refresh-1")).unwrap();
    let mut auth = Auth::new(cfg(), fake, Arc::new(store.clone()));
    assert!(auth.restore().unwrap());
    assert_eq!(auth.sign_out(), Ok(()));
    assert_eq!(store.get(&handle()).unwrap(), None);
}

#[test]
fn numbering_doesnt_depend_on_answer_order_with_a_real_numbered_name() {
    let folder = |id: &str, name: &str| DriveFile { id: id.into(), name: name.into(), mime_type: drive::MIME_FOLDER.into(), ..DriveFile::default() };
    let sorted = |files: &[DriveFile]| -> Vec<(String, String)> {
        let mut v: Vec<_> = files.iter().map(|f| f.id.clone()).zip(crate::backend::child_paths("", files)).collect();
        v.sort();
        v
    };
    let a = [folder("a", "Reports"), folder("b", "Reports"), folder("c", "Reports"), folder("d", "Reports (2)")];
    let b = [a[0].clone(), a[2].clone(), a[1].clone(), a[3].clone()];
    let c = [a[3].clone(), a[2].clone(), a[1].clone(), a[0].clone()];
    assert_eq!(sorted(&a), sorted(&b));
    assert_eq!(sorted(&a), sorted(&c));
    assert_eq!(
        sorted(&a),
        [
            ("a".into(), "/Reports".into()),
            ("b".into(), "/Reports (3)".into()),
            ("c".into(), "/Reports (4)".into()),
            ("d".into(), "/Reports (2)".into())
        ]
    );
}

fn decode_url(url: &str) -> String {
    encode::parse_query(url.split_once('?').map_or("", |(_, q)| q), 64).into_iter().map(|(k, v)| format!("{k}={v}&")).collect()
}
