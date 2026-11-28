//! Integration tests of craft-ai-core: contract regressions.

use craft_ai_core::*;

fn draft(endpoint: &str) -> ConnectionDraft {
    let mut draft = ConnectionDraft::default();
    draft.label = "Test".into();
    draft.endpoint = endpoint.into();
    draft.uses_key = false;
    draft
}

fn models(id: &str) -> Vec<ModelInfo> {
    vec![ModelInfo::new(ModelId::new(id).unwrap(), id)]
}

#[test]
fn an_answer_is_judged_by_its_address_not_by_how_long_it_waited() {
    // Identity here is the address, not the test. After a.example → b.example → a.example, an
    // answer from the first test at a.example is still taken: the field holds a.example and those
    // models do belong to it. What must never happen is b.example's models being filed under
    // a.example, which is checked below.
    let mut draft = draft("https://a.example/v1");
    draft.endpoint = "https://b.example/v1".into();
    assert!(draft.set_models("https://b.example/v1", models("b-only")));
    draft.endpoint = "https://a.example/v1".into();
    assert!(draft.prepare(&[]).unwrap().connection.models.is_empty(), "b.example's models are not saved under a.example");
    assert!(draft.set_models("https://a.example/v1", models("a-only")), "an answer for the address on the form is taken");
    assert_eq!(draft.prepare(&[]).unwrap().connection.models, models("a-only"));
}
