//! u1: the rules for a custom (unlisted, OpenAI-shaped) connection: a named key header, and what
//! a stored file may carry. The form is `ConnectionDraft`; these tests go through its public API.

use craft_ai_core::connection::ConnectionError;
use craft_ai_core::{Billing, Connection, ConnectionDraft, Field, KeyChange, Locality};

fn custom(endpoint: &str, header: &str) -> ConnectionDraft {
    let mut d = ConnectionDraft::custom();
    d.label = "Studio gateway".to_owned();
    d.endpoint = endpoint.to_owned();
    d.key = "k-123".to_owned();
    d.key_header = header.to_owned();
    d
}

#[test]
fn the_custom_form_starts_from_the_other_preset_with_billing_unknown() {
    let d = ConnectionDraft::custom();
    assert!(d.is_custom());
    assert_eq!(d.provider, "openai-compatible");
    assert_eq!(d.billing, Billing::Unknown);
    assert!(d.uses_key);
    assert!(d.key_header.is_empty());
    assert!(d.models.is_empty());
}

#[test]
fn without_a_header_name_the_key_goes_as_a_bearer_token() {
    let p = custom("https://gw.example.com/ai/v7", "").prepare(&[]).unwrap();
    assert_eq!(p.connection.key_header, None);
    let json = serde_json::to_string(&p.connection).unwrap();
    assert!(!json.contains("key_header"), "{json}");
}

#[test]
fn a_named_header_is_kept_saved_and_edited_back() {
    let p = custom("https://gw.example.com/ai/v7", " X-Api-Key ").prepare(&[]).unwrap();
    assert_eq!(p.connection.key_header.as_deref(), Some("X-Api-Key"));
    assert_eq!(p.connection.endpoint.as_deref(), Some("https://gw.example.com/ai/v7"));
    assert_eq!(p.connection.locality, Locality::Remote);
    assert!(matches!(p.key, KeyChange::Set(_)));

    let again: Connection = serde_json::from_str(&serde_json::to_string(&p.connection).unwrap()).unwrap();
    assert_eq!(again, p.connection);
    let edit = ConnectionDraft::from_connection(&again);
    assert_eq!(edit.key_header, "X-Api-Key");
    let kept = edit.prepare(std::slice::from_ref(&again)).unwrap();
    assert_eq!(kept.connection.key_header.as_deref(), Some("X-Api-Key"));
    assert!(matches!(kept.key, KeyChange::Keep));
}

#[test]
fn header_names_that_could_break_the_request_are_refused() {
    let long = "a".repeat(65);
    for bad in [
        "host",
        "Content-Length",
        "content-type",
        "transfer-encoding",
        "connection",
        "cookie",
        "proxy-authorization",
        "proxy-anything",
        "x api key",
        "x:api",
        "x-api\r\nhost",
        "x-clé",
        "authorization",
        "Authorization",
        long.as_str(),
    ] {
        let e = custom("https://gw.example.com/v1", bad).prepare(&[]).expect_err(bad);
        assert_eq!(e.field, Field::KeyHeader, "{bad:?}");
        assert!(!e.message.contains("k-123"), "{bad:?}");
    }
}

#[test]
fn ordinary_key_header_names_are_accepted() {
    for ok in ["x-api-key", "api-key", "X-Goog-Api-Key", "apikey", "x_token"] {
        custom("https://gw.example.com/v1", ok).prepare(&[]).unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
}

#[test]
fn a_header_name_without_a_key_is_refused() {
    let mut d = custom("http://127.0.0.1:9000/v1", "x-api-key");
    d.uses_key = false;
    d.key.clear();
    assert_eq!(d.prepare(&[]).unwrap_err().field, Field::KeyHeader);
}

#[test]
fn a_stored_file_with_a_bad_header_name_is_refused_on_validate() {
    let mut c = custom("https://gw.example.com/v1", "x-api-key").prepare(&[]).unwrap().connection;
    assert!(c.validate().is_ok());
    c.key_header = Some("host".to_owned());
    assert!(matches!(c.validate(), Err(ConnectionError::KeyHeader(_))));
    c.key_header = Some("x-api-key".to_owned());
    c.auth = craft_ai_core::Auth::None;
    assert!(matches!(c.validate(), Err(ConnectionError::KeyHeader(_))));
}

#[test]
fn plain_http_to_a_remote_host_is_still_refused_for_a_custom_address() {
    let e = custom("http://gw.example.com/v1", "x-api-key").prepare(&[]).unwrap_err();
    assert_eq!(e.field, Field::Address);
}
