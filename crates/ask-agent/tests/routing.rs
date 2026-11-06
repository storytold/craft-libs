#![allow(clippy::unwrap_used)]

use std::time::Duration;

use craft_ask_agent::{AgentHub, Request, Target};
use serde_json::json;

#[test]
/// Routing follows an authorized edit's revision and forgets closed requests.
fn routing_follows_authorized_revisions_without_exposing_mutable_state() {
    let hub = AgentHub::default();
    let request: Request = serde_json::from_value(json!({
        "id": 0, "instruction": "Edit", "history": [], "selection": {}, "document": {},
        "target": { "document": "first", "revision": 1 }, "agent": null,
    }))
    .unwrap();
    let original = request.target.clone();
    let id = hub.ask(request).unwrap();
    let mut routing = hub.request_target(id).unwrap();
    routing.document = "untrusted replacement".into();
    assert_eq!(hub.request_target(id), Some(original.clone()));
    hub.listen(1, None, Duration::ZERO, || true).unwrap().unwrap();
    let next = Target { document: "first".into(), revision: 2 };
    hub.advance(1, id, &original, next.clone(), json!({})).unwrap();
    assert_eq!(hub.request_target(id), Some(next));
    hub.close(id);
    assert_eq!(hub.request_target(id), None);
}
