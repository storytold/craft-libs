use crate::{FormatError, Limits, bytes::Reader};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Dict(Vec<(String, Value)>),
    Array(Vec<Value>),
    String(String),
    Integer(i128),
    Real(f64),
    Bool(bool),
    /// Seconds since 2001-01-01T00:00:00Z (binary), or the ISO text (XML).
    Date(DateValue),
    Data(Vec<u8>),
    /// A keyed-archive object reference.
    Uid(u64),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DateValue {
    Seconds(f64),
    Text(String),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(d) => d.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_uid(&self) -> Option<u64> {
        match self {
            Value::Uid(u) => Some(*u),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i128> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }
}
struct Bin<'a> {
    r: Reader<'a>,
    offsets: Vec<usize>,
    ref_size: usize,
    nodes: usize,
    /// Data and text bytes still allowed to be copied out.
    bytes_left: usize,
    limits: Limits,
}

/// Parse a `bplist00` binary property list.
pub fn parse_binary(data: &[u8]) -> Result<Value, FormatError> {
    parse_binary_with_limits(data, Limits::default())
}

/// Parse a binary property list, with caller-selected input, object, expansion and nesting limits.
pub fn parse_binary_with_limits(data: &[u8], limits: Limits) -> Result<Value, FormatError> {
    let (mut b, top) = reader(data, limits)?;
    let mut stack = reserved(limits.max_depth.min(256))?;
    b.object(top, &mut stack)
}

fn reader(data: &[u8], limits: Limits) -> Result<(Bin<'_>, usize), FormatError> {
    if data.len() > limits.max_input_bytes {
        return Err(FormatError::TooLarge("input bytes".into()));
    }
    if !data.starts_with(b"bplist") {
        return Err(FormatError::Malformed("not a binary plist".into()));
    }
    if !data.starts_with(b"bplist00") {
        return Err(FormatError::Unsupported("unknown binary plist version".into()));
    }
    let len = data.len();
    if len < 8 + 32 {
        return Err(FormatError::Truncated("binary plist trailer".into()));
    }
    let r = Reader::new(data);
    let t = len - 32;
    let off_size = usize::from(r.u8(t + 6).unwrap_or(0));
    let ref_size = usize::from(r.u8(t + 7).unwrap_or(0));
    let num = r.u64_be(t + 8).unwrap_or(0);
    let top = r.u64_be(t + 16).unwrap_or(0);
    let table = r.u64_be(t + 24).unwrap_or(0);
    if !(1..=8).contains(&off_size) || !(1..=8).contains(&ref_size) {
        return Err(FormatError::Malformed("bad offset or reference size".into()));
    }
    let num = usize::try_from(num).map_err(|_| FormatError::Malformed("object count".into()))?;
    let table = usize::try_from(table).map_err(|_| FormatError::Malformed("offset table".into()))?;
    if num == 0 || num > len {
        return Err(FormatError::Malformed(format!("{num} objects in a {len}-byte file")));
    }
    if num > limits.max_objects {
        return Err(FormatError::TooLarge("object count".into()));
    }
    let need = num.checked_mul(off_size).and_then(|n| n.checked_add(table)).ok_or_else(|| FormatError::Malformed("offset table".into()))?;
    if need > t || table < 8 {
        return Err(FormatError::Truncated("offset table runs past the trailer".into()));
    }
    let mut offsets = reserved(num)?;
    for i in 0..num {
        let o = read_be(&r, table + i * off_size, off_size).ok_or_else(|| FormatError::Truncated("offset table".into()))?;
        let o = usize::try_from(o).map_err(|_| FormatError::Malformed("object offset".into()))?;
        if o < 8 || o >= table {
            return Err(FormatError::Malformed(format!("object {i} lies outside the object area")));
        }
        offsets.push(o);
    }
    let top = usize::try_from(top).map_err(|_| FormatError::Malformed("top object".into()))?;
    // Object payloads and references must not borrow bytes from the offset table or trailer.
    let objects = data.get(..table).ok_or_else(|| FormatError::Truncated("object area".into()))?;
    let b = Bin { r: Reader::new(objects), offsets, ref_size, nodes: 0, bytes_left: limits.max_expanded_bytes, limits };
    Ok((b, top))
}

