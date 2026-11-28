//! Tokens, the authentication handshake and the limits every transport applies.
//!
//! A connection authenticates within [`AUTH_DEADLINE`] ([`ServerHandshake`]): with the challenge
//! handshake, which never sends the token, or with the one-frame
//! `{"id": 1, "method": "auth", "params": {"token": "<64 hex>"}}`. Until then nothing else is
//! answered. A request line is at most
//! [`MAX_REQUEST_BYTES`]; a transport serves at most [`MAX_CONNECTIONS`] at once; a connection
//! idle for [`IO_TIMEOUT`] is closed. Tokens are 64 hex characters in either case, kept
//! lower-cased by the server and compared in constant time with hex letters folded. None of this depends on the transport: the TCP server and a test channel share it.

#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::io::BufRead;
#[cfg(not(windows))]
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

/// The request that authenticates a connection: with the token itself, or with the client's
/// proof after [`CHALLENGE_METHOD`].
pub const AUTH_METHOD: &str = "auth";
/// The first frame of challenge authentication: it carries a client nonce, never the token.
pub const CHALLENGE_METHOD: &str = "auth.challenge";
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
    random_hex("control token")
}

/// A fresh nonce for challenge authentication ([`CHALLENGE_METHOD`]), made as
/// [`generate_token`] makes a token.
pub fn generate_nonce() -> Result<String, AutomationError> {
    random_hex("authentication nonce")
}

