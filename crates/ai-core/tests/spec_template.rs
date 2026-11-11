//! Substitution rules for declarative specs: what a template may name, and that the structure of
//! a request can never come from the user's text.

use craft_ai_core::spec::template::{Values, render_text, render_value};
use serde_json::json;

fn values() -> Values<'static> {
    Values { prompt: Some("a red door"), model: Some("img-1"), job_id: Some("job-7") }
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
