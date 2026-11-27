use super::*;
use crate::address::AddressError;
use crate::error::Unusable;
use crate::http::TransportError;
use crate::retry::RetryAfter;
use crate::session::tests::{EXAMPLE, session_address};
use crate::test_support::{FakeExchange, block_on, made_secret};
use serde_json::{Map, Value, json};
use std::time::Duration;

fn credentials() -> Credentials {
    Credentials::basic("john@example.com", &made_secret("open-sesame-42")).unwrap()
}

fn json_response(status: u16, body: &Value) -> HttpResponse {
    HttpResponse::new(status, body.to_string().into_bytes()).with_field("Content-Type", "application/json")
}

fn client() -> Client {
    let session = Session::parse(EXAMPLE.as_bytes(), &session_address()).unwrap();
    Client::new(session, credentials()).unwrap()
}

// RFC 8620 §2: the session is fetched with an authenticated GET and parsed.
#[test]
fn fetches_session() {
    let exchange = FakeExchange::new(vec![Ok(HttpResponse::new(200, EXAMPLE.as_bytes().to_vec()).with_field("Content-Type", "application/json"))]);
    let session = block_on(fetch_session(&exchange, &session_address(), &credentials())).unwrap();
    assert_eq!(session.state(), "75128aab4b1b");
    let sent = exchange.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method(), Method::Get);
    assert_eq!(sent[0].url(), "https://jmap.example.com/.well-known/jmap");
    assert!(sent[0].fields().any(|(name, value)| name == "Authorization" && value == "Basic am9obkBleGFtcGxlLmNvbTpvcGVuLXNlc2FtZS00Mg=="));
}

// Requirement 1.2: credentials are never sent over plain http to a remote host.
#[test]
fn refuses_insecure_session_address() {
    let exchange = FakeExchange::new(Vec::new());
    let address = Address::parse("http://jmap.example.com/").unwrap();
    assert_eq!(block_on(fetch_session(&exchange, &address, &credentials())), Err(Error::Address(AddressError::InsecureTransport)));
    assert!(exchange.sent().is_empty());
}

// RFC 8620 §2.2 ("following any redirects") with requirement 3.1: the redirect comes back to the caller.
#[test]
fn session_redirect_is_returned() {
    let exchange = FakeExchange::new(vec![Ok(HttpResponse::new(301, Vec::new()).with_field("Location", "https://mail.example.net/jmap/session"))]);
    match block_on(fetch_session(&exchange, &session_address(), &credentials())) {
        Err(Error::Redirected { target, same_origin: false }) => assert_eq!(target.as_str(), "https://mail.example.net/jmap/session"),
        other => panic!("{other:?}"),
    }
    let exchange = FakeExchange::new(vec![Ok(HttpResponse::new(302, Vec::new()).with_field("Location", "http://mail.example.net/"))]);
    assert_eq!(block_on(fetch_session(&exchange, &session_address(), &credentials())), Err(Error::RedirectRefused(AddressError::InsecureTransport)));
}

// Requirement 3.2: the account, limits, capabilities, download and upload addresses.
#[test]
fn client_exposes_what_mail_code_needs() {
    let client = client();
    assert_eq!(client.account_id().as_str(), "A13824");
    assert_eq!(client.limits().max_calls_in_request(), 32);
    assert!(client.has_capability(CAPABILITY_MAIL));
    assert!(client.has_mail());
    assert!(!client.has_capability("urn:ietf:params:jmap:calendars"));
    let blob = Id::new("Gb1").unwrap();
    assert_eq!(
        client.download_address(&blob, "image/png", "photo 1.png").unwrap().as_str(),
        "https://jmap.example.com/download/A13824/Gb1/photo%201.png?accept=image%2Fpng"
    );
    assert_eq!(client.upload_address().unwrap().as_str(), "https://jmap.example.com/upload/A13824/");
    assert!(matches!(client.download_address(&blob, "image/png", ".."), Err(Error::Template(_))));
}

