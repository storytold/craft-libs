//! A test client for an MCP server that runs as a child process and exchanges JSON-RPC 2.0
//! messages, one per line, on the child's stdin and stdout.
//!
//! [`McpChild::spawn`] starts the program with an environment list for the child only;
//! [`McpChild::connect`] uses any reader and writer, for a server that runs in the test process.
//! Every wait has a deadline and every line read from the child has a length limit, both set in
//! [`McpLimits`]. A child that does not answer makes the call return [`McpError::Deadline`]; the
//! call does not hang.
//!
//! After a deadline, a protocol error, an over-long line or the end of the child's output, the
//! connection is closed: the child is stopped and reaped, and later calls return
//! [`McpError::Closed`]. A JSON-RPC error reply returns [`McpError::Rpc`] and the connection stays
//! usable. A tool result with `isError: true` is a successful reply and is returned unchanged.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

/// The MCP revision the Craft applications' servers speak today.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// The default name of the variable read by [`McpLimits::timeouts_from_var`].
pub const TIMEOUT_VAR: &str = "CRAFT_TEST_MCP_TIMEOUT_SECS";
/// The longest deadline or close grace period [`McpLimits`] accepts.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(3600);
/// The most notifications [`McpLimits::max_notifications`] may keep.
pub const MAX_NOTIFICATIONS: usize = 4096;
/// The most bytes of stderr [`McpLimits::stderr_tail_bytes`] may keep.
pub const MAX_STDERR_TAIL: usize = 16 << 20;

/// Parsed messages the reader thread holds before the client takes them. When the queue is full,
/// the reader thread waits, and so does a child writing more.
const READ_QUEUE: usize = 4;
/// Lines waiting for the writer thread. A full queue means the child stopped reading its input.
const WRITE_QUEUE: usize = 64;
/// How often [`McpChild::close`] checks whether the child has exited.
const EXIT_POLL: Duration = Duration::from_millis(10);
/// Bytes of an unreadable stdout line quoted in a protocol error.
const PREVIEW: usize = 80;
/// The JSON-RPC error code for a method the client does not offer.
const METHOD_NOT_FOUND: i64 = -32601;

/// Limits and deadlines of one connection. [`McpChild::spawn`] and [`McpChild::connect`] return
/// [`McpError::Setup`] for a value out of range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpLimits {
    /// The longest stdout line read, without its newline, and the most bytes of JSON
    /// [`McpChild::list_tools`] returns in all. Default 64 MiB; must be at least 1.
    pub max_line_bytes: usize,
    /// How long [`McpChild::request`], [`McpChild::call_tool`] and each page of
    /// [`McpChild::list_tools`] wait for the reply. Default 30 s; from 1 ns to [`MAX_TIMEOUT`].
    pub request_timeout: Duration,
    /// How long [`McpChild::initialize`] waits. Default 60 s; from 1 ns to [`MAX_TIMEOUT`].
    pub initialize_timeout: Duration,
    /// How long [`McpChild::close`] waits for the child to exit after closing its stdin before it
    /// stops the child. Default 2 s; at most [`MAX_TIMEOUT`].
    pub close_grace: Duration,
    /// The most pages [`McpChild::list_tools`] reads. Default 64; must be at least 1.
    pub max_pages: usize,
    /// The most notifications kept for [`McpChild::take_notifications`]; when the queue is full,
    /// the oldest is dropped. Default 0, which keeps none; at most [`MAX_NOTIFICATIONS`].
    pub max_notifications: usize,
    /// The most bytes of the child's stderr kept, the last ones written. Default 0, which sends
    /// stderr to the null device; at most [`MAX_STDERR_TAIL`]. Not used by [`McpChild::connect`].
    pub stderr_tail_bytes: usize,
}

impl Default for McpLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: 64 << 20,
            request_timeout: Duration::from_secs(30),
            initialize_timeout: Duration::from_secs(60),
            close_grace: Duration::from_secs(2),
            max_pages: 64,
            max_notifications: 0,
            stderr_tail_bytes: 0,
        }
    }
}

