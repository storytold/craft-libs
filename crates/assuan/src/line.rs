//! Assuan's lines: reading them from a stream that may be hostile, telling the server's kinds of
//! line apart, and its percent-escaping.

use std::io::{ErrorKind, Read, Write};

use crate::{Error, MAX_LINE, ServerError};

/// How much is asked of the stream in one read.
const CHUNK: usize = 4 * 1024;
/// Reads interrupted in a row before giving up.
const MAX_INTERRUPTS: u32 = 1_000;
/// Longest description of an error kept.
const MAX_TEXT: usize = 200;

/// One line from the server.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// `OK`, with what follows it.
    Ok(Vec<u8>),
    /// `ERR <code> <description>`.
    Err(ServerError),
    /// `S <keyword> <arguments>`.
    Status(Vec<u8>, Vec<u8>),
    /// `D <data>`, decoded.
    Data(Vec<u8>),
    /// `INQUIRE <keyword> <arguments>`.
    Inquire(Vec<u8>, Vec<u8>),
    /// `# …`
    Comment,
}

/// `text` cut at its first space: the word and what follows the space.
fn split_word(text: &[u8]) -> (&[u8], &[u8]) {
    match text.iter().position(|b| *b == b' ') {
        Some(i) => (text.get(..i).unwrap_or(&[]), text.get(i.saturating_add(1)..).unwrap_or(&[])),
        None => (text, &[]),
    }
}

/// A line as the server sent it, without its line feed. `Err` for a line that isn't one of
/// Assuan's.
pub(crate) fn parse(line: &[u8]) -> Result<Line, Error> {
    if line.first() == Some(&b'#') {
        return Ok(Line::Comment);
    }
    let (word, rest) = split_word(line);
    match word {
        b"OK" => Ok(Line::Ok(unescape(rest))),
        b"D" => Ok(Line::Data(unescape(rest))),
        b"ERR" => {
            let (code, text) = split_word(rest);
            let code = std::str::from_utf8(code)
                .ok()
                .filter(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|c| c.parse::<u32>().ok())
                .ok_or_else(|| Error::Protocol("an ERR line without its number".into()))?;
            Ok(Line::Err(ServerError { code, text: crate::tidy(&unescape(text), MAX_TEXT) }))
        }
        b"S" | b"INQUIRE" => {
            let (keyword, args) = split_word(rest);
            if keyword.is_empty() || !keyword.iter().all(|b| b.is_ascii_graphic()) {
                return Err(Error::Protocol("a status line or inquiry without its keyword".into()));
            }
            let (keyword, args) = (keyword.to_vec(), unescape(args));
            Ok(if word == b"S" { Line::Status(keyword, args) } else { Line::Inquire(keyword, args) })
        }
        _ => Err(Error::Protocol(format!("a line beginning \"{}\"", crate::tidy(word, 16)))),
    }
}

/// Undo Assuan's escaping: `%XX` is the byte XX. A `%` not followed by two hex digits is kept
/// as it is.
pub(crate) fn unescape(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0usize;
    while let Some(&b) = text.get(i) {
        if b == b'%'
            && let (Some(h), Some(l)) = (text.get(i.saturating_add(1)).and_then(|c| hex(*c)), text.get(i.saturating_add(2)).and_then(|c| hex(*c)))
        {
            out.push((h << 4) | l);
            i = i.saturating_add(3);
            continue;
        }
        out.push(b);
        i = i.saturating_add(1);
    }
    out
}

pub(crate) fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// `%XX` for `byte`.
pub(crate) fn push_escaped(out: &mut Vec<u8>, byte: u8) {
    out.push(b'%');
    out.push(HEX.get(usize::from(byte >> 4)).copied().unwrap_or(b'0'));
    out.push(HEX.get(usize::from(byte & 0x0F)).copied().unwrap_or(b'0'));
}

/// Assuan's escaping of data: `%`, CR and LF become `%25`, `%0D` and `%0A`.
pub(crate) fn escape_data(out: &mut Vec<u8>, byte: u8) {
    match byte {
        b'%' | b'\r' | b'\n' => push_escaped(out, byte),
        _ => out.push(byte),
    }
}

