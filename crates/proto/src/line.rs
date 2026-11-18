// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The bounded reader both protocols share: lines and counted blocks from a stream that may be hostile.

use std::io::{ErrorKind, Read, Write};

use crate::said;
use crate::{MAX_LINE, MAX_LITERAL, ProtoError};

/// How much is asked of the stream in one read.
const CHUNK: usize = 16 * 1024;
/// Most bytes of one over-long untagged line read past ([`Wire::read_line_or_skip`]).
const MAX_SKIPPED: usize = 4 * 1024 * 1024;
/// Reads interrupted in a row before giving up.
const MAX_INTERRUPTS: u32 = 1_000;

/// A stream with the bytes read ahead of what was asked for.
pub(crate) struct Wire<S> {
    stream: S,
    /// Bytes read from the stream and not handed out yet (from `start`).
    buf: Vec<u8>,
    start: usize,
    // Bytes already searched for a newline in the current pending polling line.
    poll_scanned: usize,
}

fn closed() -> ProtoError {
    ProtoError::Io(said!("the server closed the connection"))
}

fn io_error(e: &std::io::Error) -> ProtoError {
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => ProtoError::Io(said!("the server took too long to answer")),
        ErrorKind::UnexpectedEof | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset | ErrorKind::BrokenPipe => closed(),
        // The system's own words, as it said them.
        _ => ProtoError::Io(crate::tidy(&e.to_string()).into()),
    }
}

impl<S> Wire<S> {
    pub(crate) fn new(stream: S) -> Self {
        Wire { stream, buf: Vec::new(), start: 0, poll_scanned: 0 }
    }

    /// Whether the server sent more than has been read so far.
    pub(crate) fn has_pending(&self) -> bool {
        self.start < self.buf.len()
    }

    pub(crate) fn into_inner(self) -> S {
        self.stream
    }

    /// The stream itself, for its owner to set up (what a socket waits for, say). Reading from
    /// it or writing to it here would take the session apart.
    pub(crate) fn get_mut(&mut self) -> &mut S {
        &mut self.stream
    }
}

impl<S: Read + Write> Wire<S> {
    /// Read some more into the buffer; the end of the stream is an error.
    fn fill(&mut self) -> Result<(), ProtoError> {
        self.poll_scanned = 0;
        if self.start > 0 {
            self.buf.drain(..self.start.min(self.buf.len()));
            self.start = 0;
        }
        let mut tmp = vec![0u8; CHUNK];
        let n = read_some(&mut self.stream, &mut tmp)?;
        self.buf.extend_from_slice(tmp.get(..n).unwrap_or(&[]));
        Ok(())
    }

    /// The next line without its line ending (CRLF, or a bare LF).
    pub(crate) fn read_line(&mut self) -> Result<Vec<u8>, ProtoError> {
        self.next_line(false)?.ok_or_else(too_long)
    }

    /// The next line, or `None` for an untagged one (`* …`) longer than [`MAX_LINE`], which is
    /// read past to its end (at most [`MAX_SKIPPED`] bytes of it) so that the session goes on.
    /// Any other line that long is an error, as for [`Wire::read_line`].
    pub(crate) fn read_line_or_skip(&mut self) -> Result<Option<Vec<u8>>, ProtoError> {
        self.next_line(true)
    }

    fn next_line(&mut self, skip: bool) -> Result<Option<Vec<u8>>, ProtoError> {
        self.poll_scanned = 0;
        // How much of the pending bytes has been searched for a line feed already.
        let mut scanned = 0usize;
        // Bytes of an over-long line dropped so far, when one is being read past.
        let mut skipped: Option<usize> = None;
        loop {
            let pending = self.buf.get(self.start..).unwrap_or(&[]);
            if let Some(i) = pending.get(scanned..).and_then(|rest| rest.iter().position(|b| *b == b'\n')) {
                let end = scanned.saturating_add(i);
                let line = pending.get(..end).unwrap_or(&[]);
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                let line = if skipped.is_some() {
                    None
                } else if line.len() > MAX_LINE {
                    if !(skip && line.starts_with(b"* ")) {
                        return Err(too_long());
                    }
                    None
                } else {
                    Some(line.to_vec())
                };
                self.start = self.start.saturating_add(end).saturating_add(1);
                return Ok(line);
            }
            scanned = pending.len();
            // The limit is on the line itself; a CR may still come on top.
            if scanned > MAX_LINE + 1 {
                let dropped = match skipped {
                    Some(n) => n,
                    None if skip && pending.starts_with(b"* ") => 0,
                    None => return Err(too_long()),
                };
                let dropped = dropped.saturating_add(scanned);
                if dropped > MAX_SKIPPED {
                    return Err(too_long());
                }
                skipped = Some(dropped);
                self.start = self.buf.len();
                scanned = 0;
            }
            // Each round reads at least one byte, so this ends at the line feed, the cap or the end of the stream.
            self.fill()?;
        }
    }