impl McpLimits {
    /// Sets [`McpLimits::request_timeout`] and [`McpLimits::initialize_timeout`] from the variable
    /// `name` ([`TIMEOUT_VAR`] by default), a whole number of seconds from 1 to 3600. An unset
    /// variable leaves the limits unchanged; any other value, and a name that is empty or holds `=`
    /// or NUL, returns [`McpError::Setup`].
    pub fn timeouts_from_var(mut self, name: &str) -> Result<Self, McpError> {
        if name.is_empty() || name.contains(['=', '\0']) {
            return Err(McpError::Setup(format!("{name:?} is not a variable name")));
        }
        let Some(value) = std::env::var_os(name) else { return Ok(self) };
        let timeout = parse_timeout_secs(name, &value)?;
        self.request_timeout = timeout;
        self.initialize_timeout = timeout;
        Ok(self)
    }

    fn validate(&self) -> Result<(), McpError> {
        if self.max_line_bytes == 0 {
            return Err(McpError::Setup("max_line_bytes must be at least 1".into()));
        }
        check_timeout("request_timeout", self.request_timeout)?;
        check_timeout("initialize_timeout", self.initialize_timeout)?;
        if self.close_grace > MAX_TIMEOUT {
            return Err(McpError::Setup(format!("close_grace must be at most {MAX_TIMEOUT:?}")));
        }
        if self.max_pages == 0 {
            return Err(McpError::Setup("max_pages must be at least 1".into()));
        }
        if self.max_notifications > MAX_NOTIFICATIONS {
            return Err(McpError::Setup(format!("max_notifications must be at most {MAX_NOTIFICATIONS}")));
        }
        if self.stderr_tail_bytes > MAX_STDERR_TAIL {
            return Err(McpError::Setup(format!("stderr_tail_bytes must be at most {MAX_STDERR_TAIL}")));
        }
        Ok(())
    }
}

fn check_timeout(what: &str, timeout: Duration) -> Result<(), McpError> {
    if timeout.is_zero() || timeout > MAX_TIMEOUT {
        return Err(McpError::Setup(format!("{what} must be more than zero and at most {MAX_TIMEOUT:?}")));
    }
    Ok(())
}

fn parse_timeout_secs(name: &str, value: &OsStr) -> Result<Duration, McpError> {
    let bad = || McpError::Setup(format!("{name} must be a whole number of seconds from 1 to {}, not {value:?}", MAX_TIMEOUT.as_secs()));
    let text = value.to_str().ok_or_else(bad)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let secs: u64 = text.parse().map_err(|_| bad())?;
    if secs == 0 || secs > MAX_TIMEOUT.as_secs() {
        return Err(bad());
    }
    Ok(Duration::from_secs(secs))
}

/// Why a call failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum McpError {
    /// A limit, a timeout, an environment pair or a variable is not valid. Nothing was started.
    Setup(String),
    /// The program could not be started.
    Spawn {
        /// The program as given to [`McpChild::spawn`].
        program: OsString,
        /// The error from the operating system.
        source: io::Error,
    },
    /// Reading from, writing to or waiting for the child failed.
    Io {
        /// What failed, such as "writing to the child".
        what: &'static str,
        /// The underlying error.
        source: io::Error,
    },
    /// The reply to `method` did not arrive within `after`.
    Deadline {
        /// The method of the request.
        method: String,
        /// The deadline that passed.
        after: Duration,
    },
    /// The child wrote a stdout line longer than `limit` bytes.
    LineTooLong {
        /// [`McpLimits::max_line_bytes`].
        limit: usize,
    },
    /// The child closed its stdout.
    Ended,
    /// The child wrote something on stdout that is not a JSON-RPC 2.0 message, or a reply that
    /// does not hold exactly one of `result` and `error`.
    Protocol(String),
    /// The child answered `method` with a JSON-RPC error. The connection stays usable.
    Rpc {
        /// The method of the request.
        method: String,
        /// The error's `code`.
        code: i64,
        /// The error's `message`.
        message: String,
        /// The error's `data`, when present.
        data: Option<Value>,
    },
    /// The child answered `initialize` with a protocol version other than the one asked for.
    UnsupportedVersion {
        /// The version sent in `initialize`.
        requested: String,
        /// The version in the child's reply.
        returned: String,
    },
    /// [`McpChild::list_tools`] read [`McpLimits::max_pages`] pages and the list went on.
    TooManyPages {
        /// [`McpLimits::max_pages`].
        limit: usize,
    },
    /// The tools [`McpChild::list_tools`] read add up to more than `limit` bytes of JSON.
    ToolListTooLarge {
        /// [`McpLimits::max_line_bytes`].
        limit: usize,
    },
    /// An earlier failure closed the connection; this is its message.
    Closed(String),
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Setup(why) => write!(f, "invalid MCP test setup: {why}"),
            Self::Spawn { program, source } => write!(f, "could not start {}: {source}", program.to_string_lossy()),
            Self::Io { what, source } => write!(f, "{what} failed: {source}"),
            Self::Deadline { method, after } => write!(f, "the child did not answer `{method}` within {after:?}"),
            Self::LineTooLong { limit } => write!(f, "the child wrote a line longer than {limit} bytes"),
            Self::Ended => write!(f, "the child closed its output"),
            Self::Protocol(why) => write!(f, "MCP protocol error: {why}"),
            Self::Rpc { method, code, message, .. } => write!(f, "`{method}` returned error {code}: {message}"),
            Self::UnsupportedVersion { requested, returned } => {
                write!(f, "asked for MCP version {requested}, and the child answered with {returned}")
            }
            Self::TooManyPages { limit } => write!(f, "the tool list did not end within {limit} pages"),
            Self::ToolListTooLarge { limit } => write!(f, "the tool list is larger than {limit} bytes"),
            Self::Closed(why) => write!(f, "the MCP connection is closed: {why}"),
        }
    }
}