/// Bytes that are wiped before their memory is given back: what a stream brought in may be a
/// session key.
#[derive(Default)]
struct Held(Vec<u8>);

impl Drop for Held {
    fn drop(&mut self) {
        crate::wipe(&mut self.0);
    }
}

/// A stream with the bytes read ahead of what was asked for.
pub(crate) struct Wire<S> {
    stream: S,
    /// Bytes read from the stream and not handed out yet (from `start`).
    buf: Held,
    start: usize,
}

fn io_error(e: &std::io::Error) -> Error {
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => Error::Io("gpg-agent took too long to answer".into()),
        ErrorKind::UnexpectedEof | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset | ErrorKind::BrokenPipe => Error::Closed,
        kind => Error::Io(format!("{kind}")),
    }
}

impl<S> Wire<S> {
    pub(crate) fn new(stream: S) -> Self {
        Wire { stream, buf: Held::default(), start: 0 }
    }

    pub(crate) fn into_inner(self) -> S {
        self.stream
    }

    pub(crate) fn get_mut(&mut self) -> &mut S {
        &mut self.stream
    }
}

impl<S: Read + Write> Wire<S> {
    /// The next line without its line feed. A line longer than [`MAX_LINE`] (its line feed
    /// included) is an error, and so is the end of the stream.
    pub(crate) fn read_line(&mut self) -> Result<Vec<u8>, Error> {
        let mut scanned = 0usize;
        loop {
            let pending = self.buf.0.get(self.start..).unwrap_or(&[]);
            if let Some(i) = pending.get(scanned..).and_then(|rest| rest.iter().position(|b| *b == b'\n')) {
                let end = scanned.saturating_add(i);
                if end >= MAX_LINE {
                    return Err(Error::LineTooLong);
                }
                let line = pending.get(..end).unwrap_or(&[]);
                let line = line.strip_suffix(b"\r").unwrap_or(line).to_vec();
                self.start = self.start.saturating_add(end).saturating_add(1);
                return Ok(line);
            }
            scanned = pending.len();
            if scanned >= MAX_LINE {
                return Err(Error::LineTooLong);
            }
            self.fill()?;
        }
    }

    /// Read some more into the buffer; the end of the stream is an error.
    fn fill(&mut self) -> Result<(), Error> {
        if self.start > 0 {
            crate::discard_front(&mut self.buf.0, self.start);
            self.start = 0;
        }
        let mut tmp = Held(vec![0u8; CHUNK]);
        let mut interrupts = 0u32;
        let n = loop {
            match self.stream.read(&mut tmp.0) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => break n,
                Err(e) if e.kind() == ErrorKind::Interrupted && interrupts < MAX_INTERRUPTS => interrupts = interrupts.saturating_add(1),
                Err(e) => return Err(io_error(&e)),
            }
        };
        crate::extend_wiped(&mut self.buf.0, tmp.0.get(..n).unwrap_or(&[]));
        Ok(())
    }

    /// Send one line: `line` must hold no line feed and be short enough (the caller checks).
    pub(crate) fn write_line(&mut self, line: &[u8]) -> Result<(), Error> {
        let mut out = Held(Vec::with_capacity(line.len().saturating_add(1)));
        out.0.extend_from_slice(line);
        out.0.push(b'\n');
        self.stream.write_all(&out.0).map_err(|e| io_error(&e))?;
        self.stream.flush().map_err(|e| io_error(&e))
    }

    /// Send `data` as `D` lines, each at most [`MAX_LINE`] bytes, then `END`.
    pub(crate) fn write_data(&mut self, data: &[u8]) -> Result<(), Error> {
        let mut out = Held(Vec::with_capacity(MAX_LINE));
        let mut bytes = data.iter().copied().peekable();
        while bytes.peek().is_some() {
            out.0.clear();
            out.0.extend_from_slice(b"D ");
            // Room for one more escaped byte (three) and the line feed.
            while out.0.len().saturating_add(4) <= MAX_LINE {
                match bytes.next() {
                    Some(b) => escape_data(&mut out.0, b),
                    None => break,
                }
            }
            out.0.push(b'\n');
            self.stream.write_all(&out.0).map_err(|e| io_error(&e))?;
        }
        self.write_line(b"END")
    }
}
