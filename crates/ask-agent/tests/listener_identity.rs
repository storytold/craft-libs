#![allow(clippy::unwrap_used)]
use craft_ask_agent::{AgentHub, Request, Target};
use serde_json::json;
use std::time::Duration;

fn request(agent: &str) -> Request {
    Request {
        id: 0,
        instruction: "Review synthetic selection".into(),
        history: vec![],
        selection: json!({"page":1}),
        document: json!({"id":"synthetic:1"}),
        agent: Some(agent.into()),
        target: Target { document: "synthetic:1".into(), revision: 0 },
    }
}

#[test]
fn a_native_listener_keeps_its_registered_name_when_later_polls_omit_it() {
    let hub = AgentHub::default();
    assert!(hub.listen(7, Some("editor"), Duration::ZERO, || true).unwrap().is_none());
    let id = hub.ask(request("editor")).unwrap();
    let claimed = hub.listen(7, None, Duration::ZERO, || true).unwrap().unwrap();
    assert_eq!(claimed.id, id);
    assert_eq!(hub.snapshot(id).unwrap().agent.as_deref(), Some("editor"));
}

#[test]
fn valid_renames_are_explicit_and_invalid_names_cannot_replace_an_identity() {
    let hub = AgentHub::default();
    hub.listen(7, None, Duration::ZERO, || true).unwrap();
    assert_eq!(hub.agents(), ["Agent"]);
    hub.listen(7, Some(" editor "), Duration::ZERO, || true).unwrap();
    assert_eq!(hub.agents(), ["editor"]);
    hub.listen(7, Some(" \t "), Duration::ZERO, || true).unwrap();
    assert_eq!(hub.agents(), ["editor"], "blank names act like omitted names");
    hub.listen(7, Some("writer"), Duration::ZERO, || true).unwrap();
    assert_eq!(hub.agents(), ["writer"]);
    let oversized = "x".repeat(257);
    assert!(hub.listen(7, Some(&oversized), Duration::ZERO, || true).is_err());
    assert!(hub.listen(8, Some(&oversized), Duration::ZERO, || true).is_err());
    assert_eq!(hub.agents(), ["writer"], "reject before replacing or registering an identity");
    let id = hub.ask(request("writer")).unwrap();
    assert_eq!(hub.listen(7, None, Duration::ZERO, || true).unwrap().unwrap().id, id);
    hub.listen(9, Some(" "), Duration::ZERO, || true).unwrap();
    assert!(hub.agents().contains(&"Agent".into()), "a fresh unnamed connection has the default identity");
}
