//! The connection file: where an app's control server listens for agents.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The connection file in the app's settings folder, present while the app accepts agents.
pub const CONNECTION_FILE: &str = "agent-connection.json";
/// The token file the app keeps in its settings folder for agent connections.
pub const TOKEN_FILE: &str = "agent-control.token";
/// How long [`probe`] waits for a connection to be accepted.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// The version of the [`Connection`] file format.
const CONNECTION_VERSION: u32 = 1;
/// The largest connection file read; anything bigger is not one.
const MAX_CONNECTION_BYTES: u64 = 1 << 20;

/// What an app publishes in [`CONNECTION_FILE`]: `{"version", "pid", "port", "tokenFile", "app"}`.
/// Holds no secret: the token stays in its own owner-only file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    /// The file format's version (1).
    pub version: u32,
    /// The app's process id: only that process removes the file.
    pub pid: u32,
    /// The loopback port of the control server.
    pub port: u16,
    /// The file holding the token.
    pub token_file: PathBuf,
    /// The app's version, for diagnostics.
    pub app: String,
}

impl Connection {
    /// This process's connection on `port`, naming `app` (the app's version string).
    pub fn new(port: u16, token_file: PathBuf, app: impl Into<String>) -> Self {
        Self { version: CONNECTION_VERSION, pid: std::process::id(), port, token_file, app: app.into() }
    }

    /// `127.0.0.1:<port>`.
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

fn io_error(path: &Path, e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(format!("{}: {e}", path.display()))
}

/// Publishes `c` in `dir` (made if missing), written atomically so a reader never sees half a
/// file.
pub fn write_connection(dir: &Path, c: &Connection) -> std::io::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    let path = dir.join(CONNECTION_FILE);
    let text = serde_json::to_string_pretty(c).map_err(|e| io_error(&path, e))?;
    craft_atomic::atomic_write(&path, text.as_bytes()).map_err(|e| io_error(&path, e))
}

/// The connection published in `dir`, if any. A file of another version, with port 0, or one
/// that does not parse (invalid UTF-8 included) counts as none: the app rewrites it when it next
/// starts listening. Anything but a regular file (a FIFO would block the read) or a file over
/// 1 MiB is an error, and left alone.
pub fn read_connection(dir: &Path) -> std::io::Result<Option<Connection>> {
    let path = dir.join(CONNECTION_FILE);
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(&path, e)),
    };
    if !meta.is_file() {
        return Err(io_error(&path, "not a regular file"));
    }
    if meta.len() > MAX_CONNECTION_BYTES {
        return Err(io_error(&path, format!("larger than {MAX_CONNECTION_BYTES} bytes")));
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(&path, e)),
    };
    Ok(serde_json::from_slice::<Connection>(&bytes).ok().filter(|c| c.version == CONNECTION_VERSION && c.port != 0))
}

/// Removes the connection file in `dir` if process `pid` wrote it (another instance may have
/// published its own since). Errors are ignored: a stale file only fails [`probe`].
pub fn remove_connection(dir: &Path, pid: u32) {
    if let Ok(Some(c)) = read_connection(dir)
        && c.pid == pid
    {
        let _ = std::fs::remove_file(dir.join(CONNECTION_FILE));
    }
}

/// Removes the connection file in `dir` when nothing listens on its port any more (an app that
/// crashed or quit without removing it), or when it does not read as a connection. A live
/// instance's file is kept, and so is anything but a regular file. Errors are ignored.
pub fn remove_stale_connection(dir: &Path) {
    let path = dir.join(CONNECTION_FILE);
    let stale = match read_connection(dir) {
        Ok(Some(c)) => !probe(c.port, PROBE_TIMEOUT),
        Ok(None) => path.is_file(),
        Err(_) => false,
    };
    if stale {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether something accepts connections on loopback `port` within `timeout`; port 0 never does.
pub fn probe(port: u16, timeout: Duration) -> bool {
    port != 0 && TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), timeout).is_ok()
}
