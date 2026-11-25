//! u1: the rules for a custom (unlisted, OpenAI-shaped) connection: a named key header, and what
//! a stored file may carry. The form is `ConnectionDraft`; these tests go through its public API.

use craft_ai_core::{ConnectionDraft, Field};

fn custom(endpoint: &str, header: &str) -> ConnectionDraft {
    let mut d = ConnectionDraft::custom();
    d.label = "Studio gateway".to_owned();
    d.endpoint = endpoint.to_owned();
    d.key = "k-123".to_owned();
    d.key_header = header.to_owned();
    d
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
