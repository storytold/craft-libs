use super::*;
use crate::address::AddressError;
use crate::template::TemplateError;

/// The example session of RFC 8620 §2.1, with its line breaks and missing comma repaired.
pub(crate) const EXAMPLE: &str = r#"{
      "capabilities": {
        "urn:ietf:params:jmap:core": {
          "maxSizeUpload": 50000000,
          "maxConcurrentUpload": 8,
          "maxSizeRequest": 10000000,
          "maxConcurrentRequest": 8,
          "maxCallsInRequest": 32,
          "maxObjectsInGet": 256,
          "maxObjectsInSet": 128,
          "collationAlgorithms": ["i;ascii-numeric", "i;ascii-casemap", "i;unicode-casemap"]
        },
        "urn:ietf:params:jmap:mail": {},
        "urn:ietf:params:jmap:contacts": {},
        "https://example.com/apis/foobar": { "maxFoosFinangled": 42 }
      },
      "accounts": {
        "A13824": {
          "name": "john@example.com",
          "isPersonal": true,
          "isReadOnly": false,
          "accountCapabilities": {
            "urn:ietf:params:jmap:mail": { "maxMailboxesPerEmail": null, "maxMailboxDepth": 10 },
            "urn:ietf:params:jmap:contacts": {}
          }
        },
        "A97813": {
          "name": "jane@example.com",
          "isPersonal": false,
          "isReadOnly": true,
          "accountCapabilities": {
            "urn:ietf:params:jmap:mail": { "maxMailboxesPerEmail": 1, "maxMailboxDepth": 10 }
          }
        }
      },
      "primaryAccounts": {
        "urn:ietf:params:jmap:mail": "A13824",
        "urn:ietf:params:jmap:contacts": "A13824"
      },
      "username": "john@example.com",
      "apiUrl": "https://jmap.example.com/api/",
      "downloadUrl": "https://jmap.example.com/download/{accountId}/{blobId}/{name}?accept={type}",
      "uploadUrl": "https://jmap.example.com/upload/{accountId}/",
      "eventSourceUrl": "https://jmap.example.com/eventsource/?types={types}&closeafter={closeafter}&ping={ping}",
      "state": "75128aab4b1b"
    }"#;

pub(crate) fn session_address() -> Address {
    Address::parse("https://jmap.example.com/.well-known/jmap").unwrap()
}

fn parse(text: &str) -> Result<Session, Error> {
    Session::parse(text.as_bytes(), &session_address())
}

fn with(key: &str, value: &str) -> String {
    let mut object: Value = serde_json::from_str(EXAMPLE).unwrap();
    object[key] = serde_json::from_str(value).unwrap();
    object.to_string()
}

fn without(key: &str) -> String {
    let mut object: Value = serde_json::from_str(EXAMPLE).unwrap();
    object.as_object_mut().unwrap().remove(key);
    object.to_string()
}

// RFC 8620 §2 and §2.1: the example session reads in full.
#[test]
fn reads_example_session() {
    let session = parse(EXAMPLE).unwrap();
    assert!(session.has_capability(CAPABILITY_CORE));
    assert!(session.has_capability("https://example.com/apis/foobar"));
    assert!(!session.has_capability("urn:ietf:params:jmap:calendars"));
    let limits = session.core_limits();
    assert_eq!(limits.max_size_upload(), 50_000_000);
    assert_eq!(limits.max_concurrent_upload(), 8);
    assert_eq!(limits.max_size_request(), 10_000_000);
    assert_eq!(limits.max_concurrent_requests(), 8);
    assert_eq!(limits.max_calls_in_request(), 32);
    assert_eq!(limits.max_objects_in_get(), 256);
    assert_eq!(limits.max_objects_in_set(), 128);
    assert_eq!(session.accounts().count(), 2);
    let jane = session.account(&Id::new("A97813").unwrap()).unwrap();
    assert!(jane.is_read_only() && !jane.is_personal() && jane.has_capability(CAPABILITY_MAIL));
    assert_eq!(session.mail_account().unwrap().as_str(), "A13824");
    assert_eq!(session.username(), "john@example.com");
    assert_eq!(session.api_address().as_str(), "https://jmap.example.com/api/");
    assert!(session.event_source_template().is_some());
    assert_eq!(session.state(), "75128aab4b1b");
}

