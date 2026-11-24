use super::*;
use crate::request::ApiRequest;
use serde_json::json;

fn calls(count: usize) -> (ApiRequest, Vec<CallId>) {
    let mut request = ApiRequest::new();
    let ids = (0..count).map(|_| request.add_call("Core/echo", Map::new()).unwrap()).collect();
    (request, ids)
}

// RFC 8620 §3.4.1: the example response, with an error for the third call.
#[test]
fn reads_example_response() {
    let (_, ids) = calls(3);
    let body = br#"{
          "methodResponses": [
            ["method1", {"arg1": 3, "arg2": "foo"}, "c0"],
            ["method2", {"isBlah": true}, "c1"],
            ["anotherResponseFromMethod2", {"data": 10, "yetmoredata": "Hello"}, "c1"],
            ["error", {"type": "unknownMethod"}, "c2"]
          ],
          "sessionState": "75128aab4b1b"
        }"#;
    let mut response = ApiResponse::parse(body, &Redactor::new()).unwrap();
    assert_eq!(response.session_state(), "75128aab4b1b");
    assert_eq!(response.take_named(&ids[0], "method1").unwrap()["arg2"], json!("foo"));
    let second = response.take(&ids[1]).unwrap();
    assert_eq!(second.iter().map(MethodResponse::name).collect::<Vec<_>>(), ["method2", "anotherResponseFromMethod2"]);
    match response.take(&ids[2]) {
        Err(CallFailure::Method(error)) => {
            assert_eq!(error.kind(), &MethodErrorKind::UnknownMethod)
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(response.take(&ids[0]), Err(CallFailure::Missing));
}

// RFC 8620 §3.6.2: an error is one call's failure; unknown types are kept (and treated like serverFail).
#[test]
fn method_errors() {
    let (_, ids) = calls(2);
    let body = json!({
        "methodResponses": [
            ["error", {"type": "somethingNew", "description": "Line one\nwith secret-pass inside"}, "c0"],
            ["Core/echo", {}, "c1"]
        ],
        "sessionState": "s1"
    });
    let mut redactor = Redactor::new();
    redactor.add("secret-pass");
    let mut response = ApiResponse::parse(body.to_string().as_bytes(), &redactor).unwrap();
    match response.take(&ids[0]) {
        Err(CallFailure::Method(error)) => {
            assert_eq!(error.kind(), &MethodErrorKind::Other("somethingNew".into()));
            assert_eq!(error.description().unwrap().as_str(), "Line one with [redacted] inside");
            assert!(!error.to_string().contains('\n'));
        }
        other => panic!("{other:?}"),
    }
    assert!(response.take(&ids[1]).is_ok());
    assert!(MethodErrorKind::ServerUnavailable.may_succeed_later());
    assert_eq!(MethodErrorKind::from_type(None), MethodErrorKind::Other(String::new()));
    assert_eq!(MethodErrorKind::from_type(Some("x y")), MethodErrorKind::Other(String::new()));
    assert_eq!(MethodErrorKind::from_type(Some("stateMismatch")), MethodErrorKind::StateMismatch);
}

// Requirement 3.5: an error type that echoes a secret is not kept.
#[test]
fn error_type_echoing_a_secret() {
    let (_, ids) = calls(1);
    let mut redactor = Redactor::new();
    redactor.add("hunter22");
    let body = json!({"methodResponses": [["error", {"type": "hunter22"}, "c0"]], "sessionState": "s"});
    let mut response = ApiResponse::parse(body.to_string().as_bytes(), &redactor).unwrap();
    match response.take(&ids[0]) {
        Err(CallFailure::Method(error)) => {
            assert_eq!(error.kind(), &MethodErrorKind::Other(String::new()))
        }
        other => panic!("{other:?}"),
    }
}

// RFC 8620 §3.6.2: nothing more happens within a call after its error.
#[test]
fn responses_after_error_are_ignored() {
    let (_, ids) = calls(1);
    let body = json!({
        "methodResponses": [["Foo/set", {}, "c0"], ["error", {"type": "serverPartialFail"}, "c0"], ["Foo/get", {}, "c0"]],
        "sessionState": "s"
    });
    let mut response = ApiResponse::parse(body.to_string().as_bytes(), &Redactor::new()).unwrap();
    assert!(matches!(response.take(&ids[0]), Err(CallFailure::Method(error)) if error.kind() == &MethodErrorKind::ServerPartialFail));
}

// Requirement 3.3: a name other than the one asked for is not silently accepted.
#[test]
fn unexpected_names() {
    let (_, ids) = calls(1);
    let body = json!({"methodResponses": [["Other/get", {}, "c0"]], "sessionState": "s"});
    let mut response = ApiResponse::parse(body.to_string().as_bytes(), &Redactor::new()).unwrap();
    assert_eq!(response.take_named(&ids[0], "Foo/get"), Err(CallFailure::UnexpectedResponse));
}

// RFC 8620 §3.3–§3.4: createdIds are returned when given; invalid ids are dropped.
#[test]
fn created_ids() {
    let body = json!({"methodResponses": [], "sessionState": "s", "createdIds": {"k1": "M1", "bad key": "M2", "k3": "bad value"}});
    let response = ApiResponse::parse(body.to_string().as_bytes(), &Redactor::new()).unwrap();
    let created = response.created_ids().unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(created.get(&Id::new("k1").unwrap()).unwrap().as_str(), "M1");
}

// RFC 8620 §3.2 and §3.4: anything that is not a proper Response object is unusable.
#[test]
fn refuses_bad_shapes() {
    for body in [
        r#"[]"#,
        r#"{"sessionState":"s"}"#,
        r#"{"methodResponses":[]}"#,
        r#"{"methodResponses":{},"sessionState":"s"}"#,
        r#"{"methodResponses":[["a",{},"c0","extra"]],"sessionState":"s"}"#,
        r#"{"methodResponses":[["a",[],"c0"]],"sessionState":"s"}"#,
        r#"{"methodResponses":[[1,{},"c0"]],"sessionState":"s"}"#,
        r#"{"methodResponses":[["a",{}]],"sessionState":"s"}"#,
        r#"{"methodResponses":["x"],"sessionState":"s"}"#,
        r#"{"methodResponses":[["",{},"c0"]],"sessionState":"s"}"#,
        r#"{"methodResponses":[["a b",{},"c0"]],"sessionState":"s"}"#,
        r#"{"methodResponses":[],"sessionState":7}"#,
        r#"{"methodResponses":[],"sessionState":"s","createdIds":[]}"#,
    ] {
        assert!(matches!(ApiResponse::parse(body.as_bytes(), &Redactor::new()), Err(Error::Unusable(Unusable::Shape(_)))), "{body}");
    }
    assert_eq!(ApiResponse::parse(b"{", &Redactor::new()).unwrap_err(), Error::Unusable(Unusable::InvalidJson));
}

// RFC 8620 §8.4 and §8.5 (resource exhaustion): the number of responses is bounded.
#[test]
fn bounds() {
    let responses: Vec<Value> = (0..=MAX_METHOD_RESPONSES).map(|i| json!(["Core/echo", {}, format!("c{i}")])).collect();
    let body = json!({"methodResponses": responses, "sessionState": "s"});
    assert!(matches!(ApiResponse::parse(body.to_string().as_bytes(), &Redactor::new()), Err(Error::Unusable(Unusable::Shape(_)))));
    let body = json!({"methodResponses": [["Core/echo", {}, "c".repeat(256)]], "sessionState": "s"});
    assert!(ApiResponse::parse(body.to_string().as_bytes(), &Redactor::new()).is_err());
}