/// 32 bytes from the OS randomness source as lower-case hex.
fn random_hex(what: &str) -> Result<String, AutomationError> {
    let mut bytes = [0u8; TOKEN_HEX_LEN / 2];
    getrandom::fill(&mut bytes).map_err(|e| AutomationError::Io(format!("cannot generate {what}: {e}")))?;
    Ok(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Whether `s` has the shape of a token or a nonce: exactly [`TOKEN_HEX_LEN`] hex digits.
pub fn is_hex256(s: &str) -> bool {
    s.len() == TOKEN_HEX_LEN && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Exactly [`TOKEN_HEX_LEN`] ASCII hex digits, upper or lower case; otherwise
/// `BadRequest("control token must contain exactly 64 hexadecimal characters")`.
pub fn validate_token(token: &str) -> Result<(), AutomationError> {
    if !is_hex256(token) {
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

/// Which side of the challenge handshake a proof comes from. Each side's proof is keyed by the
/// token over both nonces under its own label, so one side's proof never passes for the other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofRole {
    /// The server's answer to `auth.challenge` (`serverProof`).
    Server,
    /// The client's `auth {proof}`.
    Client,
}

/// HMAC-SHA256 keyed by the lower-cased token over `<label>\n<client nonce>\n<server nonce>`
/// (nonces lower-cased), as 64 lower-case hex characters. The labels are
/// `photocraft-control-v1 server` and `photocraft-control-v1 client`.
pub fn handshake_proof(token: &str, role: ProofRole, client_nonce: &str, server_nonce: &str) -> Result<String, AutomationError> {
    // The wire label of protocol version 1, which PhotoCraft's clients and scripts already speak:
    // changing these bytes is a protocol version change.
    let label: &[u8] = match role {
        ProofRole::Server => b"photocraft-control-v1 server",
        ProofRole::Client => b"photocraft-control-v1 client",
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(token.to_ascii_lowercase().as_bytes())
        .map_err(|e| AutomationError::Io(format!("cannot key the authentication proof: {e}")))?;
    for part in [label, b"\n", client_nonce.to_ascii_lowercase().as_bytes(), b"\n", server_nonce.to_ascii_lowercase().as_bytes()] {
        mac.update(part);
    }
    Ok(hex(&mac.finalize().into_bytes()))
}

/// Whether `supplied` is `role`'s proof for these nonces, compared without an early exit.
pub fn proof_matches(token: &str, role: ProofRole, client_nonce: &str, server_nonce: &str, supplied: &str) -> bool {
    handshake_proof(token, role, client_nonce, server_nonce).is_ok_and(|expected| token_matches(&expected, supplied))
}

/// Where a connection's authentication stands after one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStep {
    /// Authenticated: dispatch the frames that follow.
    Authenticated,
    /// A challenge was answered: the next frame must be `auth` with the client's proof.
    Challenged,
    /// Refused: send the reply, then close the connection.
    Rejected,
}

/// The server side of a connection's authentication, one per connection. It accepts either
///
/// - the challenge handshake: `auth.challenge {clientNonce}` is answered with
///   `{serverNonce, serverProof}`, then `auth {proof}` authenticates. The token never crosses the
///   wire, and the client learns that the server holds the token before it reveals anything
///   derived from it; or
/// - the one-frame `auth {token}`, which existing clients and scripts send.
///
/// A refusal is `{"id", "ok": false, "error": "authentication required"}` (`"id": null` for a line
/// that is not JSON); success is `{"id", "ok": true, "result": {"authenticated": true}}`. No control
/// method is answered before authentication.
#[derive(Debug, Default)]
pub struct ServerHandshake {
    /// The client and server nonces of the challenge this connection was given, until its `auth`.
    challenge: Option<(String, String)>,
}

impl ServerHandshake {
    /// A handshake for a freshly accepted connection: no challenge issued yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers one frame received before authentication. Never dispatches a control method.
    pub fn reply(&mut self, line: &str, token: &str) -> (Value, AuthStep) {
        let refused = |id: Value| (json!({"id": id, "ok": false, "error": "authentication required"}), AuthStep::Rejected);
        let authenticated = |id: Value| (json!({"id": id, "ok": true, "result": {"authenticated": true}}), AuthStep::Authenticated);
        let Ok(req) = serde_json::from_str::<Value>(line) else {
            return refused(Value::Null);
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let param = |key: &str| req.get("params").and_then(|p| p.get(key)).and_then(Value::as_str);
        match (req.get("method").and_then(Value::as_str), self.challenge.take()) {
            (Some(CHALLENGE_METHOD), None) => {
                let Some(client_nonce) = param("clientNonce").filter(|n| is_hex256(n)).map(str::to_ascii_lowercase) else {
                    return refused(id);
                };
                let Ok(server_nonce) = generate_nonce() else {
                    return (json!({"id": id, "ok": false, "error": "cannot generate an authentication challenge"}), AuthStep::Rejected);
                };
                let Ok(server_proof) = handshake_proof(token, ProofRole::Server, &client_nonce, &server_nonce) else {
                    return refused(id);
                };
                let reply = json!({"id": id, "ok": true, "result": {"serverNonce": server_nonce, "serverProof": server_proof}});
                self.challenge = Some((client_nonce, server_nonce));
                (reply, AuthStep::Challenged)
            }
            // After a challenge only the proof authenticates; the token is never needed.
            (Some(AUTH_METHOD), Some((client_nonce, server_nonce))) => match param("proof") {
                Some(proof) if proof_matches(token, ProofRole::Client, &client_nonce, &server_nonce, proof) => authenticated(id),
                _ => refused(id),
            },
            (Some(AUTH_METHOD), None) if token_matches(token, param("token").unwrap_or("")) => authenticated(id),
            _ => refused(id),
        }
    }
}

/// An `auth` or `auth.challenge` request on an already authenticated connection gets
/// `{"id", "ok": false, "error": "already authenticated"}`; any other line (or one that is not
/// JSON) is `None` and goes to the method dispatcher as usual.
pub fn repeat_auth_reply(line: &str) -> Option<Value> {
    let req: Value = serde_json::from_str(line).ok()?;
    if !matches!(req.get("method").and_then(Value::as_str), Some(AUTH_METHOD | CHALLENGE_METHOD)) {
        return None;
    }
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    Some(json!({"id": id, "ok": false, "error": "already authenticated"}))
}

fn token_conflict() -> AutomationError {
    AutomationError::BadRequest("use either a control token or a control token file, not both".into())
}

/// The most a Windows token file read takes: a token, its newline and room for stray whitespace.
#[cfg(windows)]
const TOKEN_FILE_MAX_BYTES: u64 = 4096;

/// Validates what was opened, not just the name: the file must be a regular file that no other
/// local principal can read, so a group- or world-readable token is refused rather than trusted.
fn read_token_file(path: &Path) -> Result<String, AutomationError> {
    let io = |e: std::io::Error| AutomationError::Io(format!("{}: {e}", path.display()));
    let file = std::fs::File::open(path).map_err(io)?;
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
    // A Windows file inherits its folder's access list, so the owner and the access list are read with the contents
    // through one handle: nothing swapped in at the path between the check and the read gets through.

    #[cfg(windows)]
    let token = {
        use craft_platform::owner_only;
        drop(file);
        let me = owner_only::current_user_sid().map_err(io)?;
        let (sddl, bytes) = owner_only::read_with_sddl(path, TOKEN_FILE_MAX_BYTES).map_err(io)?;
        if let Some(problem) = owner_only::foreign_access(&sddl, &me) {
            return Err(AutomationError::Io(format!(
                "{}: control token file can be reached by others: {problem}; restrict it to its owner",
                path.display()
            )));
        }
        String::from_utf8(bytes).map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))?
    };
    #[cfg(not(windows))]
    let token = std::io::read_to_string(&file).map_err(io)?;
    let token = token.trim().to_owned();
    validate_token(&token)?;
    Ok(token)
}

/// Create the token file `path` holding `token` and a newline. An error keeps the kind of the one
/// that caused it, with the path in its message.
fn create_token_file(path: &Path, token: &str) -> std::io::Result<()> {
    let at = |p: &Path, e: std::io::Error| std::io::Error::new(e.kind(), format!("{}: {e}", p.display()));
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| at(parent, e))?;
    }
    // On Windows the file is created with an access list of its user and SYSTEM only, so it never has its folder's
    // access list, and the token is written through the handle that created it.
    #[cfg(windows)]
    {
        use craft_platform::owner_only;
        owner_only::current_user_sid()
            .and_then(|me| owner_only::create_private_file(path, &me, format!("{token}\n").as_bytes()))
            .map_err(|e| std::io::Error::new(e.kind(), format!("{}: cannot create the control token file for its owner only: {e}", path.display())))
    }
    #[cfg(not(windows))]
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|e| at(path, e))?;
        writeln!(file, "{token}").map_err(|e| at(path, e))
    }
}

