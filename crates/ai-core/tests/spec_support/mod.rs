//! Fixtures for the spec tests: the two documented shapes and small edit helpers.
#![allow(dead_code)]

use craft_ai_core::spec::{Spec, SpecError};
use serde_json::{Value, json};

pub fn queue() -> Value {
    json!({
        "spec_version": 1, "id": "example-images", "label": "Example Images",
        "base": "https://api.example.com/v2",
        "auth": {"kind": "header", "name": "x-api-key"},
        "probe": {"method": "GET", "path": "/account"},
        "models": [{"id": "img-1", "label": "Image 1", "capability": "image.generate",
                    "accepts": {"prompt": 1}, "requires": ["prompt"]}],
        "flow": {"kind": "queue",
            "submit": {"call": {"method": "POST", "path": "/images",
                                "body": {"prompt": "{prompt}", "n": 1}}, "job_id_at": "/id"},
            "poll": {"call": {"method": "GET", "path": "/result", "query": {"id": "{job_id}"}},
                     "status_at": "/status",
                     "states": {"Pending": "queued", "Working": "running",
                                "Ready": "succeeded", "Error": "failed"},
                     "result": {"at": "/result/sample", "as": "url", "media_type": "image/png"}},
            "cancel": {"method": "DELETE", "path": "/jobs/{job_id}"}}
    })
}

pub fn direct() -> Value {
    json!({
        "spec_version": 1, "id": "plain-images", "label": "Plain Images",
        "base": "http://127.0.0.1:8080",
        "models": [{"id": "m", "label": "M", "capability": "image.generate",
                    "accepts": {"prompt": 1}, "requires": ["prompt"]}],
        "flow": {"kind": "request_response",
            "call": {"method": "POST", "path": "/images",
                     "body": {"model": "{model}", "prompt": "{prompt}"}},
            "result": {"at": "/data/0/b64_json", "as": "base64", "media_type": "image/png"}}
    })
}

/// Replaces the value at `pointer`, or adds it when the parent object lacks that key.
pub fn set(mut v: Value, pointer: &str, to: Value) -> Value {
    let (parent, key) = pointer.rsplit_once('/').unwrap_or(("", pointer));
    let slot = v.pointer_mut(parent).unwrap_or_else(|| panic!("no {parent}"));
    match slot {
        Value::Object(map) => {
            map.insert(key.to_owned(), to);
        }
        Value::Array(items) => items[key.parse::<usize>().unwrap()] = to,
        _ => panic!("{parent} holds no fields"),
    }
    v
}

pub fn parse(v: &Value) -> Result<Spec, SpecError> {
    Spec::parse(&v.to_string())
}

/// The spec is refused, the error points at `at` and says `why`.
pub fn refused(v: Value, at: &str, why: &str) {
    let e = parse(&v).expect_err(&format!("accepted, wanted {at}: {why}"));
    assert!(e.at.contains(at), "pointed at {:?}, wanted {at:?} ({e})", e.at);
    assert!(e.why.contains(why), "said {:?}, wanted {why:?} ({e})", e.why);
}
