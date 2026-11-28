//! Byte-window reader and element header parsing.

use super::{RECOVERY_BUDGET, WIN};
use crate::matroska::ebml::{Size, read_id, read_size};
use crate::matroska::error::{Error, Result};
use crate::matroska::ids::{CLUSTER, CRC32, EBML, SEGMENT, TIMESTAMP, VOID, is_top_level};
use crate::source::ByteSource;

#[derive(Clone, Debug)]
pub(super) struct Cache {
    pub(super) buf: Vec<u8>,
    pub(super) start: u64,
    /// Bytes read from the source to refill the window, all told.
    pub(super) refilled: u64,
    /// Source bytes left for damage recovery ([`Io::resync`]) in this file. Each candidate it tests can
    /// refill the window several times, so the work is charged by what was actually read.
    pub(super) recovery_left: u64,
}

impl Default for Cache {
    fn default() -> Self {
        Self { buf: Vec::new(), start: 0, refilled: 0, recovery_left: RECOVERY_BUDGET }
    }
}

pub(super) struct Io<'a, S: ByteSource + ?Sized> {
    pub(super) src: &'a S,
    pub(super) len: u64,
    /// Bytes the caller has available (an [`OpenOptions::index_limit`](super::OpenOptions::index_limit)):
    /// the read window does not run past it unless a read needs the bytes, so a source that has
    /// loaded only the front of the file is not asked for the rest. `len` without a limit.
    pub(super) avail: u64,
    pub(super) cache: &'a mut Cache,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Hdr {
    pub(super) id: u32,
    pub(super) pos: u64,
    pub(super) size: Size,
    pub(super) hlen: usize,
}

impl Hdr {
    pub(super) fn data_start(&self) -> u64 {
        self.pos + self.hlen as u64
    }
    /// End offset, clamped to `limit`; unknown sizes run to `limit`.
    pub(super) fn end(&self, limit: u64) -> u64 {
        match self.size {
            Size::Known(n) => self.data_start().saturating_add(n).min(limit),
            Size::Unknown => limit,
        }
    }
    pub(super) fn overruns(&self, limit: u64) -> bool {
        matches!(self.size, Size::Known(n) if self.data_start().saturating_add(n) > limit)
    }
}

impl<'a, S: ByteSource + ?Sized> Io<'a, S> {
    pub(super) fn new(src: &'a S, cache: &'a mut Cache) -> Self {
        let len = src.len();
        Self { src, len, avail: len, cache }
    }

    /// Up to `n` (≤ WIN) bytes at `pos` (fewer at EOF).
    pub(super) fn peek(&mut self, pos: u64, n: usize) -> Result<&[u8]> {
        let n = n.min(WIN);
        if pos >= self.len {
            return Ok(&[]);
        }
        let want_end = (pos + n as u64).min(self.len);
        let c = &*self.cache;
        if !(pos >= c.start && want_end <= c.start + c.buf.len() as u64) {
            let window_end = (pos + WIN as u64).min(self.len);
            // stay inside what is available, but never fetch less than this read needs
            let end = if self.avail > pos { window_end.min(self.avail).max(want_end) } else { want_end };
            let fill = (end - pos) as usize;
            let mut buf = std::mem::take(&mut self.cache.buf);
            buf.resize(fill, 0);
            self.src.read_at(pos, &mut buf)?;
            self.cache.refilled = self.cache.refilled.saturating_add(fill as u64);
            self.cache.buf = buf;
            self.cache.start = pos;
        }
        let o = (pos - self.cache.start) as usize;
        Ok(&self.cache.buf[o..o + (want_end - pos) as usize])
    }

