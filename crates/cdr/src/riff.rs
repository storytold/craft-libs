//! The chunk index: one flat table of every chunk, a list's children contiguous, built without
//! recursion. Payloads are not copied: an entry names a range of the RIFF stream, of the bytes
//! inflated from `LIST cmpr`, or of a package's data stream (layouts R1 to R4, R27).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Read as _;

use crate::bytes::{Rd, u32_at};
use crate::layout;
use crate::limits::Budget;
use crate::package::Streams;
use crate::{Error, Warning};

pub(crate) const LIST: [u8; 4] = *b"LIST";
const CMPR: [u8; 4] = *b"cmpr";
const CPNG: [u8; 4] = *b"CPng";
/// What follows `CPng` before the zlib stream (R3).
const CPNG_HEAD: [u8; 4] = [1, 0, 4, 0];
/// The size of a redirect record, and the stream number of one that holds its body (R27).
const REDIRECT: usize = 16;
const INLINE: u32 = u32::MAX;

/// One chunk: 28 bytes. `start` and `len` give the payload (after a list's form type) in its
/// source: 0 is the RIFF stream followed by the inflated bytes, `s + 1` is data stream `s`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub(crate) id: [u8; 4],
    /// A list's form type; zeros for other chunks.
    pub(crate) form: [u8; 4],
    src: u32,
    start: u32,
    len: u32,
    first: u32,
    count: u32,
}

impl Chunk {
    pub(crate) fn is_list(&self, form: &[u8; 4]) -> bool {
        self.id == LIST && &self.form == form
    }
}

/// Every chunk of a file, and the bytes they point into.
pub(crate) struct Index<'a> {
    input: &'a [u8],
    inflated: Vec<u8>,
    chunks: Vec<Chunk>,
    /// The version the form type names: hundreds, as `vrsn` stores it (0 when the `vrsn` chunk
    /// decides).
    pub(crate) form_version: u16,
    /// A version 16+ package's data streams; chunks of 16 bytes are redirect records then.
    streams: Option<Streams<'a>>,
    /// Bytes that inflating may still produce.
    left: Cell<usize>,
    /// The bytes redirected lists have taken: (package member, start) to end.
    lists: Cell<BTreeMap<(usize, usize), usize>>,
    /// The bytes other redirected chunks have taken, the same way.
    leaves: Cell<BTreeMap<(usize, usize), usize>>,
    /// Bytes that redirects to bytes another chunk already took may still read.
    shared_left: Cell<usize>,
    /// The first `vrsn` chunk among the file's own chunks, once seen, and the version it names
    /// (`None` when it can't be read) (R2).
    vrsn: Option<Option<u16>>,
}

/// A list still to be split into its children.
struct Pending {
    entry: usize,
    src: u32,
    /// Its children's bytes.
    start: usize,
    end: usize,
    /// The size pool of a compressed list's stream (`start..end` in source 0).
    pool: Option<(usize, usize)>,
    depth: usize,
}

/// What reading one chunk header gives.
enum Read {
    Chunk(Chunk),
    /// A redirect record whose body can't be found: the chunk is left out, its list read on.
    Lost,
    /// A redirect record into a data stream past a limit: the chunk is left out without the
    /// damage warning.
    PastLimit,
    /// A redirect record to bytes another chunk took, past the shared-bytes limit: the chunk is
    /// left out with the limit warning.
    SharedPastLimit,
}

/// The version the RIFF form type names (R1): `CDR5`..`CDR9`, `CDRA` (10) to `CDRH` (17) with `I`
/// unused, then `J` (18) on; `CDR ` is version 3. Some files use a lower-case `cdr`, which names
/// the same versions except that `cdr8` is the 8.01 variant. Templates of version 21 have the form
/// type `CDTQ`; for `CDT` the `vrsn` chunk names the version, so this gives 0 (R28).
pub(crate) fn form_version(form: [u8; 4]) -> Option<u16> {
    let [a, b, c, v] = form;
    match (&[a, b, c], v) {
        (b"cdr", b'8') => Some(801),
        (b"CDR" | b"cdr", b' ') => Some(300),
        (b"CDR" | b"cdr", b'1'..=b'9') => Some(100 * u16::from(v - b'0')),
        (b"CDR" | b"cdr", b'A'..=b'H') => Some(100 * u16::from(v - b'A' + 10)),
        (b"CDR" | b"cdr", b'J'..=b'W') => Some(100 * u16::from(v - b'J' + 18)),
        (b"CDT", b'A'..=b'Z') => Some(0),
        _ => None,
    }
}

