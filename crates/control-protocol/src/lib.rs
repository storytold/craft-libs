//! Authenticated JSON-lines request framing and dispatch for local control services.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use subtle::ConstantTimeEq;

/// Whether a session token has the control protocol's 64 hexadecimal character form.
pub fn valid_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Encode one client request, including its terminating newline.
pub fn request_line(id: u64, token: Option<&str>, method: &str, params: &Value) -> String {
    format!("{}\n", json!({"id": id, "token": token, "method": method, "params": params}))
}

/// Extract the result or application error from a parsed control reply.
pub fn reply_result(reply: &Value) -> Result<Value, String> {
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(reply.get("error").and_then(Value::as_str).unwrap_or("error").to_owned())
    }
}

/// One parsed line.
enum Line {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Blank,
    /// Not a request: reply with this error and close the connection.
    Reject(String),
}

fn parse_line(bytes: &[u8], token: &str) -> Line {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Line::Reject("not a control request (invalid UTF-8); closing the connection".into());
    };
    let text = text.trim();
    if text.is_empty() {
        return Line::Blank;
    }
    let msg = match serde_json::from_str::<Value>(text) {
        Ok(v) => v,
        Err(e) => return Line::Reject(format!("bad JSON ({e}); expected one JSON request object per line; closing the connection")),
    };
    let Some(obj) = msg.as_object() else {
        return Line::Reject("not a control request (expected a JSON object); closing the connection".into());
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Line::Reject("not a control request (missing string `method`); closing the connection".into());
    };
    // Authenticate every request before constructing or enqueueing a ControlRequest. A valid
    // earlier request must not authorize a later one with a missing or incorrect credential.
    let authenticated = obj.get("token").and_then(Value::as_str).is_some_and(|supplied| bool::from(supplied.as_bytes().ct_eq(token.as_bytes())));
    if !authenticated {
        return Line::Reject("control authentication failed; closing the connection".into());
    }
    Line::Request {
        id: obj.get("id").cloned().unwrap_or(Value::Null),
        method: method.to_string(),
        params: obj.get("params").cloned().unwrap_or(json!({})),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadFailure {
    Timeout,
    Rejected(String),
}

/// Read one `\n`-terminated line of at most `max` bytes into `buf` (newline stripped).
/// `Ok(false)` at end of stream; `Err` when the line is too long or the read fails.
pub fn read_bounded_line(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> Result<bool, ReadFailure> {
    buf.clear();
    // `take` bounds the bytes buffered for one line: a peer can't make us allocate more than `max`.
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let n = reader.take(limit).read_until(b'\n', buf).map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => ReadFailure::Timeout,
        _ => ReadFailure::Rejected(e.to_string()),
    })?;
    if n == 0 {
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        return Ok(true);
    }
    if buf.len() > max {
        return Err(ReadFailure::Rejected(format!("request line longer than {max} bytes; closing the connection")));
    }
    // Last line without a newline, then end of stream.
    Ok(true)
}

