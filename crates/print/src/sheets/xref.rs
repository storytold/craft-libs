//! The lenient reader: cross-reference sections and streams, object streams, Flate and PNG predictors, and the decoding budget.

use super::*;

use std::borrow::Cow;
use std::collections::HashMap;

/// Where an object is: at an offset, or the `index`th object of an object stream.
#[derive(Clone, Copy, Debug)]
pub(super) enum Entry {
    Free,
    At(usize),
    In(u32, u32),
}

/// An object stream, inflated: each object's number and offset within `data`.
pub(super) struct ObjStm {
    pub(super) data: Vec<u8>,
    pub(super) objects: Vec<(u32, usize)>,
}

#[derive(Default)]
pub(super) struct Budget {
    pub(super) records: usize,
    pub(super) decoded: usize,
    pub(super) nodes: usize,
    pub(super) tokens: usize,
    pub(super) exhausted: bool,
}

impl Budget {
    pub(super) fn charge(&mut self, records: usize, decoded: usize, nodes: usize, tokens: usize) -> Option<()> {
        if self.exhausted {
            return None;
        }
        let fits = self.records.checked_add(records).is_some_and(|n| n <= MAX_RECORDS)
            && self.decoded.checked_add(decoded).is_some_and(|n| n <= MAX_DECODED)
            && self.nodes.checked_add(nodes).is_some_and(|n| n <= MAX_NODES)
            && self.tokens.checked_add(tokens).is_some_and(|n| n <= MAX_TOKENS);
        if !fits {
            self.exhausted = true;
            return None;
        }
        self.records += records;
        self.decoded += decoded;
        self.nodes += nodes;
        self.tokens += tokens;
        Some(())
    }

    pub(super) fn parsed(&mut self, lx: &Lexer<'_>) -> Option<()> {
        if lx.exhausted {
            self.exhausted = true;
            return None;
        }
        self.charge(0, 0, lx.nodes, lx.tokens)
    }
}

pub(super) struct Reader<'a> {
    pub(super) data: &'a [u8],
    pub(super) entries: HashMap<u32, Entry>,
    pub(super) objstms: HashMap<u32, Option<ObjStm>>,
    pub(super) budget: Budget,
}

/// An indirect object read at an offset: its number, value and stream data.
pub(super) struct Indirect<'a> {
    pub(super) num: u32,
    pub(super) obj: Obj,
    pub(super) stream: Option<&'a [u8]>,
}

pub(super) fn indirect<'a>(data: &'a [u8], offset: usize, budget: &mut Budget) -> Option<Indirect<'a>> {
    let mut lx = Lexer::bounded(data, offset, budget);
    let num = u32::try_from(lx.unsigned()?).ok()?;
    lx.unsigned()?;
    if !lx.keyword(b"obj") {
        return None;
    }
    let obj = lx.object(0);
    budget.parsed(&lx)?;
    let obj = obj?;
    let mut stream = None;
    if let Obj::Dict(dict) = &obj
        && lx.keyword(b"stream")
    {
        // The data starts after the end of line following `stream` (CR LF or LF; a lone CR is
        // tolerated).
        match lx.rest() {
            [b'\r', b'\n', ..] => lx.pos += 2,
            [b'\n' | b'\r', ..] => lx.pos += 1,
            _ => {}
        }
        let start = lx.pos;
        let by_length = get(dict, b"Length")
            .and_then(Obj::int)
            .and_then(|l| usize::try_from(l).ok())
            .and_then(|l| start.checked_add(l))
            .filter(|&end| Lexer::new(data, end).keyword(b"endstream"));
        let end = match by_length {
            Some(end) => end,
            // An indirect or wrong /Length: the data runs to `endstream`, less its end of line.
            None => {
                let rest = data.get(start..)?;
                let at = rest.windows(9).position(|w| w == b"endstream")?;
                let mut end = start + at;
                if data.get(end.wrapping_sub(1)) == Some(&b'\n') {
                    end -= 1;
                }
                if data.get(end.wrapping_sub(1)) == Some(&b'\r') {
                    end -= 1;
                }
                end.max(start)
            }
        };
        stream = data.get(start..end);
    }
    Some(Indirect { num, obj, stream })
}

/// How an inflate ended.
pub(super) enum Inflated {
    /// The whole stream, within the limit.
    Done(Vec<u8>),
    /// The output would pass the limit.
    Exhausted,
    /// A damaged or truncated stream: only the bytes actually written before the damage.
    Damaged(Vec<u8>),
}

