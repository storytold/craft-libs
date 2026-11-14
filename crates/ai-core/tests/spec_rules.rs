//! Rules a declarative spec must keep (task u2). Each case feeds an input that only the named rule
//! refuses, and checks where the error points and what it says.

mod spec_support;

use craft_ai_core::spec::{MAX_SPEC_BYTES, Spec};
use serde_json::{Value, json};
use spec_support::{direct, parse, queue, refused, set};

#[test]
fn both_documented_shapes_are_accepted_and_round_trip() {
    for v in [queue(), direct()] {
        let spec = parse(&v).unwrap();
        assert_eq!(Spec::parse(&serde_json::to_string(&spec).unwrap()).unwrap(), spec);
    }
}

#[test]
fn the_file_size_limit_holds_at_its_edge() {
    let text = queue().to_string();
    let pad = MAX_SPEC_BYTES - text.len();
    assert!(Spec::parse(&format!("{text}{}", " ".repeat(pad))).is_ok());
    let e = Spec::parse(&format!("{text}{}", " ".repeat(pad + 1))).unwrap_err();
    assert!(e.why.contains("too large"), "{e}");
}

#[test]
fn a_feature_this_version_lacks_is_refused_by_name_never_ignored() {
    refused(set(queue(), "/flow/submit", json!({"webhook": "x"})), "spec", "webhook");
    let mut v = queue();
    v["streaming"] = json!(true);
    refused(v, "spec", "streaming");
    let mut v = queue();
    v["models"][0]["params"] = json!(["seed"]);
    refused(v, "spec", "params");
}

#[test]
fn only_version_one_is_read() {
    refused(set(queue(), "/spec_version", json!(2)), "spec_version", "only version 1");
}

#[test]
fn the_base_address_follows_the_connection_rules() {
    refused(set(queue(), "/base", json!("http://api.example.com")), "base", "plain http");
    refused(set(queue(), "/base", json!("https://u:p@api.example.com")), "base", "user-info");
    refused(set(queue(), "/base", json!("https://api.example.com/v2?x=1")), "base", "no query");
    refused(set(queue(), "/base", json!("https://api.example.com/v2#top")), "base", "no query");
    refused(set(queue(), "/base", json!("ftp://api.example.com")), "base", "http");
    refused(set(queue(), "/base", json!("https://api.example.com/v2/../x")), "base", "'..'");
    assert!(parse(&set(queue(), "/base", json!("https://api.example.com"))).is_ok());
}

#[test]
fn a_path_cannot_leave_the_base() {
    let path = |p: &str| set(queue(), "/flow/submit/call/path", json!(p));
    let at = "flow.submit.call.path";
    refused(path("images"), at, "starts with /");
    refused(path("https://evil.example/x"), at, "another host");
    refused(path("//evil.example/x"), at, "another host");
    refused(path("/a/../b"), at, "'..'");
    refused(path("/.."), at, "'..'");
    refused(path("/a//b"), at, "'//'");
    refused(path("/a?x=1"), at, "no ? or #");
    refused(path("/a#x"), at, "no ? or #");
    refused(path("/a%2e%2e/b"), at, "no backslash or % escape");
    refused(path("/a\\b"), at, "no backslash or % escape");
    refused(path("/a b"), at, "plain visible");
    refused(path("/a\nb"), at, "plain visible");
    refused(path(&format!("/{}", "a".repeat(256))), at, "too long");
    assert!(parse(&path(&format!("/{}", "a".repeat(255)))).is_ok());
}

#[test]
fn the_key_header_follows_the_connection_rules() {
    let name = |n: &str| set(queue(), "/auth/name", json!(n));
    refused(name("authorization"), "auth", "Authorization");
    refused(name("host"), "auth", "cannot carry a key");
    refused(name("proxy-x"), "auth", "network");
    refused(name("bad name"), "auth", "letters, digits");
    refused(name(""), "auth", "1 to 64");
    assert!(parse(&set(queue(), "/auth", json!({"kind": "bearer"}))).is_ok());
    assert!(parse(&set(queue(), "/auth", json!({"kind": "none"}))).is_ok());
}

