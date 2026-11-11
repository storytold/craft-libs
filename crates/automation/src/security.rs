//! Tokens, the `auth` handshake and the limits every transport applies.
//!
//! A connection's first line must be `{"id": 1, "method": "auth", "params": {"token": "<64 hex>"}}`
//! within [`AUTH_DEADLINE`]; until then nothing else is answered. A request line is at most
//! [`MAX_REQUEST_BYTES`]; a transport serves at most [`MAX_CONNECTIONS`] at once; a connection
//! idle for [`IO_TIMEOUT`] is closed. Tokens are 64 hex characters in either case, kept
//! lower-cased by the server and compared in constant time with hex letters folded. None of this depends on the transport: the TCP server and a test channel share it.

use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The handshake method name.
pub const AUTH_METHOD: &str = "auth";
/// Longest request line, in bytes; a longer one gets an error reply and the connection closes.
pub const MAX_REQUEST_BYTES: usize = 1 << 20;
/// Connections served at once; the next one gets `connection limit reached` and is closed.
pub const MAX_CONNECTIONS: usize = 16;
/// Steps a `batch` request may carry.
pub const MAX_BATCH_STEPS: usize = 256;
/// Read and write timeout on a connection.
pub const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// A connection that has not authenticated this long after connecting is closed, however much it
/// sent meanwhile.
pub const AUTH_DEADLINE: Duration = Duration::from_secs(10);
/// Token length in hex characters (32 random bytes).
pub const TOKEN_HEX_LEN: usize = 64;

/// What the control channel refuses or cannot set up. `Display` is the text a user or agent
/// reads. PhotoCraft's `photocraft_automation::AutomationError` has the same two variants with the
/// same texts (`crates/automation/src/lib.rs:35`), so it wraps these one to one.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AutomationError {
    /// A bad token, a token conflict, a missing client token; the message as is.
    #[error("{0}")]
    BadRequest(String),
    /// A token file that can't be read, created or trusted; `I/O: <path>: <reason>`.
    #[error("I/O: {0}")]
    Io(String),
}

/// How a bounded line read ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineRead {
    /// `line` holds one complete line (with its newline).
    Line,
    /// The stream ended before a newline; `line` holds what arrived.
    Eof,
    /// More than [`MAX_REQUEST_BYTES`] arrived without a newline; `line` holds the bytes read so
    /// far, lossily decoded, and the rest of the line is still unread unless `line` ends with a
    /// newline (see [`discard_rest_of_line`]).
    TooLong,
}

/// Read one line into `line` (cleared first), never buffering more than [`MAX_REQUEST_BYTES`]
/// plus one byte. A line within the limit that is not UTF-8 is an `InvalidData` error, and the
/// whole line has been consumed.
pub fn read_bounded_line(reader: &mut impl BufRead, line: &mut String) -> std::io::Result<LineRead> {
    line.clear();
    let mut bytes = Vec::new();
    let n = std::io::Read::take(&mut *reader, (MAX_REQUEST_BYTES + 1) as u64).read_until(b'\n', &mut bytes)?;
    if n == 0 {
        Ok(LineRead::Eof)
    } else if n > MAX_REQUEST_BYTES {
        // A cut can split a multi-byte character; the length, not the text, decides.
        line.push_str(&String::from_utf8_lossy(&bytes));
        Ok(LineRead::TooLong)
    } else {
        *line = String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(LineRead::Line)
    }
}

/// After [`LineRead::TooLong`]: skip to the next newline so the connection can go on.
pub fn discard_rest_of_line(reader: &mut impl BufRead) -> std::io::Result<()> {
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(());
        }
        if let Some(end) = buf.iter().position(|&b| b == b'\n') {
            reader.consume(end + 1);
            return Ok(());
        }
        let n = buf.len();
        reader.consume(n);
    }
}

/// A fresh token: 32 bytes from the OS randomness source (`getrandom::fill`) as lower-case hex.
/// A failure is [`AutomationError::Io`] with `cannot generate control token: {e}` (PhotoCraft
/// uses its app-level `Other` variant for it).
pub fn generate_token() -> Result<String, AutomationError> {
    let mut bytes = [0u8; TOKEN_HEX_LEN / 2];
    getrandom::fill(&mut bytes).map_err(|e| AutomationError::Io(format!("cannot generate control token: {e}")))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(TOKEN_HEX_LEN);
    for byte in bytes {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(token)
}

/// Exactly [`TOKEN_HEX_LEN`] ASCII hex digits, upper or lower case; otherwise
/// `BadRequest("control token must contain exactly 64 hexadecimal characters")`.
pub fn validate_token(token: &str) -> Result<(), AutomationError> {
    if token.len() != TOKEN_HEX_LEN || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AutomationError::BadRequest("control token must contain exactly 64 hexadecimal characters".into()));
    }
    Ok(())
}

