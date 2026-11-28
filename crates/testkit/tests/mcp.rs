#![cfg(not(target_arch = "wasm32"))]

use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_testkit::mcp::{McpChild, McpError, McpLimits, PROTOCOL_VERSION};
use serde_json::{Value, json};

/// An in-process MCP server: answers each line it reads with `reply(method, message)`; `None`
/// sends nothing. A reply from the client has the method `""`. Returns the client side.
fn fake_with<F>(limits: McpLimits, reply: F) -> McpChild
where
    F: Fn(&str, &Value) -> Option<Vec<String>> + Send + 'static,
{
    let (client_r, mut server_w) = std::io::pipe().unwrap();
    let (server_r, client_w) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(server_r).lines() {
            let Ok(line) = line else { return };
            let Ok(req) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let method = req["method"].as_str().unwrap_or("").to_string();
            let Some(out) = reply(&method, &req) else {
                continue;
            };
            for l in out {
                if server_w.write_all(format!("{l}\n").as_bytes()).is_err() {
                    return;
                }
            }
        }
    });
    McpChild::connect(client_r, client_w, limits).unwrap()
}

fn fake<F>(reply: F) -> McpChild
where
    F: Fn(&str, &Value) -> Option<Vec<String>> + Send + 'static,
{
    fake_with(McpLimits::default(), reply)
}

fn result(req: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": req["id"], "result": result}).to_string()
}

/// A deadline for replies that never come.
const SHORT: Duration = Duration::from_millis(200);
/// A deadline that a passing test does not reach.
const LONG: Duration = Duration::from_secs(30);

#[test]
fn handshake_list_and_call() {
    let initialized = Arc::new(AtomicBool::new(false));
    let seen = initialized.clone();
    let mut c = fake(move |m, req| match m {
        "initialize" => Some(vec![result(
            req,
            json!({"protocolVersion": req["params"]["protocolVersion"], "capabilities": {}, "serverInfo": {"name": "fake", "version": "0"}}),
        )]),
        "notifications/initialized" => {
            assert!(req.get("id").is_none(), "a notification has no id");
            seen.store(true, Ordering::SeqCst);
            None
        }
        // The list is sent only after `notifications/initialized`, which the client sends first.
        "tools/list" if seen.load(Ordering::SeqCst) => Some(vec![result(req, json!({"tools": [{"name": "doc.new"}, {"no": "name"}]}))]),
        "tools/call" => Some(vec![result(req, json!({"content": [{"type": "text", "text": req["params"]["name"]}]}))]),
        _ => None,
    });
    let init = c.initialize(PROTOCOL_VERSION).unwrap();
    assert_eq!(init["protocolVersion"], PROTOCOL_VERSION);
    let tools = c.list_tools().unwrap();
    assert_eq!(tools.len(), 2, "the list is returned as the server sent it");
    assert!(initialized.load(Ordering::SeqCst));
    let r = c.call_tool_within("doc.new", json!({}), Duration::from_secs(5)).unwrap();
    assert_eq!(r["content"][0]["text"], "doc.new");
}

#[test]
fn initialize_requires_capabilities_and_server_info() {
    for fields in [
        json!({}),
        json!({"capabilities": []}),
        json!({"capabilities": {}}),
        json!({"capabilities": {}, "serverInfo": []}),
        json!({"capabilities": {}, "serverInfo": {}}),
        json!({"capabilities": {}, "serverInfo": {"name": "fake"}}),
        json!({"capabilities": {}, "serverInfo": {"name": 1, "version": "0"}}),
        json!({"capabilities": {}, "serverInfo": {"name": "fake", "version": 1}}),
    ] {
        let mut c = fake(move |_, req| {
            let mut reply = fields.clone();
            reply["protocolVersion"] = json!(PROTOCOL_VERSION);
            Some(vec![result(req, reply)])
        });
        assert!(matches!(c.initialize(PROTOCOL_VERSION), Err(McpError::Protocol(_))));
        assert!(c.closed().is_some());
    }
}

