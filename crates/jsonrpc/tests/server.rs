use craft_jsonrpc::{Limits, ParseError, RpcError, parse_error_response, parse_json, process};
use serde_json::{Value, json};

#[test]
fn batches_preserve_ids_order_and_suppress_notification_replies() {
    let input = json!([
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": "a", "method": "ping"},
        {"jsonrpc": "2.0", "id": 2, "method": "unknown"},
        17,
        {"jsonrpc": "2.0", "id": null, "method": "ping"}
    ]);
    let mut notifications = Vec::new();
    let result = process(
        &input,
        Limits::default(),
        |request| match request.method {
            "ping" => Ok(json!({})),
            _ => Err(RpcError::new(-32601, "Method not found")),
        },
        |request| notifications.push(request.method.to_owned()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(notifications, ["notifications/initialized"]);
    assert_eq!(
        result,
        json!([
            {"jsonrpc":"2.0", "id":"a", "result":{}},
            {"jsonrpc":"2.0", "id":2, "error":{"code":-32601,"message":"Method not found"}},
            {"jsonrpc":"2.0", "id":null, "error":{"code":-32600,"message":"Invalid Request"}},
            {"jsonrpc":"2.0", "id":null, "result":{}}
        ])
    );
}

#[test]
fn notifications_never_receive_responses_including_all_notification_batches() {
    let input = json!([
        {"jsonrpc":"2.0", "method":"tools/call", "params":{"name":"mutating"}},
        {"jsonrpc":"2.0", "method":"notifications/initialized"}
    ]);
    let mut requests = 0;
    let mut notices = 0;
    let result = process(
        &input,
        Limits::default(),
        |_| {
            requests += 1;
            Ok(Value::Null)
        },
        |_| notices += 1,
    )
    .unwrap();
    assert!(result.is_none());
    assert_eq!(requests, 0);
    assert_eq!(notices, 2);
}

#[test]
fn invalid_envelopes_and_params_return_standard_errors() {
    for id in [json!(true), json!([]), json!({})] {
        let input = json!({"jsonrpc":"2.0", "id":id, "method":"ping"});
        let reply = process(&input, Limits::default(), |_| panic!("invalid id dispatched"), |_| panic!("invalid id notified")).unwrap().unwrap();
        assert_eq!(reply["id"], Value::Null);
        assert_eq!(reply["error"]["code"], -32600);
    }
    for input in [json!({}), json!({"jsonrpc":"1.0", "id":1, "method":"ping"}), json!({"jsonrpc":"2.0", "id":1}), json!([])] {
        let reply =
            process(&input, Limits::default(), |_| panic!("invalid request dispatched"), |_| panic!("invalid notification")).unwrap().unwrap();
        assert_eq!(reply["error"]["code"], -32600);
    }
    let input = json!({"jsonrpc":"2.0", "id":4, "method":"ping", "params":17});
    let reply = process(&input, Limits::default(), |_| panic!("invalid params dispatched"), |_| {}).unwrap().unwrap();
    assert_eq!(reply["id"], 4);
    assert_eq!(reply["error"]["code"], -32602);
    let notification = json!({"jsonrpc":"2.0", "method":"notice", "params":17});
    assert!(process(&notification, Limits::default(), |_| panic!("invalid request"), |_| panic!("invalid notification")).unwrap().is_none());
}

#[test]
fn arrays_are_valid_params_and_errors_can_include_data() {
    let input = json!({"jsonrpc":"2.0", "id":-7, "method":"test", "params":[1, 2]});
    let reply = process(
        &input,
        Limits::default(),
        |request| {
            assert_eq!(request.params, Some(&json!([1, 2])));
            let mut error = RpcError::new(-32000, "failure");
            error.data = Some(json!({"why":"example"}));
            Err(error)
        },
        |_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(reply, json!({"jsonrpc":"2.0", "id":-7, "error":{"code":-32000,"message":"failure","data":{"why":"example"}}}));
}

#[test]
fn parse_and_batch_limits_apply_before_dispatch() {
    let limits = Limits { max_message_bytes: 3, max_batch_items: 1, ..Limits::default() };
    assert_eq!(parse_json(b"null", limits), Err(ParseError::MessageTooLarge));
    assert_eq!(parse_json(b"123", limits).unwrap(), json!(123));
    assert_eq!(parse_json(b"{", limits), Err(ParseError::InvalidJson));
    assert_eq!(parse_error_response()["error"]["code"], -32700);
    let input = json!([{"jsonrpc":"2.0", "id":1, "method":"ping"}, {"jsonrpc":"2.0", "method":"notice"}]);
    assert_eq!(
        process(&input, limits, |_| panic!("over-limit batch dispatched"), |_| panic!("over-limit batch notified")),
        Err(ParseError::BatchTooLarge)
    );
    let one = json!([{"jsonrpc":"2.0", "id":1, "method":"ping"}]);
    assert_eq!(process(&one, limits, |_| Ok(json!({})), |_| {}).unwrap().unwrap().as_array().unwrap().len(), 1);
}

#[test]
fn invalid_utf8_and_excessive_nesting_are_rejected() {
    assert_eq!(parse_json(&[0xff], Limits::default()), Err(ParseError::InvalidJson));
    let nested = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    assert_eq!(parse_json(nested.as_bytes(), Limits::default()), Err(ParseError::InvalidJson));
}

#[test]
fn bounded_line_read_handles_chunks_empty_lines_and_eof() {
    use std::io::{BufReader, Cursor};
    let mut reader = BufReader::with_capacity(2, Cursor::new(b"abc\n\nxy"));
    assert_eq!(craft_jsonrpc::read_line(&mut reader, 3).unwrap(), Some(b"abc".to_vec()));
    assert_eq!(craft_jsonrpc::read_line(&mut reader, 3).unwrap(), Some(Vec::new()));
    assert_eq!(craft_jsonrpc::read_line(&mut reader, 3).unwrap(), Some(b"xy".to_vec()));
    assert_eq!(craft_jsonrpc::read_line(&mut reader, 3).unwrap(), None);
    let mut too_long = BufReader::with_capacity(2, Cursor::new(b"abcd\n"));
    assert!(matches!(craft_jsonrpc::read_line(&mut too_long, 3), Err(craft_jsonrpc::LineError::TooLong)));
}

#[test]
fn bounded_line_read_propagates_io_errors() {
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("broken"))
        }
    }
    let mut reader = std::io::BufReader::new(Broken);
    assert!(matches!(craft_jsonrpc::read_line(&mut reader, 1), Err(craft_jsonrpc::LineError::Io(_))));
}