// RFC 8620 §3.1, §3.3, §3.4 and §3.7: a request with a result reference, and its per-call results.
#[test]
fn calls_the_api() {
    let client = client();
    let mut request = ApiRequest::new();
    request.using(CAPABILITY_MAIL).unwrap();
    let query = request.add_call("Email/query", Map::from_iter([("accountId".to_owned(), json!("A13824"))])).unwrap();
    let ids = request.reference(&query, "Email/query", "/ids").unwrap();
    let get = request.add_call_with_references("Email/get", Map::from_iter([("accountId".to_owned(), json!("A13824"))]), vec![("ids", ids)]).unwrap();
    let answer = json!({
        "methodResponses": [
            ["Email/query", {"accountId": "A13824", "ids": ["M1"], "position": 0}, "c0"],
            ["error", {"type": "invalidResultReference"}, "c1"]
        ],
        "sessionState": "changed"
    });
    let exchange = FakeExchange::new(vec![Ok(json_response(200, &answer))]);
    let mut response = block_on(client.call(&exchange, &request)).unwrap();
    assert!(client.is_session_stale(&response));
    assert_eq!(response.take_named(&query, "Email/query").unwrap()["ids"], json!(["M1"]));
    assert!(matches!(response.take(&get), Err(crate::response::CallFailure::Method(_))));
    let sent = &exchange.sent()[0];
    assert_eq!(sent.method(), Method::Post);
    assert_eq!(sent.url(), "https://jmap.example.com/api/");
    let body: Value = serde_json::from_slice(sent.body()).unwrap();
    assert_eq!(body["using"], json!(["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"]));
}

// RFC 8620 §2 limits: maxCallsInRequest and maxSizeRequest are enforced before sending.
#[test]
fn enforces_limits_before_sending() {
    let client = client();
    let mut request = ApiRequest::new();
    for _ in 0..33 {
        request.add_call("Core/echo", Map::new()).unwrap();
    }
    assert_eq!(client.api_request(&request).unwrap_err(), Error::Build(BuildError::TooManyCalls { limit: 32 }));
    let mut request = ApiRequest::new();
    request.add_call("Core/echo", Map::from_iter([("blob".to_owned(), json!("x".repeat(10_000_001)))])).unwrap();
    assert_eq!(client.api_request(&request).unwrap_err(), Error::Build(BuildError::TooLarge { limit: 10_000_000 }));
    let mut request = ApiRequest::new();
    request.using("urn:ietf:params:jmap:calendars").unwrap();
    request.add_call("Core/echo", Map::new()).unwrap();
    assert_eq!(client.api_request(&request).unwrap_err(), Error::Build(BuildError::UnsupportedCapability));
}

// Requirement 2.4 through the client: busy, credentials, transport overload, unusable answers.
#[test]
fn classifies_api_failures() {
    let client = client();
    let mut request = ApiRequest::new();
    request.add_call("Core/echo", Map::new()).unwrap();
    let cases: Vec<(Result<HttpResponse, TransportError>, Error)> = vec![
        (
            Ok(HttpResponse::new(429, Vec::new()).with_field("Retry-After", "7")),
            Error::Busy { retry_after: Some(RetryAfter::Delay(Duration::from_secs(7))) },
        ),
        (Ok(HttpResponse::new(401, Vec::new())), Error::CredentialsRefused),
        (Err(TransportError::Overloaded), Error::Busy { retry_after: None }),
        (Err(TransportError::TimedOut), Error::Transport(TransportError::TimedOut)),
        (Ok(HttpResponse::new(200, b"<html>".to_vec()).with_field("Content-Type", "text/html")), Error::Unusable(Unusable::WrongContentType)),
        (Ok(HttpResponse::new(200, b"{\"x\":".to_vec()).with_field("Content-Type", "application/json")), Error::Unusable(Unusable::InvalidJson)),
    ];
    for (answer, expected) in cases {
        let exchange = FakeExchange::new(vec![answer]);
        assert_eq!(block_on(client.call(&exchange, &request)).unwrap_err(), expected);
    }
}

// Requirement 3.5 through the client: the user name, the account name and the password never show.
#[test]
fn errors_hide_secrets() {
    let client = client();
    let mut request = ApiRequest::new();
    request.add_call("Core/echo", Map::new()).unwrap();
    let problem = json!({"status": 500, "detail": "user john@example.com with password open-sesame-42 at https://jmap.example.com/.well-known/jmap"});
    let response = HttpResponse::new(500, problem.to_string().into_bytes()).with_field("Content-Type", "application/problem+json");
    let exchange = FakeExchange::new(vec![Ok(response)]);
    let error = block_on(client.call(&exchange, &request)).unwrap_err();
    let shown = error.to_string();
    assert!(!shown.contains("john") && !shown.contains("open-sesame") && !shown.contains("well-known"));
}

// Requirement 2.3: the client's debug form shows no secret, no user name, no account id and no address.
#[test]
fn debug_shows_nothing_personal() {
    let client = client();
    let debug = format!("{client:?}");
    for hidden in ["john", "jane", "open-sesame", "A13824", "jmap.example.com", "75128aab4b1b", "am9obk"] {
        assert!(!debug.contains(hidden), "{hidden} shows in the client's debug form");
    }
    assert!(debug.starts_with("Client { scheme: \"Basic\""));
}