#[test]
fn server_requests_require_string_or_integer_ids() {
    for id in [Value::Null, json!(1.5), json!(1.0), json!(1e2)] {
        let mut c = fake(move |method, req| match method {
            "x" => Some(vec![json!({"jsonrpc": "2.0", "id": id, "method": "ping"}).to_string(), result(req, json!(true))]),
            _ => None,
        });
        assert!(matches!(c.request("x", Value::Null), Err(McpError::Protocol(_))));
        assert!(c.closed().is_some());
    }
    for id in [json!("server"), json!(-1), json!(u64::MAX)] {
        let expected = id.clone();
        let mut c = fake(move |method, req| match method {
            "x" => Some(vec![json!({"jsonrpc": "2.0", "id": id, "method": "ping"}).to_string()]),
            "" => {
                assert_eq!(req["id"], expected);
                Some(vec![json!({"jsonrpc": "2.0", "id": 1, "result": true}).to_string()])
            }
            _ => None,
        });
        assert_eq!(c.request("x", Value::Null).unwrap(), json!(true));
    }
}

#[test]
fn pages_are_followed() {
    let mut c = fake(|m, req| match m {
        "tools/list" if req["params"]["cursor"].is_null() => Some(vec![result(req, json!({"tools": [{"name": "a"}], "nextCursor": "p2"}))]),
        "tools/list" if req["params"]["cursor"] == "p2" => Some(vec![result(req, json!({"tools": [{"name": "b"}]}))]),
        "tools/list" => panic!("unexpected cursor: {}", req["params"]["cursor"]),
        _ => None,
    });
    let names: Vec<String> = c.list_tools().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, vec!["a", "b"]);
}

#[test]
fn tool_result_envelopes_are_required() {
    for within in [false, true] {
        for malformed in [
            json!(null),
            json!({}),
            json!({"content": true}),
            json!({"content": [], "isError": 1}),
            json!({"content": [], "structuredContent": []}),
            json!({"content": [], "_meta": null}),
        ] {
            let mut client = fake(move |_, req| Some(vec![result(req, malformed.clone())]));
            let reply = if within { client.call_tool_within("probe", json!({}), LONG) } else { client.call_tool("probe", json!({})) };
            assert!(matches!(reply, Err(McpError::Protocol(_))), "{reply:?}");
            assert!(client.closed().is_some());
        }
        for valid in [
            json!({"content": []}),
            json!({"content": [], "isError": true}),
            json!({"content": [], "structuredContent": {"value": 1}, "_meta": {}, "extra": "kept"}),
        ] {
            let sent = valid.clone();
            let mut client = fake(move |_, req| Some(vec![result(req, sent.clone())]));
            let reply = if within { client.call_tool_within("probe", json!({}), LONG) } else { client.call_tool("probe", json!({})) };
            assert_eq!(reply.unwrap(), valid);
        }
    }
}

#[test]
fn server_request_params_are_objects() {
    for params in [json!(true), json!(1), json!("bad"), json!(null), json!([])] {
        let mut client = fake(move |method, req| {
            (method == "probe")
                .then(|| vec![json!({"jsonrpc": "2.0", "id": "server", "method": "ping", "params": params}).to_string(), result(req, json!("done"))])
        });
        assert!(matches!(client.request("probe", json!({})), Err(McpError::Protocol(_))));
    }
    let mut client = fake(|method, req| {
        (method == "probe")
            .then(|| vec![json!({"jsonrpc": "2.0", "id": "server", "method": "ping", "params": {}}).to_string(), result(req, json!("done"))])
    });
    assert_eq!(client.request("probe", json!({})).unwrap(), "done");
}

