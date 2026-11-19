use super::*;
use crate::session::CAPABILITY_MAIL;
use serde_json::json;

fn args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("not an object"),
    }
}

// RFC 8620 §3.3 and §3.3.1: using, methodCalls with name, arguments and call id.
#[test]
fn builds_request_object() {
    let mut request = ApiRequest::new();
    request.using(CAPABILITY_MAIL).unwrap().using(CAPABILITY_MAIL).unwrap();
    let first = request.add_call("method1", args(json!({"arg1": "arg1data", "arg2": "arg2data"}))).unwrap();
    let second = request.add_call("method2", args(json!({"arg1": "arg1data"}))).unwrap();
    assert_ne!(first, second);
    let body: Value = serde_json::from_slice(&request.to_json().unwrap()).unwrap();
    assert_eq!(
        body,
        json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": [
                ["method1", {"arg1": "arg1data", "arg2": "arg2data"}, "c0"],
                ["method2", {"arg1": "arg1data"}, "c1"]
            ]
        })
    );
    assert_eq!(request.call_ids().map(CallId::as_str).collect::<Vec<_>>(), ["c0", "c1"]);
}

// RFC 8620 §3.7: the Email/query then Email/get example with "#ids".
#[test]
fn result_references() {
    let mut request = ApiRequest::new();
    let query = request.add_call("Email/query", args(json!({"accountId": "A1", "limit": 10}))).unwrap();
    let ids = request.reference(&query, "Email/query", "/ids").unwrap();
    let get =
        request.add_call_with_references("Email/get", args(json!({"accountId": "A1", "properties": ["threadId"]})), vec![("ids", ids)]).unwrap();
    let threads = request.reference(&get, "Email/get", "/list/*/threadId").unwrap();
    request.add_call_with_references("Thread/get", args(json!({"accountId": "A1"})), vec![("ids", threads)]).unwrap();
    let body: Value = serde_json::from_slice(&request.to_json().unwrap()).unwrap();
    assert_eq!(body["methodCalls"][1][1]["#ids"], json!({"resultOf": "c0", "name": "Email/query", "path": "/ids"}));
    assert_eq!(body["methodCalls"][2][1]["#ids"], json!({"resultOf": "c1", "name": "Email/get", "path": "/list/*/threadId"}));
}

// RFC 8620 §3.7: a reference must point to a previous call; "foo" and "#foo" together are invalid.
#[test]
fn reference_errors() {
    let mut request = ApiRequest::new();
    let first = request.add_call("Foo/changes", Map::new()).unwrap();
    let mut other = ApiRequest::new();
    assert_eq!(other.reference(&first, "Foo/changes", "/created"), Err(BuildError::UnknownCall));
    let mine = other.add_call("Foo/changes", Map::new()).unwrap();
    assert_eq!(mine.as_str(), first.as_str());
    assert_eq!(other.reference(&first, "Foo/changes", "/created"), Err(BuildError::UnknownCall));
    let reference = request.reference(&first, "Foo/changes", "/created").unwrap();
    assert_eq!(
        request.add_call_with_references("Foo/get", args(json!({"ids": []})), vec![("ids", reference.clone())]),
        Err(BuildError::DuplicateArgument)
    );
    assert_eq!(request.add_call("Foo/get", args(json!({"ids": [], "#ids": {}}))), Err(BuildError::DuplicateArgument));
    assert_eq!(other.add_call_with_references("Foo/get", Map::new(), vec![("ids", reference.clone())]), Err(BuildError::UnknownCall));
    assert_eq!(request.add_call_with_references("Foo/get", Map::new(), vec![("#ids", reference)]), Err(BuildError::InvalidArgumentName));
}

// RFC 6901 §3 and RFC 8620 §3.7: the path is a JSON Pointer, with "*" allowed.
#[test]
fn pointer_syntax() {
    let mut request = ApiRequest::new();
    let call = request.add_call("Foo/get", Map::new()).unwrap();
    for good in ["", "/", "/list/*/id", "/a~0b/c~1d"] {
        assert!(request.reference(&call, "Foo/get", good).is_ok(), "{good:?}");
    }
    for bad in ["list", "/a~2", "/a~", &format!("/{}", "a".repeat(MAX_PATH_LEN))] {
        assert_eq!(request.reference(&call, "Foo/get", bad), Err(BuildError::InvalidPath), "{bad:?}");
    }
}

// Requirement 3.3: names are checked, and an empty request is not sent.
#[test]
fn names_and_empty() {
    let mut request = ApiRequest::new();
    assert_eq!(request.to_json(), Err(BuildError::NoCalls));
    assert_eq!(request.add_call("", Map::new()), Err(BuildError::InvalidMethodName));
    assert_eq!(request.add_call("Email get", Map::new()), Err(BuildError::InvalidMethodName));
    assert!(matches!(request.using("bad capability"), Err(BuildError::InvalidCapability)));
    for _ in 0..MAX_CALLS {
        request.add_call("Core/echo", Map::new()).unwrap();
    }
    assert!(matches!(request.add_call("Core/echo", Map::new()), Err(BuildError::TooManyCalls { .. })));
}

// RFC 8620 §3.3: createdIds is sent when set.
#[test]
fn created_ids() {
    let mut request = ApiRequest::new();
    request.add_call("Core/echo", Map::new()).unwrap();
    request.set_created_ids(BTreeMap::from([(Id::new("k1").unwrap(), Id::new("M1").unwrap())]));
    let body: Value = serde_json::from_slice(&request.to_json().unwrap()).unwrap();
    assert_eq!(body["createdIds"], json!({"k1": "M1"}));
}

// Requirement 2.3 spirit: the debug form does not show arguments.
#[test]
fn debug_hides_arguments() {
    let mut request = ApiRequest::new();
    request.add_call("Email/query", args(json!({"filter": {"text": "private words"}}))).unwrap();
    assert!(!format!("{request:?}").contains("private words"));
}