/// Constant-time comparison (no early exit on a mismatching byte); false unless both are
/// [`TOKEN_HEX_LEN`] long. Only `A`–`Z` fold to lower case, so a client sending the token as it
/// was configured authenticates against the server's lower-cased copy, and a non-hex byte never
/// matches a hex digit.
pub fn token_matches(expected: &str, supplied: &str) -> bool {
    if expected.len() != TOKEN_HEX_LEN || supplied.len() != TOKEN_HEX_LEN {
        return false;
    }
    expected.bytes().zip(supplied.bytes()).fold(0u8, |different, (a, b)| different | (a.to_ascii_lowercase() ^ b.to_ascii_lowercase())) == 0
}

/// The reply to a connection's first line: `(reply, authenticated)`. A match gets
/// `{"id", "ok": true, "result": {"authenticated": true}}`. A line that is not JSON gets
/// `{"id": null, "ok": false, "error": "authentication required"}`; one that is not an `auth`
/// request, or whose `params.token` does not match, gets the same with its `id`; then the
/// connection closes.
pub fn authentication_reply(line: &str, expected_token: &str) -> (Value, bool) {
    let Ok(req) = serde_json::from_str::<Value>(line) else {
        return (json!({"id": null, "ok": false, "error": "authentication required"}), false);
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let supplied = req.get("params").and_then(|p| p.get("token")).and_then(Value::as_str).unwrap_or("");
    let ok = req.get("method").and_then(Value::as_str) == Some(AUTH_METHOD) && token_matches(expected_token, supplied);
    if ok {
        (json!({"id": id, "ok": true, "result": {"authenticated": true}}), true)
    } else {
        (json!({"id": id, "ok": false, "error": "authentication required"}), false)
    }
}

/// An `auth` request on an already authenticated connection gets
/// `{"id", "ok": false, "error": "already authenticated"}`; any other line (or one that is not
/// JSON) is `None` and goes to the method dispatcher as usual.
pub fn repeat_auth_reply(line: &str) -> Option<Value> {
    let req: Value = serde_json::from_str(line).ok()?;
    if req.get("method").and_then(Value::as_str) != Some(AUTH_METHOD) {
        return None;
    }
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    Some(json!({"id": id, "ok": false, "error": "already authenticated"}))
}

fn token_conflict() -> AutomationError {
    AutomationError::BadRequest("use either a control token or a control token file, not both".into())
}

/// Validates what was opened, not just the name: the file must be a regular file that no other
/// local principal can read, so a group- or world-readable token is refused rather than trusted.
fn read_token_file(path: &Path) -> Result<String, AutomationError> {
    let io = |e: std::io::Error| AutomationError::Io(format!("{}: {e}", path.display()));
    let mut file = std::fs::File::open(path).map_err(io)?;
    let meta = file.metadata().map_err(io)?;
    if !meta.is_file() {
        return Err(AutomationError::Io(format!("{}: control token file is not a regular file", path.display())));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o077 != 0 {
            return Err(AutomationError::Io(format!(
                "{}: control token file is readable by group or others (mode {:o}); restrict it to its owner (e.g. chmod 600)",
                path.display(),
                meta.mode() & 0o777
            )));
        }
    }
    let token = std::io::read_to_string(&mut file).map_err(io)?;
    let token = token.trim().to_owned();
    validate_token(&token)?;
    Ok(token)
}

fn create_token_file(path: &Path, token: &str) -> Result<(), AutomationError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| AutomationError::Io(format!("{}: {e}", parent.display())))?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))?;
    writeln!(file, "{token}").map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))
}

