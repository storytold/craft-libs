use super::*;
use crate::address::{Address, AddressError};
use crate::credentials::Credentials;
use crate::http::{API_RESPONSE_LIMIT, HttpRequest, Method, RequestKind};
use std::time::Duration;

fn request(kind: RequestKind) -> HttpRequest {
    let credentials = Credentials::bearer("token-abcdef").unwrap();
    let address = Address::parse("https://jmap.example.com/api/").unwrap();
    HttpRequest::new(Method::Post, address, kind, credentials.authorization())
}

fn judge(response: HttpResponse) -> Result<(), Error> {
    let mut redactor = Redactor::new();
    redactor.add("token-abcdef");
    judge_response(&request(RequestKind::Api), &response, &redactor)
}

fn problem(status: u16, body: &str) -> HttpResponse {
    HttpResponse::new(status, body.as_bytes().to_vec()).with_field("Content-Type", "application/problem+json")
}

// RFC 8620 §3.1: a successful API response is application/json.
#[test]
fn success_needs_json() {
    let ok = HttpResponse::new(200, b"{}".to_vec()).with_field("Content-Type", "Application/JSON; charset=utf-8");
    assert_eq!(judge(ok), Ok(()));
    let html = HttpResponse::new(200, b"<html>".to_vec()).with_field("Content-Type", "text/html");
    assert_eq!(judge(html), Err(Error::Unusable(Unusable::WrongContentType)));
    assert_eq!(judge(HttpResponse::new(200, b"{}".to_vec())), Err(Error::Unusable(Unusable::WrongContentType)));
    let download = HttpResponse::new(200, vec![1, 2, 3]).with_field("Content-Type", "application/pdf");
    assert_eq!(judge_response(&request(RequestKind::Download), &download, &Redactor::new()), Ok(()));
}

// Requirement 2.2: a limit the caller set for one request is enforced too, at the octet.
#[test]
fn per_request_body_limit() {
    let limited = request(RequestKind::Download).with_response_limit(10);
    let exact = HttpResponse::new(200, vec![0; 10]);
    assert_eq!(judge_response(&limited, &exact, &Redactor::new()), Ok(()));
    let over = HttpResponse::new(200, vec![0; 11]);
    assert_eq!(judge_response(&limited, &over, &Redactor::new()), Err(Error::Unusable(Unusable::BodyTooLarge)));
}

// Requirement 2.2: the crate refuses a body above the limit it states for the kind of request.
#[test]
fn success_body_limit() {
    let big = HttpResponse::new(200, vec![b' '; API_RESPONSE_LIMIT + 1]).with_field("Content-Type", "application/json");
    assert_eq!(judge(big), Err(Error::Unusable(Unusable::BodyTooLarge)));
}