#[test]
fn a_model_is_declared_completely_or_not_at_all() {
    let m = |field: &str, to: Value| set(queue(), &format!("/models/0/{field}"), to);
    refused(m("id", json!("bad id")), "models[0].id", "");
    refused(m("capability", json!("nope")), "models[0].capability", "");
    refused(m("label", json!(" ")), "models[0].label", "label");
    refused(m("accepts", json!({})), "models[0].accepts", "at least one");
    refused(m("accepts", json!({"prompt": 0})), "models[0].accepts", "must be 1");
    refused(m("accepts", json!({"prompt": 2})), "models[0].accepts", "must be 1");
    refused(m("accepts", json!({"source": 1})), "models[0].accepts", "unsupported: input 'source'");
    refused(m("requires", json!(["source"])), "models[0].requires", "must also be accepted");
    refused(m("max_outputs", json!(0)), "models[0].max_outputs", "1 to 4");
    refused(m("max_outputs", json!(5)), "models[0].max_outputs", "1 to 4");
    refused(set(queue(), "/models", json!([])), "models", "at least one");
    let one = queue()["models"][0].clone();
    refused(set(queue(), "/models", json!([one.clone(), one.clone()])), "models[1].id", "twice");
    let many: Vec<Value> = (0..17).map(|i| set(one.clone(), "/id", json!(format!("m{i}")))).collect();
    refused(set(queue(), "/models", json!(many)), "models", "at most 16");
    let sixteen: Vec<Value> = (0..16).map(|i| set(one.clone(), "/id", json!(format!("m{i}")))).collect();
    assert!(parse(&set(queue(), "/models", json!(sixteen))).is_ok());
}

#[test]
fn the_status_table_maps_words_to_the_four_states() {
    let states = |v: Value| set(queue(), "/flow/poll/states", v);
    let at = "flow.poll.states";
    refused(states(json!({"Error": "failed"})), at, "succeeded");
    refused(states(json!({"Ready": "succeeded"})), at, "failed");
    refused(states(json!({})), at, "succeeded");
    let table = |running: usize| {
        let mut t = serde_json::Map::new();
        for i in 0..running {
            t.insert(format!("w{i}"), json!("running"));
        }
        t.insert("ok".into(), json!("succeeded"));
        t.insert("bad".into(), json!("failed"));
        Value::Object(t)
    };
    assert!(parse(&states(table(14))).is_ok(), "16 entries are allowed");
    refused(states(table(15)), at, "at most 16");
}

#[test]
fn a_pointer_is_a_bounded_rfc_6901_path() {
    let at = |p: &str| set(queue(), "/flow/poll/result/at", json!(p));
    refused(at("result/sample"), "flow.poll.result.at", "starts with /");
    refused(at("/a/b/c/d/e/f/g/h/i"), "flow.poll.result.at", "8 levels");
    assert!(parse(&at("/a/b/c/d/e/f/g/h")).is_ok());
    refused(set(queue(), "/flow/submit/job_id_at", json!("id")), "flow.submit.job_id_at", "starts with /");
    refused(set(queue(), "/flow/poll/status_at", json!("")), "flow.poll.status_at", "starts with /");
}

#[test]
fn a_result_says_what_it_is() {
    let ex = |v: Value| set(queue(), "/flow/poll/result", v);
    refused(ex(json!({"at": "/x", "as": "url"})), "flow.poll.result", "media_type");
    refused(ex(json!({"at": "/x", "as": "base64"})), "flow.poll.result", "media_type");
    refused(ex(json!({"at": "/x", "as": "text", "media_type": "text/plain"})), "flow.poll.result", "text");
    refused(ex(json!({"at": "/x", "as": "url", "media_type": "not a type"})), "flow.poll.result", "media_type");
    assert!(parse(&ex(json!({"at": "/x", "as": "text"}))).is_ok());
}

#[test]
fn limits_are_bounded() {
    let l = |field: &str, to: Value| {
        let mut v = queue();
        v["limits"] = json!({ field: to });
        v
    };
    refused(l("max_polls", json!(0)), "limits.max_polls", "1 to 10000");
    refused(l("max_polls", json!(10001)), "limits.max_polls", "1 to 10000");
    refused(l("max_seconds", json!(0)), "limits.max_seconds", "1 to 3600");
    refused(l("max_seconds", json!(3601)), "limits.max_seconds", "1 to 3600");
    assert!(parse(&l("max_polls", json!(10000))).is_ok());
}

#[test]
fn the_probe_is_a_free_read() {
    refused(set(queue(), "/probe/method", json!("POST")), "probe", "GET");
    refused(set(queue(), "/probe/body", json!({"a": 1})), "probe", "only a POST has a body");
    refused(set(queue(), "/probe/path", json!("/a/../b")), "probe.path", "'..'");
}
