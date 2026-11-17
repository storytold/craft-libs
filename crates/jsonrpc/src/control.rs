//! Bounded compatibility framing for single-line `{id, method, params}` control requests.
//! This is not JSON-RPC: replies use `{id, ok, result}` or `{id, ok, error}` and every
//! request, including one without an id, receives a reply. Method routing stays with the caller.
use serde_json::{Value, json};
use std::fmt::Display;

/// Process one bounded control line. Blank lines yield no response. Missing `params` becomes null;
/// arbitrary JSON ids are echoed and malformed JSON has a null id, matching the legacy protocol.
pub fn process_line<E: Display>(line: &str, max_bytes: usize, dispatch: impl FnOnce(&str, &Value) -> Result<Value, E>) -> Option<String> {
    if line.len() > max_bytes {
        return Some(error(None, "request exceeds byte limit"));
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => return Some(error(None, &format!("bad request envelope: {e}"))),
    };
    let id = value.get("id").cloned();
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return Some(error(id, "missing `method`"));
    };
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    Some(match dispatch(method, &params) {
        Ok(result) => json!({"id": id, "ok": true, "result": result}).to_string(),
        Err(e) => error(id, &e.to_string()),
    })
}

fn error(id: Option<Value>, message: &str) -> String {
    json!({"id": id, "ok": false, "error": message}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_legacy_responses_and_null_defaults() {
        let call =
            |method: &str, params: &Value| -> Result<Value, &'static str> { if method == "echo" { Ok(params.clone()) } else { Err("unknown") } };
        assert_eq!(process_line("  ", 20, call), None);
        assert_eq!(process_line(r#"{"id":[1],"method":"echo"}"#, 100, call).unwrap(), r#"{"id":[1],"ok":true,"result":null}"#);
        assert_eq!(process_line(r#"{"method":"bad"}"#, 100, call).unwrap(), r#"{"error":"unknown","id":null,"ok":false}"#);
        assert_eq!(process_line(r#"{"method":"echo"}"#, 100, call).unwrap(), r#"{"id":null,"ok":true,"result":null}"#);
        assert!(process_line("{", 100, call).unwrap().contains("bad request envelope:"));
        assert_eq!(process_line(r#"{"id":3}"#, 100, call).unwrap(), r#"{"error":"missing `method`","id":3,"ok":false}"#);
        assert!(process_line("abcdef", 2, call).unwrap().contains("byte limit"));
        let invoked = std::cell::Cell::new(false);
        let _ = process_line("    ", 2, |_, _| -> Result<Value, &'static str> {
            invoked.set(true);
            Ok(Value::Null)
        });
        assert!(!invoked.get());
    }
}
