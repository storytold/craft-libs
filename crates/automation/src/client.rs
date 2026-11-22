//! A JSON-lines client for integration tests and CLIs.

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;

use serde_json::{Value, json};

use crate::protocol::REPLY_DEADLINE;
use crate::security::{AUTH_METHOD, IO_TIMEOUT};

/// How long a call waits for its reply: the server's [`REPLY_DEADLINE`] plus a margin, so the
/// server's own timeout error arrives before the client gives up.
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(REPLY_DEADLINE.as_secs() + 5);

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

    /// Connect (read timeout [`CALL_TIMEOUT`]), send
    /// `{"id": 1, "method": "auth", "params": {"token": token}}` and read one reply line; `Err` with
    /// the reply's `error` text as `Refused` when it isn't `ok`, or the I/O error's message as `Transport`.
    pub fn connect(addr: SocketAddr, token: &str) -> Result<Self, ConnectError> {
        let io = |e: std::io::Error| ConnectError::Transport(e.to_string());
        let stream = std::net::TcpStream::connect(addr).map_err(io)?;
        stream.set_read_timeout(Some(CALL_TIMEOUT)).map_err(io)?;
        stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(io)?;
        let reader = BufReader::new(stream.try_clone().map_err(io)?);
        let mut client = Self { stream, broken: false, reader, next_id: 2 };
        // `exchange` marks the client broken on a transport failure only, so an error reply is a refusal.
        let auth = client.exchange(1, AUTH_METHOD, json!({"token": token}));
        auth.map_err(|e| if client.broken { ConnectError::Transport(e) } else { ConnectError::Refused(e) })?;
        Ok(client)
    }

    /// Send `{"id": n, "method", "params"}` as one line (ids count up from 2) and read the reply
    /// line with that id (replies to other ids are skipped); `Ok(result)` (`null` when absent) for
    /// `"ok": true`, `Err(error)` otherwise. After an I/O error or a timeout the connection is
    /// closed and every later call fails, so a late reply can't answer the wrong call.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.exchange(id, method, params)
    }

    fn exchange(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        if self.broken {
            return Err("connection closed".into());
        }
        let result = self.round_trip(id, method, params);
        if matches!(result, Err(Failure::Io(_))) {
            self.broken = true;
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
        result.map_err(|f| match f {
            Failure::Io(e) | Failure::Reply(e) => e,
        })
    }

    fn round_trip(&mut self, id: u64, method: &str, params: Value) -> Result<Value, Failure> {
        let io = |e: std::io::Error| Failure::Io(e.to_string());
        writeln!(self.stream, "{}", json!({"id": id, "method": method, "params": params})).map_err(io)?;
        self.stream.flush().map_err(io)?;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line).map_err(io)? == 0 {
                return Err(Failure::Io("connection closed".into()));
            }
            let mut reply: Value = serde_json::from_str(&line).map_err(|e| Failure::Io(format!("bad reply: {e}")))?;
            // A reply to an earlier request (a null id is the server's answer to an unparseable line).
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
}

/// Why a round trip failed: the transport (the connection is then unusable) or an error reply.
enum Failure {
    Io(String),
    Reply(String),
}

#[cfg(all(test, feature = "ui"))]
#[path = "client_tests.rs"]
pub(crate) mod client_tests;