impl std::error::Error for McpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// How [`McpChild::close`] ended the child.
#[derive(Debug)]
pub struct CloseOutcome {
    /// The child's exit status; `None` for a connection made with [`McpChild::connect`].
    pub status: Option<ExitStatus>,
    /// The child was stopped because it did not exit within [`McpLimits::close_grace`], or an
    /// earlier failure stopped it.
    pub forced: bool,
    /// The last [`McpLimits::stderr_tail_bytes`] bytes the child wrote on stderr.
    pub stderr: Vec<u8>,
}

/// What the reader and writer threads hand to the client.
enum Incoming {
    Message(Value),
    TooLong,
    Ended,
    Invalid(String),
    ReadFailed(io::Error),
    WriteFailed(io::Error),
}

enum LineError {
    TooLong,
    Io(io::Error),
}

/// Reads one line of at most `cap` bytes, without its newline. `Ok(None)` at the end of the
/// stream. A line is rejected as soon as it passes `cap`, without waiting for its newline, and at
/// most `cap` bytes of it are kept.
fn read_line_capped(r: &mut impl BufRead, cap: usize) -> Result<Option<Vec<u8>>, LineError> {
    let mut line = Vec::new();
    loop {
        let buf = match r.fill_buf() {
            Ok(buf) => buf,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(LineError::Io(e)),
        };
        if buf.is_empty() {
            return Ok(if line.is_empty() { None } else { Some(line) });
        }
        let (chunk, used, done) = match buf.iter().position(|b| *b == b'\n') {
            Some(i) => (&buf[..i], i + 1, true),
            None => (buf, buf.len(), false),
        };
        if line.len().saturating_add(chunk.len()) > cap {
            return Err(LineError::TooLong);
        }
        line.extend_from_slice(chunk);
        r.consume(used);
        if done {
            return Ok(Some(line));
        }
    }
}

/// Whether `v` holds exactly one of `result` and an `error` object with an integer `code` and a
/// string `message`.
fn is_reply(v: &Value) -> bool {
    match (v.get("result"), v.get("error")) {
        (Some(_), None) => true,
        (None, Some(error)) => error.get("code").and_then(Value::as_i64).is_some() && error.get("message").is_some_and(Value::is_string),
        _ => false,
    }
}

/// Parses one stdout line as a JSON-RPC 2.0 message object.
fn parse_line(line: &[u8]) -> Incoming {
    let preview = || String::from_utf8_lossy(&line[..line.len().min(PREVIEW)]).into_owned();
    match serde_json::from_slice::<Value>(line) {
        Ok(v) if v.get("jsonrpc").and_then(Value::as_str) == Some("2.0") => Incoming::Message(v),
        Ok(_) => Incoming::Invalid(format!("a stdout line is not a JSON-RPC 2.0 message: {}", preview())),
        Err(e) => Incoming::Invalid(format!("a stdout line is not JSON ({e}): {}", preview())),
    }
}

