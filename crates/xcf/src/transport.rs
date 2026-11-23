//! Bounded XCF wrapper recognition and a continuing read of the same decoder.
//!
//! A probe is provisional: only [`Source::read`] verifies the complete envelope.
//! Filename/format selection belongs to the caller, not this byte reader.

use std::borrow::Cow;
use std::cell::Cell;
use std::io::{self, BufRead, Read};
use std::rc::Rc;

use crate::{Error, MAGIC, MAX_INFLATED, Result, invalid};

mod bzip;
#[cfg(test)]
mod tests;
mod xz;

const CHUNK: usize = 32 * 1024;

/// The compression envelope, independent of tile compression inside an XCF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrapper {
    Gzip,
    Bzip2,
    Xz,
}

impl Wrapper {
    /// Recognize only the fixed wrapper signature, without decompressing input.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0x1f, 0x8b]) {
            Some(Self::Gzip)
        } else if bytes.starts_with(b"BZh") {
            Some(Self::Bzip2)
        } else if bytes.starts_with(b"\xfd7zXZ\0") {
            Some(Self::Xz)
        } else {
            None
        }
    }

    /// An optional explicit XCF wrapper hint. A suffix alone is not recognition.
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
        if lower.ends_with(".xcf.gz") || lower.ends_with(".xcfgz") {
            Some(Self::Gzip)
        } else if lower.ends_with(".xcf.bz2") || lower.ends_with(".xcfbz2") {
            Some(Self::Bzip2)
        } else if lower.ends_with(".xcf.xz") || lower.ends_with(".xcfxz") {
            Some(Self::Xz)
        } else {
            None
        }
    }
}

/// Independent prefix, retained-output and decoder-memory bounds.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum complete decoded source, counting the prefix once.
    pub max_inflated: usize,
    /// Maximum provisional prefix, including the XCF magic. No trailer guarantee.
    pub probe_bytes: usize,
    /// Per-block XZ dictionary plus decoder overhead, in KiB.
    pub max_xz_memory_kib: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_inflated: MAX_INFLATED,
            probe_bytes: MAGIC.len() + 5 + 4 * 4,
            // A 64 MiB dictionary also needs the pinned decoder's bounded overhead.
            max_xz_memory_kib: lzma_rust2::lzma2_get_memory_usage(64 << 20),
        }
    }
}

/// Unrelated bytes or a source whose inflated XCF signature was recognized.
pub enum Probe<'a> {
    Unrelated,
    Xcf(Source<'a>),
}

/// A provisional source. Compressed sources keep their one live decoder.
pub struct Source<'a> {
    original: &'a [u8],
    bytes: Cow<'a, [u8]>,
    decoder: Option<Box<dyn Read + 'a>>,
    wrapper: Option<Wrapper>,
    limits: Limits,
    state: Rc<State<'a>>,
}

impl Source<'_> {
    /// Header evidence only; neither a successful parse nor a verified trailer.
    pub fn prefix(&self) -> &[u8] {
        self.bytes.get(..self.bytes.len().min(self.limits.probe_bytes)).unwrap_or_default()
    }

    pub fn wrapper(&self) -> Option<Wrapper> {
        self.wrapper
    }
}

impl<'a> Source<'a> {
    /// Finish this decoder, verifying every member, trailer and admitted padding.
    /// Plain input is borrowed; compressed input has one retained decoded buffer.
    pub fn read(mut self) -> Result<Cow<'a, [u8]>> {
        self.state.check()?;
        let Some(mut decoder) = self.decoder.take() else {
            if self.bytes.len() > self.limits.max_inflated {
                return Err(invalid("XCF source exceeds the decoded byte limit"));
            }
            return Ok(self.bytes);
        };
        let Cow::Owned(mut output) = self.bytes else { return Err(invalid("missing decoded XCF prefix")) };
        let mut chunk = [0; CHUNK];
        loop {
            self.state.check()?;
            // Read one beyond the remaining allowance to distinguish exact EOF
            // (including trailers) from an oversized source without retaining it.
            let capacity = self.limits.max_inflated.saturating_sub(output.len()).saturating_add(1).min(CHUNK);
            let target = chunk.get_mut(..capacity).ok_or_else(|| invalid("invalid XCF output capacity"))?;
            let count = decoder.read(target).map_err(|error| self.state.decode_error(error))?;
            if count == 0 {
                break;
            }
            append(&mut output, chunk.get(..count).ok_or_else(|| invalid("invalid XCF output length"))?, self.limits.max_inflated)?;
        }
        drop(decoder);
        self.state.check()?;
        if self.state.consumed.get() != self.original.len() {
            return Err(invalid("unconsumed data after the XCF wrapper"));
        }
        if self.wrapper == Some(Wrapper::Xz) {
            xz::validate_envelope(self.original, output.len(), &self.state)?;
        }
        Ok(Cow::Owned(output))
    }
}