// Requirement 3.5: a token in the session address's query is redacted on its own, and a short server-chosen
// user name does not blank every message.
#[test]
fn session_address_parts_are_secrets() {
    let address = Address::parse("https://jmap.example.com/.well-known/jmap?apikey=SECRETKEY123").unwrap();
    let text = EXAMPLE.replace(r#""username": "john@example.com""#, r#""username": "jo""#);
    let session = Session::parse(text.as_bytes(), &address).unwrap();
    let client = Client::new(session, Credentials::bearer("token-0123456789").unwrap()).unwrap();
    assert_eq!(client.excerpt("invalid key SECRETKEY123 for jo").as_str(), "invalid key [redacted] for jo");
    assert_eq!(client.excerpt("bad token-0123456789").as_str(), "bad [redacted]");
}

// RFC 8620 §6.2: a download is an authenticated GET of the expanded template; the body comes back as is.
#[test]
fn downloads() {
    let client = client();
    let exchange = FakeExchange::new(vec![Ok(HttpResponse::new(200, vec![0, 159, 146, 150]).with_field("Content-Type", "application/octet-stream"))]);
    let bytes = block_on(client.download(&exchange, &Id::new("B1").unwrap(), "application/octet-stream", "a.bin", None)).unwrap();
    assert_eq!(bytes, vec![0, 159, 146, 150]);
    assert_eq!(exchange.sent()[0].url(), "https://jmap.example.com/download/A13824/B1/a.bin?accept=application%2Foctet-stream");
    let exchange = FakeExchange::new(vec![Ok(HttpResponse::new(404, Vec::new()))]);
    assert_eq!(block_on(client.download(&exchange, &Id::new("B1").unwrap(), "", "a.bin", None)), Err(Error::NotFound));
}

// Requirement 2.2: a limit the caller sets for one request reaches the exchange before the body is read,
// and a body above it is refused, whether the exchange stops early or hands the whole body over.
#[test]
fn per_request_limits() {
    let client = client();
    let blob = Id::new("B1").unwrap();
    let body = || Ok(HttpResponse::new(200, vec![7; 5]));
    let careful = FakeExchange::capping(vec![body()]);
    assert_eq!(block_on(client.download(&careful, &blob, "", "a.bin", Some(4))), Err(Error::Unusable(Unusable::BodyTooLarge)));
    assert_eq!(careful.sent()[0].max_response_bytes(), 4);
    let careless = FakeExchange::new(vec![body()]);
    assert_eq!(block_on(client.download(&careless, &blob, "", "a.bin", Some(4))), Err(Error::Unusable(Unusable::BodyTooLarge)));
    let fits = FakeExchange::capping(vec![body()]);
    assert_eq!(block_on(client.download(&fits, &blob, "", "a.bin", Some(5))).unwrap(), vec![7; 5]);
    let unlimited = FakeExchange::new(vec![body()]);
    block_on(client.download(&unlimited, &blob, "", "a.bin", None)).unwrap();
    assert_eq!(unlimited.sent()[0].max_response_bytes(), crate::http::DOWNLOAD_RESPONSE_LIMIT);

    let mut request = ApiRequest::new();
    request.add_call("Core/echo", Map::new()).unwrap();
    request.set_response_limit(64);
    assert_eq!(client.api_request(&request).unwrap().max_response_bytes(), 64);
    let answer = json!({"methodResponses": [["Core/echo", {"pad": "x".repeat(100)}, "c0"]], "sessionState": "s"});
    let exchange = FakeExchange::capping(vec![Ok(json_response(200, &answer))]);
    assert_eq!(block_on(client.call(&exchange, &request)).unwrap_err(), Error::Unusable(Unusable::BodyTooLarge));
    request.set_response_limit(usize::MAX);
    assert_eq!(client.api_request(&request).unwrap().max_response_bytes(), crate::http::API_RESPONSE_LIMIT);
}

// Requirement 3.2: no mail account, no client.
#[test]
fn needs_a_mail_account() {
    let text = EXAMPLE.replace("urn:ietf:params:jmap:mail\": {", "urn:example:other\": {");
    let session = Session::parse(text.as_bytes(), &session_address()).unwrap();
    assert_eq!(session.mail_account(), None);
    assert_eq!(Client::new(session, credentials()).unwrap_err(), Error::NoMailAccount);
}

// Requirement 2.6 context (RFC 8620 §3.1 calls made through the caller's exchange): the helpers' futures are
// Send when the exchange's are, so native programs can spawn them.
#[test]
fn futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    let client = client();
    let exchange = FakeExchange::new(Vec::new());
    let request = ApiRequest::new();
    let future = client.call(&exchange, &request);
    assert_send(&future);
    let address = session_address();
    let credentials = credentials();
    assert_send(&fetch_session(&exchange, &address, &credentials));
}
