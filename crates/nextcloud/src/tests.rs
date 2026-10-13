//! Tests. Every one runs against a canned transport, so none of them touch a network.

use std::cell::RefCell;

use crate::http::{Http, Method, Request, Response};
use crate::{Account, Contacts, Files, NewShare, Permissions, ShareKind, Shares};

/// A transport that answers from a script and records what it was asked.
struct Fake {
    replies: RefCell<Vec<Response>>,
    seen: RefCell<Vec<Request>>,
}

impl Fake {
    fn new(replies: Vec<Response>) -> Fake {
        Fake { replies: RefCell::new(replies), seen: RefCell::new(Vec::new()) }
    }

    fn one(status: u16, body: &str) -> Fake {
        Fake::new(vec![Response::new(status, body.as_bytes().to_vec())])
    }

    fn last(&self) -> Request {
        self.seen.borrow().last().cloned().unwrap_or_else(|| Request::new(Method::Get, ""))
    }
}

impl Http for Fake {
    type Error = String;

    fn send(&self, req: Request) -> Result<Response, String> {
        self.seen.borrow_mut().push(req);
        let mut replies = self.replies.borrow_mut();
        if replies.is_empty() {
            return Err("no scripted reply left".to_owned());
        }
        Ok(replies.remove(0))
    }
}

fn acct() -> Account {
    Account::new("https://cloud.example.net/", "alice", "app-pw")
}

// ---- encoding ---------------------------------------------------------------

#[test]
fn base64_matches_known_vectors() {
    // RFC 4648 section 10.
    assert_eq!(crate::base64(b""), "");
    assert_eq!(crate::base64(b"f"), "Zg==");
    assert_eq!(crate::base64(b"fo"), "Zm8=");
    assert_eq!(crate::base64(b"foo"), "Zm9v");
    assert_eq!(crate::base64(b"foob"), "Zm9vYg==");
    assert_eq!(crate::base64(b"fooba"), "Zm9vYmE=");
    assert_eq!(crate::base64(b"foobar"), "Zm9vYmFy");
}

#[test]
fn authorization_is_basic_over_user_and_password() {
    // "alice:app-pw" in base64.
    assert_eq!(acct().authorization(), format!("Basic {}", crate::base64(b"alice:app-pw")));
    assert!(acct().authorization().starts_with("Basic "));
}

#[test]
fn account_debug_hides_the_password() {
    let shown = format!("{:?}", acct());
    assert!(shown.contains("alice"), "{shown}");
    assert!(shown.contains("<redacted>"), "{shown}");
    assert!(!shown.contains("app-pw"), "the password leaked into Debug: {shown}");
}

#[test]
fn base_loses_trailing_slashes() {
    assert_eq!(Account::new("https://x.test///", "u", "p").base(), "https://x.test");
}

#[test]
fn segments_encode_everything_outside_the_unreserved_set() {
    assert_eq!(crate::encode_segment("plain-name_1.txt~"), "plain-name_1.txt~");
    assert_eq!(crate::encode_segment("a b"), "a%20b");
    assert_eq!(crate::encode_segment("a/b"), "a%2Fb");
    assert_eq!(crate::encode_segment("ä"), "%C3%A4");
    assert_eq!(crate::encode_segment("100%"), "100%25");
    assert_eq!(crate::encode_segment("q?x=1&y=2"), "q%3Fx%3D1%26y%3D2");
}

#[test]
fn paths_keep_separators_and_collapse_empties() {
    assert_eq!(crate::encode_path("/a/b").expect("ok"), "/a/b");
    assert_eq!(crate::encode_path("a/b").expect("ok"), "/a/b");
    assert_eq!(crate::encode_path("//a///b//").expect("ok"), "/a/b");
    assert_eq!(crate::encode_path("").expect("ok"), "");
    assert_eq!(crate::encode_path("/My Docs/a b.docx").expect("ok"), "/My%20Docs/a%20b.docx");
}

#[test]
fn paths_reject_traversal() {
    for bad in ["/a/../b", "..", "/./a", "a/.."] {
        let err = crate::encode_path(bad).expect_err("traversal must be refused");
        assert!(matches!(err, crate::Error::Invalid { what: "path", .. }), "{bad}: {err}");
    }
}

