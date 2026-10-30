//! JSON lines and bulk frame records. No sockets live here.

use std::io::{BufRead, Read, Write};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::error::LinkError;

/// Longest accepted control line, without the newline. Matches the app control servers.
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// Open connections per server, authenticated or not. Bulk connections count.
pub const MAX_CONNECTIONS: usize = 16;
/// Connections that have not authenticated yet. The other `MAX_CONNECTIONS - this` slots are
/// reserved for authenticated peers.
pub const MAX_UNAUTHENTICATED: usize = 4;
pub const EVENT_QUEUE: usize = 256;
pub const REPLY_DEADLINE_SECS: u64 = 60;
/// `link.open` payload ceiling. The control line cap is the real limit: a longer request line
/// closes the connection. This number is the same, so `OpenPayload` gives `payload_too_large`
/// to a caller that does not come through the line reader.
pub const MAX_LINK_PAYLOAD: u64 = MAX_LINE as u64;
pub const BULK_HEADER_MAX: u32 = 1024 * 1024;
pub const FRAME_PAYLOAD_MAX: u64 = 256 * 1024 * 1024;
pub const PROTOCOL_MIN: u32 = 1;
pub const PROTOCOL_MAX: u32 = 1;
pub const AUTH_ERROR: &str = "authentication required";

const CFRQ: &[u8; 4] = b"CFRQ";
const CFRM: &[u8; 4] = b"CFRM";

/// One control request. `id` is JSON null when the caller omitted it.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

/// Read one line of at most `max` bytes. `Ok(None)` is a clean end of stream.
///
/// This keeps no state between calls. A read timeout drops the bytes read so far, so use it only
/// on a stream that cannot time out. A server loop that polls with a receive timeout uses
/// [`LineReader`].
pub fn read_line(reader: &mut impl BufRead, max: usize) -> Result<Option<Vec<u8>>, LinkError> {
    LineReader::new(max).read(reader)
}

/// Reads lines from a stream whose reads can time out. Bytes read before a timeout stay in the
/// reader, so a line that arrives in pieces more than one poll apart is not lost.
#[derive(Debug)]
pub struct LineReader {
    buf: Vec<u8>,
    max: usize,
    started: Option<Instant>,
    patience: Option<Duration>,
}

impl LineReader {
    pub fn new(max: usize) -> Self {
        Self { buf: Vec::new(), max, started: None, patience: None }
    }

    /// How long the current unfinished line has been open. `None` when no line is in progress.
    pub fn partial_age(&self) -> Option<Duration> {
        self.started.map(|start| start.elapsed())
    }

    /// Return `Err(Timeout)` once a line has been open this long, even when bytes keep arriving.
    /// Without it a peer that sends one byte per poll holds the connection for ever.
    pub fn with_patience(mut self, patience: Duration) -> Self {
        self.patience = Some(patience);
        self
    }

    /// Change the limit that [`LineReader::with_patience`] sets.
    pub fn set_patience(&mut self, patience: Duration) {
        self.patience = Some(patience);
    }

    /// Read the next line. On `Err(Timeout)` the partial line stays here and the next call
    /// continues it. `Ok(None)` is a clean end of stream with no partial line.
    pub fn read(&mut self, reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, LinkError> {
        loop {
            let available = match reader.fill_buf() {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err.into()),
            };
            if available.is_empty() {
                // End of stream. A final line without a newline is still a line.
                if self.buf.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(self.finish()));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(available.len(), |at| at + 1);
            if self.buf.len().saturating_add(take) > self.max.saturating_add(1) {
                return Err(LinkError::TooLarge);
            }
            self.buf.extend_from_slice(&available[..take]);
            reader.consume(take);
            if self.started.is_none() {
                self.started = Some(Instant::now());
            }
            if newline.is_some() {
                let mut line = self.finish();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line.len() > self.max {
                    return Err(LinkError::TooLarge);
                }
                return Ok(Some(line));
            }
            if self.patience.zip(self.partial_age()).is_some_and(|(limit, age)| age >= limit) {
                return Err(LinkError::Timeout);
            }
        }
    }