// RFC 9110 §15.5.2 and §15.5.4, RFC 9457 §3.1.2: 401, and 403 when the problem says so.
#[test]
fn credentials_refused() {
    assert_eq!(judge(HttpResponse::new(401, Vec::new())), Err(Error::CredentialsRefused));
    assert_eq!(judge(problem(403, r#"{"status":401,"title":"Unauthorized"}"#)), Err(Error::CredentialsRefused));
    assert_eq!(judge(problem(403, r#"{"type":"https://example.com/probs/invalid_token"}"#)), Err(Error::CredentialsRefused));
    assert!(matches!(judge(problem(403, r#"{"status":403,"detail":"no access to this account"}"#)), Err(Error::Forbidden { detail: Some(_) })));
    assert!(matches!(judge(HttpResponse::new(403, Vec::new())), Err(Error::Forbidden { detail: None })));
}

// Requirement 2.4 (429 and 503), RFC 9110 §15.6.4 and §10.2.3: busy, with the delay asked for.
#[test]
fn busy() {
    let response = HttpResponse::new(503, Vec::new()).with_field("Retry-After", "120");
    assert_eq!(judge(response), Err(Error::Busy { retry_after: Some(RetryAfter::Delay(Duration::from_secs(120))) }));
    let response = HttpResponse::new(429, Vec::new()).with_field("Retry-After", "soon");
    assert_eq!(judge(response), Err(Error::Busy { retry_after: None }));
    let concurrency = problem(400, r#"{"type":"urn:ietf:params:jmap:error:limit","limit":"maxConcurrentRequests","status":400}"#);
    assert_eq!(judge(concurrency), Err(Error::Busy { retry_after: None }));
}

// RFC 9110 §15.5.14 and RFC 8620 §3.6.1: 413 and the JMAP limit problem with its limit name.
#[test]
fn too_large() {
    assert_eq!(judge(HttpResponse::new(413, Vec::new())), Err(Error::TooLarge { limit: None }));
    let limit = problem(400, r#"{"type":"urn:ietf:params:jmap:error:limit","limit":"maxSizeRequest","status":400,"detail":"too big"}"#);
    assert_eq!(judge(limit), Err(Error::TooLarge { limit: LimitName::new("maxSizeRequest") }));
    let odd = problem(400, r#"{"type":"urn:ietf:params:jmap:error:limit","limit":"max size\n"}"#);
    assert_eq!(judge(odd), Err(Error::TooLarge { limit: None }));
}

// RFC 8620 §3.6.1: the other request-level problems are recognised; RFC 9110 §15.5.5 and §15.5.11.
#[test]
fn refused_and_not_found() {
    let unknown = problem(
        400,
        r#"{"type":"urn:ietf:params:jmap:error:unknownCapability","status":400,"detail":"The Request object used capability 'https://example.com/apis/foobar', which is not supported by this server."}"#,
    );
    match judge(unknown) {
        Err(Error::Refused { problem: Some(RequestProblem::UnknownCapability), detail: Some(detail) }) => {
            assert!(detail.as_str().starts_with("The Request object used capability"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        judge(problem(400, r#"{"type":"urn:ietf:params:jmap:error:notJSON"}"#)),
        Err(Error::Refused { problem: Some(RequestProblem::NotJson), .. })
    ));
    assert!(matches!(
        judge(problem(400, r#"{"type":"urn:ietf:params:jmap:error:notRequest"}"#)),
        Err(Error::Refused { problem: Some(RequestProblem::NotRequest), .. })
    ));
    assert!(matches!(
        judge(problem(400, r#"{"type":"URN:IETF:params:jmap:error:somethingNew"}"#)),
        Err(Error::Refused { problem: Some(RequestProblem::Other(_)), .. })
    ));
    assert!(matches!(judge(HttpResponse::new(400, b"not json".to_vec())), Err(Error::Refused { problem: None, detail: None })));
    assert_eq!(judge(HttpResponse::new(404, Vec::new())), Err(Error::NotFound));
    assert_eq!(judge(HttpResponse::new(410, Vec::new())), Err(Error::NotFound));
}

// Requirement 3.5: a server-chosen identifier that echoes a secret is not shown.
#[test]
fn identifiers_echoing_secrets_are_dropped() {
    let mut redactor = Redactor::new();
    redactor.add("hunter22");
    let judge = |body: &str| judge_response(&request(RequestKind::Api), &problem(400, body), &redactor);
    assert_eq!(
        judge(r#"{"type":"urn:ietf:params:jmap:error:hunter22"}"#),
        Err(Error::Refused { problem: Some(RequestProblem::Other(String::new())), detail: None })
    );
    assert_eq!(judge(r#"{"type":"urn:ietf:params:jmap:error:limit","limit":"xhunter22"}"#), Err(Error::TooLarge { limit: None }));
}

// RFC 9457 §3.1: a member of the wrong type is ignored.
#[test]
fn wrong_member_types_are_ignored() {
    let response = problem(400, r#"{"type":42,"status":"401","limit":[],"detail":{"x":1},"title":"Bad"}"#);
    match judge(response) {
        Err(Error::Refused { problem: None, detail: Some(detail) }) => assert_eq!(detail.as_str(), "Bad"),
        other => panic!("{other:?}"),
    }
}

// Requirement 2.4: server text is a short, clean excerpt without secrets.
#[test]
fn detail_is_cleaned() {
    let body = format!(r#"{{"detail":"bad token token-abcdef\n\u001b[2J{}"}}"#, "x".repeat(500));
    match judge(problem(500, &body)) {
        Err(Error::Http { status: 500, detail: Some(detail) }) => {
            assert!(detail.as_str().starts_with("bad token [redacted] [2J"));
            assert!(!detail.as_str().contains('\n') && !detail.as_str().contains('\u{1b}'));
            assert!(detail.as_str().chars().count() <= crate::redact::EXCERPT_MAX_CHARS + 1);
        }
        other => panic!("{other:?}"),
    }
    let html = HttpResponse::new(502, b"<h1>Bad gateway</h1>".to_vec()).with_field("Content-Type", "text/html");
    assert_eq!(judge(html), Err(Error::Http { status: 502, detail: None }));
}

// RFC 9110 §10.2.2 and §15.4: redirects are judged, not followed; http and loopback targets are refused.
#[test]
fn redirects() {
    let response = HttpResponse::new(307, Vec::new()).with_field("Location", "/api/v2/");
    match judge(response) {
        Err(Error::Redirected { target, same_origin: true }) => assert_eq!(target.as_str(), "https://jmap.example.com/api/v2/"),
        other => panic!("{other:?}"),
    }
    let response = HttpResponse::new(301, Vec::new()).with_field("Location", "https://jmap.example.net/api/");
    assert!(matches!(judge(response), Err(Error::Redirected { same_origin: false, .. })));
    let response = HttpResponse::new(302, Vec::new()).with_field("Location", "http://jmap.example.com/api/");
    assert_eq!(judge(response), Err(Error::RedirectRefused(AddressError::InsecureTransport)));
    let response = HttpResponse::new(308, Vec::new()).with_field("Location", "http://127.0.0.1:8080/");
    assert_eq!(judge(response), Err(Error::RedirectRefused(AddressError::LocalFromRemote)));
    let response = HttpResponse::new(302, Vec::new()).with_field("Location", "https://user@example.com/");
    assert_eq!(judge(response), Err(Error::RedirectRefused(AddressError::UserInfo)));
    assert_eq!(judge(HttpResponse::new(302, Vec::new())), Err(Error::Http { status: 302, detail: None }));
    assert_eq!(judge(HttpResponse::new(304, Vec::new())), Err(Error::Http { status: 304, detail: None }));
}

// RFC 9110 §15: anything else is an HTTP error with its status.
#[test]
fn other_statuses() {
    assert_eq!(judge(HttpResponse::new(500, Vec::new())), Err(Error::Http { status: 500, detail: None }));
    assert_eq!(judge(HttpResponse::new(418, Vec::new())), Err(Error::Http { status: 418, detail: None }));
    assert_eq!(judge(HttpResponse::new(100, Vec::new())), Err(Error::Http { status: 100, detail: None }));
    assert_eq!(judge(HttpResponse::new(0, Vec::new())), Err(Error::Http { status: 0, detail: None }));
}

// RFC 8259 §8.1: a byte order mark may be ignored.
#[test]
fn json_with_bom() {
    assert!(parse_json(b"\xEF\xBB\xBF{\"a\":1}").is_ok());
    assert_eq!(parse_json(b"{\"a\":"), Err(Unusable::InvalidJson));
    let deep = "[".repeat(10_000);
    assert_eq!(parse_json(deep.as_bytes()), Err(Unusable::InvalidJson));
}

// RFC 8259 §9 and RFC 8620 §8.4: a text that would expand into too many values is refused before parsing.
#[test]
fn json_budget() {
    let many_objects = format!("[{}{{}}]", "{},".repeat(MAX_JSON_OBJECTS));
    assert_eq!(parse_json(many_objects.as_bytes()), Err(Unusable::TooComplex));
    let many_values = format!("[{}0]", "0,".repeat(MAX_JSON_VALUES));
    assert_eq!(parse_json(many_values.as_bytes()), Err(Unusable::TooComplex));
    let in_strings = format!("[\"{}\"]", "{},:[\\\"".repeat(200_000));
    assert!(parse_json(in_strings.as_bytes()).is_ok());
}