fn read_be(r: &Reader<'_>, off: usize, n: usize) -> Option<u64> {
    let s = r.slice(off, n)?;
    Some(s.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
}

/// UTF-8 size of `String::from_utf8_lossy(bytes)`: each damaged sequence becomes a U+FFFD.
fn lossy_utf8_len(bytes: &[u8]) -> usize {
    bytes.utf8_chunks().fold(0usize, |n, c| {
        let replacement = if c.invalid().is_empty() { 0 } else { char::REPLACEMENT_CHARACTER.len_utf8() };
        n.saturating_add(c.valid().len()).saturating_add(replacement)
    })
}

/// UTF-8 size of `String::from_utf16_lossy` over big-endian code units: a lone surrogate becomes
/// a U+FFFD.
fn lossy_utf16_len(bytes: &[u8]) -> usize {
    let units = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c));
    char::decode_utf16(units).fold(0usize, |n, c| n.saturating_add(c.map_or(char::REPLACEMENT_CHARACTER.len_utf8(), char::len_utf8)))
}

impl Bin<'_> {
    fn count(&self, at: usize, low: u8) -> Result<(usize, usize), FormatError> {
        if low != 0xf {
            return Ok((usize::from(low), at + 1));
        }
        let m = self.r.u8(at + 1).ok_or_else(|| FormatError::Truncated("length".into()))?;
        if m & 0xf0 != 0x10 {
            return Err(FormatError::Malformed("bad extended length marker".into()));
        }
        if m & 0x0f > 3 {
            return Err(FormatError::Malformed("extended length width exceeds 64 bits".into()));
        }
        let n = 1usize << (m & 0x0f);
        let v = read_be(&self.r, at + 2, n).ok_or_else(|| FormatError::Truncated("length".into()))?;
        let v = usize::try_from(v).map_err(|_| FormatError::Malformed("length".into()))?;
        Ok((v, at + 2 + n))
    }

    fn reference(&self, at: usize, i: usize) -> Result<usize, FormatError> {
        let off = i.checked_mul(self.ref_size).and_then(|x| x.checked_add(at)).ok_or_else(|| FormatError::Malformed("reference".into()))?;
        let v = read_be(&self.r, off, self.ref_size).ok_or_else(|| FormatError::Truncated("object reference".into()))?;
        usize::try_from(v).map_err(|_| FormatError::Malformed("reference".into()))
    }

    /// Refuse a container whose `children` would pass the node limit, before space is reserved for them.
    fn room_for_nodes(&self, children: usize) -> Result<(), FormatError> {
        if children > self.limits.max_nodes.saturating_sub(self.nodes) {
            return Err(FormatError::TooLarge("expanded node count".into()));
        }
        Ok(())
    }

    /// Spend `n` bytes of the data and text budget before copying them out.
    fn spend(&mut self, n: usize) -> Result<(), FormatError> {
        self.bytes_left =
            self.bytes_left.checked_sub(n).ok_or_else(|| FormatError::TooLarge("repeated data and text expand past the plist byte budget".into()))?;
        Ok(())
    }

    /// Read object `idx`. `stack` holds the containers on the current path, so a cycle is refused.
    fn object(&mut self, idx: usize, stack: &mut Vec<usize>) -> Result<Value, FormatError> {
        self.nodes += 1;
        if self.nodes > self.limits.max_nodes {
            return Err(FormatError::TooLarge("expanded node count".into()));
        }
        if stack.len() >= self.limits.max_depth.min(256) {
            return Err(FormatError::Malformed("nesting too deep".into()));
        }
        if stack.contains(&idx) {
            return Err(FormatError::Malformed(format!("object {idx} contains itself")));
        }
        let at = *self.offsets.get(idx).ok_or_else(|| FormatError::Malformed(format!("reference to missing object {idx}")))?;
        let m = self.r.u8(at).ok_or_else(|| FormatError::Truncated("object marker".into()))?;
        let (hi, lo) = (m >> 4, m & 0x0f);
        let v = match hi {
            0x0 => match lo {
                0x0 => Value::Null,
                0x8 => Value::Bool(false),
                0x9 => Value::Bool(true),
                _ => return Err(FormatError::Malformed(format!("unknown simple plist object {m:#04x}"))),
            },
            0x1 => {
                if lo > 4 {
                    return Err(FormatError::Malformed("integer width exceeds 128 bits".into()));
                }
                let n = 1usize << lo;
                let bytes = self.r.slice(at + 1, n).ok_or_else(|| FormatError::Truncated("integer".into()))?;
                let v = match n {
                    8 => i128::from(i64::from_be_bytes(bytes.try_into().map_err(|_| FormatError::Truncated("integer".into()))?)),
                    16 => i128::from_be_bytes(bytes.try_into().map_err(|_| FormatError::Truncated("integer".into()))?),
                    _ => i128::from(bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b))),
                };
                Value::Integer(v)
            }
            0x2 => {
                let v = match lo {
                    2 => self.r.u32_be(at + 1).map(|b| f64::from(f32::from_bits(b))),
                    3 => self.r.u64_be(at + 1).map(f64::from_bits),
                    _ => None,
                };
                Value::Real(v.ok_or_else(|| FormatError::Truncated("real".into()))?)
            }
            0x3 if lo == 3 => {
                Value::Date(DateValue::Seconds(self.r.u64_be(at + 1).map(f64::from_bits).ok_or_else(|| FormatError::Truncated("date".into()))?))
            }
            0x4 => {
                let (n, s) = self.count(at, lo)?;
                let bytes = self.r.slice(s, n).ok_or_else(|| FormatError::Truncated("data".into()))?;
                self.spend(bytes.len())?;
                {
                    let mut out = reserved(bytes.len())?;
                    out.extend_from_slice(bytes);
                    Value::Data(out)
                }
            }
            0x5 => {
                let (n, s) = self.count(at, lo)?;
                let bytes = self.r.slice(s, n).ok_or_else(|| FormatError::Truncated("string".into()))?;
                let size = lossy_utf8_len(bytes);
                self.spend(size)?;
                let mut out = reserved_string(size)?;
                for chunk in bytes.utf8_chunks() {
                    out.push_str(chunk.valid());
                    if !chunk.invalid().is_empty() {
                        out.push(char::REPLACEMENT_CHARACTER);
                    }
                }
                Value::String(out)
            }
            0x6 => {
                let (n, s) = self.count(at, lo)?;
                let bytes = self
                    .r
                    .slice(s, n.checked_mul(2).ok_or_else(|| FormatError::Malformed("string".into()))?)
                    .ok_or_else(|| FormatError::Truncated("string".into()))?;
                let size = lossy_utf16_len(bytes);
                self.spend(size)?;
                let mut out = reserved_string(size)?;
                let units = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c));
                for ch in char::decode_utf16(units) {
                    out.push(ch.unwrap_or(char::REPLACEMENT_CHARACTER));
                }
                Value::String(out)
            }
            0x8 => {
                if lo > 7 {
                    return Err(FormatError::Malformed("uid width exceeds 64 bits".into()));
                }
                let n = usize::from(lo) + 1;
                Value::Uid(read_be(&self.r, at + 1, n).ok_or_else(|| FormatError::Truncated("uid".into()))?)
            }
            0xa | 0xc => {
                let (n, s) = self.count(at, lo)?;
                // Writers store equal values once, so an array may name one object many times:
                // only its references have to fit in the file. the node limit and the byte budget bound
                // the expansion.
                if s.saturating_add(n.saturating_mul(self.ref_size)) > self.r.len() {
                    return Err(FormatError::Malformed("array larger than the file".into()));
                }
                self.room_for_nodes(n)?;
                stack.push(idx);
                let mut out = reserved(n)?;
                for i in 0..n {
                    let child = self.reference(s, i)?;
                    out.push(self.object(child, stack)?);
                }
                stack.pop();
                Value::Array(out)
            }
            0xd => {
                let (n, s) = self.count(at, lo)?;
                if s.saturating_add(n.saturating_mul(self.ref_size).saturating_mul(2)) > self.r.len() {
                    return Err(FormatError::Malformed("dictionary larger than the file".into()));
                }
                self.room_for_nodes(n.saturating_mul(2))?;
                stack.push(idx);
                let mut out = reserved(n)?;
                for i in 0..n {
                    let k = self.reference(s, i)?;
                    let v = self.reference(s, i + n)?;
                    let key = match self.object(k, stack)? {
                        Value::String(s) => s,
                        _ => return Err(FormatError::Unsupported("non-string dictionary keys cannot be represented in this plist view".into())),
                    };
                    out.push((key, self.object(v, stack)?));
                }
                stack.pop();
                Value::Dict(out)
            }
            _ => return Err(FormatError::Malformed(format!("unknown object marker {m:#04x}"))),
        };
        Ok(v)
    }
}