fn start_thread(name: &str, f: impl FnOnce() + Send + 'static) -> Result<(), McpError> {
    std::thread::Builder::new().name(name.into()).spawn(f).map(drop).map_err(|source| McpError::Io { what: "starting a client thread", source })
}

/// The last bytes of the child's stderr, kept by a thread that reads it until it ends.
struct StderrTail {
    bytes: Arc<Mutex<VecDeque<u8>>>,
    done: Receiver<()>,
}

impl StderrTail {
    fn start(mut stderr: ChildStderr, keep: usize) -> Result<Self, McpError> {
        let bytes: Arc<Mutex<VecDeque<u8>>> = Arc::default();
        let (done_tx, done) = sync_channel(1);
        let store = Arc::clone(&bytes);
        start_thread("craft-testkit-mcp-stderr", move || {
            let mut chunk = [0u8; 8192];
            loop {
                let n = match stderr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                };
                let mut tail = store.lock().unwrap_or_else(PoisonError::into_inner);
                tail.extend(chunk.iter().take(n));
                let extra = tail.len().saturating_sub(keep);
                tail.drain(..extra);
            }
            let _ = done_tx.try_send(());
        })?;
        Ok(Self { bytes, done })
    }

    fn copy(&self) -> Vec<u8> {
        self.bytes.lock().unwrap_or_else(PoisonError::into_inner).iter().copied().collect()
    }
}

/// A connection to an MCP server. Dropping it stops and reaps a child that is still running.
pub struct McpChild {
    /// Lines for the writer thread. `None` once the connection is closed, which ends the thread
    /// and closes the child's stdin.
    writer: Option<SyncSender<Vec<u8>>>,
    rx: Receiver<Incoming>,
    next_id: u64,
    limits: McpLimits,
    child: Option<Child>,
    /// The exit status of a child already reaped, and whether it was stopped.
    exit: Option<(ExitStatus, bool)>,
    closed: Option<String>,
    notifications: VecDeque<Value>,
    stderr: Option<StderrTail>,
}

impl fmt::Debug for McpChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpChild").field("next_id", &self.next_id).field("closed", &self.closed).finish_non_exhaustive()
    }
}

impl McpChild {
    /// Starts `path` with `args` and, for the child only, the variables in `env`, with
    /// [`McpLimits::default`]. The process's own environment is not changed.
    pub fn spawn<K, V>(
        path: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
        env: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self, McpError>
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        Self::spawn_with_limits(path, args, env, McpLimits::default())
    }

