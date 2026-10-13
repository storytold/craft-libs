//! A deliberately small HTTP/1.1 server: one request per connection, handled one at a time,
//! small bounded requests, `Connection: close`. `craft-updated` only ever talks to the reverse
//! proxy in front of it (which buffers slow clients) and serves a handful of JSON requests, so
//! nothing more is needed, and every limit is explicit.

use super::Daemon;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// Largest request line plus headers.
pub const MAX_HEAD_BYTES: usize = 8 * 1024;
/// Largest body (a sign-in is well under 1 KiB).
pub const MAX_BODY_BYTES: usize = 4 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// The path without the query string.
    pub path: String,
    pub authorization: Option<String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// JSON, or empty for `204`.
    pub body: String,
}

impl Response {
    pub fn json(status: u16, body: &impl serde::Serialize) -> Response {
        Response { status, body: serde_json::to_string(body).unwrap_or_else(|_| "{}".into()) }
    }

    pub fn error(status: u16, message: &str) -> Response {
        Response::json(status, &crate::protocol::ErrorReply { error: message.into() })
    }
}

/// Reads one request. `Err` carries the status to answer with.
pub fn read_request(stream: impl Read) -> Result<Request, u16> {
    let mut reader = BufReader::new(stream.take((MAX_HEAD_BYTES + MAX_BODY_BYTES) as u64));
    let mut head_bytes = 0usize;
    let mut line = String::new();
    let mut next_line = |reader: &mut BufReader<_>, line: &mut String| -> Result<(), u16> {
        line.clear();
        let n = reader.read_line(line).map_err(|_| 400u16)?;
        head_bytes += n;
        if n == 0 || head_bytes > MAX_HEAD_BYTES {
            return Err(if n == 0 { 400 } else { 431 });
        }
        if !line.ends_with('\n') {
            return Err(431);
        }
        Ok(())
    };

    next_line(&mut reader, &mut line)?;
    let mut parts = line.trim_end().split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(400);
    };
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") || method.is_empty() || !target.starts_with('/') {
        return Err(400);
    }
    let mut request =
        Request { method: method.to_string(), path: target.split(['?', '#']).next().unwrap_or_default().to_string(), ..Request::default() };

    let mut content_length = 0usize;
    loop {
        next_line(&mut reader, &mut line)?;
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').ok_or(400u16)?;
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                content_length = value.parse().map_err(|_| 400u16)?;
                if content_length > MAX_BODY_BYTES {
                    return Err(413);
                }
            }
            "transfer-encoding" => return Err(501),
            "authorization" => request.authorization = Some(value.to_string()),
            "content-type" => request.content_type = Some(value.to_ascii_lowercase()),
            _ => {}
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).map_err(|_| 400u16)?;
    request.body = body;
    Ok(request)
}

pub fn write_response(mut stream: impl Write, response: &Response) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {} {reason}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\nContent-Length: {}\r\n",
        response.status,
        response.body.len()
    );
    if !response.body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    if response.status == 401 {
        head.push_str("WWW-Authenticate: Bearer\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(response.body.as_bytes())?;
    stream.flush()
}

/// Serves requests until the listener fails. Connections from anything but loopback are closed
/// unanswered (the listener is bound to loopback anyway).
pub fn serve(daemon: &Arc<Daemon>, listener: &TcpListener) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_connection(daemon, stream),
            Err(e) => eprintln!("craft-updated: accepting a connection failed: {e}"),
        }
    }
}

fn handle_connection(daemon: &Arc<Daemon>, stream: TcpStream) {
    if !stream.peer_addr().is_ok_and(|a| a.ip().is_loopback()) {
        return;
    }
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let response = match read_request(&stream) {
        Ok(request) => daemon.handle(&request),
        Err(status) => Response::error(status, "bad request"),
    };
    let _ = write_response(&stream, &response);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_with_its_headers_and_body() {
        let raw = b"POST /gridcraft/craft-update/api/login?x=1 HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nAuthorization: Bearer abc\r\nContent-Length: 5\r\n\r\nhello";
        let r = read_request(&raw[..]).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/gridcraft/craft-update/api/login");
        assert_eq!(r.content_type.as_deref(), Some("application/json"));
        assert_eq!(r.authorization.as_deref(), Some("Bearer abc"));
        assert_eq!(r.body, b"hello");
    }

    #[test]
    fn oversized_chunked_and_malformed_requests_are_refused() {
        assert_eq!(read_request(&b"GET / HTTP/1.1\r\nContent-Length: 999999\r\n\r\n"[..]), Err(413));
        assert_eq!(read_request(&b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"[..]), Err(501));
        let huge = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD_BYTES));
        assert_eq!(read_request(huge.as_bytes()), Err(431));
        for bad in [
            &b""[..],
            b"GET\r\n\r\n",
            b"GET / HTTP/2\r\n\r\n",
            b"GET nopath HTTP/1.1\r\n\r\n",
            b"GET / HTTP/1.1\r\nNoColon\r\n\r\n",
            b"GET / HTTP/1.1\r\nContent-Length: 4\r\n\r\nab",
        ] {
            assert!(read_request(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn responses_are_json_and_never_cached() {
        let mut out = Vec::new();
        write_response(&mut out, &Response::error(401, "sign in")).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 401 Unauthorized\r\n"));
        assert!(text.contains("Cache-Control: no-store\r\n") && text.contains("WWW-Authenticate: Bearer\r\n"));
        assert!(text.ends_with("\r\n\r\n{\"error\":\"sign in\"}"));
    }
}