/// Does `bytes` start like a RIFF `.cdr` drawing (not the `CDRX` exchange variant)?
pub(crate) fn is_cdr_riff(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12).and_then(|f| f.try_into().ok()).and_then(form_version).is_some()
}

impl<'a> Index<'a> {
    pub(crate) fn build(input: &'a [u8], streams: Option<Streams<'a>>, budget: &mut Budget, warn: &mut Vec<Warning>) -> Result<Self, Error> {
        if input.len() > budget.limits.input {
            return Err(Error::Limit("input size"));
        }
        let mut r = Rd::new(input);
        if r.fourcc() != Some(*b"RIFF") {
            return Err(Error::NotCdr);
        }
        let declared = r.u32().ok_or(Error::NotCdr)? as usize;
        let form = r.fourcc().ok_or(Error::NotCdr)?;
        let version = form_version(form).ok_or(Error::NotCdr)?;
        // A file cut short keeps what it holds: its last chunk is truncated with a warning. A
        // length that ends inside the form type is damage too, and the chunks after it are read.
        let end = match declared.checked_add(8) {
            Some(e) if (12..=input.len()).contains(&e) => e,
            _ => {
                push(warn, Warning::Damaged);
                input.len()
            }
        };
        let left = Cell::new(budget.inflate_left());
        let mut ix = Index {
            input,
            inflated: Vec::new(),
            chunks: Vec::new(),
            form_version: version,
            streams,
            left,
            lists: Cell::default(),
            leaves: Cell::default(),
            shared_left: Cell::new(budget.limits.member),
            vrsn: None,
        };
        ix.chunks.push(Chunk { id: *b"RIFF", form, src: 0, start: 12, len: 0, first: 0, count: 0 });
        let mut queue = vec![Pending { entry: 0, src: 0, start: 12, end, pool: None, depth: 0 }];
        while let Some(p) = queue.pop() {
            ix.split(p, &mut queue, budget, warn)?;
        }
        Ok(ix)
    }

    /// The bytes `start..end` of source `src`, or `None` when the range crosses its end. A data
    /// stream is read on first use.
    fn span(&self, src: u32, start: usize, end: usize) -> Option<&[u8]> {
        if src > 0 {
            let s = self.streams.as_ref()?;
            return s.get(src as usize - 1, &self.left)?.get(start..end);
        }
        let n = self.input.len();
        if end <= n {
            self.input.get(start..end)
        } else if start >= n {
            self.inflated.get(start - n..end - n)
        } else {
            None
        }
    }

    pub(crate) fn data(&self, c: &Chunk) -> &[u8] {
        let start = c.start as usize;
        match self.span(c.src, start, start + c.len as usize) {
            Some(b) => b,
            None => {
                if let Some(s) = &self.streams
                    && !self.past_limit(c.src)
                {
                    s.note_damage();
                }
                &[]
            }
        }
    }

    /// Was source `src` a data stream left out because it is past a limit? Its chunks are then
    /// left out with that warning alone.
    fn past_limit(&self, src: u32) -> bool {
        src > 0 && self.streams.as_ref().is_some_and(|s| s.past_limit(src as usize - 1))
    }

    pub(crate) fn children(&self, c: &Chunk) -> &[Chunk] {
        let first = c.first as usize;
        self.chunks.get(first..first + c.count as usize).unwrap_or_default()
    }

    pub(crate) fn root(&self) -> Option<&Chunk> {
        self.chunks.first()
    }

