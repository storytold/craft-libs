//! ASCII85 (`ASCII85Encode` / `ASCII85Decode`): four bytes as five characters `!`…`u`, four zero
//! bytes as `z`, ended by `~>`.
//!
//! [`decode`] takes the whole text; [`Decoder`] decodes as the text is read, by the same rules.

use std::fmt;
use std::io::{self, BufRead, Read};

/// Characters a line of encoded data has, about.
const LINE: usize = 72;

/// `bytes` in ASCII85 with its `~>` end, in lines of about 72 characters, none starting with `%`
/// (a DSC reader would take such a line for a comment).
pub fn encode(bytes: &[u8]) -> String {
    let mut raw = String::with_capacity(bytes.len() * 5 / 4 + 8);
    let (chunks, rest) = bytes.as_chunks::<4>();
    for c in chunks {
        let v = u32::from_be_bytes(*c);
        if v == 0 {
            raw.push('z');
        } else {
            push85(&mut raw, v, 5);
        }
    }
    if !rest.is_empty() {
        let mut c = [0u8; 4];
        for (d, s) in c.iter_mut().zip(rest) {
            *d = *s;
        }
        push85(&mut raw, u32::from_be_bytes(c), rest.len() + 1);
    }
    raw.push_str("~>");
    let mut out = String::with_capacity(raw.len() + raw.len() / LINE + 2);
    if raw.starts_with('%') {
        // Whitespace is ignored by readers, and keeps the first line from starting with `%`.
        out.push(' ');
    }
    let mut col = 0;
    for ch in raw.chars() {
        if col >= LINE && ch != '%' {
            out.push('\n');
            col = 0;
        }
        out.push(ch);
        col += 1;
    }
    out.push('\n');
    out
}

/// The first `n` base-85 digits of `v`.
fn push85(s: &mut String, mut v: u32, n: usize) {
    let mut d = [0u8; 5];
    for slot in d.iter_mut().rev() {
        *slot = (v % 85) as u8 + b'!';
        v /= 85;
    }
    for b in d.iter().take(n) {
        s.push(char::from(*b));
    }
}

/// ASCII85 `text` (whitespace ignored, up to `~>`) decoded. `None` when it is malformed, has no
/// end, or decodes to more than `max` bytes: a `z` is four bytes for one character, so the length
/// of the text doesn't bound what it becomes.
pub fn decode(text: &[u8], max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity((text.len() / 5 * 4).min(max));
    let mut group = [0u8; 5];
    let mut n = 0;
    let mut chars = text.iter().copied().filter(|b| !b.is_ascii_whitespace());
    while let Some(b) = chars.next() {
        if out.len() > max {
            return None;
        }
        match b {
            b'~' => {
                // A single left-over character can't be a byte.
                if chars.next() != Some(b'>') || n == 1 {
                    return None;
                }
                if n > 1 {
                    let mut padded = [b'u'; 5];
                    for (d, s) in padded.iter_mut().zip(group.iter().take(n)) {
                        *d = *s;
                    }
                    out.extend(value85(&padded)?.to_be_bytes().iter().take(n - 1));
                }
                return (out.len() <= max).then_some(out);
            }
            b'z' if n == 0 => out.extend([0; 4]),
            b'!'..=b'u' => {
                *group.get_mut(n)? = b;
                n += 1;
                if n == 5 {
                    out.extend(value85(&group)?.to_be_bytes());
                    n = 0;
                }
            }
            _ => return None,
        }
    }
    None
}

/// The value of five base-85 digits; `None` past `u32::MAX`.
fn value85(digits: &[u8]) -> Option<u32> {
    digits.iter().try_fold(0u32, |acc, d| acc.checked_mul(85)?.checked_add(u32::from(d.checked_sub(b'!')?)))
}

/// Why a [`Decoder`] stopped, carried by the [`io::Error`] it returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// A character outside the encoding, a group past `u32::MAX`, or a single character left
    /// over before `~>`.
    Malformed,
    /// The text ended before `~>`.
    Unterminated,
    /// The text decodes to more than the most bytes allowed.
    TooLarge,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "the ASCII85 data is damaged",
            Self::Unterminated => "the ASCII85 data has no end",
            Self::TooLarge => "the ASCII85 data decodes to more bytes than allowed",
        })
    }
}

impl std::error::Error for DecodeError {}

impl From<DecodeError> for io::Error {
    fn from(e: DecodeError) -> Self {
        let kind = if e == DecodeError::Unterminated { io::ErrorKind::UnexpectedEof } else { io::ErrorKind::InvalidData };
        io::Error::new(kind, e)
    }
}