#[test]
fn notifications_are_bounded_by_total_json_bytes() {
    let notes: Vec<Value> = (0..3)
        .map(|index| json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"index": index, "data": "x".repeat(96)}}))
        .collect();
    let limits = McpLimits { max_notifications: 4, max_line_bytes: notes[0].to_string().len() * 3 / 2, ..McpLimits::default() };
    let sent = notes.clone();
    let mut c = fake_with(limits, move |_, req| {
        let mut lines: Vec<String> = sent.iter().map(Value::to_string).collect();
        lines.push(result(req, json!(true)));
        Some(lines)
    });
    assert_eq!(c.request("x", Value::Null).unwrap(), json!(true));
    assert_eq!(c.take_notifications(), vec![notes[2].clone()]);
    assert!(c.take_notifications().is_empty());
}

#[test]
fn a_notification_larger_than_the_capture_budget_keeps_earlier_messages() {
    let oversized = r#"{"jsonrpc":"2.0","method":"n","params":{"n":1e1}}"#;
    let kept = json!({"jsonrpc": "2.0", "method": "n", "params": {}});
    let limits = McpLimits { max_notifications: 4, max_line_bytes: oversized.len(), ..McpLimits::default() };
    let first = kept.to_string();
    let mut c = fake_with(limits, move |_, req| Some(vec![first.clone(), oversized.to_string(), result(req, json!(true))]));
    assert_eq!(c.request("x", Value::Null).unwrap(), json!(true));
    assert_eq!(c.take_notifications(), vec![kept]);
}

#[test]
fn notifications_log_lines_and_stale_replies_are_skipped() {
    // A log line on stdout is a protocol error (`non_json_stdout_is_protocol_error`); a
    // notification and a stale reply are skipped.
    let mut c = fake(|_, req| {
        Some(vec![
            json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "starting up"}}).to_string(),
            json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {}}).to_string(),
            json!({"jsonrpc": "2.0", "id": 999, "result": "stale"}).to_string(),
            result(req, json!("mine")),
        ])
    });
    assert_eq!(c.request_within("x", json!({}), Duration::from_secs(5)).unwrap(), json!("mine"));
}

#[test]
fn a_request_from_the_app_is_refused_and_the_reply_still_arrives() {
    let refusal: Arc<Mutex<Option<Value>>> = Arc::default();
    let log = refusal.clone();
    let mut c = fake(move |m, req| match m {
        "x" => Some(vec![json!({"jsonrpc": "2.0", "id": "srv-1", "method": "sampling/createMessage"}).to_string()]),
        // The reply to `x` follows the client's answer to the server's request.
        "" => {
            *log.lock().unwrap() = Some(req.clone());
            Some(vec![json!({"jsonrpc": "2.0", "id": 1, "result": 1}).to_string()])
        }
        _ => None,
    });
    assert_eq!(c.request_within("x", json!({}), Duration::from_secs(5)).unwrap(), json!(1));
    let refusal = refusal.lock().unwrap().clone().unwrap();
    assert_eq!(refusal["id"], "srv-1");
    assert_eq!(refusal["error"]["code"], -32601);
    assert!(refusal.get("result").is_none());
}

#[test]
fn an_error_reply_is_an_err_and_the_connection_stays_usable() {
    let mut c = fake(|m, req| match m {
        "bad" => {
            Some(vec![json!({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -1, "message": "nope", "data": {"why": "test"}}}).to_string()])
        }
        _ => Some(vec![result(req, json!(true))]),
    });
    match c.request_within("bad", json!({}), Duration::from_secs(5)) {
        Err(McpError::Rpc { method, code, message, data }) => {
            assert_eq!((method.as_str(), code, message.as_str()), ("bad", -1, "nope"));
            assert_eq!(data, Some(json!({"why": "test"})));
        }
        other => panic!("{other:?}"),
    }
    assert!(c.closed().is_none());
    assert_eq!(c.request_within("ok", json!({}), Duration::from_secs(5)).unwrap(), json!(true));
}

