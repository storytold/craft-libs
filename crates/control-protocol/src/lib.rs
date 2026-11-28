//! Authenticated JSON-lines request framing and dispatch for local control services.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
use serde_json::{Value, json};
use std::cell::Cell;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

/// Whether a session token has the control protocol's 64 hexadecimal character form.
pub fn valid_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A new session token that [`valid_token`] accepts: 32 bytes from the operating system's random
/// source, as lowercase hexadecimal. `None` when the system cannot supply random bytes.
#[cfg(not(target_arch = "wasm32"))]
pub fn new_token() -> Option<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
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

/// Read one `\n`-terminated line of at most `max` bytes into `buf` (newline stripped, and a `\r`
/// before it). The `\r\n` or `\n` does not count toward `max`.
/// `Ok(false)` at end of stream; `Err` when the line is too long or the read fails.
pub fn read_bounded_line(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> Result<bool, ReadFailure> {
    buf.clear();
    // `take` bounds the bytes buffered for one line: a peer can't make us allocate more than
    // `max + 2`, the line and its `\r\n`.
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(2);
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
    }
    if buf.len() > max {
        return Err(ReadFailure::Rejected(format!("request line longer than {max} bytes; closing the connection")));
    }
    // A complete line, or the last line without a newline, then end of stream.
    Ok(true)
}

/// Serve requests from `read`, answering on `out`, until the peer closes, sends something that is
/// not a request, or `handle` returns `None` (app gone). `handle` gets `(method, params)` and
/// returns the reply object (the request's `id` is added here).
pub fn serve_lines<R: Read, W: Write>(read: R, out: W, token: &str, max_line: usize, handle: impl FnMut(String, Value) -> Option<Value>) {
    serve(BufReader::new(read), out, token, max_line, |_| Ok(()), |_| false, handle);
}

/// Serve one TCP connection as [`serve_lines`] does, but give the peer `first_request` from now
/// to send its first authenticated request. Until then every read shares that one deadline, so
/// blank lines or a line sent a byte at a time do not extend it; when it passes, the connection
/// closes without a reply and nothing is dispatched. After the first authenticated request, each
/// read waits at most `idle` (`None` waits without limit). A `first_request` too long for the
/// clock to represent waits without limit. The connection is closed when this returns.
///
/// Returns an `InvalidInput` error, before reading anything, when `first_request` or `idle` is
/// zero, and any error from setting the socket's read timeout.
pub fn serve_tcp(
    stream: TcpStream,
    token: &str,
    max_line: usize,
    first_request: Duration,
    idle: Option<Duration>,
    handle: impl FnMut(String, Value) -> Option<Value>,
) -> std::io::Result<()> {
    if first_request.is_zero() || idle.is_some_and(|d| d.is_zero()) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "control wait times must be longer than zero"));
    }
    let deadline = Instant::now().checked_add(first_request);
    if deadline.is_none() {
        // No deadline to apply on each read: clear any read timeout the stream already has.
        stream.set_read_timeout(None)?;
    }
    let timeout_error = Cell::new(None);
    let input = FirstRequestDeadline { socket: &stream, deadline, timeout_error: &timeout_error };
    serve(
        BufReader::new(input),
        &stream,
        token,
        max_line,
        |input: &mut FirstRequestDeadline<'_>| input.admit(idle),
        FirstRequestDeadline::expired,
        handle,
    );
    timeout_error.take().map_or(Ok(()), Err)
}

/// A TCP connection's input whose reads share one deadline until the peer is admitted.
struct FirstRequestDeadline<'a> {
    socket: &'a TcpStream,
    deadline: Option<Instant>,
    /// The error from setting the socket's read timeout, which [`serve_tcp`] returns.
    timeout_error: &'a Cell<Option<std::io::Error>>,
}

impl FirstRequestDeadline<'_> {
    fn set_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.socket.set_read_timeout(timeout).map_err(|e| {
            let kind = e.kind();
            self.timeout_error.set(Some(e));
            kind.into()
        })
    }

    /// The peer sent an authenticated request: from now on each read waits at most `idle`. A
    /// request admitted after the first-request deadline returns `TimedOut` instead, and the
    /// deadline stays in place.
    fn admit(&mut self, idle: Option<Duration>) -> std::io::Result<()> {
        if self.expired() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.deadline = None;
        self.set_timeout(idle)
    }

    /// Whether the first-request deadline has passed and no request was admitted before it.
    fn expired(&self) -> bool {
        self.deadline.is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl Read for FirstRequestDeadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            self.set_timeout(Some(left))?;
        }
        let mut socket = self.socket;
        socket.read(buf)
    }
}

