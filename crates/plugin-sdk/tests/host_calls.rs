//! The typed host helpers against a fake host that follows ABI.md section 4: it records each
//! call and answers with a status and a JSON body, honoring the output capacity.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use craft_plugin_abi::ops::pack_call_return;
use craft_plugin_abi::{Op, Status};
use craft_plugin_sdk::host::Level;
use craft_plugin_sdk::testing::{reply, status, with_host};
use craft_plugin_sdk::{HostError, OpError, Value, clock, cmd, doc, host, json, link, random, state};

type Calls = Rc<RefCell<Vec<(u32, Vec<u8>, usize)>>>;

/// A fake host: records `(op, request bytes, out capacity)`, then runs `answer`.
fn recording(answer: impl Fn(Op, &Value, &mut [u8]) -> i64 + 'static) -> (impl FnMut(u32, &[u8], &mut [u8]) -> i64, Calls) {
    let calls: Calls = Rc::default();
    let log = calls.clone();
    let f = move |op: u32, input: &[u8], out: &mut [u8]| {
        log.borrow_mut().push((op, input.to_vec(), out.len()));
        let request: Value = if input.is_empty() { Value::Null } else { serde_json::from_slice(input).unwrap() };
        answer(Op::from_number(op).expect("known op"), &request, out)
    };
    (f, calls)
}

#[test]
fn without_a_host_every_helper_says_no_host() {
    assert_eq!(host::cancelled(), Err(HostError::NoHost));
    assert_eq!(state::get("k"), Err(HostError::NoHost));
}

#[test]
fn log_sends_op_1_with_level_and_message() {
    let (f, calls) = recording(|_, _, out| reply(out, &json!({})));
    with_host(f, || host::log(Level::Warn, "careful").unwrap());
    let calls = calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, 0x0001);
    assert_eq!(serde_json::from_slice::<Value>(&calls[0].1).unwrap(), json!({ "level": "warn", "msg": "careful" }));
}

#[test]
fn a_request_with_no_body_sends_empty_input() {
    let (f, calls) = recording(|_, _, out| reply(out, &json!({ "cancelled": true })));
    assert_eq!(with_host(f, host::cancelled), Ok(true));
    assert_eq!(calls.borrow()[0].1, Vec::<u8>::new(), "ABI.md section 4: empty input is allowed, and the SDK does not send the text null");
}

#[test]
fn e_too_small_retries_once_with_the_size_the_host_asked_for() {
    let big = "x".repeat(10_000);
    let body = json!({ "found": true, "value": big });
    let (f, calls) = recording(move |_, _, out| reply(out, &body));
    let got = with_host(f, || state::get("k")).unwrap();
    assert_eq!(got, Some(Value::String(big)));
    let calls = calls.borrow();
    assert_eq!(calls.len(), 2, "first call fails with E_TOO_SMALL, the retry succeeds");
    assert_eq!(calls[0].2, 4096);
    assert!(calls[1].2 > 10_000, "the retry buffer holds the whole reply: {}", calls[1].2);
    assert_eq!(calls[0].1, calls[1].1, "the retry repeats the same request");
}

#[test]
fn a_host_that_keeps_saying_too_small_stops_after_one_retry() {
    let (f, calls) = recording(|_, _, _| pack_call_return(Status::TooSmall, 1_000_000));
    let err = with_host(f, || state::get("k")).unwrap_err();
    assert_eq!(err, HostError::TooSmall { required: 1_000_000 });
    assert_eq!(calls.borrow().len(), 2);
}

#[test]
fn an_absurd_size_request_is_refused_without_allocating_it() {
    let (f, calls) = recording(|_, _, _| pack_call_return(Status::TooSmall, u32::MAX));
    let err = with_host(f, || state::get("k")).unwrap_err();
    assert!(matches!(err, HostError::TooSmall { .. }));
    assert_eq!(calls.borrow().len(), 1, "no retry for a 4 GiB buffer");
}

#[test]
fn every_status_code_maps_to_its_error() {
    for (s, want) in [
        (Status::Denied, HostError::Denied),
        (Status::Unsupported, HostError::Unsupported),
        (Status::BadArg, HostError::BadArg),
        (Status::Cancelled, HostError::Cancelled),
        (Status::Limit, HostError::Limit),
    ] {
        let (f, _) = recording(move |_, _, _| status(s));
        assert_eq!(with_host(f, host::cancelled), Err(want), "{s:?}");
    }
    let (f, _) = recording(|_, _, _| 0x0000_0000_FFFF_FF9Cu32 as i64 & 0xFFFF_FFFF);
    assert!(matches!(with_host(f, host::cancelled), Err(HostError::Status(_))), "an unknown code survives as Status");
}

#[test]
fn a_reply_of_the_wrong_shape_is_a_bad_reply_not_a_default() {
    let (f, _) = recording(|_, _, out| reply(out, &json!({ "cancelled": "yes" })));
    assert!(matches!(with_host(f, host::cancelled), Err(HostError::BadReply(_))));
}

#[test]
fn host_info_parses_the_documented_reply() {
    let (f, _) = recording(|_, _, out| {
        reply(
            out,
            &json!({ "abi": 1, "app": "effectcraft", "version": "9", "engine": "wasmi", "plugin": { "id": "p", "generation": 3 }, "ops": ["host.log"] }),
        )
    });
    let info = with_host(f, host::host_info).unwrap();
    assert_eq!((info.app.as_str(), info.plugin.generation, info.ops), ("effectcraft", 3, vec!["host.log".to_owned()]));
}