    /// [`McpChild::spawn`] with the given limits. A variable name that is empty or holds `=` or
    /// NUL, or a value that holds NUL, returns [`McpError::Setup`].
    pub fn spawn_with_limits<K, V>(
        path: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
        env: impl IntoIterator<Item = (K, V)>,
        limits: McpLimits,
    ) -> Result<Self, McpError>
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        limits.validate()?;
        let program = path.as_ref();
        let mut cmd = Command::new(program);
        cmd.args(args);
        for (key, value) in env {
            let (key, value) = (key.as_ref(), value.as_ref());
            let key_bytes = key.as_encoded_bytes();
            if key_bytes.is_empty() || key_bytes.contains(&b'=') || key_bytes.contains(&0) {
                return Err(McpError::Setup(format!("{key:?} is not a variable name")));
            }
            if value.as_encoded_bytes().contains(&0) {
                return Err(McpError::Setup(format!("the value of {key:?} holds NUL")));
            }
            cmd.env(key, value);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        cmd.stderr(if limits.stderr_tail_bytes == 0 { Stdio::null() } else { Stdio::piped() });
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            /// Starts a console program without opening a console window.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|source| McpError::Spawn { program: program.to_os_string(), source })?;
        let started = Self::attach(&mut child, limits);
        match started {
            Ok(mut conn) => {
                conn.child = Some(child);
                Ok(conn)
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    }

    fn attach(child: &mut Child, limits: McpLimits) -> Result<Self, McpError> {
        let missing = || McpError::Io { what: "connecting to the child", source: io::Error::other("a pipe was not opened") };
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let stdout = child.stdout.take().ok_or_else(missing)?;
        let stderr = match child.stderr.take() {
            Some(stderr) if limits.stderr_tail_bytes > 0 => Some(StderrTail::start(stderr, limits.stderr_tail_bytes)?),
            _ => None,
        };
        let mut conn = Self::start(stdout, stdin, limits)?;
        conn.stderr = stderr;
        Ok(conn)
    }

    /// Connects to a server that reads `writer` and writes `reader`, such as one running on a
    /// thread of the test. [`McpLimits::stderr_tail_bytes`] is not used.
    pub fn connect(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static, limits: McpLimits) -> Result<Self, McpError> {
        limits.validate()?;
        Self::start(reader, writer, limits)
    }

    fn start(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static, limits: McpLimits) -> Result<Self, McpError> {
        let (tx, rx) = sync_channel(READ_QUEUE);
        let write_failed = tx.clone();
        let cap = limits.max_line_bytes;
        start_thread("craft-testkit-mcp-read", move || {
            let mut r = BufReader::new(reader);
            loop {
                let incoming = match read_line_capped(&mut r, cap) {
                    Ok(None) => Incoming::Ended,
                    Ok(Some(line)) => parse_line(&line),
                    Err(LineError::TooLong) => Incoming::TooLong,
                    Err(LineError::Io(e)) => Incoming::ReadFailed(e),
                };
                let last = !matches!(incoming, Incoming::Message(_));
                if tx.send(incoming).is_err() || last {
                    return;
                }
            }
        })?;
        let (wtx, wrx) = sync_channel::<Vec<u8>>(WRITE_QUEUE);
        start_thread("craft-testkit-mcp-write", move || {
            let mut w = writer;
            for line in wrx {
                if let Err(e) = w.write_all(&line).and_then(|()| w.flush()) {
                    // Wakes a request waiting for its reply. Ends when the client is dropped.
                    let _ = write_failed.send(Incoming::WriteFailed(e));
                    return;
                }
            }
        })?;
        Ok(Self { writer: Some(wtx), rx, next_id: 1, limits, child: None, exit: None, closed: None, notifications: VecDeque::new(), stderr: None })
    }

    /// The message of the failure that closed the connection, or `None` while it is usable.
    pub fn closed(&self) -> Option<&str> {
        self.closed.as_deref()
    }

    /// The last bytes the child wrote on stderr so far (see [`McpLimits::stderr_tail_bytes`]).
    pub fn stderr_tail(&self) -> Vec<u8> {
        self.stderr.as_ref().map(StderrTail::copy).unwrap_or_default()
    }

    /// Sends `initialize` with `version` (for example [`PROTOCOL_VERSION`]), empty capabilities and
    /// this crate's client info, waits up to [`McpLimits::initialize_timeout`] for the reply, then
    /// sends `notifications/initialized`. Returns the reply's result. A reply with another protocol
    /// version returns [`McpError::UnsupportedVersion`] and closes the connection.
    pub fn initialize(&mut self, version: &str) -> Result<Value, McpError> {
        let params = json!({
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION")},
        });
        let result = self.request_within("initialize", params, self.limits.initialize_timeout)?;
        let returned = match result.get("protocolVersion").and_then(Value::as_str) {
            Some(returned) => returned.to_string(),
            None => return Err(self.fail(McpError::Protocol("the `initialize` result has no `protocolVersion`".into()))),
        };
        if returned != version {
            return Err(self.fail(McpError::UnsupportedVersion { requested: version.to_string(), returned }));
        }
        self.notify("notifications/initialized", Value::Null)?;
        Ok(result)
    }

    /// Sends a notification: a message without an id, to which the child sends no reply.
    /// `Value::Null` params are left out of the message.
    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError> {
        self.check_open()?;
        self.send(&Value::Object(message(method, params)))
    }

    /// Sends a request and waits up to [`McpLimits::request_timeout`] for the reply with its id.
    /// Returns the reply's `result`. `Value::Null` params are left out of the message.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        self.request_within(method, params, self.limits.request_timeout)
    }

    /// [`McpChild::request`] with its own deadline, from 1 ns to [`MAX_TIMEOUT`].
    ///
    /// While it waits, it answers the child's `ping` requests with an empty result and its other
    /// requests with error -32601, keeps notifications for [`McpChild::take_notifications`], and
    /// skips well-formed replies with another id. None of these extend the deadline.
    pub fn request_within(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, McpError> {
        self.check_open()?;
        check_timeout("the request deadline", timeout)?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| McpError::Setup("the request deadline is too far away".into()))?;
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut msg = message(method, params);
        msg.insert("id".into(), json!(id));
        self.send(&Value::Object(msg))?;
        loop {
            let left = deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero());
            let Some(left) = left else {
                return Err(self.fail(McpError::Deadline { method: method.to_string(), after: timeout }));
            };
            let incoming = match self.rx.recv_timeout(left) {
                Ok(incoming) => incoming,
                Err(RecvTimeoutError::Timeout) => return Err(self.fail(McpError::Deadline { method: method.to_string(), after: timeout })),
                Err(RecvTimeoutError::Disconnected) => return Err(self.fail(McpError::Ended)),
            };
            let v = self.receive(incoming)?;
            if let Some(reply) = self.route(v, Some(id))? {
                return self.reply(method, reply);
            }
        }
    }