/// Probe without validating unread bytes. A wrapper hint makes malformed/non-XCF
/// content an error; otherwise an unrecognized inflated signature is unrelated.
/// Cancellation always wins, including inside headers and empty members.
pub fn probe<'a>(bytes: &'a [u8], hint: Option<Wrapper>, limits: Limits, keep_going: &'a dyn Fn() -> bool) -> Result<Probe<'a>> {
    let state = Rc::new(State { keep_going, cancelled: Cell::new(false), consumed: Cell::new(0) });
    state.check()?;
    if limits.probe_bytes < MAGIC.len() || limits.max_inflated < MAGIC.len() {
        return Err(invalid("XCF transport limits cannot hold its signature"));
    }
    if bytes.starts_with(MAGIC) {
        return Ok(Probe::Xcf(Source { original: bytes, bytes: Cow::Borrowed(bytes), decoder: None, wrapper: None, limits, state }));
    }
    let Some(wrapper) = Wrapper::sniff(bytes) else {
        return if hint.is_some() { Err(invalid("missing XCF wrapper signature")) } else { Ok(Probe::Unrelated) };
    };
    let input = Input { remaining: bytes, state: Rc::clone(&state) };
    let mut decoder: Box<dyn Read + 'a> = match wrapper {
        Wrapper::Gzip => Box::new(Gzip::new(input)),
        Wrapper::Bzip2 => Box::new(bzip::Reader::new(input)),
        Wrapper::Xz => Box::new(lzma_rust2::XzReader::new_mem_limit(input, true, limits.max_xz_memory_kib)),
    };
    let mut magic = [0; MAGIC.len()];
    let mut length = 0;
    while length < magic.len() {
        state.check()?;
        match decoder.read(magic.get_mut(length..).ok_or_else(|| invalid("invalid XCF prefix length"))?) {
            Ok(0) => break,
            Ok(count) => length += count,
            Err(error) => {
                let error = state.decode_error(error);
                return if error == Error::Cancelled || hint.is_some() { Err(error) } else { Ok(Probe::Unrelated) };
            }
        }
    }
    state.check()?;
    if length != magic.len() || magic != MAGIC {
        return if hint.is_some() {
            Err(invalid("wrapper does not hold a plain XCF; recursive wrappers are unsupported"))
        } else {
            Ok(Probe::Unrelated)
        };
    }
    let mut output = Vec::new();
    append(&mut output, &magic, limits.max_inflated)?;
    let prefix_limit = limits.probe_bytes.min(limits.max_inflated);
    let mut chunk = [0; CHUNK];
    while output.len() < prefix_limit {
        state.check()?;
        let capacity = (prefix_limit - output.len()).min(CHUNK);
        let count = decoder
            .read(chunk.get_mut(..capacity).ok_or_else(|| invalid("invalid XCF prefix capacity"))?)
            .map_err(|error| state.decode_error(error))?;
        if count == 0 {
            break;
        }
        append(&mut output, chunk.get(..count).ok_or_else(|| invalid("invalid XCF prefix length"))?, limits.max_inflated)?;
    }
    state.check()?;
    Ok(Probe::Xcf(Source { original: bytes, bytes: Cow::Owned(output), decoder: Some(decoder), wrapper: Some(wrapper), limits, state }))
}

fn append(output: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<()> {
    let end =
        output.len().checked_add(bytes.len()).filter(|end| *end <= limit).ok_or_else(|| invalid("XCF wrapper exceeds the decoded byte limit"))?;
    if end > output.capacity() {
        let capacity = end.max(CHUNK).max(output.capacity().saturating_mul(2)).min(limit);
        output.try_reserve_exact(capacity - output.len()).map_err(|_| invalid("unable to reserve decoded XCF storage"))?;
    }
    output.extend_from_slice(bytes);
    Ok(())
}

struct State<'a> {
    keep_going: &'a dyn Fn() -> bool,
    cancelled: Cell<bool>,
    consumed: Cell<usize>,
}

impl State<'_> {
    fn check(&self) -> Result<()> {
        if self.cancelled.get() || !(self.keep_going)() {
            self.cancelled.set(true);
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    fn decode_error(&self, error: io::Error) -> Error {
        if self.check().is_err() { Error::Cancelled } else { invalid(format!("XCF wrapper: {error}")) }
    }
}

struct Input<'a> {
    remaining: &'a [u8],
    state: Rc<State<'a>>,
}

impl BufRead for Input<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.state.check().map_err(io::Error::other)?;
        Ok(self.remaining.get(..self.remaining.len().min(CHUNK)).unwrap_or_default())
    }

    fn consume(&mut self, amount: usize) {
        let count = amount.min(self.remaining.len());
        self.remaining = self.remaining.get(count..).unwrap_or_default();
        self.state.consumed.set(self.state.consumed.get().saturating_add(count));
    }
}

impl Read for Input<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let input = self.fill_buf()?;
        let count = input.len().min(output.len());
        output
            .get_mut(..count)
            .ok_or_else(|| io::Error::other("invalid input destination"))?
            .copy_from_slice(input.get(..count).ok_or_else(|| io::Error::other("invalid input range"))?);
        self.consume(count);
        Ok(count)
    }
}

/// XCF permits verified concatenated members and terminal zero padding only.
struct Gzip<R: BufRead> {
    member: Option<flate2::bufread::GzDecoder<R>>,
}

impl<R: BufRead> Gzip<R> {
    fn new(input: R) -> Self {
        Self { member: Some(flate2::bufread::GzDecoder::new(input)) }
    }
}

impl<R: BufRead> Read for Gzip<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            let Some(member) = &mut self.member else { return Ok(0) };
            let count = member.read(output)?;
            if count != 0 {
                return Ok(count);
            }
            let member = self.member.take().ok_or_else(|| io::Error::other("missing gzip member"))?;
            let mut input = member.into_inner();
            let next = input.fill_buf()?;
            if next.is_empty() {
                return Ok(0);
            }
            if next.first() == Some(&0) {
                loop {
                    let padding = input.fill_buf()?;
                    if padding.is_empty() {
                        return Ok(0);
                    }
                    if padding.iter().any(|byte| *byte != 0) {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "nonzero data after gzip padding"));
                    }
                    let count = padding.len();
                    input.consume(count);
                }
            }
            self.member = Some(flate2::bufread::GzDecoder::new(input));
        }
    }
}