/// Inflate `raw` (zlib-wrapped or bare deflate) to at most `limit` bytes. Unlike the
/// `decompress_to_vec*` helpers it reports the bytes the decompressor really wrote, so a stream
/// rejected early yields an empty `Damaged`, never the zero-filled output buffer.
pub(super) fn inflate(raw: &[u8], zlib: bool, limit: usize) -> Inflated {
    use miniz_oxide::inflate::TINFLStatus;
    use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
    let flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF | if zlib { inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER } else { 0 };
    let mut out = vec![0u8; raw.len().saturating_mul(2).max(64).min(limit)];
    let mut state = Box::<DecompressorOxide>::default();
    let (mut input, mut written) = (raw, 0usize);
    loop {
        let (status, consumed, wrote) = decompress(&mut state, input, &mut out, written, flags);
        written = written.saturating_add(wrote).min(out.len());
        match status {
            TINFLStatus::Done => {
                out.truncate(written);
                return Inflated::Done(out);
            }
            TINFLStatus::HasMoreOutput => {
                if out.len() >= limit {
                    return Inflated::Exhausted;
                }
                let Some(rest) = input.get(consumed..) else {
                    out.truncate(written);
                    return Inflated::Damaged(out);
                };
                input = rest;
                out.resize(out.len().saturating_mul(2).min(limit), 0);
            }
            _ => {
                out.truncate(written);
                return Inflated::Damaged(out);
            }
        }
    }
}

/// A stream's data with its filters undone: FlateDecode (with PNG predictors), or none.
pub(super) fn decode(dict: &Dict, raw: &[u8], budget: &mut Budget) -> Option<Vec<u8>> {
    if budget.exhausted {
        return None;
    }
    let limit = MAX_STREAM.min(MAX_DECODED.saturating_sub(budget.decoded));
    if limit == 0 {
        budget.exhausted = true;
        return None;
    }
    let filters: Vec<&[u8]> = match get(dict, b"Filter") {
        None => Vec::new(),
        Some(Obj::Name(n)) => vec![n.as_slice()],
        Some(Obj::Array(a)) => a.iter().map(Obj::name).collect::<Option<_>>()?,
        Some(_) => return None,
    };
    let parms = match get(dict, b"DecodeParms") {
        Some(Obj::Dict(d)) => Some(d),
        Some(Obj::Array(a)) => a.first().and_then(Obj::dict),
        _ => None,
    };
    let data = match filters.as_slice() {
        [] => {
            if raw.len() > limit {
                budget.exhausted = true;
                return None;
            }
            Some(raw.to_vec())
        }
        [b"FlateDecode" | b"Fl"] => {
            let inflated = match inflate(raw, true, limit) {
                Inflated::Done(v) => v,
                Inflated::Exhausted => {
                    budget.exhausted = true;
                    return None;
                }
                // A damaged stream: what inflated before the damage, as other readers do.
                Inflated::Damaged(v) if !v.is_empty() => v,
                // Rejected before any output: retry as a bare deflate stream.
                Inflated::Damaged(_) => match inflate(raw, false, limit) {
                    Inflated::Done(v) => v,
                    Inflated::Exhausted => {
                        budget.exhausted = true;
                        return None;
                    }
                    Inflated::Damaged(_) => return None,
                },
            };
            unpredict(&inflated, parms, limit)
        }
        _ => None,
    }?;
    budget.charge(0, data.len(), 0, 0)?;
    Some(data)
}