    /// The version: the `vrsn` chunk's when it names one from 3 to 27, else the form type's (R1,
    /// R2). A template's form type names no version, so its `vrsn` chunk decides whatever it names
    /// (R28). Letters count differently from the version 24 update on (R1), so when both name a
    /// version after 27 the `vrsn` chunk's is the one refused. The `cdr8` form type alone makes a
    /// file the 8.01 variant, whatever its `vrsn` chunk names (R1).
    pub(crate) fn version(&self) -> u16 {
        match self.vrsn.flatten() {
            _ if self.form_version == 801 => 801,
            Some(v) if (layout::OLDEST_RIFF..=layout::NEWEST).contains(&v) => v,
            Some(v) if self.form_version == 0 => v,
            Some(v) if v > layout::NEWEST && self.form_version > layout::NEWEST => v,
            _ => self.form_version,
        }
    }

    /// The warnings for data streams that were left out.
    pub(crate) fn stream_warnings(&self, warn: &mut Vec<Warning>) {
        if let Some(s) = &self.streams {
            s.warnings(warn);
        }
    }

    /// Split list `p` into its children, appended to the table together; their lists are queued.
    fn split(&mut self, p: Pending, queue: &mut Vec<Pending>, budget: &mut Budget, warn: &mut Vec<Warning>) -> Result<(), Error> {
        let first = self.chunks.len();
        let mut at = p.start;
        let mut lists = Vec::new();
        while at < p.end {
            let Some((read, next, cut)) = self.read_chunk(p.src, at, p.end, p.pool) else {
                push(warn, Warning::Damaged);
                break;
            };
            if cut {
                push(warn, Warning::Damaged);
            }
            at = next;
            // A redirect record whose body can't be found counts as well, so that lists redirected
            // to again and again can't repeat such records past the limit.
            budget.take_chunk()?;
            let chunk = match read {
                Read::Chunk(chunk) => chunk,
                Read::Lost => {
                    push(warn, Warning::Damaged);
                    continue;
                }
                Read::PastLimit => continue,
                Read::SharedPastLimit => {
                    push(warn, Warning::TooDeep);
                    continue;
                }
            };
            if p.entry == 0 && chunk.id == *b"vrsn" && self.vrsn.is_none() {
                let at = chunk.start as usize;
                self.vrsn = Some(self.span(chunk.src, at, at + chunk.len as usize).and_then(|b| Rd::new(b).u16()));
            }
            if chunk.id == LIST {
                lists.push(self.chunks.len());
            }
            self.chunks.push(chunk);
        }
        let count = (self.chunks.len() - first) as u32;
        if let Some(parent) = self.chunks.get_mut(p.entry) {
            parent.first = first as u32;
            parent.count = count;
        }
        for i in lists {
            let Some(c) = self.chunks.get(i).copied() else { continue };
            // From version 7 a `LIST stlt` holds the style tables, not chunks (R4); its body is read
            // whole (R45). Before version 7 it holds chunks. The file's own chunks, its `vrsn`
            // among them, are all read before this.
            if c.form == *b"stlt" && self.version() >= 700 {
                continue;
            }
            if p.depth + 1 > budget.limits.depth {
                push(warn, Warning::TooDeep);
                continue;
            }
            let (start, end) = (c.start as usize, c.start as usize + c.len as usize);
            if c.form == CMPR && p.pool.is_none() {
                match self.inflate_cmpr(c.src, start, end) {
                    Ok((s, e, pool)) => queue.push(Pending { entry: i, src: 0, start: s, end: e, pool: Some(pool), depth: p.depth + 1 }),
                    Err(Error::Limit(what)) => return Err(Error::Limit(what)),
                    Err(_) => push(warn, Warning::Damaged),
                }
            } else {
                queue.push(Pending { entry: i, src: c.src, start, end, pool: p.pool, depth: p.depth + 1 });
            }
        }
        Ok(())
    }