    /// The next line after at most one underlying read; `None` for a partial line or when
    /// the stream times out (IDLE waits like this, a second at a time, so that its owner can
    /// look at the clock and at whether it was told to stop). Bytes of a line that came so far are
    /// kept for the next call. A line over [`MAX_LINE`] is an error, as for [`Wire::read_line`].
    pub(crate) fn poll_line(&mut self) -> Result<Option<Vec<u8>>, ProtoError> {
        let mut read_once = false;
        loop {
            let pending = self.buf.get(self.start..).unwrap_or(&[]);
            let scanned = self.poll_scanned.min(pending.len());
            if let Some(offset) = pending.get(scanned..).unwrap_or(&[]).iter().position(|b| *b == b'\n') {
                let end = scanned.saturating_add(offset);
                let line = pending.get(..end).unwrap_or(&[]);
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                if line.len() > MAX_LINE {
                    return Err(too_long());
                }
                let line = line.to_vec();
                self.start = self.start.saturating_add(end).saturating_add(1);
                self.poll_scanned = 0;
                return Ok(Some(line));
            }
            if pending.len() > MAX_LINE + 1 {
                return Err(too_long());
            }
            self.poll_scanned = pending.len();
            // One underlying read per poll lets the caller check cancellation even when
            // a hostile peer trickles bytes without ever triggering the read timeout.
            if read_once {
                return Ok(None);
            }
            if self.start > 0 {
                self.buf.drain(..self.start.min(self.buf.len()));
                self.start = 0;
            }
            let mut tmp = vec![0u8; CHUNK];
            read_once = true;
            match self.stream.read(&mut tmp) {
                Ok(0) => return Err(closed()),
                Ok(n) => self.buf.extend_from_slice(tmp.get(..n.min(tmp.len())).unwrap_or(&[])),
                Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted) => return Ok(None),
                Err(e) => return Err(io_error(&e)),
            }
        }
    }

    /// Exactly `n` bytes (at most [`MAX_LITERAL`]).
    pub(crate) fn read_block(&mut self, n: usize) -> Result<Vec<u8>, ProtoError> {
        self.poll_scanned = 0;
        if n > MAX_LITERAL {
            return Err(ProtoError::Protocol(said!("the server sent {n} bytes in one piece, more than MailCraft accepts", n)));
        }
        // The size is the server's word: memory is taken as the bytes arrive.
        let mut out = Vec::with_capacity(n.min(CHUNK));
        let pending = self.buf.get(self.start..).unwrap_or(&[]);
        let take = pending.len().min(n);
        out.extend_from_slice(pending.get(..take).unwrap_or(&[]));
        self.start = self.start.saturating_add(take);
        let mut tmp = vec![0u8; CHUNK];
        while out.len() < n {
            let want = (n - out.len()).min(tmp.len());
            let got = read_some(&mut self.stream, tmp.get_mut(..want).unwrap_or(&mut []))?;
            out.extend_from_slice(tmp.get(..got.min(want)).unwrap_or(&[]));
        }
        Ok(out)
    }

    pub(crate) fn write(&mut self, mut bytes: &[u8]) -> Result<(), ProtoError> {
        // Not `write_all`: that retries an interrupted write for ever.
        let mut interrupts = 0;
        while !bytes.is_empty() {
            match self.stream.write(bytes) {
                Ok(0) => return Err(closed()),
                Ok(n) => bytes = bytes.get(n..).unwrap_or(&[]),
                Err(e) if e.kind() == ErrorKind::Interrupted && interrupts < MAX_INTERRUPTS => interrupts += 1,
                Err(e) => return Err(io_error(&e)),
            }
        }
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> Result<(), ProtoError> {
        self.stream.flush().map_err(|e| io_error(&e))
    }
}