#[test]
fn decode_is_lenient_about_bad_escapes() {
    assert_eq!(crate::dav::decode("a%20b"), "a b");
    assert_eq!(crate::dav::decode("%C3%A4"), "ä");
    // A bare or truncated escape survives as written.
    assert_eq!(crate::dav::decode("100%"), "100%");
    assert_eq!(crate::dav::decode("a%zzb"), "a%zzb");
    assert_eq!(crate::dav::decode("a%4"), "a%4");
}

#[test]
fn encode_then_decode_round_trips() {
    for name in ["plain.txt", "a b", "ä ö ü", "100% done.docx", "a&b;c", "q?x=1"] {
        assert_eq!(crate::dav::decode(&crate::encode_segment(name)), name, "{name}");
    }
}

// ---- WebDAV -----------------------------------------------------------------

const LISTING: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
  <d:response>
    <d:href>/remote.php/dav/files/alice/Documents/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/></d:resourcetype>
      <d:getetag>"abc"</d:getetag>
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/alice/Documents/Report%20Q3.docx</d:href>
    <d:propstat><d:prop>
      <d:resourcetype/>
      <d:getcontentlength>20480</d:getcontentlength>
      <d:getcontenttype>application/vnd.openxmlformats-officedocument.wordprocessingml.document</d:getcontenttype>
      <d:getlastmodified>Sat, 11 Oct 2026 19:02:11 GMT</d:getlastmodified>
      <d:getetag>"deadbeef"</d:getetag>
      <oc:fileid>12345</oc:fileid>
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/alice/Documents/Archive/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/></d:resourcetype>
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
</d:multistatus>"#;

#[test]
fn list_parses_entries_and_drops_the_collection_itself() {
    let a = acct();
    let http = Fake::one(207, LISTING);
    let entries = Files::new(&a).list(&http, "/Documents").expect("list");

    assert_eq!(entries.len(), 2, "the folder itself must not appear: {entries:#?}");

    let doc = &entries[0];
    assert_eq!(doc.name, "Report Q3.docx", "the name must be percent-decoded");
    assert_eq!(doc.path, "/Documents/Report Q3.docx");
    assert!(!doc.is_dir);
    assert_eq!(doc.size, 20480);
    assert_eq!(doc.etag, "deadbeef", "quotes must be stripped");
    assert_eq!(doc.file_id, "12345");
    assert_eq!(doc.modified, "Sat, 11 Oct 2026 19:02:11 GMT");
    assert!(doc.content_type.ends_with("wordprocessingml.document"));

    let dir = &entries[1];
    assert_eq!(dir.name, "Archive");
    assert!(dir.is_dir, "an empty <d:collection/> marks a folder");
    assert_eq!(dir.size, 0);
}

#[test]
fn list_sends_a_depth_one_propfind_to_the_right_url() {
    let a = acct();
    let http = Fake::one(207, LISTING);
    let _ = Files::new(&a).list(&http, "/My Docs");
    let req = http.last();

    assert_eq!(req.method, Method::Propfind);
    assert_eq!(req.url, "https://cloud.example.net/remote.php/dav/files/alice/My%20Docs");
    assert_eq!(req.get_header("depth"), Some("1"));
    assert_eq!(req.get_header("ocs-apirequest"), Some("true"));
    assert!(req.get_header("authorization").is_some_and(|h| h.starts_with("Basic ")));
    assert!(String::from_utf8_lossy(&req.body).contains("<d:propfind"));
}