#[test]
fn inspect_single_request_distinguishes_null_ids_and_invalid_envelopes() {
    let request = json!({"jsonrpc":"2.0", "id":null, "method":"ping"});
    let parsed = craft_jsonrpc::request(&request, Limits::default()).unwrap();
    assert_eq!(parsed.id, Some(&Value::Null));
    assert_eq!(parsed.method, "ping");
    let notification = json!({"jsonrpc":"2.0", "method":"ping"});
    assert!(craft_jsonrpc::request(&notification, Limits::default()).unwrap().id.is_none());
    assert!(craft_jsonrpc::request(&json!({"jsonrpc":"2.0", "id":false, "method":"ping"}), Limits::default()).is_none());
    assert!(craft_jsonrpc::request(&json!({"method":"ping"}), Limits::default()).is_none());
}

#[test]
fn numeric_ids_follow_the_selected_protocol_policy() {
    let input = json!({"jsonrpc":"2.0", "id":1.5, "method":"ping"});
    let reply = process(&input, Limits::default(), |_| Ok(json!({})), |_| {}).unwrap().unwrap();
    assert_eq!(reply["id"], json!(1.5));
    let strict = Limits { integer_ids_only: true, ..Limits::default() };
    let reply = process(&input, strict, |_| panic!("fractional ID dispatched"), |_| {}).unwrap().unwrap();
    assert_eq!(reply["id"], Value::Null);
    assert_eq!(reply["error"]["code"], -32600);
    let no_null = Limits { allow_null_ids: false, ..strict };
    let null_id = json!({"jsonrpc":"2.0", "id":null, "method":"ping"});
    let reply = process(&null_id, no_null, |_| panic!("null ID dispatched"), |_| {}).unwrap().unwrap();
    assert_eq!(reply["error"]["code"], -32600);
}