    fn finish(&mut self) -> Vec<u8> {
        self.started = None;
        std::mem::take(&mut self.buf)
    }
}

/// Parse one control line.
///
/// `Ok(None)` is a blank line (the caller skips it). `Err` is the existing
/// control-server reject text: one reply, then close.
pub fn parse_request(bytes: &[u8]) -> Result<Option<Request>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not a control request (invalid UTF-8); closing the connection".to_string())?;
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let msg: Value =
        serde_json::from_str(text).map_err(|err| format!("bad JSON ({err}); expected one JSON request object per line; closing the connection"))?;
    let Some(obj) = msg.as_object() else {
        return Err("not a control request (expected a JSON object); closing the connection".into());
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Err("not a control request (missing string `method`); closing the connection".into());
    };
    if method.is_empty() {
        return Err("not a control request (missing string `method`); closing the connection".into());
    }
    let id = obj.get("id").cloned().unwrap_or(Value::Null);
    if !(id.is_null() || id.is_number() || id.is_string()) {
        return Err("not a control request (id must be a number, string, or null); closing the connection".into());
    }
    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
    Ok(Some(Request { id, method: method.to_string(), params }))
}

pub fn params_object(params: &Value) -> Result<&Map<String, Value>, LinkError> {
    params.as_object().ok_or_else(|| LinkError::BadParams("params must be an object".into()))
}

pub fn auth_failure() -> Value {
    json!({"id": 0, "ok": false, "error": AUTH_ERROR})
}

pub fn auth_success(id: &Value) -> Value {
    let id = if id.is_null() { json!(0) } else { id.clone() };
    json!({"id": id, "ok": true, "result": {"authenticated": true}})
}

pub fn reply_ok(id: &Value, result: Value) -> Value {
    json!({"id": id, "ok": true, "result": result})
}

pub fn reply_err_text(id: &Value, error: &str) -> Value {
    json!({"id": id, "ok": false, "error": error})
}

pub fn reply_err(id: &Value, err: &LinkError) -> Value {
    json!({"id": id, "ok": false, "error": err.to_object()})
}

pub fn push_event(event: &str, seq: u64, data: Value) -> Value {
    json!({"event": event, "seq": seq, "data": data})
}

pub fn write_line(writer: &mut impl Write, value: &Value) -> Result<(), LinkError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

pub fn protocols_overlap(min: u32, max: u32) -> bool {
    min <= max && min <= PROTOCOL_MAX && max >= PROTOCOL_MIN
}

pub fn write_cfrq(writer: &mut impl Write, header: &Value) -> Result<(), LinkError> {
    write_magic(writer, CFRQ, header)?;
    Ok(())
}

pub fn read_cfrq(reader: &mut impl Read) -> Result<Value, LinkError> {
    read_magic(reader, CFRQ)
}