#[test]
fn list_tolerates_missing_properties() {
    let body = r#"<d:multistatus xmlns:d="DAV:">
      <d:response><d:href>/remote.php/dav/files/alice/x/odd</d:href>
      <d:propstat><d:prop></d:prop></d:propstat></d:response>
    </d:multistatus>"#;
    let a = acct();
    let http = Fake::one(207, body);
    let entries = Files::new(&a).list(&http, "/x").expect("a bare response must still parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "odd");
    assert_eq!(entries[0].size, 0);
    assert_eq!(entries[0].etag, "");
    assert!(!entries[0].is_dir);
}

#[test]
fn put_sends_an_if_match_when_given_an_etag() {
    let a = acct();
    let http = Fake::new(vec![Response::new(204, Vec::new())]);
    Files::new(&a).put(&http, "/a.docx", b"bytes".to_vec(), Some("deadbeef")).expect("put");
    let req = http.last();
    assert_eq!(req.method, Method::Put);
    assert_eq!(req.get_header("if-match"), Some("\"deadbeef\""), "the etag must be re-quoted");
    assert_eq!(req.body, b"bytes");
}

#[test]
fn put_without_an_etag_sends_no_if_match() {
    let a = acct();
    let http = Fake::new(vec![Response::new(201, Vec::new())]);
    Files::new(&a).put(&http, "/a.docx", b"x".to_vec(), None).expect("put");
    assert_eq!(http.last().get_header("if-match"), None);
}

#[test]
fn exists_reads_404_as_an_answer() {
    let a = acct();
    let missing = Fake::new(vec![Response::new(404, Vec::new())]);
    assert!(!Files::new(&a).exists(&missing, "/nope").expect("404 is not an error"));

    let present = Fake::new(vec![Response::new(207, LISTING.as_bytes().to_vec())]);
    assert!(Files::new(&a).exists(&present, "/yes").expect("ok"));
}

#[test]
fn a_server_error_becomes_a_status_error() {
    let a = acct();
    let http = Fake::one(503, "<html>down</html>");
    let err = Files::new(&a).list(&http, "/x").expect_err("503 must fail");
    assert!(matches!(err, crate::Error::Status { status: 503, .. }), "{err}");
}

#[test]
fn a_transport_failure_is_reported_as_one() {
    let a = acct();
    let http = Fake::new(Vec::new());
    let err = Files::new(&a).list(&http, "/x").expect_err("must fail");
    assert!(matches!(err, crate::Error::Transport(_)), "{err}");
}

#[test]
fn lock_prefers_the_header_token() {
    let a = acct();
    let resp = Response::new(
        200,
        br#"<d:prop xmlns:d="DAV:"><d:lockdiscovery><d:activelock>
        <d:locktoken><d:href>urn:uuid:from-body</d:href></d:locktoken>
      </d:activelock></d:lockdiscovery></d:prop>"#
            .to_vec(),
    )
    .header("lock-token", "<urn:uuid:from-header>");
    let http = Fake::new(vec![resp]);
    let lock = Files::new(&a).lock(&http, "/a.docx", 600).expect("lock");
    assert_eq!(lock.token, "urn:uuid:from-header");
    assert_eq!(http.last().get_header("timeout"), Some("Second-600"));
}

#[test]
fn lock_falls_back_to_the_body_token() {
    let a = acct();
    let resp = Response::new(
        200,
        br#"<d:prop xmlns:d="DAV:"><d:lockdiscovery><d:activelock>
        <d:locktoken><d:href>urn:uuid:from-body</d:href></d:locktoken>
      </d:activelock></d:lockdiscovery></d:prop>"#
            .to_vec(),
    );
    let http = Fake::new(vec![resp]);
    let lock = Files::new(&a).lock(&http, "/a.docx", 60).expect("lock");
    assert_eq!(lock.token, "urn:uuid:from-body");
}

// ---- OCS sharing ------------------------------------------------------------

const SHARE_OK: &str = r#"<?xml version="1.0"?>
<ocs>
  <meta><status>ok</status><statuscode>200</statuscode><message>OK</message></meta>
  <data>
    <id>42</id>
    <share_type>0</share_type>
    <uid_owner>alice</uid_owner>
    <path>/Documents/Report Q3.docx</path>
    <permissions>3</permissions>
    <share_with>bob</share_with>
    <share_with_displayname>Bob Jones</share_with_displayname>
  </data>
</ocs>"#;

#[test]
fn create_share_posts_a_form_and_parses_the_result() {
    let a = acct();
    let http = Fake::one(200, SHARE_OK);
    let share = Shares::new(&a).create(&http, &NewShare::with_user("/Documents/Report Q3.docx", "bob")).expect("create");

    assert_eq!(share.id, "42");
    assert_eq!(share.kind, Some(ShareKind::User));
    assert_eq!(share.share_with, "bob");
    assert_eq!(share.share_with_name, "Bob Jones");
    assert_eq!(share.permissions, Permissions::EDIT);
    assert!(share.permissions.contains(Permissions::READ));
    assert!(share.permissions.contains(Permissions::UPDATE));
    assert!(!share.permissions.contains(Permissions::DELETE));

    let req = http.last();
    assert_eq!(req.method, Method::Post);
    assert_eq!(req.url, "https://cloud.example.net/ocs/v2.php/apps/files_sharing/api/v1/shares");
    assert_eq!(req.get_header("content-type"), Some("application/x-www-form-urlencoded"));
    let body = String::from_utf8_lossy(&req.body);
    assert!(body.contains("shareType=0"), "{body}");
    assert!(body.contains("shareWith=bob"), "{body}");
    assert!(body.contains("permissions=3"), "{body}");
    assert!(body.contains("path=%2FDocuments%2FReport%20Q3.docx"), "{body}");
}

#[test]
fn a_public_link_omits_share_with_and_can_carry_a_password() {
    let a = acct();
    let http = Fake::one(200, SHARE_OK);
    let new = NewShare::public_link("/a.docx").password("hunter2").expires("2026-12-31");
    let _ = Shares::new(&a).create(&http, &new).expect("create");
    let req = http.last();
    let body = String::from_utf8_lossy(&req.body);
    assert!(body.contains("shareType=3"), "{body}");
    assert!(!body.contains("shareWith="), "a link share has no recipient: {body}");
    assert!(body.contains("password=hunter2"), "{body}");
    assert!(body.contains("expireDate=2026-12-31"), "{body}");
}

#[test]
fn an_ocs_refusal_is_an_error_even_at_http_200() {
    let body = r#"<ocs><meta><status>failure</status><statuscode>404</statuscode>
      <message>Wrong path, file/folder doesn't exist</message></meta><data/></ocs>"#;
    let a = acct();
    let http = Fake::one(200, body);
    let err = Shares::new(&a).create(&http, &NewShare::with_user("/gone.docx", "bob")).expect_err("an OCS failure must surface");
    match err {
        crate::Error::Ocs { code, message } => {
            assert_eq!(code, 404);
            assert!(message.contains("doesn't exist"), "{message}");
        }
        other => panic!("expected an Ocs error, got {other}"),
    }
}

#[test]
fn ocs_accepts_both_success_codes() {
    for code in ["100", "200"] {
        let body = format!(r#"<ocs><meta><statuscode>{code}</statuscode></meta><data><element><id>7</id></element></data></ocs>"#);
        let a = acct();
        let http = Fake::one(200, &body);
        let shares = Shares::new(&a).list(&http, None).expect("list");
        assert_eq!(shares.len(), 1, "statuscode {code} must be accepted");
        assert_eq!(shares[0].id, "7");
    }
}

#[test]
fn list_shares_for_a_path_puts_it_in_the_query() {
    let body = r#"<ocs><meta><statuscode>200</statuscode></meta><data/></ocs>"#;
    let a = acct();
    let http = Fake::one(200, body);
    let _ = Shares::new(&a).list(&http, Some("/My Docs/a.docx")).expect("list");
    let url = http.last().url;
    assert!(url.contains("?path=%2FMy%20Docs%2Fa.docx"), "{url}");
    assert!(url.contains("reshares=true"), "{url}");
}

#[test]
fn delete_share_refuses_a_non_numeric_id() {
    let a = acct();
    let http = Fake::one(200, "<ocs/>");
    let err = Shares::new(&a).delete(&http, "7/../9").expect_err("must refuse");
    assert!(matches!(err, crate::Error::Invalid { what: "share id", .. }), "{err}");
    assert!(http.seen.borrow().is_empty(), "nothing should have been sent");
}

#[test]
fn permissions_combine() {
    let p = Permissions::READ.with(Permissions::UPDATE).with(Permissions::SHARE);
    assert!(p.contains(Permissions::READ));
    assert!(p.contains(Permissions::EDIT));
    assert!(!p.contains(Permissions::DELETE));
    assert!(Permissions::ALL.contains(Permissions::DELETE));
}

// ---- CardDAV ----------------------------------------------------------------

const BOOKS: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
  <d:response>
    <d:href>/remote.php/dav/addressbooks/users/alice/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/addressbooks/users/alice/contacts/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/><card:addressbook/></d:resourcetype>
      <d:displayname>Contacts</d:displayname>
    </d:prop></d:propstat>
  </d:response>
</d:multistatus>"#;

#[test]
fn books_keeps_only_real_address_books() {
    let a = acct();
    let http = Fake::one(207, BOOKS);
    let books = Contacts::new(&a).books(&http).expect("books");
    assert_eq!(books.len(), 1, "the home collection is not an address book: {books:#?}");
    assert_eq!(books[0].display_name, "Contacts");
    assert_eq!(books[0].href, "/remote.php/dav/addressbooks/users/alice/contacts/");
}

#[test]
fn contacts_parse_from_vcards() {
    let body = r#"<d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
  <d:response><d:propstat><d:prop><card:address-data>BEGIN:VCARD
VERSION:4.0
UID:u-1
FN:Bob Jones
EMAIL;TYPE=work:bob@example.net
EMAIL;TYPE=home:bob@home.example
X-NC-USER-ID:bob
END:VCARD
</card:address-data></d:prop></d:propstat></d:response>
  <d:response><d:propstat><d:prop><card:address-data>BEGIN:VCARD
UID:u-2
FN:Carol
END:VCARD
</card:address-data></d:prop></d:propstat></d:response>
</d:multistatus>"#;
    let a = acct();
    let http = Fake::one(207, body);
    let list = Contacts::new(&a).list(&http, "/remote.php/dav/addressbooks/users/alice/contacts/").expect("list");

    assert_eq!(list.len(), 2);
    assert_eq!(list[0].uid, "u-1");
    assert_eq!(list[0].name, "Bob Jones");
    assert_eq!(list[0].emails, vec!["bob@example.net", "bob@home.example"]);
    assert_eq!(list[0].user_id, "bob");
    assert_eq!(list[0].share_target(), Some("bob"), "a local account shares by user id");

    assert_eq!(list[1].name, "Carol");
    assert_eq!(list[1].share_target(), None, "no id and no address means nothing to share with");

    assert_eq!(http.last().method, Method::Report);
}

#[test]
fn vcard_folding_and_escaping_are_handled() {
    let card = "BEGIN:VCARD\r\nUID:u-3\r\nFN:A Very Long\r\n  Name Here\r\nEMAIL:a\\,b@x.test\r\nEND:VCARD";
    let c = crate::contacts::parse_vcard(card).expect("parses");
    assert_eq!(c.uid, "u-3");
    assert_eq!(c.name, "A Very Long Name Here", "a continuation line must fold in");
    assert_eq!(c.emails, vec!["a,b@x.test"], "an escaped comma must unescape");
}

#[test]
fn a_vcard_with_nothing_showable_is_skipped() {
    assert!(crate::contacts::parse_vcard("BEGIN:VCARD\nUID:u-4\nEND:VCARD").is_none());
}

#[test]
fn a_nameless_contact_falls_back_to_its_address() {
    let c = crate::contacts::parse_vcard("BEGIN:VCARD\nUID:u-5\nEMAIL:x@y.test\nEND:VCARD").expect("parses");
    assert_eq!(c.name, "x@y.test");
    assert_eq!(c.share_target(), Some("x@y.test"));
}

#[test]
fn a_relative_book_href_is_refused() {
    let a = acct();
    let http = Fake::one(207, "<d:multistatus xmlns:d=\"DAV:\"/>");
    let err = Contacts::new(&a).list(&http, "contacts/").expect_err("must refuse");
    assert!(matches!(err, crate::Error::Invalid { what: "address book href", .. }), "{err}");
}

// ---- XML reader -------------------------------------------------------------

#[test]
fn the_xml_reader_ignores_namespace_prefixes() {
    for doc in [
        r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/a</d:href></d:response></d:multistatus>"#,
        r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/a</D:href></D:response></D:multistatus>"#,
        r#"<multistatus><response><href>/a</href></response></multistatus>"#,
    ] {
        let groups = crate::xml::groups(doc.as_bytes(), "response").expect("parses");
        assert_eq!(groups.len(), 1, "{doc}");
        assert_eq!(groups[0].get("href"), Some("/a"), "{doc}");
    }
}

#[test]
fn the_xml_reader_keeps_repeated_leaves() {
    let doc = r#"<r><response><href>/a</href><href>/b</href></response></r>"#;
    let groups = crate::xml::groups(doc.as_bytes(), "response").expect("parses");
    let all: Vec<&str> = groups[0].all("href").collect();
    assert_eq!(all, vec!["/a", "/b"]);
}

#[test]
fn the_xml_reader_rejects_runaway_nesting() {
    let doc = format!("{}{}", "<a>".repeat(200), "</a>".repeat(200));
    let err = crate::xml::groups(doc.as_bytes(), "response").expect_err("must be bounded");
    assert!(matches!(err, crate::Error::Malformed { what: "xml", .. }), "{err}");
}

#[test]
fn malformed_xml_is_an_error_not_a_panic() {
    let groups = crate::xml::groups(b"<r><response><href>/a", "response");
    // Either an error or a best-effort answer is acceptable. A panic is not.
    let _ = groups;
}

#[test]
fn escape_covers_the_five_characters() {
    assert_eq!(crate::xml::escape("a<b>&c\"d'e"), "a&lt;b&gt;&amp;c&quot;d&apos;e");
}