    /// Calls the tool `name` with `arguments` (`tools/call`) and returns its result unchanged,
    /// `content` and `isError` included.
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, McpError> {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }

    /// [`McpChild::call_tool`] with its own deadline.
    pub fn call_tool_within(&mut self, name: &str, arguments: Value, timeout: Duration) -> Result<Value, McpError> {
        self.request_within("tools/call", json!({"name": name, "arguments": arguments}), timeout)
    }

    /// Every tool the child lists, following `nextCursor` for at most [`McpLimits::max_pages`]
    /// pages. The entries are returned as the child sent them. A list that goes on past the
    /// limit returns [`McpError::TooManyPages`]. Tools whose JSON text adds up to more than
    /// [`McpLimits::max_line_bytes`] return [`McpError::ToolListTooLarge`].
    pub fn list_tools(&mut self) -> Result<Vec<Value>, McpError> {
        let mut tools = Vec::new();
        let mut total = 0usize;
        let mut cursor: Option<String> = None;
        for _ in 0..self.limits.max_pages {
            let params = cursor.take().map_or(Value::Null, |c| json!({"cursor": c}));
            let mut page = self.request("tools/list", params)?;
            let Some(list) = page.get_mut("tools").and_then(Value::as_array_mut) else {
                return Err(self.fail(McpError::Protocol("a `tools/list` result has no `tools` array".into())));
            };
            let mut count = ByteCount(0);
            for tool in list.iter() {
                // Writing into a counter does not fail.
                let _ = serde_json::to_writer(&mut count, tool);
            }
            total = total.saturating_add(count.0);
            if total > self.limits.max_line_bytes {
                return Err(McpError::ToolListTooLarge { limit: self.limits.max_line_bytes });
            }
            tools.append(list);
            cursor = match page.get("nextCursor") {
                None | Some(Value::Null) => return Ok(tools),
                Some(Value::String(next)) => Some(next.clone()),
                Some(_) => return Err(self.fail(McpError::Protocol("a `tools/list` result has a `nextCursor` that is not a string".into()))),
            };
        }
        Err(McpError::TooManyPages { limit: self.limits.max_pages })
    }