/// The request loop of [`serve_lines`] and [`serve_tcp`]: `admit` runs once, on the input, after
/// the first authenticated request and before it is dispatched; an error from it closes the
/// connection. Before the first request is admitted, a rejection closes the connection without a
/// reply when `expired` returns true for the input.
fn serve<R: Read, W: Write>(
    mut reader: BufReader<R>,
    mut out: W,
    token: &str,
    max_line: usize,
    mut admit: impl FnMut(&mut R) -> std::io::Result<()>,
    expired: impl Fn(&R) -> bool,
    mut handle: impl FnMut(String, Value) -> Option<Value>,
) {
    let mut buf = Vec::new();
    let mut admitted = false;
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
                    if !admitted {
                        if admit(reader.get_mut()).is_err() {
                            return;
                        }
                        admitted = true;
                    }
                    let Some(mut r) = handle(method, params) else { return };
                    if let Some(o) = r.as_object_mut() {
                        o.insert("id".into(), id);
                    }
                    if write_line(&mut out, &r).is_err() {
                        return;
                    }
                    continue;
                }
            },
        };
        if !admitted && expired(reader.get_ref()) {
            return;
        }
        let _ = write_line(&mut out, &json!({"ok": false, "error": reject}));
        return;
    }
}

/// Write `reply` and its newline to `out` through a buffer, then flush. `writeln!` with a JSON
/// value writes each string, number and separator separately, which is one system call apiece on
/// an unbuffered socket. The buffer holds at most `REPLY_BUFFER` bytes, so a large reply is not
/// copied whole before it is sent.
fn write_line(out: &mut impl Write, reply: &Value) -> std::io::Result<()> {
    const REPLY_BUFFER: usize = 64 * 1024;
    let mut buffered = BufWriter::with_capacity(REPLY_BUFFER, out);
    serde_json::to_writer(&mut buffered, reply).map_err(std::io::Error::other)?;
    buffered.write_all(b"\n")?;
    buffered.flush()
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
    fn replies_reach_the_writer_in_few_bounded_writes() {
        struct Writes(Vec<Vec<u8>>);
        impl Write for Writes {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.push(buf.to_vec());
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let input = b"{\"id\":1,\"method\":\"tree\",\"token\":\"s\"}\n{\"id\":2,\"method\":\"tree\"}\n";
        let mut writes = Writes(Vec::new());
        serve_lines(&input[..], &mut writes, "s", 1024, |_, _| Some(json!({"ok": true, "result": {"names": ["a", "b", "c"], "sizes": [1, 2, 3]}})));
        assert_eq!(writes.0.len(), 2, "one write for the reply and one for the rejection");
        for line in &writes.0 {
            assert_eq!(line.last(), Some(&b'\n'));
            serde_json::from_slice::<Value>(line).unwrap();
        }
        // A large reply goes out in a few buffer-sized writes, never as one whole copy.
        let items: Vec<u32> = (0..100_000).collect();
        let mut writes = Writes(Vec::new());
        serve_lines(&b"{\"method\":\"tree\",\"token\":\"s\"}\n"[..], &mut writes, "s", 1024, |_, _| Some(json!({"ok": true, "result": items})));
        let total: usize = writes.0.iter().map(Vec::len).sum();
        assert!(writes.0.len() < items.len() / 100);
        assert!(writes.0.iter().all(|w| w.len() < total));
        let reply: Value = serde_json::from_slice(&writes.0.concat()).unwrap();
        assert_eq!(reply["result"].as_array().map(Vec::len), Some(items.len()));
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
    #[cfg(not(target_arch = "wasm32"))]
    fn new_tokens_are_valid_and_distinct() {
        let first = new_token().unwrap();
        let second = new_token().unwrap();
        assert!(valid_token(&first));
        assert!(valid_token(&second));
        assert_ne!(first, second);
        let input = request_line(1, Some(&first), "status", &json!({}));
        let mut output = Vec::new();
        serve_lines(input.as_bytes(), &mut output, &first, 4096, |_, _| Some(json!({"ok": true})));
        assert_eq!(serde_json::from_slice::<Value>(&output).unwrap(), json!({"ok": true, "id": 1}));
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
    /// The serving thread: what `serve_tcp` returned and the methods it dispatched.
    type Server = std::thread::JoinHandle<(std::io::Result<()>, Vec<String>)>;
    /// A loopback connection whose client sent `input` before `serve_tcp` starts serving it on
    /// another thread, so the server has read everything the client sends when a wait runs out.
    fn tcp_session(input: &[u8], first_request: Duration, idle: Option<Duration>) -> (TcpStream, Server) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        // Guards the test against a server that never closes; the assertions do not depend on it.
        client.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        client.write_all(input).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let server = std::thread::spawn(move || {
            let mut calls = Vec::new();
            let result = serve_tcp(stream, "secret", 1024, first_request, idle, |method, _| {
                calls.push(method);
                Some(json!({"ok": true}))
            });
            (result, calls)
        });
        (client, server)
    }
    #[test]
    fn tcp_peer_without_a_request_loses_the_connection() {
        let (mut client, server) = tcp_session(b"\n{\"method\":", Duration::from_millis(50), None);
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).expect("the server closes the connection");
        assert!(rest.is_empty());
        let (result, calls) = server.join().unwrap();
        assert!(result.is_ok());
        assert!(calls.is_empty());
    }
    #[test]
    fn tcp_input_waiting_after_the_deadline_is_not_read() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client.write_all(b"\n\n").unwrap();
        let (stream, _) = listener.accept().unwrap();
        let timeout_error = Cell::new(None);
        let mut input = FirstRequestDeadline { socket: &stream, deadline: Some(Instant::now()), timeout_error: &timeout_error };
        assert_eq!(input.read(&mut [0; 8]).unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(timeout_error.take().is_none());
    }
    #[test]
    fn tcp_request_read_after_the_deadline_is_not_admitted() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let timeout_error = Cell::new(None);
        let mut input = FirstRequestDeadline { socket: &stream, deadline: Some(Instant::now()), timeout_error: &timeout_error };
        assert_eq!(input.admit(None).unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(input.deadline.is_some());
        assert!(timeout_error.take().is_none());
    }
    #[test]
    fn rejection_read_after_the_deadline_gets_no_reply() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let timeout_error = Cell::new(None);
        let mut input = FirstRequestDeadline { socket: &stream, deadline: Some(Instant::now()), timeout_error: &timeout_error };
        assert!(input.expired());
        input.deadline = None;
        assert!(!input.expired());
        let long = format!("{}\n", "x".repeat(100));
        for line in [&b"{broken}\n"[..], &b"{\"method\":\"x\",\"token\":\"wrong\"}\n"[..], long.as_bytes()] {
            let mut out = Vec::new();
            serve(BufReader::new(line), &mut out, "secret", 64, |_| Ok(()), |_| true, |_, _| panic!("must not dispatch"));
            assert!(out.is_empty());
        }
    }
    #[test]
    fn tcp_peer_waits_idle_time_after_its_first_request() {
        let input = b"{\"id\":1,\"method\":\"first\",\"token\":\"secret\"}\n";
        let (mut client, server) = tcp_session(input, Duration::from_secs(600), Some(Duration::from_millis(50)));
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).expect("the server closes the idle connection");
        assert_eq!(serde_json::from_slice::<Value>(&rest).unwrap(), json!({"ok": true, "id": 1}));
        let (result, calls) = server.join().unwrap();
        assert!(result.is_ok());
        assert_eq!(calls, ["first"]);
    }
    #[test]
    fn tcp_peer_sending_blank_lines_still_loses_the_connection_at_the_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let server =
            std::thread::spawn(move || serve_tcp(stream, "secret", 1024, Duration::from_millis(200), None, |_, _| panic!("must not dispatch")));
        // Every read finds a new line, so a deadline that restarted on each read would never expire.
        // The loop ends when the server closes the connection; the guard only stops a hung test.
        client.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
        let guard = Instant::now() + Duration::from_secs(30);
        while client.write_all(b"\n").is_ok() {
            assert!(Instant::now() < guard, "the server kept reading past the first-request deadline");
            std::thread::yield_now();
        }
        assert!(Instant::now() < guard, "the server stopped reading but kept the connection open");
        assert!(server.join().unwrap().is_ok());
    }
    #[test]
    fn tcp_rejects_zero_wait_times() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        for (first, idle) in [(Duration::ZERO, None), (Duration::from_secs(1), Some(Duration::ZERO))] {
            let err = serve_tcp(stream.try_clone().unwrap(), "secret", 1024, first, idle, |_, _| panic!("must not dispatch")).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        }
    }
    #[test]
    fn bounds_requests_and_preserves_last_unterminated_line() {
        let mut buf = Vec::new();
        assert_eq!(read_bounded_line(&mut &b"abc"[..], &mut buf, 3), Ok(true));
        assert_eq!(buf, b"abc");
        assert!(matches!(read_bounded_line(&mut &b"abcd"[..], &mut buf, 3), Err(ReadFailure::Rejected(_))));
        // The cap counts the line without its `\r\n` or `\n`.
        assert_eq!(read_bounded_line(&mut &b"abc\r\n"[..], &mut buf, 3), Ok(true));
        assert_eq!(buf, b"abc");
        assert!(matches!(read_bounded_line(&mut &b"abcd\n"[..], &mut buf, 3), Err(ReadFailure::Rejected(_))));
        assert!(matches!(read_bounded_line(&mut &b"abcd\r\n"[..], &mut buf, 3), Err(ReadFailure::Rejected(_))));
        let mut out = Vec::new();
        serve_lines(&b"{broken}\n"[..], &mut out, "secret", 1024, |_, _| panic!("must not dispatch"));
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert!(reply["error"].as_str().unwrap().starts_with("bad JSON ("));
    }
}