pub fn write_cfrm(writer: &mut impl Write, header: &Value, payload: &[u8]) -> Result<(), LinkError> {
    let header_bytes = header_bytes(header)?;
    let payload_len = u64::try_from(payload.len()).map_err(|_| LinkError::TooLarge)?;
    if payload_len > FRAME_PAYLOAD_MAX {
        return Err(LinkError::TooLarge);
    }
    writer.write_all(CFRM)?;
    writer.write_all(&u32_len(&header_bytes)?.to_le_bytes())?;
    writer.write_all(&payload_len.to_le_bytes())?;
    writer.write_all(&header_bytes)?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

pub fn read_cfrm(reader: &mut impl Read) -> Result<(Value, Vec<u8>), LinkError> {
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != CFRM {
        return Err(LinkError::Invalid("bulk reply magic".into()));
    }
    let header_len = read_u32(reader)?;
    if header_len > BULK_HEADER_MAX {
        return Err(LinkError::TooLarge);
    }
    let payload_len = read_u64(reader)?;
    if payload_len > FRAME_PAYLOAD_MAX {
        return Err(LinkError::TooLarge);
    }
    let header = read_header(reader, header_len)?;
    let mut payload = vec![0u8; usize::try_from(payload_len).map_err(|_| LinkError::TooLarge)?];
    reader.read_exact(&mut payload)?;
    Ok((header, payload))
}

fn write_magic(writer: &mut impl Write, magic: &[u8; 4], header: &Value) -> Result<(), LinkError> {
    let bytes = header_bytes(header)?;
    writer.write_all(magic)?;
    writer.write_all(&u32_len(&bytes)?.to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

fn read_magic(reader: &mut impl Read, expect: &[u8; 4]) -> Result<Value, LinkError> {
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != expect {
        return Err(LinkError::Invalid("bulk request magic".into()));
    }
    let len = read_u32(reader)?;
    if len > BULK_HEADER_MAX {
        return Err(LinkError::TooLarge);
    }
    read_header(reader, len)
}

fn header_bytes(header: &Value) -> Result<Vec<u8>, LinkError> {
    if !header.is_object() {
        return Err(LinkError::BadParams("bulk header must be an object".into()));
    }
    let bytes = serde_json::to_vec(header)?;
    if bytes.len() > usize::try_from(BULK_HEADER_MAX).unwrap_or(usize::MAX) {
        return Err(LinkError::TooLarge);
    }
    Ok(bytes)
}

fn u32_len(bytes: &[u8]) -> Result<u32, LinkError> {
    u32::try_from(bytes.len()).map_err(|_| LinkError::TooLarge)
}

fn read_u32(reader: &mut impl Read) -> Result<u32, LinkError> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, LinkError> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_header(reader: &mut impl Read, len: u32) -> Result<Value, LinkError> {
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf)?;
    let value: Value = serde_json::from_slice(&buf)?;
    if !value.is_object() {
        return Err(LinkError::BadParams("bulk header must be an object".into()));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn blank_and_request_round_trip() {
        assert!(parse_request(b"  \n").unwrap().is_none());
        let req = parse_request(br#"{"id":1,"method":"engine.execute","params":{"command":"x"}}"#).unwrap().unwrap();
        assert_eq!(req.method, "engine.execute");
        assert_eq!(req.id, json!(1));
        assert!(parse_request(b"[]").is_err());
        assert!(parse_request(b"{").is_err());
    }

    #[test]
    fn line_limit_and_crlf() {
        let mut cur = Cursor::new(b"abc\r\n".to_vec());
        let line = read_line(&mut cur, 8).unwrap().unwrap();
        assert_eq!(line, b"abc");
        let mut huge = vec![b'a'; 20];
        huge.push(b'\n');
        let mut cur = Cursor::new(huge);
        assert!(matches!(read_line(&mut cur, 8), Err(LinkError::TooLarge)));
    }

    /// A reader that hands out `chunks` and fails with a timeout after each one, like a socket
    /// with a receive timeout and a slow peer.
    struct Stuttering {
        chunks: std::collections::VecDeque<Vec<u8>>,
        pending_timeout: bool,
        current: Vec<u8>,
        pos: usize,
    }

    impl Stuttering {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self { chunks: chunks.into(), pending_timeout: false, current: Vec::new(), pos: 0 }
        }
    }

    impl Read for Stuttering {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.current.len() {
                if self.pending_timeout {
                    self.pending_timeout = false;
                    return Err(std::io::ErrorKind::WouldBlock.into());
                }
                let Some(next) = self.chunks.pop_front() else { return Ok(0) };
                self.current = next;
                self.pos = 0;
                if self.current.is_empty() {
                    return Err(std::io::ErrorKind::WouldBlock.into());
                }
                self.pending_timeout = true;
            }
            let n = out.len().min(self.current.len() - self.pos);
            out[..n].copy_from_slice(&self.current[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    fn collect_lines(chunks: Vec<Vec<u8>>, max: usize) -> Vec<Vec<u8>> {
        let mut reader = std::io::BufReader::with_capacity(4, Stuttering::new(chunks));
        let mut lines = LineReader::new(max);
        let mut got = Vec::new();
        loop {
            match lines.read(&mut reader) {
                Ok(Some(line)) => got.push(line),
                Ok(None) => return got,
                Err(LinkError::Timeout) => {}
                Err(other) => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_line_split_at_every_offset_survives_a_timeout() {
        let text = b"{\"id\":1,\"method\":\"link.hello\"}\r\n{\"id\":2}\n";
        let want = vec![b"{\"id\":1,\"method\":\"link.hello\"}".to_vec(), b"{\"id\":2}".to_vec()];
        for cut in 0..=text.len() {
            let chunks = vec![text[..cut].to_vec(), text[cut..].to_vec()];
            assert_eq!(collect_lines(chunks, 1024), want, "cut {cut}");
        }
        // Two cuts, and one byte at a time.
        for a in 0..text.len() {
            for b in a..text.len() {
                let chunks = vec![text[..a].to_vec(), text[a..b].to_vec(), text[b..].to_vec()];
                assert_eq!(collect_lines(chunks, 1024), want, "cuts {a} {b}");
            }
        }
        let bytes: Vec<Vec<u8>> = text.iter().map(|byte| vec![*byte]).collect();
        assert_eq!(collect_lines(bytes, 1024), want);
    }

    #[test]
    fn the_line_cap_holds_across_timeouts() {
        let mut reader = std::io::BufReader::new(Stuttering::new(vec![vec![b'a'; 6], vec![b'b'; 6], b"\n".to_vec()]));
        let mut lines = LineReader::new(8);
        let mut result = None;
        for _ in 0..8 {
            match lines.read(&mut reader) {
                Err(LinkError::Timeout) => {}
                other => {
                    result = Some(other);
                    break;
                }
            }
        }
        assert!(matches!(result, Some(Err(LinkError::TooLarge))), "{result:?}");
    }

    /// One byte per read, a short pause each, and never a newline: a slow-loris peer whose reads
    /// never time out.
    struct Trickle;

    impl Read for Trickle {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(Duration::from_millis(10));
            out[0] = b'a';
            Ok(1)
        }
    }

    #[test]
    fn a_trickling_line_is_cut_by_patience() {
        let mut reader = std::io::BufReader::with_capacity(1, Trickle);
        let mut lines = LineReader::new(MAX_LINE).with_patience(Duration::from_millis(100));
        let start = Instant::now();
        assert!(matches!(lines.read(&mut reader), Err(LinkError::Timeout)));
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
        assert!(lines.partial_age().is_some_and(|age| age >= Duration::from_millis(100)));
    }

    #[test]
    fn partial_age_tracks_an_open_line() {
        let mut reader = std::io::BufReader::new(Stuttering::new(vec![b"abc".to_vec(), b"d\n".to_vec()]));
        let mut lines = LineReader::new(64);
        assert!(lines.partial_age().is_none());
        assert!(matches!(lines.read(&mut reader), Err(LinkError::Timeout)));
        assert!(lines.partial_age().is_some());
        assert_eq!(lines.read(&mut reader).unwrap().unwrap(), b"abcd");
        assert!(lines.partial_age().is_none());
    }

    #[test]
    fn bulk_round_trip() {
        let header = json!({"req": 7, "time_s": 1.5, "epoch": 3, "region": null});
        let mut buf = Vec::new();
        write_cfrq(&mut buf, &header).unwrap();
        let got = read_cfrq(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(got["req"], json!(7));

        let mut buf = Vec::new();
        let reply = json!({"req": 7, "error": "no"});
        write_cfrm(&mut buf, &reply, b"").unwrap();
        let (h, payload) = read_cfrm(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(h["error"], json!("no"));
        assert!(payload.is_empty());

        let mut buf = Vec::new();
        write_cfrm(&mut buf, &json!({"req": 1, "w": 2}), &[9, 8, 7]).unwrap();
        let (_h, payload) = read_cfrm(&mut Cursor::new(buf)).unwrap();
        assert_eq!(payload, vec![9, 8, 7]);
    }

    #[test]
    fn protocol_window() {
        assert!(protocols_overlap(1, 1));
        assert!(!protocols_overlap(2, 2));
        assert!(!protocols_overlap(2, 1));
    }
}