#[test]
fn progress_sends_fraction_and_optional_message_and_surfaces_cancel() {
    let (f, calls) = recording(|_, _, _| status(Status::Cancelled));
    assert_eq!(with_host(f, || host::progress(0.25, Some("working"))), Err(HostError::Cancelled));
    let sent: Value = serde_json::from_slice(&calls.borrow()[0].1).unwrap();
    assert_eq!(sent, json!({ "fraction": 0.25, "message": "working" }));
}

#[test]
fn cmd_execute_returns_result_or_the_commands_own_error() {
    let (f, calls) = recording(|_, req, out| match req["id"].as_str() {
        Some("app.ok") => reply(out, &json!({ "result": { "n": 2 } })),
        Some("app.bad") => reply(out, &json!({ "error": "no layer" })),
        _ => status(Status::BadArg),
    });
    with_host(f, || {
        assert_eq!(cmd::execute("app.ok", &json!({ "a": 1 })).unwrap(), json!({ "n": 2 }));
        assert_eq!(cmd::execute("app.bad", &json!({})), Err(OpError::Failed("no layer".into())));
        assert_eq!(cmd::execute("app.unknown", &json!({})), Err(OpError::Host(HostError::BadArg)));
    });
    let first: Value = serde_json::from_slice(&calls.borrow()[0].1).unwrap();
    assert_eq!(first, json!({ "id": "app.ok", "params": { "a": 1 } }));
    assert_eq!(calls.borrow()[0].0, 0x0011);
}

#[test]
fn cmd_list_reads_commands() {
    let (f, _) = recording(|_, _, out| reply(out, &json!({ "commands": [{ "id": "a.b", "label": "AB" }] })));
    let rows = with_host(f, cmd::list).unwrap();
    assert_eq!((rows[0].id.as_str(), rows[0].label.as_str()), ("a.b", "AB"));
}

#[test]
fn state_put_then_get_round_trips_and_missing_is_none() {
    let store = Rc::new(RefCell::new(std::collections::BTreeMap::<String, Value>::new()));
    let s = store.clone();
    let (f, _) = recording(move |op, req, out| {
        let key = req["key"].as_str().unwrap().to_owned();
        match op {
            Op::StatePut => {
                s.borrow_mut().insert(key, req["value"].clone());
                reply(out, &json!({}))
            }
            Op::StateGet => match s.borrow().get(&key) {
                Some(v) => reply(out, &json!({ "found": true, "value": v })),
                None => reply(out, &json!({ "found": false })),
            },
            _ => status(Status::BadArg),
        }
    });
    with_host(f, || {
        assert_eq!(state::get("n").unwrap(), None);
        state::put("n", &41u32).unwrap();
        assert_eq!(state::get_as::<u32>("n").unwrap(), Some(41));
        assert!(matches!(state::get_as::<String>("n"), Err(HostError::BadReply(_))), "a wrong type is an error, not None");
    });
}

#[test]
fn doc_query_clock_and_random() {
    let (f, calls) = recording(|op, req, out| match op {
        Op::DocQuery => reply(out, &json!({ "layers": req["scope"] })),
        Op::TimeNow => reply(out, &json!({ "unix_ms": 1_700_000_000_123u64 })),
        Op::RandomBytes => reply(out, &json!({ "hex": "00ff10" })),
        _ => status(Status::BadArg),
    });
    with_host(f, || {
        assert_eq!(doc::query(&json!({ "scope": "metadata" })).unwrap(), json!({ "layers": "metadata" }));
        assert_eq!(clock::now_ms().unwrap(), 1_700_000_000_123);
        assert_eq!(random::bytes(3).unwrap(), vec![0x00, 0xFF, 0x10]);
    });
    let ops: Vec<u32> = calls.borrow().iter().map(|c| c.0).collect();
    assert_eq!(ops, vec![0x0020, 0x0080, 0x0090]);
}

#[test]
fn random_rejects_bad_hex() {
    for bad in ["0", "zz", "0g"] {
        let (f, _) = recording(move |_, _, out| reply(out, &json!({ "hex": bad })));
        assert!(matches!(with_host(f, || random::bytes(1)), Err(HostError::BadReply(_))), "{bad}");
    }
}

#[test]
fn link_call_uses_the_hosts_request_shape_and_reads_apps() {
    let (f, calls) = recording(|op, req, out| match op {
        Op::LinkCall if req["app"] == "effectcraft" => reply(out, &json!({ "result": { "ok": true } })),
        Op::LinkCall => reply(out, &json!({ "error": "missing permission link.call:filmcraft" })),
        Op::LinkApps => reply(out, &json!({ "apps": [{ "app": "effectcraft", "app_version": "1.0" }] })),
        _ => status(Status::BadArg),
    });
    with_host(f, || {
        assert_eq!(link::call("effectcraft", "engine.execute", &json!({ "command": "x" })).unwrap(), json!({ "ok": true }));
        assert_eq!(link::call("filmcraft", "m", &json!({})), Err(OpError::Failed("missing permission link.call:filmcraft".into())));
        assert_eq!(link::apps().unwrap()[0].app, "effectcraft");
    });
    let sent: Value = serde_json::from_slice(&calls.borrow()[0].1).unwrap();
    assert_eq!(sent, json!({ "app": "effectcraft", "method": "engine.execute", "params": { "command": "x" } }));
}

#[test]
fn the_fake_host_is_removed_after_the_closure_even_on_panic() {
    let (f, _) = recording(|_, _, out| reply(out, &json!({ "cancelled": false })));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| with_host(f, || panic!("boom"))));
    assert!(result.is_err());
    assert_eq!(host::cancelled(), Err(HostError::NoHost));
}