/// ASCII85 decoded as it is read from `R`: whitespace ignored, up to `~>` and not a byte past it,
/// so `R` goes on right after the end. It gives what [`decode`] gives for the same text, or an
/// error where [`decode`] gives `None` (a [`DecodeError`] inside the [`io::Error`]); the bytes
/// before the fault may already have been given out. The error comes as soon as the output passes
/// `max`, so no more text is read than that takes.
pub struct Decoder<R> {
    inner: R,
    state: State,
}

/// Where a [`Decoder`] is in the text.
struct State {
    group: [u8; 5],
    /// Digits in `group`.
    n: usize,
    /// A `~` was read: `>` (after any whitespace) ends the data.
    tilde: bool,
    /// Decoded bytes not yet given out: `out[at..len]`.
    out: [u8; 4],
    at: usize,
    len: usize,
    total: usize,
    max: usize,
    done: bool,
    failed: Option<DecodeError>,
}

impl State {
    /// Take one character of the text.
    fn step(&mut self, b: u8) -> Result<(), DecodeError> {
        if b.is_ascii_whitespace() {
            return Ok(());
        }
        if self.tilde {
            // A single left-over character can't be a byte.
            if b != b'>' || self.n == 1 {
                return Err(DecodeError::Malformed);
            }
            if self.n > 1 {
                let mut padded = [b'u'; 5];
                for (d, s) in padded.iter_mut().zip(self.group.iter().take(self.n)) {
                    *d = *s;
                }
                let v = value85(&padded).ok_or(DecodeError::Malformed)?;
                self.push(v.to_be_bytes(), self.n - 1)?;
            }
            self.done = true;
            return Ok(());
        }
        match b {
            b'~' => self.tilde = true,
            b'z' if self.n == 0 => self.push([0; 4], 4)?,
            b'!'..=b'u' => {
                *self.group.get_mut(self.n).ok_or(DecodeError::Malformed)? = b;
                self.n += 1;
                if self.n == 5 {
                    let v = value85(&self.group).ok_or(DecodeError::Malformed)?;
                    self.n = 0;
                    self.push(v.to_be_bytes(), 4)?;
                }
            }
            _ => return Err(DecodeError::Malformed),
        }
        Ok(())
    }

    /// The first `n` of `bytes` decoded, waiting to be given out.
    fn push(&mut self, bytes: [u8; 4], n: usize) -> Result<(), DecodeError> {
        self.total = self.total.saturating_add(n);
        if self.total > self.max {
            return Err(DecodeError::TooLarge);
        }
        self.out = bytes;
        self.at = 0;
        self.len = n;
        Ok(())
    }

    /// Give out decoded bytes into `buf` → how many.
    fn drain(&mut self, buf: &mut [u8]) -> usize {
        let pending = self.out.get(self.at..self.len).unwrap_or_default();
        let n = pending.len().min(buf.len());
        if let (Some(dst), Some(src)) = (buf.get_mut(..n), pending.get(..n)) {
            dst.copy_from_slice(src);
        }
        self.at += n;
        n
    }
}

impl<R: BufRead> Decoder<R> {
    /// A decoder of the text `inner` reads, giving at most `max` bytes.
    pub fn new(inner: R, max: usize) -> Self {
        Self { inner, state: State { group: [0; 5], n: 0, tilde: false, out: [0; 4], at: 0, len: 0, total: 0, max, done: false, failed: None } }
    }

    /// The reader, right after the `~>` once the data has been read to its end.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: BufRead> Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(e) = self.state.failed {
            return Err(e.into());
        }
        let mut written = self.state.drain(buf);
        while written < buf.len() && !self.state.done {
            let input = match self.inner.fill_buf() {
                Ok(input) => input,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // The bytes decoded so far are given out first; the next read asks the reader
                // again.
                Err(_) if written > 0 => break,
                Err(e) => return Err(e),
            };
            let mut failure = input.is_empty().then_some(DecodeError::Unterminated);
            let mut used = 0;
            for &b in input {
                if failure.is_some() || written == buf.len() || self.state.done {
                    break;
                }
                used += 1;
                match self.state.step(b) {
                    Ok(()) => written += self.state.drain(buf.get_mut(written..).unwrap_or_default()),
                    Err(e) => failure = Some(e),
                }
            }
            self.inner.consume(used);
            if let Some(e) = failure {
                self.state.failed = Some(e);
                if written == 0 {
                    return Err(e.into());
                }
                break;
            }
        }
        Ok(written)
    }
}
