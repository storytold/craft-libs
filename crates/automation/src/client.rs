//! A JSON-lines client for integration tests and CLIs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::SocketAddr;

use serde_json::{Value, json};

use crate::protocol::REPLY_DEADLINE;
use crate::security::{AUTH_METHOD, IO_TIMEOUT};

/// How long a call waits for its reply: the server's [`REPLY_DEADLINE`] plus a margin, so the
/// server's own timeout error arrives before the client gives up.
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(REPLY_DEADLINE.as_secs() + 5);

/// How long [`ControlClient::connect`] waits for the connection: 800 ms, as the server is on loopback only.
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(800);

/// The longest reply line a client reads: 256 MiB. A longer one is an error and closes the connection.
pub const MAX_REPLY_BYTES: u64 = 256 << 20;

/// Why [`ControlClient::connect`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectError {
    /// A socket or read failure: the server was not reached.
    Transport(String),
    /// The server answered the `auth` request with an error.
    Refused(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Transport(why) | ConnectError::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for ConnectError {}

/// One authenticated connection.
pub struct ControlClient {
    stream: std::net::TcpStream,
    broken: bool,
    reader: BufReader<std::net::TcpStream>,
    next_id: u64,
}

impl ControlClient {
    /// True after a transport failure closed the connection; an error reply leaves it false. A
    /// caller tells a lost connection from a refused request and never replays a mutation.
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// Whether a call can be sent now: false when the connection is broken, the server closed it,
    /// or it holds bytes no call asked for (a server that says why it closes), as no reply to the
    /// next call could be read from it. Every call checks this before it sends and, when false,
    /// closes the connection and fails with nothing sent; a caller reconnects instead.
    pub fn is_usable(&self) -> bool {
        if self.broken || !self.reader.buffer().is_empty() || self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let probe = self.stream.peek(&mut [0]);
        self.stream.set_nonblocking(false).is_ok() && probe.is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
    }

    /// Connect (read timeout [`CALL_TIMEOUT`]), send
    /// `{"id": 1, "method": "auth", "params": {"token": token}}` and read one reply line; `Err` with
    /// the reply's `error` text as `Refused` when it isn't `ok`, or the I/O error's message as `Transport`.
    /// A line the server wrote before the request (a refusal at accept) is the reply, and nothing is sent.
    pub fn connect(addr: SocketAddr, token: &str) -> Result<Self, ConnectError> {
        let io = |e: std::io::Error| ConnectError::Transport(e.to_string());
        let stream = std::net::TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(io)?;
        Self::over(stream).map_err(io)?.sign_in(token)
    }

    /// A client over a connected `stream`, not yet signed in.
    fn over(stream: std::net::TcpStream) -> std::io::Result<Self> {
        stream.set_read_timeout(Some(CALL_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self { stream, broken: false, reader, next_id: 2 })
    }

    /// Sends the sign-in and reads its answer. A server that refuses the connection at once (over its connection
    /// limit) writes why before any request: that line answers the sign-in, which is then not sent, so only its
    /// error counts.
    fn sign_in(mut self, token: &str) -> Result<Self, ConnectError> {
        let auth = if self.is_usable() {
            self.round_trip(1, AUTH_METHOD, json!({"token": token}), None)
        } else {
            self.read_reply(1, None).and_then(|_| Err(Failure::Io("reply before the sign-in".into())))
        };
        match auth {
            Ok(_) => Ok(self),
            Err(Failure::Reply(why)) => Err(ConnectError::Refused(why)),
            Err(Failure::Io(why)) => {
                self.close();
                Err(ConnectError::Transport(why))
            }
        }
    }

    /// Send `{"id": n, "method", "params"}` as one line (ids count up from 2) and read the reply
    /// line with that id (replies to other ids are skipped); `Ok(result)` (`null` when absent) for
    /// `"ok": true`, `Err(error)` otherwise. After an I/O error or a timeout the connection is
    /// closed and every later call fails, so a late reply can't answer the wrong call.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.call_with(method, params, None)
    }

    /// As [`ControlClient::call`], but gives up with `Err("cancelled")` and closes the connection when `cancelled()`
    /// turns true while it waits for the reply.
    pub fn call_until(&mut self, method: &str, params: Value, cancelled: &dyn Fn() -> bool) -> Result<Value, String> {
        self.call_with(method, params, Some(cancelled))
    }

    fn call_with(&mut self, method: &str, params: Value, cancelled: Option<&dyn Fn() -> bool>) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.exchange(id, method, params, cancelled)
    }

    fn exchange(&mut self, id: u64, method: &str, params: Value, cancelled: Option<&dyn Fn() -> bool>) -> Result<Value, String> {
        if !self.is_usable() {
            self.close();
            return Err("connection closed".into());
        }
        let result = self.round_trip(id, method, params, cancelled);
        if matches!(result, Err(Failure::Io(_))) {
            self.close();
        }
        result.map_err(|f| match f {
            Failure::Io(e) | Failure::Reply(e) => e,
        })
    }

    fn close(&mut self) {
        self.broken = true;
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    fn round_trip(&mut self, id: u64, method: &str, params: Value, cancelled: Option<&dyn Fn() -> bool>) -> Result<Value, Failure> {
        let io = |e: std::io::Error| Failure::Io(e.to_string());
        writeln!(self.stream, "{}", json!({"id": id, "method": method, "params": params})).map_err(io)?;
        self.stream.flush().map_err(io)?;
        self.read_reply(id, cancelled)
    }

    /// Reads reply lines until the one with `id`, or with a null id (the server's answer to an unparseable line or a
    /// refused connection).
    fn read_reply(&mut self, id: u64, cancelled: Option<&dyn Fn() -> bool>) -> Result<Value, Failure> {
        let io = |e: std::io::Error| Failure::Io(e.to_string());
        loop {
            if let Some(cancelled) = cancelled {
                self.wait_for_reply(cancelled)?;
            }
            let mut line = String::new();
            let read = (&mut self.reader).take(MAX_REPLY_BYTES + 1).read_line(&mut line).map_err(io)?;
            if read == 0 {
                return Err(Failure::Io("connection closed".into()));
            }
            if read as u64 > MAX_REPLY_BYTES {
                return Err(Failure::Io(format!("reply longer than {MAX_REPLY_BYTES} bytes")));
            }
            let mut reply: Value = serde_json::from_str(&line).map_err(|e| Failure::Io(format!("bad reply: {e}")))?;
            // A reply to an earlier request.
            if reply.get("id").is_some_and(|got| !got.is_null() && got != &json!(id)) {
                continue;
            }
            return if reply.get("ok") == Some(&Value::Bool(true)) {
                Ok(reply.get_mut("result").map(Value::take).unwrap_or(Value::Null))
            } else {
                Err(Failure::Reply(reply.get("error").and_then(Value::as_str).unwrap_or("request failed").to_string()))
            };
        }
    }

    /// Waits until a reply line starts to arrive, asking `cancelled` between short reads, for at most
    /// [`CALL_TIMEOUT`]. A cancel is a transport failure, so the connection closes and a late reply can't answer the
    /// next call.
    fn wait_for_reply(&self, cancelled: &dyn Fn() -> bool) -> Result<(), Failure> {
        const POLL: std::time::Duration = std::time::Duration::from_millis(100);
        if !self.reader.buffer().is_empty() {
            return Ok(());
        }
        let io = |e: std::io::Error| Failure::Io(e.to_string());
        let start = std::time::Instant::now();
        self.stream.set_read_timeout(Some(POLL)).map_err(io)?;
        let outcome = loop {
            match self.stream.peek(&mut [0]) {
                Ok(_) => break Ok(()),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if cancelled() {
                        break Err(Failure::Io("cancelled".into()));
                    }
                    if start.elapsed() >= CALL_TIMEOUT {
                        break Err(io(e));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => break Err(io(e)),
            }
        };
        self.stream.set_read_timeout(Some(CALL_TIMEOUT)).map_err(io)?;
        outcome
    }
}

/// Why a round trip failed: the transport (the connection is then unusable) or an error reply.
enum Failure {
    Io(String),
    Reply(String),
}

#[cfg(all(test, feature = "ui"))]
#[path = "client_tests.rs"]
pub(crate) mod client_tests;