// RFC 8620 §2: core limits get the suggested minimums as defaults, and absurd values are capped.
#[test]
fn core_limit_defaults_and_caps() {
    let text = with(
        "capabilities",
        r#"{"urn:ietf:params:jmap:core":{"maxCallsInRequest":0,"maxObjectsInGet":1e15,"maxSizeRequest":-5,"maxObjectsInSet":"500","maxConcurrentRequests":2.0,"maxSizeUpload":9007199254740992}}"#,
    );
    let limits = *parse(&text).unwrap().core_limits();
    assert_eq!(limits.max_calls_in_request(), 1);
    assert_eq!(limits.max_objects_in_get(), 100_000);
    assert_eq!(limits.max_size_request(), 10_000_000);
    assert_eq!(limits.max_objects_in_set(), 500);
    assert_eq!(limits.max_concurrent_requests(), 2);
    assert_eq!(limits.max_size_upload(), 1_099_511_627_776);
    assert_eq!(CoreLimits::default().max_calls_in_request(), 16);
}

// Requirement 1.2 and RFC 8620 §2: a session is only read for a usable address; endpoints may be elsewhere.
#[test]
fn session_address_and_origins() {
    let plain = Address::parse("http://jmap.example.com/.well-known/jmap").unwrap();
    assert_eq!(Session::parse(EXAMPLE.as_bytes(), &plain).unwrap_err(), Error::Address(AddressError::InsecureTransport));
    assert!(parse(EXAMPLE).unwrap().endpoints_share_origin());
    let elsewhere = parse(&with("apiUrl", r#""https://api.example.net/jmap/""#)).unwrap();
    assert!(!elsewhere.endpoints_share_origin());
}

// RFC 8620 §2: the core capability is mandatory.
#[test]
fn needs_core_capability() {
    let text = with("capabilities", r#"{"urn:ietf:params:jmap:mail":{}}"#);
    assert!(matches!(parse(&text), Err(Error::Unusable(Unusable::Shape(_)))));
    assert!(matches!(parse(&without("capabilities")), Err(Error::Unusable(Unusable::Shape(_)))));
    assert!(matches!(parse(&without("apiUrl")), Err(Error::Unusable(Unusable::Shape(_)))));
    assert!(matches!(parse(&without("state")), Err(Error::Unusable(Unusable::Shape(_)))));
    assert!(matches!(parse("[]"), Err(Error::Unusable(Unusable::Shape(_)))));
    assert_eq!(parse("{").unwrap_err(), Error::Unusable(Unusable::InvalidJson));
}

// RFC 3986 §5.2 and requirement 3.1: session addresses resolve against the session address and obey rule 1.2.
#[test]
fn session_addresses_are_resolved_and_checked() {
    let session = parse(&with("apiUrl", r#""/api/v2""#)).unwrap();
    assert_eq!(session.api_address().as_str(), "https://jmap.example.com/api/v2");
    let error = parse(&with("apiUrl", r#""http://jmap.example.com/api/""#)).unwrap_err();
    assert_eq!(error, Error::Unusable(Unusable::Address("apiUrl", AddressError::InsecureTransport)));
    let error = parse(&with("apiUrl", r#""http://localhost:8080/api/""#)).unwrap_err();
    assert_eq!(error, Error::Unusable(Unusable::Address("apiUrl", AddressError::LocalFromRemote)));
    let error = parse(&with("uploadUrl", r#""http://jmap.example.com/upload/{accountId}/""#)).unwrap_err();
    assert_eq!(error, Error::Unusable(Unusable::Template("uploadUrl", TemplateError::Address(AddressError::InsecureTransport))));
    let error = parse(&with("downloadUrl", r#""https://jmap.example.com/download/{accountId}""#)).unwrap_err();
    assert_eq!(error, Error::Unusable(Unusable::Template("downloadUrl", TemplateError::MissingVariable("blobId"))));
    let error = parse(&with("downloadUrl", r#""https://{accountId}.example.com/{blobId}""#)).unwrap_err();
    assert_eq!(error, Error::Unusable(Unusable::Template("downloadUrl", TemplateError::ExpressionOutsidePathOrQuery)));
    let session = parse(&with("eventSourceUrl", r#""http://jmap.example.com/es""#)).unwrap();
    assert!(session.event_source_template().is_none());
    let local = Address::parse("http://localhost:8080/jmap").unwrap();
    let text = EXAMPLE.replace("https://jmap.example.com", "http://localhost:8080");
    let session = Session::parse(text.as_bytes(), &local).unwrap();
    assert_eq!(session.api_address().as_str(), "http://localhost:8080/api/");
}

// RFC 8620 §1.2: account ids and primary account ids are checked; bad ones are skipped.
#[test]
fn ids_are_checked() {
    let text =
        with("accounts", r#"{"bad id":{"name":"x"},"A1":{"name":"a","isPersonal":true,"accountCapabilities":{"urn:ietf:params:jmap:mail":{}}}}"#);
    let text = {
        let mut object: Value = serde_json::from_str(&text).unwrap();
        object["primaryAccounts"] = serde_json::from_str(r#"{"urn:ietf:params:jmap:mail":"../A1"}"#).unwrap();
        object.to_string()
    };
    let session = parse(&text).unwrap();
    assert_eq!(session.accounts().count(), 1);
    assert_eq!(session.primary_account(CAPABILITY_MAIL), None);
    assert_eq!(session.mail_account().unwrap().as_str(), "A1");
}

// RFC 8620 §2 (Account object): an account that cannot be read is skipped, the others stay usable.
#[test]
fn malformed_accounts_are_skipped() {
    let text = with(
        "accounts",
        r#"{"A1":{"name":"a","accountCapabilities":[]},"A2":{"name":"b","accountCapabilities":{"urn:ietf:params:jmap:mail":{}}},"A3":7}"#,
    );
    let session = parse(&text).unwrap();
    let ids: Vec<&str> = session.accounts().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["A2"]);
}

// RFC 8620 §2: primaryAccounts may lack mail; with several mail accounts and no primary there is no guess.
#[test]
fn mail_account_choice() {
    let session = parse(&with("primaryAccounts", "{}")).unwrap();
    assert_eq!(session.mail_account().unwrap().as_str(), "A13824");
    let mut shared_only: Value = serde_json::from_str(EXAMPLE).unwrap();
    shared_only["primaryAccounts"] = serde_json::json!({});
    shared_only["accounts"]["A13824"]["isPersonal"] = Value::Bool(false);
    let session = parse(&shared_only.to_string()).unwrap();
    assert_eq!(session.mail_account(), None);
    let session = parse(&with("primaryAccounts", r#"{"urn:ietf:params:jmap:mail":"A99999"}"#)).unwrap();
    assert_eq!(session.mail_account(), None);
}

// RFC 8620 §8.4 and §8.5 (resource exhaustion): counts and string lengths are bounded.
#[test]
fn bounds() {
    let many: Map<String, Value> = (0..=MAX_ACCOUNTS).map(|i| (format!("A{i}"), serde_json::json!({"name": "x"}))).collect();
    let text = with("accounts", &Value::Object(many).to_string());
    assert!(matches!(parse(&text), Err(Error::Unusable(Unusable::Shape(_)))));
    let text = with("state", &Value::String("s".repeat(MAX_STATE_LEN + 1)).to_string());
    assert!(matches!(parse(&text), Err(Error::Unusable(Unusable::Shape(_)))));
    let text = with("username", &Value::String("u".repeat(MAX_NAME_LEN + 1)).to_string());
    assert!(matches!(parse(&text), Err(Error::Unusable(Unusable::Shape(_)))));
    let text = with("state", "42");
    assert!(matches!(parse(&text), Err(Error::Unusable(Unusable::Shape(_)))));
}

// Requirement 2.3: the debug form shows no user name or account name.
#[test]
fn debug_hides_personal_data() {
    let shown = format!("{:?}", parse(EXAMPLE).unwrap());
    assert!(!shown.contains("john@") && !shown.contains("jane@"));
    assert!(shown.contains("75128aab4b1b"));
}