    /// Takes the messages already received without waiting: answers the child's requests as
    /// [`McpChild::request_within`] does and returns the notifications kept so far, oldest first.
    /// One call handles at most as many messages as the reader thread queues, so a child that
    /// keeps writing does not keep it running. A failure found here closes the connection, and
    /// [`McpChild::closed`] reports it.
    pub fn take_notifications(&mut self) -> Vec<Value> {
        // The reader's queue and the writer thread's failure message.
        for _ in 0..=READ_QUEUE {
            if self.closed.is_some() {
                break;
            }
            let incoming = match self.rx.try_recv() {
                Ok(incoming) => incoming,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.fail(McpError::Ended);
                    break;
                }
            };
            let Ok(v) = self.receive(incoming) else { break };
            if self.route(v, None).is_err() {
                break;
            }
        }
        self.notifications.drain(..).collect()
    }

    /// Closes the child's stdin, waits up to [`McpLimits::close_grace`] for it to exit, then stops
    /// it, and returns its exit status. For a connection made with [`McpChild::connect`], it ends
    /// the writer thread, which drops the writer once the lines already queued are written, and
    /// returns no status without waiting for that thread. A writer blocked on a server that has
    /// stopped reading stays with the thread until the server drops its reading end.
    pub fn close(mut self) -> Result<CloseOutcome, McpError> {
        self.writer = None;
        let grace = self.limits.close_grace;
        let (status, forced) = match self.child.take() {
            Some(mut child) => match reap(&mut child, grace) {
                Ok((status, forced)) => (Some(status), forced),
                Err(source) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(McpError::Io { what: "waiting for the child", source });
                }
            },
            None => self.exit.map_or((None, false), |(status, forced)| (Some(status), forced)),
        };
        let stderr = match &self.stderr {
            Some(tail) => {
                let _ = tail.done.recv_timeout(grace);
                tail.copy()
            }
            None => Vec::new(),
        };
        Ok(CloseOutcome { status, forced, stderr })
    }

    fn check_open(&self) -> Result<(), McpError> {
        match &self.closed {
            Some(why) => Err(McpError::Closed(why.clone())),
            None => Ok(()),
        }
    }

    fn send(&mut self, msg: &Value) -> Result<(), McpError> {
        let Some(writer) = &self.writer else { return Err(McpError::Closed(self.closed.clone().unwrap_or_default())) };
        let mut line = serde_json::to_vec(msg).map_err(|e| McpError::Protocol(format!("a message could not be written as JSON: {e}")))?;
        line.push(b'\n');
        // Never waits for room: a child that stops reading fills the queue, and the connection is
        // closed then.
        match writer.try_send(line) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(self.fail(McpError::Io {
                what: "writing to the child",
                source: io::Error::new(io::ErrorKind::WouldBlock, "the child stopped reading its input"),
            })),
            Err(TrySendError::Disconnected(_)) => {
                Err(self.fail(McpError::Io { what: "writing to the child", source: io::Error::from(io::ErrorKind::BrokenPipe) }))
            }
        }
    }

    /// Turns what the threads handed over into a message, or closes the connection.
    fn receive(&mut self, incoming: Incoming) -> Result<Value, McpError> {
        Err(self.fail(match incoming {
            Incoming::Message(v) => return Ok(v),
            Incoming::TooLong => McpError::LineTooLong { limit: self.limits.max_line_bytes },
            Incoming::Ended => McpError::Ended,
            Incoming::Invalid(why) => McpError::Protocol(why),
            Incoming::ReadFailed(source) => McpError::Io { what: "reading from the child", source },
            Incoming::WriteFailed(source) => McpError::Io { what: "writing to the child", source },
        }))
    }

    /// Handles one message: answers a request from the child, keeps a notification, skips a
    /// well-formed reply to another id, and returns the reply to `waiting`.
    fn route(&mut self, v: Value, waiting: Option<u64>) -> Result<Option<Value>, McpError> {
        if v.get("id").is_some_and(|id| !matches!(id, Value::String(_) | Value::Number(_) | Value::Null)) {
            return Err(self.fail(McpError::Protocol("a message has an `id` that is not a string, a number or null".into())));
        }
        match (v.get("method"), v.get("id")) {
            (Some(Value::String(method)), Some(id)) => {
                let reply = if method == "ping" {
                    json!({"jsonrpc": "2.0", "id": id, "result": {}})
                } else {
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": METHOD_NOT_FOUND, "message": "Method not found"}})
                };
                self.send(&reply)?;
                Ok(None)
            }
            (Some(Value::String(_)), None) => {
                if self.limits.max_notifications > 0 {
                    if self.notifications.len() >= self.limits.max_notifications {
                        self.notifications.pop_front();
                    }
                    self.notifications.push_back(v);
                }
                Ok(None)
            }
            (Some(_), _) => Err(self.fail(McpError::Protocol("a message has a `method` that is not a string".into()))),
            (None, Some(id)) if waiting.is_some() && id.as_u64() == waiting => Ok(Some(v)),
            (None, Some(_)) if is_reply(&v) => Ok(None),
            (None, Some(_)) => {
                Err(self.fail(McpError::Protocol("a reply to another id does not hold exactly one of `result` and a valid `error`".into())))
            }
            (None, None) => Err(self.fail(McpError::Protocol("a message has neither `method` nor `id`".into()))),
        }
    }

    /// The result of the reply to `method`, or its error.
    fn reply(&mut self, method: &str, mut v: Value) -> Result<Value, McpError> {
        let malformed = || McpError::Protocol(format!("the reply to `{method}` does not hold exactly one of `result` and a valid `error`"));
        let Some(obj) = v.as_object_mut() else { return Err(self.fail(malformed())) };
        match (obj.remove("result"), obj.remove("error")) {
            (Some(result), None) => Ok(result),
            (None, Some(Value::Object(mut error))) => {
                let code = error.get("code").and_then(Value::as_i64);
                let message = error.get("message").and_then(Value::as_str).map(str::to_string);
                match (code, message) {
                    (Some(code), Some(message)) => Err(McpError::Rpc { method: method.to_string(), code, message, data: error.remove("data") }),
                    _ => Err(self.fail(malformed())),
                }
            }
            _ => Err(self.fail(malformed())),
        }
    }

    /// Closes the connection with `err`: ends the writer, which closes the child's stdin, and stops
    /// and reaps the child. A child that closed its output or input gets
    /// [`McpLimits::close_grace`] to exit first.
    fn fail(&mut self, err: McpError) -> McpError {
        if self.closed.is_none() {
            self.closed = Some(err.to_string());
        }
        self.writer = None;
        if let Some(mut child) = self.child.take() {
            let grace = match err {
                McpError::Ended | McpError::Io { .. } => self.limits.close_grace,
                _ => Duration::ZERO,
            };
            match reap(&mut child, grace) {
                Ok(exit) => self.exit = Some(exit),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        err
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        self.writer = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A writer that only counts the bytes written to it.
struct ByteCount(usize);

impl Write for ByteCount {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A JSON-RPC 2.0 message for `method`; `Value::Null` params are left out.
fn message(method: &str, params: Value) -> Map<String, Value> {
    let mut msg = Map::new();
    msg.insert("jsonrpc".into(), json!("2.0"));
    msg.insert("method".into(), json!(method));
    if !params.is_null() {
        msg.insert("params".into(), params);
    }
    msg
}

/// Waits up to `grace` for the child to exit, then stops it. Returns its status and whether it
/// was stopped.
fn reap(child: &mut Child, grace: Duration) -> io::Result<(ExitStatus, bool)> {
    let deadline = Instant::now().checked_add(grace);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false));
        }
        let left = deadline.and_then(|d| d.checked_duration_since(Instant::now())).filter(|left| !left.is_zero());
        let Some(left) = left else { break };
        std::thread::sleep(left.min(EXIT_POLL));
    }
    match child.kill() {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => {}
        Err(e) => return Err(e),
    }
    Ok((child.wait()?, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_line_reader() {
        let mut r = io::Cursor::new(b"abc\nabcdefgh\nxy".to_vec());
        assert_eq!(read_line_capped(&mut r, 4).ok(), Some(Some(b"abc".to_vec())));
        assert!(matches!(read_line_capped(&mut r, 4), Err(LineError::TooLong)));
        let mut r = io::Cursor::new(b"xy".to_vec());
        assert_eq!(read_line_capped(&mut r, 4).ok(), Some(Some(b"xy".to_vec())), "a last line without a newline");
        assert_eq!(read_line_capped(&mut r, 4).ok(), Some(None));
    }

    #[test]
    fn an_over_long_line_is_rejected_before_its_newline_arrives() {
        // A reader that fails after the first bytes: a reader that waited for the newline would
        // return the error instead.
        struct Then(io::Cursor<Vec<u8>>);
        impl Read for Then {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                match self.0.read(buf)? {
                    0 => Err(io::Error::other("no newline yet")),
                    n => Ok(n),
                }
            }
        }
        let mut r = BufReader::new(Then(io::Cursor::new(vec![b'x'; 64])));
        assert!(matches!(read_line_capped(&mut r, 16), Err(LineError::TooLong)));
    }

    #[test]
    fn timeout_variable_values() {
        let parse = |v: &str| parse_timeout_secs(TIMEOUT_VAR, OsStr::new(v)).ok();
        assert_eq!(parse("5"), Some(Duration::from_secs(5)));
        assert_eq!(parse("3600"), Some(MAX_TIMEOUT));
        for bad in ["", "0", "3601", "-1", "+5", " 5", "5s", "1.5", "99999999999999999999999"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_bad_timeout_variable_name_is_a_setup_error_not_a_panic() {
        for name in ["", "A=B", "A\0B"] {
            assert!(matches!(McpLimits::default().timeouts_from_var(name), Err(McpError::Setup(_))), "{name:?}");
        }
    }
}