fn reserved<T>(n: usize) -> Result<Vec<T>, FormatError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| FormatError::TooLarge("allocation".into()))?;
    Ok(v)
}

fn reserved_string(n: usize) -> Result<String, FormatError> {
    let mut s = String::new();
    s.try_reserve_exact(n).map_err(|_| FormatError::TooLarge("string allocation".into()))?;
    Ok(s)
}

/// Select the first string in one top-level dictionary array without expanding unrelated values.
/// First matching dictionary key wins. Missing/wrong-typed values return `None`; malformed
/// selected objects and resource exhaustion return errors. Unselected value graphs are not
/// validated. This permits preference readers to ignore unrelated future object kinds.
pub fn first_string_in_array(data: &[u8], key: &str, limits: Limits) -> Result<Option<String>, FormatError> {
    let (mut b, top) = reader(data, limits)?;
    let (kind, n, start) = b.header(top)?;
    if kind != 0xd {
        return Ok(None);
    }
    if n > limits.max_nodes {
        return Err(FormatError::TooLarge("dictionary lookup entries".into()));
    }
    let bytes = n.checked_mul(2).and_then(|n| n.checked_mul(b.ref_size)).ok_or_else(|| FormatError::Malformed("dictionary references".into()))?;
    b.r.slice(start, bytes).ok_or_else(|| FormatError::Truncated("dictionary references".into()))?;
    for i in 0..n {
        let k = b.reference(start, i)?;
        if b.selected_string(k)?.as_deref() != Some(key) {
            continue;
        }
        let value = b.reference(start, n + i)?;
        let (kind, count, payload) = b.header(value)?;
        if kind != 0xa || count == 0 {
            return Ok(None);
        }
        let first = b.reference(payload, 0)?;
        return b.selected_string(first);
    }
    Ok(None)
}
impl Bin<'_> {
    fn header(&self, idx: usize) -> Result<(u8, usize, usize), FormatError> {
        let at = *self.offsets.get(idx).ok_or_else(|| FormatError::Malformed("missing object".into()))?;
        let marker = self.r.u8(at).ok_or_else(|| FormatError::Truncated("object marker".into()))?;
        let kind = marker >> 4;
        // Only containers and strings use an extended length. Unknown types are not traversed.
        let (n, start) = if matches!(kind, 0x5 | 0x6 | 0xa | 0xd) { self.count(at, marker & 15)? } else { (usize::from(marker & 15), at + 1) };
        Ok((kind, n, start))
    }
    fn selected_string(&mut self, idx: usize) -> Result<Option<String>, FormatError> {
        let (kind, _, _) = self.header(idx)?;
        if !matches!(kind, 0x5 | 0x6) {
            return Ok(None);
        }
        match self.object(idx, &mut Vec::new())? {
            Value::String(s) => Ok(Some(s)),
            _ => Ok(None),
        }
    }
}