    /// The chunk at `at` (before `end`) of source `src`, and where the next one starts. In a
    /// compressed stream the size field is an index into the size pool (R3); in a version 16+
    /// package a chunk of 16 bytes outside one is a redirect record (R27). The flag is set for a
    /// list cut short at `end`.
    fn read_chunk(&self, src: u32, at: usize, end: usize, pool: Option<(usize, usize)>) -> Option<(Read, usize, bool)> {
        let head = self.span(src, at, at.checked_add(8)?.min(end))?;
        let id: [u8; 4] = head.get(..4)?.try_into().ok()?;
        let raw = u32_at(head, 4)? as usize;
        let size = match pool {
            None => raw,
            Some((ps, pe)) => {
                let off = raw.checked_mul(4)?.checked_add(ps)?;
                (off.checked_add(4)? <= pe).then_some(())?;
                u32_at(self.span(0, off, off + 4)?, 0)? as usize
            }
        };
        let body = at + 8;
        // A list cut short by the end of its parent keeps the bytes it has, so the children
        // before the cut are read; the caller warns that it is damaged. Other chunks are lost.
        let (size, cut) = match body.checked_add(size) {
            Some(e) if e <= end => (size, false),
            _ if id == LIST => (end.checked_sub(body)?, true),
            _ => return None,
        };
        let next = (body + size + (size & 1)).min(end);
        if cut {
            return Some((self.chunk(id, src, body, size).map_or(Read::Lost, Read::Chunk), next, true));
        }
        if self.streams.is_some() && pool.is_none() && size == REDIRECT {
            let read = match self.redirect(id, src, body) {
                Some(read) => read,
                None if self.redirect_past_limit(src, body) => Read::PastLimit,
                None => Read::Lost,
            };
            return Some((read, next, false));
        }
        Some((Read::Chunk(self.chunk(id, src, body, size)?), next, false))
    }

    /// Chunk `id` whose body is `len` bytes at `body` of source `src`; a list's body starts with
    /// its form type.
    fn chunk(&self, id: [u8; 4], src: u32, body: usize, len: usize) -> Option<Chunk> {
        let (form, start) = if id == LIST {
            let f: [u8; 4] = self.span(src, body, body.checked_add(4).filter(|_| len >= 4)?)?.try_into().ok()?;
            (f, body + 4)
        } else {
            ([0; 4], body)
        };
        let len = len - (start - body);
        Some(Chunk { id, form, src, start: u32::try_from(start).ok()?, len: u32::try_from(len).ok()?, first: 0, count: 0 })
    }

    /// The chunk the redirect record at `body` of `src` stands for (R27): stream, length, offset
    /// and a reserved u32. Stream `0xFFFFFFFF` holds at most 8 bytes in the record itself, after
    /// the length. The body is not read again as a redirect record. `None` when the body can't
    /// be found.
    ///
    /// A redirected list whose bytes overlap those of an earlier redirected list, in any stream of
    /// the same package member, is left out, so that no list is split twice. Other chunks may
    /// share bytes: each one after the first to read them takes their length from the
    /// shared-bytes limit (the member limit), so that many redirects to one body can't decode it
    /// without end.
    fn redirect(&self, id: [u8; 4], src: u32, body: usize) -> Option<Read> {
        let mut r = Rd::new(self.span(src, body, body + REDIRECT)?);
        let stream = r.u32()?;
        let len = r.u32()? as usize;
        if stream == INLINE {
            return (len <= 8).then(|| self.chunk(id, src, body + 8, len).map(Read::Chunk))?;
        }
        let off = r.u32()? as usize;
        let streams = self.streams.as_ref()?;
        let size = streams.size(stream as usize)?;
        let end = off.checked_add(len).filter(|&e| e <= size)?;
        let member = streams.member_of(stream as usize)?;
        if id == LIST {
            if !claim(&self.lists, member, off, end) {
                return None;
            }
        } else if !claim(&self.leaves, member, off, end) {
            let left = self.shared_left.get();
            if len > left {
                return Some(Read::SharedPastLimit);
            }
            self.shared_left.set(left - len);
        }
        // Only a list's form type is read now; other bodies are read when the mapping asks.
        self.chunk(id, stream.checked_add(1)?, off, len).map(Read::Chunk)
    }