    /// Exactly `n` bytes at `pos`.
    pub(super) fn read_vec(&mut self, pos: u64, n: u64) -> Result<Vec<u8>> {
        if pos.checked_add(n).is_none_or(|e| e > self.len) {
            return Err(Error::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "read past end of file")));
        }
        if n as usize <= WIN / 2 {
            return Ok(self.peek(pos, n as usize)?.to_vec());
        }
        let mut v = vec![0u8; n as usize];
        self.src.read_at(pos, &mut v)?;
        Ok(v)
    }

    /// Element header at `pos` (`None` if undecodable or past `limit`).
    pub(super) fn header(&mut self, pos: u64, limit: u64) -> Result<Option<Hdr>> {
        if pos >= limit {
            return Ok(None);
        }
        // never more than the parent holds: its last element may be shorter than a maximal header
        let b = self.peek(pos, 12.min(usize::try_from(limit - pos).unwrap_or(12)))?;
        let Some((id, il)) = read_id(b) else { return Ok(None) };
        let Some((size, sl)) = read_size(&b[il..]) else { return Ok(None) };
        let hlen = il + sl;
        if pos + hlen as u64 > limit {
            return Ok(None);
        }
        Ok(Some(Hdr { id, pos, size, hlen }))
    }

    /// Scan forward from `from` for a plausible level-1 element (Cluster preferred), up to `limit`.
    ///
    /// The scan stops (`None`) once the file's recovery budget is spent: every candidate can make
    /// [`Io::plausible`] refill the window at a far offset, which a damaged region would repeat per byte.
    pub(super) fn resync(&mut self, from: u64, limit: u64) -> Result<Option<u64>> {
        let mut pos = from;
        while pos + 4 <= limit {
            if self.cache.recovery_left == 0 {
                return Ok(None);
            }
            let before = self.cache.refilled;
            let chunk = self.peek(pos, WIN)?.to_vec();
            self.charge_recovery(before);
            if chunk.len() < 4 {
                return Ok(None);
            }
            for i in 0..chunk.len() - 3 {
                if chunk[i] & 0xF0 != 0x10 {
                    continue;
                }
                let id = u32::from_be_bytes([chunk[i], chunk[i + 1], chunk[i + 2], chunk[i + 3]]);
                if !is_top_level(id) || id == EBML || id == SEGMENT {
                    continue;
                }
                let p = pos + i as u64;
                let before = self.cache.refilled;
                let found = self.plausible(p, id, limit)?;
                self.charge_recovery(before);
                if found {
                    return Ok(Some(p));
                }
                if self.cache.recovery_left == 0 {
                    return Ok(None);
                }
            }
            pos += (chunk.len() - 3) as u64;
        }
        Ok(None)
    }

    /// Charge the recovery budget for the window refills since `refilled_before`.
    fn charge_recovery(&mut self, refilled_before: u64) {
        let spent = self.cache.refilled.saturating_sub(refilled_before);
        self.cache.recovery_left = self.cache.recovery_left.saturating_sub(spent);
    }

    pub(super) fn plausible(&mut self, p: u64, id: u32, limit: u64) -> Result<bool> {
        let Some(h) = self.header(p, limit)? else { return Ok(false) };
        if h.id != id || h.overruns(limit) {
            return Ok(false);
        }
        if h.size == Size::Unknown && id != CLUSTER {
            return Ok(false);
        }
        // first child must decode and fit; clusters must start with Timestamp (after CRC-32/Void)
        let mut cp = h.data_start();
        let end = h.end(limit);
        for _ in 0..3 {
            let Some(c) = self.header(cp, end)? else { return Ok(false) };
            if c.overruns(end) {
                return Ok(false);
            }
            if id != CLUSTER {
                return Ok(true);
            }
            match c.id {
                TIMESTAMP => return Ok(true),
                CRC32 | VOID => cp = c.end(end),
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    /// End of an unknown-size master element: the first child that is a top-level ID or undecodable.
    pub(super) fn unknown_end(&mut self, data_start: u64, limit: u64) -> Result<u64> {
        let mut pos = data_start;
        while pos < limit {
            let Some(h) = self.header(pos, limit)? else { return Ok(pos) };
            if is_top_level(h.id) || h.size == Size::Unknown {
                return Ok(pos);
            }
            pos = h.end(limit);
        }
        Ok(limit)
    }
}