/// Undo a PNG predictor (`/Predictor` 10–15), which cross-reference streams use.
pub(super) fn unpredict(data: &[u8], parms: Option<&Dict>, limit: usize) -> Option<Vec<u8>> {
    let int = |key: &[u8], default: i64| parms.and_then(|p| get(p, key)).and_then(Obj::int).unwrap_or(default);
    let predictor = int(b"Predictor", 1);
    if predictor < 10 {
        return (predictor == 1).then(|| data.to_vec());
    }
    let colors = usize::try_from(int(b"Colors", 1)).ok().filter(|c| (1..=32).contains(c))?;
    let bits = usize::try_from(int(b"BitsPerComponent", 8)).ok().filter(|b| [1, 2, 4, 8, 16].contains(b))?;
    let columns = usize::try_from(int(b"Columns", 1)).ok().filter(|c| (1..=1 << 20).contains(c))?;
    let bpp = (colors * bits).div_ceil(8).max(1);
    let row = (columns * colors * bits).div_ceil(8);
    if row > limit || !data.len().is_multiple_of(row + 1) || data.len() / (row + 1) * row > limit {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let (&kind, line) = chunk.split_first()?;
        let mut cur = line.to_vec();
        cur.resize(row, 0);
        for i in 0..row {
            let a = if i >= bpp { cur.get(i - bpp).copied().unwrap_or(0) } else { 0 };
            let b = prev.get(i).copied().unwrap_or(0);
            let c = if i >= bpp { prev.get(i - bpp).copied().unwrap_or(0) } else { 0 };
            let add = match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return None,
            };
            if let Some(v) = cur.get_mut(i) {
                *v = v.wrapping_add(add);
            }
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Some(out)
}

/// One cross-reference section and its trailer (for a stream, its dictionary).
pub(super) fn xref_section(data: &[u8], offset: usize, entries: &mut HashMap<u32, Entry>, budget: &mut Budget) -> Option<(Dict, bool)> {
    let mut lx = Lexer::bounded(data, offset, budget);
    if !lx.keyword(b"xref") {
        let x = indirect(data, offset, budget)?;
        let dict = x.obj.dict()?.clone();
        xref_stream(&dict, x.stream?, entries, budget)?;
        return Some((dict, true));
    }
    let mut table = Vec::new();
    let trailer = loop {
        if lx.keyword(b"trailer") {
            let trailer = lx.object(0);
            budget.parsed(&lx)?;
            break trailer?.dict()?.clone();
        }
        let start = u32::try_from(lx.unsigned()?).ok()?;
        let count = u32::try_from(lx.unsigned()?).ok()?;
        budget.charge(usize::try_from(count).ok()?, 0, 0, 0)?;
        start.checked_add(count.saturating_sub(1))?;
        for i in 0..count {
            let offset = usize::try_from(lx.unsigned()?).ok()?;
            lx.unsigned()?;
            lx.skip_ws();
            let kind = lx.word();
            let Some(num) = start.checked_add(i) else { break };
            table.push((num, if kind == b"n" { Entry::At(offset) } else { Entry::Free }));
        }
    };
    // A hybrid file's stream lists the objects its table leaves out (ISO 32000-1, 7.5.8.4).
    if let Some(at) = get(&trailer, b"XRefStm").and_then(Obj::int).and_then(|v| usize::try_from(v).ok())
        && let Some(x) = indirect(data, at, budget)
        && let (Some(dict), Some(stream)) = (x.obj.dict(), x.stream)
    {
        xref_stream(dict, stream, entries, budget)?;
    }
    for (num, entry) in table {
        entries.entry(num).or_insert(entry);
    }
    Some((trailer, false))
}

pub(super) fn xref_stream(dict: &Dict, raw: &[u8], entries: &mut HashMap<u32, Entry>, budget: &mut Budget) -> Option<()> {
    if get(dict, b"Type").and_then(Obj::name) != Some(b"XRef") {
        return None;
    }

    let widths: Vec<usize> =
        get(dict, b"W")?.array()?.iter().map(|w| w.int().and_then(|w| usize::try_from(w).ok()).filter(|w| *w <= 8)).collect::<Option<_>>()?;
    let [w0, w1, w2] = widths.as_slice() else { return None };
    let entry_len = w0 + w1 + w2;
    if entry_len == 0 {
        return None;
    }
    let size = get(dict, b"Size").and_then(Obj::int).unwrap_or(0);
    let index: Vec<i64> = match get(dict, b"Index").and_then(Obj::array) {
        Some(a) => a.iter().map(Obj::int).collect::<Option<_>>()?,
        None => vec![0, size],
    };
    if !index.len().is_multiple_of(2) {
        return None;
    }
    let mut count = 0usize;
    for &[start, n] in index.as_chunks::<2>().0 {
        let (start, n) = (u32::try_from(start).ok()?, u32::try_from(n).ok()?);
        start.checked_add(n.saturating_sub(1))?;
        count = count.checked_add(usize::try_from(n).ok()?)?;
    }
    // Check counts before inflating or expanding the xref map, including duplicate records.
    budget.charge(count, 0, 0, 0)?;
    let data = decode(dict, raw, budget)?;
    let field = |bytes: &[u8]| bytes.iter().fold(0u64, |v, &b| v.wrapping_shl(8) | u64::from(b));
    let mut records = data.chunks_exact(entry_len);
    for &[start, count] in index.as_chunks::<2>().0 {
        let (Ok(start), Ok(count)) = (u32::try_from(start), u32::try_from(count)) else { return None };
        for i in 0..count {
            let Some(record) = records.next() else { return Some(()) };
            let (kind, rest) = record.split_at_checked(*w0)?;
            let (second, third) = rest.split_at_checked(*w1)?;
            let kind = if *w0 == 0 { 1 } else { field(kind) };
            let (second, third) = (field(second), field(third));
            let entry = match kind {
                0 => Entry::Free,
                1 => Entry::At(usize::try_from(second).ok()?),
                2 => Entry::In(u32::try_from(second).ok()?, u32::try_from(third).ok()?),
                _ => continue,
            };
            let Some(num) = start.checked_add(i) else { break };
            entries.entry(num).or_insert(entry);
        }
    }
    Some(())
}

/// The offset `startxref` names.
pub(super) fn startxref(data: &[u8]) -> Option<usize> {
    let at = data.windows(9).rposition(|w| w == b"startxref")?;
    usize::try_from(Lexer::new(data, at + 9).unsigned()?).ok()
}

impl Reader<'_> {
    pub(super) fn object(&mut self, num: u32) -> Option<Obj> {
        // Geometry fallback must not resume parsing after any job-wide budget is spent.
        if self.budget.exhausted {
            return None;
        }
        match *self.entries.get(&num)? {
            Entry::Free => None,
            Entry::At(offset) => indirect(self.data, offset, &mut self.budget).filter(|x| x.num == num).map(|x| x.obj),
            Entry::In(stm, index) => {
                if !self.objstms.contains_key(&stm) {
                    let loaded = self.load_objstm(stm);
                    self.objstms.insert(stm, loaded);
                }
                let objstm = self.objstms.get(&stm)?.as_ref()?;
                let &(found, offset) = objstm.objects.get(usize::try_from(index).ok()?)?;
                if found != num {
                    return None;
                }
                let mut lx = Lexer::bounded(&objstm.data, offset, &self.budget);
                let obj = lx.object(0);
                self.budget.parsed(&lx)?;
                obj
            }
        }
    }

    pub(super) fn load_objstm(&mut self, stm: u32) -> Option<ObjStm> {
        let Entry::At(offset) = *self.entries.get(&stm)? else { return None };
        let x = indirect(self.data, offset, &mut self.budget).filter(|x| x.num == stm)?;
        let dict = x.obj.dict()?;
        if get(dict, b"Type").and_then(Obj::name) != Some(b"ObjStm") {
            return None;
        }
        let n = get(dict, b"N").and_then(Obj::int).and_then(|n| usize::try_from(n).ok())?;
        self.budget.charge(n, 0, 0, 0)?;
        let data = decode(dict, x.stream?, &mut self.budget)?;
        let first = get(dict, b"First").and_then(Obj::int).and_then(|f| usize::try_from(f).ok())?;
        let mut lx = Lexer::new(&data, 0);
        let mut objects = Vec::new();
        for _ in 0..n {
            let (Some(num), Some(at)) = (lx.unsigned(), lx.unsigned()) else { break };
            let (Ok(num), Some(at)) = (u32::try_from(num), usize::try_from(at).ok().and_then(|a| a.checked_add(first))) else { break };
            objects.push((num, at));
        }
        self.budget.parsed(&lx)?;
        Some(ObjStm { data, objects })
    }

    /// The object a value names, when it is a reference (a few levels at most).
    pub(super) fn resolve(&mut self, obj: &Obj) -> Option<Obj> {
        let mut current = obj.clone();
        for _ in 0..8 {
            match current {
                Obj::Ref(num, _) => current = self.object(num)?,
                other => return Some(other),
            }
        }
        None
    }

    /// Like [`resolve`](Self::resolve), but a direct object is borrowed, not copied. Inherited
    /// page attributes are read once per page, so copying a large inline value would repeat that
    /// work for every page outside the budget.
    pub(super) fn resolve_borrowed<'o>(&mut self, obj: &'o Obj) -> Option<Cow<'o, Obj>> {
        match obj {
            Obj::Ref(..) => self.resolve(obj).map(Cow::Owned),
            direct => Some(Cow::Borrowed(direct)),
        }
    }

    pub(super) fn rect(&mut self, obj: Option<&Obj>) -> Option<[f64; 4]> {
        let obj = self.resolve_borrowed(obj?)?;
        let items = obj.array()?;
        // The shape is checked before reading any element.
        if items.len() != 4 {
            return None;
        }
        let mut v = [0.0; 4];
        for (slot, item) in v.iter_mut().zip(items) {
            *slot = self.resolve_borrowed(item)?.num()?;
        }
        let [x0, y0, x1, y1] = v;
        Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
    }
}