#[test]
fn a_silent_app_times_out_and_the_connection_is_closed() {
    let mut c = fake(|_, _| None);
    let err = c.request_within("x", json!({}), Duration::from_millis(100)).unwrap_err();
    assert!(matches!(&err, McpError::Deadline { method, .. } if method == "x"), "{err}");
    assert!(c.closed().is_some());
    assert!(matches!(c.request_within("y", json!({}), Duration::from_secs(5)), Err(McpError::Closed(_))));
}

#[test]
fn an_app_that_exits_is_reported() {
    let (client_r, server_w) = std::io::pipe().unwrap();
    let (_server_r, client_w) = std::io::pipe().unwrap();
    drop(server_w);
    let mut c = McpChild::connect(client_r, client_w, McpLimits::default()).unwrap();
    let err = c.request_within("x", json!({}), Duration::from_secs(5)).unwrap_err();
    assert!(matches!(err, McpError::Ended), "{err}");
    assert!(c.closed().is_some());
}

#[test]
fn a_missing_cli_is_an_error_not_a_panic() {
    let program = std::env::temp_dir().join("no-such-craft-cli-for-tests");
    let err = McpChild::spawn(&program, ["mcp"], [("CRAFT_TESTKIT_UNUSED", "1")]).unwrap_err();
    assert!(matches!(&err, McpError::Spawn { program: p, .. } if p.as_os_str() == program.as_os_str()), "{err}");
    assert!(err.to_string().contains("could not start"), "{err}");
}

#[test]
fn a_tool_list_that_never_ends_is_an_error_not_a_partial_list() {
    let mut c = fake(|m, req| match m {
        "tools/list" => Some(vec![result(req, json!({"tools": [{"name": "a"}], "nextCursor": "more"}))]),
        _ => None,
    });
    let err = c.list_tools().unwrap_err();
    assert!(matches!(err, McpError::TooManyPages { limit: 64 }), "{err}");
}

#[test]
fn a_tool_list_larger_than_the_limit_is_an_error() {
    // Each page fits in a line; the pages together do not.
    let limits = McpLimits { max_line_bytes: 256, ..McpLimits::default() };
    let mut c = fake_with(limits, |m, req| match m {
        "tools/list" => Some(vec![result(req, json!({"tools": [{"name": "x".repeat(100)}], "nextCursor": "more"}))]),
        _ => None,
    });
    let err = c.list_tools().unwrap_err();
    assert!(matches!(err, McpError::ToolListTooLarge { limit: 256 }), "{err}");
}

#[test]
fn an_app_that_stops_reading_times_out_instead_of_blocking_the_write() {
    // The server side never reads, so the pipe fills and the write blocks the writer thread, not
    // the request.
    let (client_r, _server_w) = std::io::pipe().unwrap();
    let (server_r, client_w) = std::io::pipe().unwrap();
    let mut c = McpChild::connect(client_r, client_w, McpLimits::default()).unwrap();
    let big = "x".repeat(4 << 20);
    let err = c.call_tool_within("t", json!({"data": big}), Duration::from_millis(300)).unwrap_err();
    assert!(matches!(err, McpError::Deadline { .. }), "{err}");
    drop(server_r);
}

#[test]
fn a_tools_changed_notification_is_remembered_once() {
    let limits = McpLimits { max_notifications: 8, ..McpLimits::default() };
    let mut c = fake_with(limits, |m, req| match m {
        "x" => Some(vec![json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}).to_string(), result(req, json!(1))]),
        _ => None,
    });
    assert!(c.take_notifications().is_empty());
    c.request_within("x", json!({}), Duration::from_secs(5)).unwrap();
    let kept = c.take_notifications();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0]["method"], "notifications/tools/list_changed");
    assert!(c.take_notifications().is_empty());
}

