//! Illustrations of optional message-based adapters, not a native host or an FFI contract.
use craft_jsonrpc::{Limits, ParseError, Request, RpcError, parse_json, process};
use serde_json::{Value, json};

const LIMITS: Limits = Limits { max_message_bytes: 256, max_batch_items: 1, integer_ids_only: true, allow_null_ids: false };

fn dispatch(input: &[u8], route: impl FnMut(Request<'_>) -> Result<Value, RpcError>) -> Result<(Option<Value>, Vec<String>), ParseError> {
    let value = parse_json(input, LIMITS)?;
    let mut notifications = Vec::new();
    let response = process(&value, LIMITS, route, |request| notifications.push(request.method.to_owned()))?;
    Ok((response, notifications))
}

fn drawing(request: Request<'_>) -> Result<Value, RpcError> {
    match request.method {
        "drawing/brush" => {
            let size = request.params.and_then(|p| p.get("size")).and_then(Value::as_u64).filter(|n| (1..=100).contains(n));
            let size = size.ok_or_else(|| RpcError::new(-32602, "Brush size must be 1..=100"))?;
            Ok(json!({"size": size}))
        }
        _ => Err(RpcError::new(-32601, "Method not found")),
    }
}

fn spreadsheet(request: Request<'_>) -> Result<Value, RpcError> {
    match request.method {
        "sheet/cell" => {
            let row = request.params.and_then(|p| p.get("row")).and_then(Value::as_u64).filter(|n| *n > 0);
            let row = row.ok_or_else(|| RpcError::new(-32602, "Row must be positive"))?;
            Ok(json!({"row": row, "column": 2}))
        }
        _ => Err(RpcError::new(-32601, "Method not found")),
    }
}

#[test]
fn independent_adapters_keep_routing_and_results_separate() {
    let brush = br#"{"jsonrpc":"2.0","id":1,"method":"drawing/brush","params":{"size":12}}"#;
    let cell = br#"{"jsonrpc":"2.0","id":2,"method":"sheet/cell","params":{"row":3}}"#;
    assert_eq!(dispatch(brush, drawing).unwrap(), (Some(json!({"jsonrpc":"2.0","id":1,"result":{"size":12}})), vec![]));
    assert_eq!(dispatch(cell, spreadsheet).unwrap(), (Some(json!({"jsonrpc":"2.0","id":2,"result":{"row":3,"column":2}})), vec![]));
    assert_eq!(dispatch(brush, spreadsheet).unwrap().0.unwrap()["error"]["code"], -32601);
    assert_eq!(dispatch(cell, drawing).unwrap().0.unwrap()["error"]["code"], -32601);
    let bad_brush = br#"{"jsonrpc":"2.0","id":3,"method":"drawing/brush","params":{"size":0}}"#;
    let bad_cell = br#"{"jsonrpc":"2.0","id":4,"method":"sheet/cell","params":{"row":0}}"#;
    assert_eq!(dispatch(bad_brush, drawing).unwrap().0.unwrap()["error"]["message"], "Brush size must be 1..=100");
    assert_eq!(dispatch(bad_cell, spreadsheet).unwrap().0.unwrap()["error"]["message"], "Row must be positive");
    let notification = br#"{"jsonrpc":"2.0","method":"host/changed"}"#;
    assert_eq!(dispatch(notification, drawing).unwrap(), (None, vec!["host/changed".into()]));
}

#[test]
fn bad_input_and_over_limit_batch_do_not_reach_either_adapter() {
    for route in [drawing as fn(Request<'_>) -> Result<Value, RpcError>, spreadsheet] {
        assert_eq!(dispatch(&vec![b' '; 257], route), Err(ParseError::MessageTooLarge));
        assert_eq!(dispatch(b"{", route), Err(ParseError::InvalidJson));
        assert_eq!(
            dispatch(br#"[{"jsonrpc":"2.0","id":1,"method":"a"},{"jsonrpc":"2.0","id":2,"method":"b"}]"#, |_| {
                panic!("oversized batch must not dispatch")
            }),
            Err(ParseError::BatchTooLarge)
        );
        let invalid = dispatch(br#"{"jsonrpc":"2.0","id":true,"method":"a"}"#, |_| panic!("invalid request must not dispatch")).unwrap().0.unwrap();
        assert_eq!(invalid["error"]["code"], -32600);
    }
}