/// Serve requests from `read`, answering on `out`, until the peer closes, sends something that is
/// not a request, or `handle` returns `None` (app gone). `handle` gets `(method, params)` and
/// returns the reply object (the request's `id` is added here).
pub fn serve_lines<R: Read, W: Write>(read: R, mut out: W, token: &str, max_line: usize, mut handle: impl FnMut(String, Value) -> Option<Value>) {
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    loop {
        let reject = match read_bounded_line(&mut reader, &mut buf, max_line) {
            Ok(false) => return,
            // Expiration is not a request: a pooled client must see EOF and reconnect, rather
            // than read a stale error reply. A partial line must not be parsed on timeout.
            Err(ReadFailure::Timeout) => return,
            Err(ReadFailure::Rejected(e)) => e,
            Ok(true) => match parse_line(&buf, token) {
                Line::Blank => continue,
                Line::Reject(e) => e,
                Line::Request { id, method, params } => {
                    let Some(mut r) = handle(method, params) else { return };
                    if let Some(o) = r.as_object_mut() {
                        o.insert("id".into(), id);
                    }
                    if writeln!(out, "{r}").and_then(|()| out.flush()).is_err() {
                        return;
                    }
                    continue;
                }
            },
        };
        let _ = writeln!(out, "{}", json!({"ok": false, "error": reject})).and_then(|()| out.flush());
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_request_shapes_without_dispatching() {
        for input in [&b"\xff\n"[..], &b"[]\n"[..], &b"{}\n"[..]] {
            let mut out = Vec::new();
            serve_lines(input, &mut out, "secret", 1024, |_, _| panic!("must not dispatch"));
            let reply: Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(reply["ok"], false);
            assert!(reply["error"].as_str().unwrap().contains("closing the connection"));
        }
    }
    #[test]
    fn timeout_after_partial_line_never_dispatches() {
        struct Partial(bool);
        impl Read for Partial {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if !self.0 {
                    self.0 = true;
                    buf[..2].copy_from_slice(b"{\"");
                    return Ok(2);
                }
                Err(std::io::ErrorKind::TimedOut.into())
            }
        }
        let mut out = Vec::new();
        serve_lines(Partial(false), &mut out, "secret", 1024, |_, _| panic!("must not dispatch"));
        assert!(out.is_empty());
    }
    #[test]
    fn line_boundaries_and_closed_dispatch_are_preserved() {
        let mut buf = Vec::new();
        assert_eq!(read_bounded_line(&mut &b"abc\r\n"[..], &mut buf, 4), Ok(true));
        assert_eq!(buf, b"abc");
        assert_eq!(read_bounded_line(&mut &b"abcd\n"[..], &mut buf, 4), Ok(true));
        assert_eq!(buf, b"abcd");
        let mut out = Vec::new();
        serve_lines(&b"{\"method\":\"x\",\"token\":\"s\"}\n"[..], &mut out, "s", 1024, |_, _| None);
        assert!(out.is_empty());
    }
    #[test]
    fn output_failure_stops_following_requests() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let input = b"{\"method\":\"x\",\"token\":\"s\"}\n{\"method\":\"y\",\"token\":\"s\"}\n";
        let mut calls = 0;
        serve_lines(&input[..], Closed, "s", 1024, |_, _| {
            calls += 1;
            Some(json!({"ok": true}))
        });
        assert_eq!(calls, 1);
    }
    #[test]
    fn client_request_and_reply_keep_optional_fields() {
        let line = request_line(7, None, "status", &json!({"a": 1}));
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), json!({"id": 7, "token": null, "method": "status", "params": {"a": 1}}));
        assert_eq!(reply_result(&json!({"ok": true})).unwrap(), Value::Null);
        assert_eq!(reply_result(&json!({"ok": false, "error": "busy"})).unwrap_err(), "busy");
    }
    #[test]
    fn validates_session_token_shape() {
        assert!(valid_token(&"aB09".repeat(16)));
        assert!(!valid_token(&"a".repeat(63)));
        assert!(!valid_token(&"g".repeat(64)));
    }
    #[test]
    fn authenticates_each_request_and_closes_after_failure() {
        let input =
            b"{\"id\":1,\"method\":\"first\",\"token\":\"secret\"}\n{\"id\":2,\"method\":\"second\"}\n{\"method\":\"third\",\"token\":\"secret\"}\n";
        let mut output = Vec::new();
        let mut calls = Vec::new();
        serve_lines(&input[..], &mut output, "secret", 4096, |method, _| {
            calls.push(method);
            Some(json!({"ok": true}))
        });
        assert_eq!(calls, ["first"]);
        let lines: Vec<Value> = std::str::from_utf8(&output).unwrap().lines().map(|s| serde_json::from_str(s).unwrap()).collect();
        assert_eq!(lines[0], json!({"ok": true, "id": 1}));
        assert_eq!(lines[1]["error"], "control authentication failed; closing the connection");
    }
    #[test]
    fn bounds_requests_and_preserves_last_unterminated_line() {
        let mut buf = Vec::new();
        assert_eq!(read_bounded_line(&mut &b"abc"[..], &mut buf, 3), Ok(true));
        assert_eq!(buf, b"abc");
        assert!(matches!(read_bounded_line(&mut &b"abcd"[..], &mut buf, 3), Err(ReadFailure::Rejected(_))));
        let mut out = Vec::new();
        serve_lines(&b"{broken}\n"[..], &mut out, "secret", 1024, |_, _| panic!("must not dispatch"));
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert!(reply["error"].as_str().unwrap().starts_with("bad JSON ("));
    }
}