    /// Does the redirect record at `body` of `src` point into a data stream past a limit?
    fn redirect_past_limit(&self, src: u32, body: usize) -> bool {
        let stream = self.span(src, body, body + 4).and_then(|b| u32_at(b, 0));
        stream.is_some_and(|s| s != INLINE && self.past_limit(s.saturating_add(1)))
    }

    /// Inflate the two blocks of a `LIST cmpr` body at `start..end` of `src` (R3) → the chunk
    /// stream and the size pool, as ranges of source 0.
    fn inflate_cmpr(&mut self, src: u32, start: usize, end: usize) -> Result<(usize, usize, (usize, usize)), Error> {
        let body = self.span(src, start, end).ok_or(Error::Damaged("compressed list"))?;
        let mut r = Rd::new(body);
        let c1 = r.u32().ok_or(Error::Damaged("compressed list"))? as usize;
        r.skip(4).ok_or(Error::Damaged("compressed list"))?;
        let c2 = r.u32().ok_or(Error::Damaged("compressed list"))? as usize;
        let block = |at: usize, len: usize| -> Result<&[u8], Error> {
            let b = body.get(at..at.checked_add(len).ok_or(Error::Damaged("compressed list"))?).ok_or(Error::Damaged("compressed list"))?;
            let mut h = Rd::new(b);
            if h.fourcc() != Some(CPNG) || h.fourcc() != Some(CPNG_HEAD) {
                return Err(Error::Damaged("compressed block"));
            }
            Ok(b.get(8..).unwrap_or_default())
        };
        let second_at = c1.checked_add(16).ok_or(Error::Damaged("compressed list"))?;
        let (first, second) = (block(16, c1)?.to_vec(), block(second_at, c2)?.to_vec());
        let base = self.input.len();
        let stream = self.inflate(&first)?;
        let pool = self.inflate(&second)?;
        Ok((base + stream.0, base + stream.1, (base + pool.0, base + pool.1)))
    }

    /// Inflate zlib `data` onto the end of the inflated bytes, within what is left → its range
    /// there.
    fn inflate(&mut self, data: &[u8]) -> Result<(usize, usize), Error> {
        let at = self.inflated.len();
        let left = self.left.get();
        let reader = flate2::read::ZlibDecoder::new(data);
        // One byte past what is left tells a stream that is too long from one that just fits.
        let got = match reader.take((left as u64).saturating_add(1)).read_to_end(&mut self.inflated) {
            Ok(n) => n,
            Err(_) => {
                // What a stream inflated before it failed counts toward the limit too, so that
                // streams that fail late can't inflate past it again and again.
                let produced = self.inflated.len().saturating_sub(at);
                self.left.set(left.saturating_sub(produced));
                self.inflated.truncate(at);
                return Err(Error::Damaged("compressed block"));
            }
        };
        if got > left {
            self.inflated.truncate(at);
            return Err(Error::Limit("inflated size"));
        }
        self.left.set(left - got);
        if self.input.len().saturating_add(self.inflated.len()) > u32::MAX as usize {
            return Err(Error::Limit("inflated size"));
        }
        Ok((at, at + got))
    }
}

/// Take bytes `start..end` of package member `member` in `claimed`, or `false` when an earlier
/// redirect there took any of them. Empty ranges take nothing.
fn claim(claimed: &Cell<BTreeMap<(usize, usize), usize>>, member: usize, start: usize, end: usize) -> bool {
    if start == end {
        return true;
    }
    let mut taken = claimed.take();
    let before = taken.range(..=(member, start)).next_back().is_some_and(|(&(m, _), &e)| m == member && e > start);
    let after = taken.range((member, start)..).next().is_some_and(|(&(m, s), _)| m == member && s < end);
    let free = !before && !after;
    if free {
        taken.insert((member, start), end);
    }
    claimed.set(taken);
    free
}

/// Add `w` to the warnings once.
pub(crate) fn push(warn: &mut Vec<Warning>, w: Warning) {
    if !warn.contains(&w) {
        warn.push(w);
    }
}