#[test]
fn notifications_are_kept_up_to_the_limit_newest_last() {
    let limits = McpLimits { max_notifications: 2, ..McpLimits::default() };
    let mut c = fake_with(limits, |m, req| match m {
        "x" => {
            let mut out: Vec<String> =
                (0..5).map(|n| json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"n": n}}).to_string()).collect();
            out.push(result(req, json!(1)));
            Some(out)
        }
        _ => None,
    });
    c.request_within("x", json!({}), Duration::from_secs(5)).unwrap();
    let kept: Vec<Value> = c.take_notifications().iter().map(|n| n["params"]["n"].clone()).collect();
    assert_eq!(kept, vec![json!(3), json!(4)]);
}

#[test]
fn take_notifications_returns_while_the_server_keeps_writing() {
    let (client_r, mut server_w) = std::io::pipe().unwrap();
    let (_server_r, client_w) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let note = format!("{}\n", json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {}}));
        while server_w.write_all(note.as_bytes()).is_ok() {}
    });
    let limits = McpLimits { max_notifications: 8, ..McpLimits::default() };
    let mut c = McpChild::connect(client_r, client_w, limits).unwrap();
    for _ in 0..3 {
        assert!(c.take_notifications().len() <= 8);
    }
    assert!(c.closed().is_none());
}

#[test]
fn a_failed_write_ends_the_waiting_request_at_once() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("stdin closed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // The app keeps stdout open but takes nothing on stdin. The deadline is an hour, so a write
    // failure that did not end the request would show as a hung test.
    let (client_r, _server_w) = std::io::pipe().unwrap();
    let mut c = McpChild::connect(client_r, Broken, McpLimits::default()).unwrap();
    let err = c.request_within("x", json!({}), Duration::from_secs(3600)).unwrap_err();
    assert!(matches!(&err, McpError::Io { what: "writing to the child", .. }), "{err}");
    assert!(err.to_string().contains("stdin closed"), "{err}");
    assert!(c.closed().is_some());
}

#[test]
fn an_app_ping_gets_an_empty_result_during_a_request_and_between_them() {
    // `x` brings a ping before its reply and one after it. The second is read while `y` waits;
    // the server answers `y` once both pings have their answers, with those answers.
    #[derive(Default)]
    struct State {
        answers: Vec<Value>,
        y: Option<Value>,
    }
    let state: Arc<Mutex<State>> = Arc::default();
    let mut c = fake(move |m, req| {
        let mut s = state.lock().unwrap();
        match m {
            "x" => Some(vec![
                json!({"jsonrpc": "2.0", "id": "p1", "method": "ping"}).to_string(),
                result(req, json!(1)),
                json!({"jsonrpc": "2.0", "id": "p2", "method": "ping"}).to_string(),
            ]),
            "y" => {
                s.y = Some(req.clone());
                None
            }
            "" => {
                s.answers.push(req.clone());
                None
            }
            _ => return None,
        }
        .or_else(|| {
            let ready = s.answers.len() == 2;
            let y = if ready { s.y.take() } else { None }?;
            Some(vec![result(&y, json!(s.answers))])
        })
    });
    assert_eq!(c.request_within("x", json!({}), Duration::from_secs(5)).unwrap(), json!(1));
    let answers = c.request_within("y", json!({}), Duration::from_secs(5)).unwrap();
    for id in ["p1", "p2"] {
        let reply = answers.as_array().unwrap().iter().find(|v| v["id"] == id).unwrap_or_else(|| panic!("no reply to {id}: {answers}"));
        assert_eq!(reply["result"], json!({}), "{reply}");
        assert!(reply.get("error").is_none());
    }
}

#[test]
fn over_long_line_returns_error() {
    let limits = McpLimits { max_line_bytes: 16, ..McpLimits::default() };
    // The line has no newline yet, and the server keeps its end open.
    let (client_r, mut server_w) = std::io::pipe().unwrap();
    let (_server_r, client_w) = std::io::pipe().unwrap();
    server_w.write_all(&[b'x'; 64]).unwrap();
    let mut c = McpChild::connect(client_r, client_w, limits).unwrap();
    let err = c.request_within("x", json!({}), LONG).unwrap_err();
    assert!(matches!(err, McpError::LineTooLong { limit: 16 }), "{err}");
    assert!(c.closed().is_some());
    drop(server_w);
}