/// The server's token, lower-cased. Both given is
/// `BadRequest("use either a control token or a control token file, not both")`. `supplied` is
/// validated. Neither given: a generated token. A `token_file` that exists is read: it must be a
/// regular file (`I/O: <path>: control token file is not a regular file`) and on Unix have no
/// group or other permission bits (`I/O: <path>: control token file is readable by group or
/// others (mode {mode & 0o777:o}); restrict it to its owner (e.g. chmod 600)`), and its trimmed
/// content is validated. A missing one is created with a generated token (parent directories
/// created, `create_new`, mode 0600 on Unix, the token and a newline); losing that create race to
/// another process reads the winner's file instead.
pub fn server_token(supplied: Option<&str>, token_file: Option<&Path>) -> Result<String, AutomationError> {
    if supplied.is_some() && token_file.is_some() {
        return Err(token_conflict());
    }
    if let Some(token) = supplied {
        validate_token(token)?;
        return Ok(token.to_ascii_lowercase());
    }
    let token = generate_token()?;
    let Some(path) = token_file else {
        return Ok(token);
    };
    match read_token_file(path) {
        Ok(existing) => Ok(existing.to_ascii_lowercase()),
        Err(AutomationError::Io(_)) if !path.exists() => match create_token_file(path, &token) {
            Ok(()) => Ok(token),
            // Another process won the create-new race.
            Err(AutomationError::Io(_)) if path.exists() => read_token_file(path).map(|v| v.to_ascii_lowercase()),
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    }
}

/// A client's token, lower-cased: `supplied` (validated) or the token file's content, read and
/// checked as [`server_token`] reads an existing one. Both given is the same conflict error.
/// Neither given is `BadRequest("bridge mode needs {flags.token}, {flags.token_file}, {env.token},
/// or {env.token_file}")` (PhotoCraft: `bridge mode needs --control-token, --control-token-file,
/// PHOTOCRAFT_CONTROL_TOKEN, or PHOTOCRAFT_CONTROL_TOKEN_FILE`); a client never generates one.
pub fn client_token(
    supplied: Option<&str>,
    token_file: Option<&Path>,
    flags: &crate::args::FlagNames,
    env: &crate::args::EnvNames,
) -> Result<String, AutomationError> {
    if supplied.is_some() && token_file.is_some() {
        return Err(token_conflict());
    }
    if let Some(token) = supplied {
        validate_token(token)?;
        return Ok(token.to_ascii_lowercase());
    }
    match token_file {
        Some(path) => read_token_file(path).map(|v| v.to_ascii_lowercase()),
        None => {
            Err(AutomationError::BadRequest(format!("bridge mode needs {}, {}, {}, or {}", flags.token, flags.token_file, env.token, env.token_file)))
        }
    }
}

/// Counts live connections against [`MAX_CONNECTIONS`]; a [`ConnectionPermit`] releases one when
/// dropped.
#[derive(Debug, Default)]
pub struct ConnectionLimiter {
    live: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    max: usize,
}

impl ConnectionLimiter {
    /// A limiter that allows at most `max` live permits.
    pub fn new(max: usize) -> Self {
        Self { live: std::sync::Arc::new(AtomicUsize::new(0)), max }
    }

    /// A permit, or `None` at the limit.
    pub fn try_acquire(&self) -> Option<ConnectionPermit> {
        let mut current = self.live.load(Ordering::Acquire);
        loop {
            if current >= self.max {
                return None;
            }
            match self.live.compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(ConnectionPermit { live: std::sync::Arc::clone(&self.live) }),
                Err(actual) => current = actual,
            }
        }
    }
}

/// One live connection's share of [`ConnectionLimiter`]; dropping it frees the slot.
#[derive(Debug)]
pub struct ConnectionPermit {
    live: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Sets [`IO_TIMEOUT`] as `stream`'s read and write timeout (nothing else).
pub fn configure_stream(stream: &std::net::TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))
}

/// A read wrapper that fails once [`AUTH_DEADLINE`] passes without [`Self::authenticated`] being
/// called, whatever the client sends meanwhile; after that only [`IO_TIMEOUT`] applies.
#[derive(Debug)]
pub struct AuthDeadlineStream {
    inner: std::net::TcpStream,
    deadline: Option<std::time::Instant>,
}

impl AuthDeadlineStream {
    /// Wraps `inner`; reads fail once `auth_deadline` has passed unauthenticated.
    pub fn new(inner: std::net::TcpStream, auth_deadline: Duration) -> Self {
        // A limit too large to represent expires at once rather than never.
        let now = Instant::now();
        Self { inner, deadline: Some(now.checked_add(auth_deadline).unwrap_or(now)) }
    }

    /// The handshake succeeded: lift the deadline and restore the idle timeout.
    pub fn authenticated(&mut self) -> std::io::Result<()> {
        self.deadline = None;
        self.inner.set_read_timeout(Some(IO_TIMEOUT))
    }
}

impl std::io::Read for AuthDeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "not authenticated in time"));
            }
            // Wait no longer than what is left, so a silent client is dropped at the deadline too.
            self.inner.set_read_timeout(Some(left.min(IO_TIMEOUT)))?;
        }
        self.inner.read(buf)
    }
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod security_tests;
