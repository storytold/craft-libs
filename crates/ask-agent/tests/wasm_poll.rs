#![allow(clippy::unwrap_used)]

// This binary runs through Node on wasm32-unknown-unknown. Native tests cover native waiting.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown", not(target_feature = "atomics"))))]
fn main() {}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(target_feature = "atomics")))]
fn main() {
    use craft_ask_agent::hub::{AgentHub, DEFAULT_LISTEN, Request, Target};
    use serde_json::json;

    let hub = AgentHub::default();
    assert!(hub.listen(1, None, DEFAULT_LISTEN, || true).unwrap().is_none());
    let id = hub
        .ask(Request {
            id: 0,
            instruction: "Read the selection".into(),
            history: vec![],
            selection: json!({}),
            document: json!({}),
            target: Target { document: "test-document".into(), revision: 1 },
            agent: Some("Codex".into()),
        })
        .unwrap();
    assert!(hub.listen(2, Some("Other"), DEFAULT_LISTEN, || true).unwrap().is_none());
    assert_eq!(hub.listen(3, Some("Codex"), DEFAULT_LISTEN, || true).unwrap().unwrap().id, id);
}