#[test]
fn wrong_id_does_not_satisfy_request() {
    let mut c = fake(|_, req| {
        let id = req["id"].as_u64().unwrap();
        Some(vec![
            json!({"jsonrpc": "2.0", "id": id + 1, "result": "other"}).to_string(),
            json!({"jsonrpc": "2.0", "id": id.to_string(), "result": "string id"}).to_string(),
            json!({"jsonrpc": "2.0", "id": null, "result": "null id"}).to_string(),
        ])
    });
    let err = c.request_within("x", json!({}), SHORT).unwrap_err();
    assert!(matches!(err, McpError::Deadline { .. }), "{err}");
}

#[test]
fn a_flood_of_stale_replies_does_not_extend_the_deadline() {
    let (client_r, mut server_w) = std::io::pipe().unwrap();
    let (_server_r, client_w) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let stale = format!("{}\n", json!({"jsonrpc": "2.0", "id": 999, "result": "stale"}));
        while server_w.write_all(stale.as_bytes()).is_ok() {}
    });
    let mut c = McpChild::connect(client_r, client_w, McpLimits::default()).unwrap();
    let err = c.request_within("x", json!({}), SHORT).unwrap_err();
    assert!(matches!(err, McpError::Deadline { .. }), "{err}");
}

#[test]
fn non_json_stdout_is_protocol_error() {
    for line in ["starting up...", r#"{"id": 1, "result": 1}"#, "[]", "\u{fffd}\u{0}"] {
        let line = line.to_string();
        let mut c = fake(move |_, _| Some(vec![line.clone()]));
        let err = c.request_within("x", json!({}), LONG).unwrap_err();
        assert!(matches!(err, McpError::Protocol(_)), "{err}");
        assert!(c.closed().is_some());
    }
    // Bytes that are not UTF-8.
    let (client_r, mut server_w) = std::io::pipe().unwrap();
    let (_server_r, client_w) = std::io::pipe().unwrap();
    server_w.write_all(b"{\"jsonrpc\": \"2.0\", \"method\": \"\xff\"}\n").unwrap();
    let mut c = McpChild::connect(client_r, client_w, McpLimits::default()).unwrap();
    let err = c.request_within("x", json!({}), LONG).unwrap_err();
    assert!(matches!(err, McpError::Protocol(_)), "{err}");
}

#[test]
fn a_blank_stdout_line_is_protocol_error() {
    // The fake ends each line with a newline, so these send "\n" and "  \r\n".
    for blank in ["", "  \r"] {
        let mut c = fake(move |_, req| Some(vec![blank.to_string(), result(req, json!(1))]));
        let err = c.request_within("x", json!({}), LONG).unwrap_err();
        assert!(matches!(err, McpError::Protocol(_)), "{blank:?}: {err}");
        assert!(c.closed().is_some());
        // Without a reply after it, the blank line still ends the request before its deadline.
        let mut c = fake(move |_, _| Some(vec![blank.to_string()]));
        let err = c.request_within("x", json!({}), LONG).unwrap_err();
        assert!(matches!(err, McpError::Protocol(_)), "{blank:?}: {err}");
        assert!(c.closed().is_some());
    }
}

#[test]
fn a_malformed_reply_is_a_protocol_error() {
    for reply in [json!({"result": 1, "error": {"code": 1, "message": "both"}}), json!({}), json!({"error": {"code": "x", "message": "m"}})] {
        let mut c = fake(move |_, req| {
            let mut msg = reply.clone();
            msg["jsonrpc"] = json!("2.0");
            msg["id"] = req["id"].clone();
            Some(vec![msg.to_string()])
        });
        let err = c.request_within("x", json!({}), LONG).unwrap_err();
        assert!(matches!(err, McpError::Protocol(_)), "{err}");
        assert!(c.closed().is_some());
    }
}