fn too_long() -> ProtoError {
    ProtoError::Protocol(said!("the server sent a line longer than {kb} KB", kb = MAX_LINE / 1024))
}

/// One read of at least one byte; the end of the stream is an error.
fn read_some<S: Read>(stream: &mut S, buf: &mut [u8]) -> Result<usize, ProtoError> {
    let mut interrupts = 0;
    loop {
        match stream.read(buf) {
            Ok(0) => return Err(closed()),
            // A stream that claims more than the buffer holds is lying.
            Ok(n) => return Ok(n.min(buf.len())),
            Err(e) if e.kind() == ErrorKind::Interrupted && interrupts < MAX_INTERRUPTS => interrupts += 1,
            Err(e) => return Err(io_error(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::script::{Broken, Script};

    #[test]
    fn polling_reads_once_and_retains_linear_scan_progress() {
        struct ByteAtATime {
            bytes: std::io::Cursor<Vec<u8>>,
            reads: usize,
        }
        impl Read for ByteAtATime {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                self.bytes.read(&mut out[..1])
            }
        }
        impl Write for ByteAtATime {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut wire = Wire::new(ByteAtATime { bytes: std::io::Cursor::new(b"abc\n".to_vec()), reads: 0 });
        for scanned in 1..=3 {
            assert_eq!(wire.poll_line().unwrap(), None);
            assert_eq!(wire.poll_scanned, scanned);
            assert_eq!(wire.get_mut().reads, scanned);
        }
        assert_eq!(wire.poll_line().unwrap(), Some(b"abc".to_vec()));
        assert_eq!(wire.get_mut().reads, 4);
        assert_eq!(wire.poll_scanned, 0);
    }

    #[test]
    fn lines_and_blocks_arrive_in_pieces() {
        let mut s = Script::new("first\r\nsecond\nab\r\ncd\r\nlast\r\n");
        let mut w = Wire::new(&mut s);
        assert_eq!(w.read_line().unwrap(), b"first");
        assert_eq!(w.read_line().unwrap(), b"second");
        assert_eq!(w.read_block(6).unwrap(), b"ab\r\ncd");
        assert_eq!(w.read_block(0).unwrap(), b"");
        assert_eq!(w.read_line().unwrap(), b"");
        assert_eq!(w.read_line().unwrap(), b"last");
        assert!(!w.has_pending());
    }

    #[test]
    fn the_end_of_the_stream_is_an_error() {
        let mut s = Script::new("no line ending");
        let mut w = Wire::new(&mut s);
        assert_eq!(w.read_line(), Err(ProtoError::Io(said!("the server closed the connection"))));
        let mut s = Script::new("short");
        assert!(matches!(Wire::new(&mut s).read_block(10), Err(ProtoError::Io(_))));
    }

    #[test]
    fn long_lines_and_large_blocks_are_refused() {
        let mut s = Script::whole(vec![b'x'; MAX_LINE + CHUNK * 2]);
        assert!(matches!(Wire::new(&mut s).read_line(), Err(ProtoError::Protocol(_))));
        // A line of exactly the limit is fine.
        let mut ok = vec![b'x'; MAX_LINE];
        ok.extend_from_slice(b"\r\n");
        let mut s = Script::whole(ok);
        assert_eq!(Wire::new(&mut s).read_line().unwrap().len(), MAX_LINE);
        // The claim alone is refused: nothing is read or allocated.
        let mut s = Script::new("");
        assert!(matches!(Wire::new(&mut s).read_block(MAX_LITERAL + 1), Err(ProtoError::Protocol(_))));
    }

    #[test]
    fn stream_errors_become_connection_problems() {
        let mut w = Wire::new(Broken(ErrorKind::TimedOut));
        assert_eq!(w.read_line(), Err(ProtoError::Io(said!("the server took too long to answer"))));
        assert!(matches!(w.write(b"x"), Err(ProtoError::Io(_))));
        // A stream that is interrupted for ever hangs neither the reader nor the writer.
        assert!(matches!(Wire::new(Broken(ErrorKind::Interrupted)).read_line(), Err(ProtoError::Io(_))));
        assert!(matches!(Wire::new(Broken(ErrorKind::Interrupted)).write(b"x"), Err(ProtoError::Io(_))));
    }
}
