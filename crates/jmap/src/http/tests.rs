use super::*;
use crate::credentials::Credentials;
use crate::test_support::made_secret;

fn request() -> HttpRequest {
    let credentials = Credentials::basic("john@example.com", &made_secret("s3cret-pass")).unwrap();
    let address = Address::parse("https://jmap.example.com/api/private?token=abc").unwrap();
    HttpRequest::new(Method::Post, address, RequestKind::Api, credentials.authorization()).with_json_body(b"{\"a\":1}".to_vec())
}

// RFC 8620 §3.1: an API request is a POST of application/json.
#[test]
fn api_request_fields() {
    let request = request();
    let fields: Vec<_> = request.fields().collect();
    assert!(fields.contains(&("Content-Type", "application/json")));
    assert!(fields.contains(&("Accept", "application/json")));
    assert!(fields.iter().any(|(name, value)| *name == "Authorization" && value.starts_with("Basic ")));
    assert_eq!(request.method().as_str(), "POST");
    assert_eq!(request.url(), "https://jmap.example.com/api/private?token=abc");
    assert_eq!(request.max_response_bytes(), API_RESPONSE_LIMIT);
}

// Requirement 2.3: the debug form of a request shows no secret, no user name, no path, no body.
#[test]
fn request_debug_is_clean() {
    let shown = format!("{:?}", request());
    for hidden in ["s3cret", "john", "Basic am9", "private", "token=abc", "\"a\""] {
        assert!(!shown.contains(hidden), "{hidden} shows in the request's debug form");
    }
    assert!(shown.contains("Authorization: <redacted>"));
    assert!(shown.contains("https://jmap.example.com"));
}

// RFC 9110 §5.1: field names are case-insensitive; only the fields the crate reads are kept.
#[test]
fn response_fields() {
    let response = HttpResponse::new(503, Vec::new())
        .with_field("retry-AFTER", " 120 ")
        .with_field("CONTENT-type", "application/problem+json")
        .with_field("Location", "/next")
        .with_field("Location", "/second")
        .with_field("X-Other", "ignored")
        .with_field("Content-Type", &"x".repeat(MAX_FIELD_VALUE_LEN + 1));
    assert_eq!(response.retry_after(), Some("120"));
    assert_eq!(response.content_type(), Some("application/problem+json"));
    assert_eq!(response.location(), Some("/next"));
}

// Requirement 2.2: the caller may lower the limit of one request, never raise it.
#[test]
fn per_request_limit() {
    let request = request();
    assert_eq!(request.max_response_bytes(), API_RESPONSE_LIMIT);
    let lowered = request.clone().with_response_limit(1_000);
    assert_eq!(lowered.max_response_bytes(), 1_000);
    let raised = request.with_response_limit(usize::MAX);
    assert_eq!(raised.max_response_bytes(), API_RESPONSE_LIMIT);
    assert!(format!("{lowered:?}").contains("max_response_bytes: 1000"));
}

// Requirement 2.2: the size limit is stated per kind of request.
#[test]
fn limits_per_kind() {
    assert_eq!(RequestKind::Session.max_response_bytes(), SESSION_RESPONSE_LIMIT);
    assert_eq!(RequestKind::Download.max_response_bytes(), DOWNLOAD_RESPONSE_LIMIT);
    assert!(!RequestKind::Download.expects_json());
}
