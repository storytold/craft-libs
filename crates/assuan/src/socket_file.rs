//! The files that may stand where an Assuan socket is named.
//!
//! - Windows has no Unix domain sockets for Assuan to use, so the server listens on a TCP port
//!   of the loopback address and writes a file where the socket would be: the port in decimal, a
//!   line feed, and a 16-byte nonce. A client connects to `127.0.0.1` on that port and sends the
//!   nonce before anything else; the server drops a connection that doesn't.
//! - On any system the name may hold a redirect instead: a regular file whose first line is
//!   `%Assuan%` and which names the real socket in a line `socket=<name>`.
//!
//! [`parse`] reads either, from the file's bytes; opening the connection is the program's.

/// Longest socket file read.
pub const MAX_FILE: usize = 4 * 1024;
/// Size of the nonce.
pub const NONCE_LEN: usize = 16;

/// What a socket file says.
#[derive(Clone, PartialEq, Eq)]
pub enum SocketFile {
    /// Connect to `127.0.0.1:port` and send `nonce` first.
    Loopback { port: u16, nonce: [u8; NONCE_LEN] },
    /// The socket is this one instead (an absolute name).
    Redirect(String),
}

impl std::fmt::Debug for SocketFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The nonce is what lets a program in: never shown.
            SocketFile::Loopback { port, .. } => f.debug_struct("Loopback").field("port", port).finish_non_exhaustive(),
            SocketFile::Redirect(to) => f.debug_tuple("Redirect").field(to).finish(),
        }
    }
}

/// Why a socket file couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SocketFileError {
    #[error("the socket file is too big")]
    TooBig,
    #[error("the socket file names no port")]
    NoPort,
    #[error("the socket file's nonce isn't 16 bytes")]
    BadNonce,
    #[error("the socket file's redirect names no usable socket")]
    BadRedirect,
}

/// Read a socket file.
pub fn parse(file: &[u8]) -> Result<SocketFile, SocketFileError> {
    if file.len() > MAX_FILE {
        return Err(SocketFileError::TooBig);
    }
    if let Some(rest) = file.strip_prefix(b"%Assuan%\n").or_else(|| file.strip_prefix(b"%Assuan%\r\n")) {
        return redirect(rest);
    }
    let newline = file.iter().position(|b| *b == b'\n').ok_or(SocketFileError::NoPort)?;
    let digits = file.get(..newline).ok_or(SocketFileError::NoPort)?;
    let digits = digits.strip_suffix(b"\r").unwrap_or(digits);
    if digits.is_empty() || digits.len() > 5 || !digits.iter().all(u8::is_ascii_digit) {
        return Err(SocketFileError::NoPort);
    }
    let port = std::str::from_utf8(digits).ok().and_then(|d| d.parse::<u16>().ok()).filter(|p| *p != 0).ok_or(SocketFileError::NoPort)?;
    let nonce = file.get(newline.saturating_add(1)..).ok_or(SocketFileError::BadNonce)?;
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| SocketFileError::BadNonce)?;
    Ok(SocketFile::Loopback { port, nonce })
}

fn redirect(rest: &[u8]) -> Result<SocketFile, SocketFileError> {
    let text = std::str::from_utf8(rest).map_err(|_| SocketFileError::BadRedirect)?;
    let name = text.lines().find_map(|l| l.strip_prefix("socket=")).map(str::trim_end).ok_or(SocketFileError::BadRedirect)?;
    // An absolute name on this computer, of printable characters; a variable to expand (`${…}`)
    // is not followed, nor a name on another computer (`//host/share`, `\\host\share`: on
    // Windows, opening one would reach out over SMB), nor a name with a drive given after a
    // slash (`/C:/…`, `\\?\…`).
    // A drive letter (`C:\…`, `C:/…`) is a name on this computer only on Windows.
    let drive = cfg!(windows)
        && name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && (name.get(1..3) == Some(":\\") || name.get(1..3) == Some(":/"));
    let absolute = (name.starts_with('/') && !name.starts_with("//") && !name.contains('\\')) || drive;
    let remote = name.starts_with("//") || name.starts_with("\\\\") || name.starts_with("/\\") || name.starts_with("\\/");
    if !absolute || remote || name.contains("${") || name.chars().any(char::is_control) || name.len() > 1024 {
        return Err(SocketFileError::BadRedirect);
    }
    Ok(SocketFile::Redirect(name.to_string()))
}