/// The server's token, lower-cased. Both given is
/// `BadRequest("use either a control token or a control token file, not both")`. `supplied` is
/// validated. Neither given: a generated token. A `token_file` that exists is read: it must be a
/// regular file (`I/O: <path>: control token file is not a regular file`) and on Unix have no
/// group or other permission bits (`I/O: <path>: control token file is readable by group or
/// others (mode {mode & 0o777:o}); restrict it to its owner (e.g. chmod 600)`), on Windows be
/// owned by the user with an access list of the user and SYSTEM only (`I/O: <path>: control token
/// file can be reached by others: <why>; restrict it to its owner`), and its trimmed content is
/// validated. A missing one is created with a generated token (parent directories created,
/// `create_new`, mode 0600 on Unix, the user and SYSTEM only on Windows, the token and a
/// newline); losing that create race to another process reads the winner's file instead. On
/// Windows a create that does not finish in time fails (`I/O: <path>: cannot create the control
/// token file for its owner only: <why>`), and a file it left at the path is not read as a winner's.
pub fn server_token(supplied: Option<&str>, token_file: Option<&Path>) -> Result<String, AutomationError> {
    server_token_with(supplied, token_file, create_token_file)
}

/// [`server_token`], with `create` making a missing token file.
fn server_token_with(
    supplied: Option<&str>,
    token_file: Option<&Path>,
    create: impl FnOnce(&Path, &str) -> std::io::Result<()>,
) -> Result<String, AutomationError> {
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
        Err(AutomationError::Io(_)) if !path.exists() => match create(path, &token) {
            Ok(()) => Ok(token),
            // Another process won the create-new race. A file left by a create that ran out of time is this one's.
            Err(e) if e.kind() != std::io::ErrorKind::TimedOut && path.exists() => read_token_file(path).map(|v| v.to_ascii_lowercase()),
            Err(e) => Err(AutomationError::Io(e.to_string())),
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
/// released or dropped.
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
                Ok(_) => return Some(ConnectionPermit { live: std::sync::Arc::clone(&self.live), released: false }),
                Err(actual) => current = actual,
            }
        }
    }
}

/// One live connection's share of [`ConnectionLimiter`]; [`Self::release`] or dropping it frees
/// the slot.
#[derive(Debug)]
pub struct ConnectionPermit {
    live: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// The slot was freed by [`Self::release`], so `Drop` must not free it again.
    released: bool,
}

impl ConnectionPermit {
    /// Frees the slot once, atomically, without `Drop` freeing it again; answers whether this was
    /// the last live connection. One atomic decrement decides it, so of two connections ending
    /// together exactly one sees itself as the last. A connection admitted between the decrement and
    /// `end_injection` may lose input it sends before authenticating; it sends none.
    pub fn release(mut self) -> bool {
        self.released = true;
        self.live.fetch_sub(1, Ordering::AcqRel) == 1
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        if !self.released {
            self.live.fetch_sub(1, Ordering::AcqRel);
        }
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
pub(crate) mod security_tests;