#[test]
fn a_message_with_an_invalid_id_is_a_protocol_error() {
    let replies = [
        json!({"jsonrpc": "2.0", "id": true, "result": 1}),
        json!({"jsonrpc": "2.0", "id": [1], "result": 1}),
        json!({"jsonrpc": "2.0", "id": {"n": 1}, "error": {"code": 1, "message": "m"}}),
        json!({"jsonrpc": "2.0", "id": false, "method": "ping"}),
    ];
    for msg in replies {
        let mut c = fake(move |_, _| Some(vec![msg.to_string()]));
        let err = c.request_within("x", json!({}), LONG).unwrap_err();
        assert!(matches!(err, McpError::Protocol(_)), "{err}");
        assert!(c.closed().is_some());
    }
}

#[test]
fn a_malformed_reply_to_another_id_is_a_protocol_error() {
    let replies = [
        json!({"jsonrpc": "2.0", "id": 9999, "result": 1, "error": {"code": 1, "message": "both"}}),
        json!({"jsonrpc": "2.0", "id": "other"}),
        json!({"jsonrpc": "2.0", "id": null, "error": {"code": "x", "message": "m"}}),
    ];
    for msg in replies {
        let mut c = fake(move |_, _| Some(vec![msg.to_string()]));
        let err = c.request_within("x", json!({}), LONG).unwrap_err();
        assert!(matches!(err, McpError::Protocol(_)), "{err}");
        assert!(c.closed().is_some());
    }
}

#[test]
fn a_tool_error_result_is_returned_unchanged() {
    let mut c = fake(|_, req| Some(vec![result(req, json!({"content": [{"type": "text", "text": "no document"}], "isError": true}))]));
    let r = c.call_tool("doc.save", json!({})).unwrap();
    assert_eq!(r["isError"], true);
    assert!(c.closed().is_none());
}

#[test]
fn a_different_protocol_version_is_an_error_and_closes() {
    let mut c = fake(|m, req| match m {
        "initialize" => Some(vec![result(req, json!({"protocolVersion": "2024-11-05"}))]),
        _ => None,
    });
    let err = c.initialize(PROTOCOL_VERSION).unwrap_err();
    assert!(
        matches!(&err, McpError::UnsupportedVersion { requested, returned } if requested == PROTOCOL_VERSION && returned == "2024-11-05"),
        "{err}"
    );
    assert!(c.closed().is_some());
}

#[test]
fn invalid_limits_and_environment_are_setup_errors() {
    let zero_line = McpLimits { max_line_bytes: 0, ..McpLimits::default() };
    let zero_deadline = McpLimits { request_timeout: Duration::ZERO, ..McpLimits::default() };
    let long_deadline = McpLimits { initialize_timeout: Duration::from_secs(3601), ..McpLimits::default() };
    let many_notes = McpLimits { max_notifications: 4097, ..McpLimits::default() };
    for limits in [zero_line, zero_deadline, long_deadline, many_notes] {
        let (client_r, _server_w) = std::io::pipe().unwrap();
        let (_server_r, client_w) = std::io::pipe().unwrap();
        assert!(matches!(McpChild::connect(client_r, client_w, limits.clone()), Err(McpError::Setup(_))), "{limits:?}");
    }
    let program = std::env::temp_dir().join("no-such-craft-cli-for-tests");
    for (key, value) in [("", "1"), ("A=B", "1"), ("A\0B", "1"), ("A", "1\0")] {
        let err = McpChild::spawn(&program, ["mcp"], [(key, value)]).unwrap_err();
        assert!(matches!(err, McpError::Setup(_)), "{key:?}={value:?}: {err}");
    }
    let mut c = fake(|_, _| None);
    assert!(matches!(c.request_within("x", json!({}), Duration::ZERO), Err(McpError::Setup(_))));
    assert!(c.closed().is_none(), "a bad deadline does not close the connection");
}
