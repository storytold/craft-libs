//! Per-read cancellation. Callbacks run on the reader's thread and a request stays latched.

use std::cell::Cell;
use std::io::{self, Read};

use crate::Error;

pub(crate) const CHUNK: usize = 64 * 1024;

pub(crate) fn check(control: Option<&Control<'_>>) -> Result<(), Error> {
    control.map_or(Ok(()), Control::check)
}

/// A byte scanner can skip over escaped characters, so test a threshold rather than a modulus.
pub(crate) struct Scan<'a> {
    control: Option<&'a Control<'a>>,
    next: usize,
}

impl<'a> Scan<'a> {
    pub(crate) fn new(control: Option<&'a Control<'a>>) -> Self {
        Self { control, next: 0 }
    }

    pub(crate) fn check(&mut self, at: usize) -> Result<(), Error> {
        if self.control.is_some() && at >= self.next {
            check(self.control)?;
            self.next = at.saturating_add(CHUNK);
        }
        Ok(())
    }
}

pub(crate) struct Control<'a> {
    cancelled: &'a dyn Fn() -> bool,
    stopped: Cell<bool>,
}

impl<'a> Control<'a> {
    fn new(cancelled: &'a dyn Fn() -> bool) -> Self {
        Self { cancelled, stopped: Cell::new(false) }
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        if self.stopped.get() || (self.cancelled)() {
            self.stopped.set(true);
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub(crate) fn run<T>(cancelled: &dyn Fn() -> bool, read: impl FnOnce(&Control<'_>) -> Result<T, Error>) -> Result<T, Error> {
    let control = Control::new(cancelled);
    control.check()?;
    let result = read(&control);
    control.check()?;
    result
}

/// Poll before each bounded portion of input. In particular, a one-byte JSON reader does not
/// call the callback for every byte, and a decoder cannot consume an unbounded read request.
pub(crate) struct Reader<'a, R> {
    inner: R,
    control: Option<&'a Control<'a>>,
    left: usize,
}

impl<'a, R> Reader<'a, R> {
    pub(crate) fn new(inner: R, control: Option<&'a Control<'a>>) -> Self {
        Self { inner, control, left: 0 }
    }
}

impl<R: Read> Read for Reader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(control) = self.control else { return self.inner.read(buf) };
        if buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            control.check().map_err(|_| io::Error::other("document read canceled"))?;
            self.left = CHUNK;
        }
        let len = buf.len().min(self.left);
        let n = self.inner.read(&mut buf[..len])?;
        self.left -= n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_reader_stops_after_a_partial_read_and_latches_the_request() {
        let polls = Cell::new(0);
        let cancelled = || {
            polls.set(polls.get() + 1);
            polls.get() == 2
        };
        let control = Control::new(&cancelled);
        let mut input = io::Cursor::new(vec![1; CHUNK * 3]);
        let mut reader = Reader::new(&mut input, Some(&control));
        let mut bytes = vec![0; CHUNK * 3];
        assert_eq!(reader.read(&mut bytes).unwrap(), CHUNK);
        assert!(reader.read(&mut bytes).is_err());
        assert!(reader.read(&mut bytes).is_err(), "a one-shot true remains canceled");
        assert_eq!(input.position(), CHUNK as u64, "the remaining input was not consumed");
        assert_eq!(polls.get(), 2);
    }
}
