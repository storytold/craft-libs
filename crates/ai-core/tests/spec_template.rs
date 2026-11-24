//! Substitution rules for declarative specs: what a template may name, and that the structure of
//! a request can never come from the user's text.

use craft_ai_core::spec::template::{Slot, Values, render_text, render_value, slots, value_slots};
use serde_json::json;

fn values() -> Values<'static> {
    Values { prompt: Some("a red door"), model: Some("img-1"), job_id: Some("job-7") }
}

#[test]
fn the_three_names_are_known_and_nothing_else() {
    assert_eq!(slots("{prompt} {model} {job_id}").unwrap(), vec![Slot::Prompt, Slot::Model, Slot::JobId]);
    assert_eq!(slots("no slots here").unwrap(), vec![]);
    for other in ["{seed}", "{param.size}", "{Prompt}", "{}", "{ prompt }"] {
        let why = slots(other).unwrap_err();
        assert!(why.contains("unsupported: placeholder"), "{other}: {why}");
    }
    assert_eq!(slots("{seed}").unwrap_err(), "unsupported: placeholder {seed}");
}

#[test]
fn braces_are_closed_or_doubled() {
    assert!(slots("a { b").unwrap_err().contains("never closed"));
    assert!(slots("a } b").unwrap_err().contains("lone }"));
    assert_eq!(render_text("{{literal}}", &values()).unwrap(), "{literal}");
    assert_eq!(render_text("{{{prompt}}}", &values()).unwrap(), "{a red door}");
}

#[test]
fn text_is_inserted_and_never_read_again() {
    let v = Values { prompt: Some("{job_id} and {{ and }"), ..values() };
    assert_eq!(render_text("say: {prompt}!", &v).unwrap(), "say: {job_id} and {{ and }!");
}

#[test]
fn a_prompt_cannot_add_a_field_or_change_a_key() {
    let template = json!({"model": "{model}", "prompt": "{prompt}", "n": 1});
    let v = Values { prompt: Some(r#"x","admin":true,"y":"#), ..values() };
    let body = render_value(&template, &v).unwrap();
    assert_eq!(body["prompt"], json!(r#"x","admin":true,"y":"#));
    assert_eq!(body.as_object().unwrap().len(), 3);
    assert!(body.get("admin").is_none());
    assert_eq!(body["n"], json!(1));
}

#[test]
fn keys_are_literal_and_nested_values_are_rendered() {
    let template = json!({"a": [{"b": "{model}"}, "{job_id}"], "c": {"d": "{prompt}"}});
    let body = render_value(&template, &values()).unwrap();
    assert_eq!(body, json!({"a": [{"b": "img-1"}, "job-7"], "c": {"d": "a red door"}}));
    assert_eq!(value_slots(&template).unwrap(), vec![Slot::Model, Slot::JobId, Slot::Prompt]);
    assert!(value_slots(&json!({"{prompt}": 1})).unwrap_err().contains("key"));
}

#[test]
fn a_key_is_never_substituted_even_when_it_looks_like_a_slot() {
    let body = render_value(&json!({"{prompt}": "{model}"}), &values()).unwrap();
    assert_eq!(body, json!({"{prompt}": "img-1"}));
}

#[test]
fn a_slot_with_no_value_is_an_error_not_an_empty_string() {
    let v = Values { prompt: Some("x"), ..Values::default() };
    let why = render_text("/jobs/{job_id}", &v).unwrap_err();
    assert!(why.contains("{job_id} is not available"), "{why}");
}

#[test]
fn repeated_slots_share_one_rendered_budget() {
    let prompt = "x".repeat(64 * 1024);
    let v = Values { prompt: Some(&prompt), ..values() };
    let slots = vec![json!("{prompt}"); 16];
    assert!(render_value(&json!({ "a": slots }), &v).is_ok());
    let slots = vec![json!("{prompt}{prompt}{prompt}{prompt}"); 32];
    let why = render_value(&json!({ "a": slots }), &v).unwrap_err();
    assert!(why.contains("at most"), "{why}");
    assert!(render_text(&"{prompt}".repeat(128), &v).is_err());
}
