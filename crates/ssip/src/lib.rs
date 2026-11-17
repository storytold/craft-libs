#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Speech Dispatcher SSIP wire helpers extracted from WordCraft's contributor implementation.
//! Socket discovery, deadlines, voice selection, retry and cancellation policy stay in the app.
use std::io::{self, ErrorKind, Read};

/// Maximum reply bytes before LF, including an optional CR.
pub const MAX_LINE: usize = 4096;

/// Encode one sentence as data, never as protocol commands. CR/LF become spaces; a leading
/// dot is doubled and the message terminator is appended. The caller bounds input bytes.
pub fn message_data(text: &str, max_input_bytes: usize) -> io::Result<String> {
    if text.len() > max_input_bytes {
        return Err(ErrorKind::InvalidInput.into());
    }
    let size = text.len().checked_add(6).ok_or(ErrorKind::InvalidInput)?;
    let mut out = String::new();
    out.try_reserve_exact(size).map_err(|_| io::Error::other("SSIP message allocation failed"))?;
    if text.starts_with('.') {
        out.push('.');
    }
    out.extend(text.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }));
    out.push_str("\r\n.\r\n");
    Ok(out)
}

/// The three ASCII digits of a final response; continuation (`225-...`) is not final.
pub fn final_code(line: &str) -> Option<u16> {
    let digits = line.as_bytes().get(..3)?;
    if !digits.iter().all(u8::is_ascii_digit) || !(line.len() == 3 || line.as_bytes().get(3) == Some(&b' ')) {
        return None;
    }
    line.get(..3)?.parse().ok()
}

/// Incremental replies. At most one read per call lets the app enforce its own deadline.
/// Invalid UTF-8 retains WordCraft's replacement-character behavior. Overlong replies fail,
/// including those whose terminating newline arrives in the same read that exceeds the cap.
pub struct Lines<R> {
    reader: R,
    pending: Vec<u8>,
}

impl<R: Read> Lines<R> {
    pub fn new(reader: R) -> Self {
        Self { reader, pending: Vec::new() }
    }

    /// Complete next line without CR/LF, or None after a partial read or transient timeout.
    /// The caller must set transport timeouts and enforce cancellation/deadlines.
    pub fn next_line(&mut self) -> io::Result<Option<String>> {
        if let Some(line) = self.complete_line()? {
            return Ok(Some(line));
        }
        let mut buf = [0u8; 512];
        match self.reader.read(&mut buf) {
            Ok(0) => Err(ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                let bytes = buf.get(..n).ok_or(ErrorKind::InvalidData)?;
                // Invariant: pending <= MAX_LINE before this read, so storage <= MAX_LINE+512.
                self.pending.try_reserve(bytes.len()).map_err(|_| io::Error::other("SSIP reply allocation failed"))?;
                self.pending.extend_from_slice(bytes);
                self.complete_line()
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn complete_line(&mut self) -> io::Result<Option<String>> {
        let end = self.pending.iter().position(|&b| b == b'\n');
        if end.unwrap_or(self.pending.len()) > MAX_LINE {
            return Err(ErrorKind::InvalidData.into());
        }
        let Some(end) = end else { return Ok(None) };
        let bytes = self.pending.get(..end).ok_or(ErrorKind::InvalidData)?;
        let line = String::from_utf8_lossy(bytes).trim_end_matches('\r').to_string();
        self.pending.drain(..=end);
        Ok(Some(line))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::collections::VecDeque;
    struct Chunks(VecDeque<io::Result<Vec<u8>>>);
    impl Read for Chunks {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let bytes = self.0.pop_front().unwrap_or(Ok(Vec::new()))?;
            out[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }
    }
    #[test]
    fn message_text_cannot_inject_commands() {
        assert_eq!(message_data(".\r\nCANCEL ALL\n!-!", 64).unwrap(), "..  CANCEL ALL !-!\r\n.\r\n");
        assert_eq!(message_data("你好", 6).unwrap(), "你好\r\n.\r\n");
        assert!(message_data("你好", 5).is_err());
    }
    #[test]
    fn final_codes_are_ascii_and_not_continuations() {
        assert_eq!(final_code("225 OK"), Some(225));
        assert_eq!(final_code("701"), Some(701));
        for bad in ["225-12", "+22 OK", "22 OK", "２２５ OK", "225x", ""] {
            assert_eq!(final_code(bad), None);
        }
    }
    #[test]
    fn fragmented_utf8_timeout_and_buffered_lines() {
        let mut lines = Lines::new(Chunks(VecDeque::from([
            Ok(vec![b'2', b'2', b'5', b' ', 0xc3]),
            Err(ErrorKind::TimedOut.into()),
            Ok(vec![0xa9, b'\r', b'\n', b'7', b'0', b'2', b'\n']),
        ])));
        assert!(lines.next_line().unwrap().is_none());
        assert!(lines.next_line().unwrap().is_none());
        assert_eq!(lines.next_line().unwrap().as_deref(), Some("225 é"));
        assert_eq!(lines.next_line().unwrap().as_deref(), Some("702"));
        assert_eq!(lines.next_line().unwrap_err().kind(), ErrorKind::UnexpectedEof);
    }
    #[test]
    fn completed_overlong_line_is_rejected_at_boundary() {
        for extra in [0, 1, 511] {
            let mut chunks: VecDeque<_> = (0..8).map(|_| Ok(vec![b'x'; 512])).collect();
            let mut last = vec![b'x'; extra];
            last.push(b'\n');
            chunks.push_back(Ok(last));
            let mut lines = Lines::new(Chunks(chunks));
            for _ in 0..8 {
                assert!(lines.next_line().unwrap().is_none());
            }
            if extra == 0 {
                assert_eq!(lines.next_line().unwrap().unwrap().len(), MAX_LINE);
            } else {
                assert_eq!(lines.next_line().unwrap_err().kind(), ErrorKind::InvalidData);
            }
        }
    }
}
