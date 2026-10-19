//! How calls may use substitutions and what a flow must send (task u2). Each case feeds an input
//! that only the named rule refuses.

mod spec_support;

use serde_json::{Value, json};
use spec_support::{direct, parse, queue, refused, set};

#[test]
fn only_job_id_may_appear_in_a_path_and_only_where_a_job_exists() {
    refused(set(queue(), "/flow/submit/call/path", json!("/images/{prompt}")), "flow.submit.call.path", "only {job_id} may be used in a path");
    refused(set(queue(), "/flow/submit/call/path", json!("/images/{model}")), "flow.submit.call.path", "only {job_id} may be used in a path");
    refused(set(queue(), "/flow/submit/call/path", json!("/images/{job_id}")), "flow.submit.call", "{job_id} is not available");
    refused(set(direct(), "/flow/call/path", json!("/images/{job_id}")), "flow.call", "{job_id} is not available");
}

#[test]
fn a_placeholder_this_version_lacks_is_refused_by_name() {
    refused(
        set(queue(), "/flow/submit/call/body", json!({"seed": "{seed}", "p": "{prompt}"})),
        "flow.submit.call.body",
        "unsupported: placeholder {seed}",
    );
    refused(
        set(queue(), "/flow/poll/call/query", json!({"id": "{job_id}", "x": "{param.size}"})),
        "flow.poll.call.query",
        "unsupported: placeholder {param.size}",
    );
}

#[test]
fn a_body_key_cannot_be_a_template() {
    refused(set(queue(), "/flow/submit/call/body", json!({"{prompt}": 1, "p": "{prompt}"})), "flow.submit.call.body", "a key cannot hold a brace");
}

#[test]
fn the_polling_and_cancel_calls_must_name_the_job() {
    refused(set(queue(), "/flow/poll/call/query", json!({"id": "fixed"})), "flow.poll.call", "never uses {job_id}");
    refused(set(queue(), "/flow/cancel/path", json!("/jobs")), "flow.cancel", "never uses {job_id}");
    assert!(parse(&set(queue(), "/flow/cancel", Value::Null)).is_ok(), "cancel is optional");
}

#[test]
fn the_prompt_must_be_sent_and_every_model_must_require_it() {
    refused(set(queue(), "/flow/submit/call/body", json!({"n": 1})), "flow", "the prompt is never sent");
    refused(set(queue(), "/models/0/requires", json!([])), "models[0].requires", "needs every model to require a prompt");
}

#[test]
fn a_body_belongs_to_a_post_and_stays_small() {
    refused(set(queue(), "/flow/poll/call/body", json!({"id": "{job_id}"})), "flow.poll.call.body", "only a POST has a body");
    let pad = "x".repeat(4096);
    refused(set(queue(), "/flow/submit/call/body", json!({"p": "{prompt}", "pad": pad})), "flow.submit.call.body", "at most 4 KiB");
    let fits = "x".repeat(4096 - r#"{"p":"{prompt}","pad":""}"#.len());
    assert!(parse(&set(queue(), "/flow/submit/call/body", json!({"p": "{prompt}", "pad": fits}))).is_ok());
}

#[test]
fn query_values_are_few_plain_and_short() {
    let q = |v: Value| set(queue(), "/flow/poll/call/query", v);
    refused(q(json!({"bad name": "{job_id}"})), "flow.poll.call.query", "plain");
    refused(q(json!({"id": "x".repeat(257)})), "flow.poll.call.query", "plain");
    let nine: serde_json::Map<String, Value> = (0..9).map(|i| (format!("k{i}"), json!("{job_id}"))).collect();
    refused(q(Value::Object(nine)), "flow.poll.call.query", "at most 8");
    let eight: serde_json::Map<String, Value> = (0..8).map(|i| (format!("k{i}"), json!("{job_id}"))).collect();
    assert!(parse(&q(Value::Object(eight))).is_ok());
}

#[test]
fn a_literal_brace_is_written_doubled() {
    let body = json!({"p": "{prompt}", "note": "{{not a slot}}"});
    assert!(parse(&set(queue(), "/flow/submit/call/body", body)).is_ok());
    refused(set(queue(), "/flow/submit/call/body", json!({"p": "{prompt}", "note": "{oops"})), "flow.submit.call.body", "never closed");
}
