use crate::{FormatError, bytes::Reader};

pub const MAX_DEPTH: usize = 256;
pub const MAX_NODES: usize = 2_000_000;
pub const MAX_EXPANDED_BYTES: usize = 64 << 20;

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
}

/// Parse a `bplist00` binary property list.
pub fn parse_binary(data: &[u8]) -> Result<Value, FormatError> {
    if !data.starts_with(b"bplist") {
        return Err(FormatError::Malformed("not a binary plist".into()));
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
    let need = num.checked_mul(off_size).and_then(|n| n.checked_add(table)).ok_or_else(|| FormatError::Malformed("offset table".into()))?;
    if need > t || table < 8 {
        return Err(FormatError::Truncated("offset table runs past the trailer".into()));
    }
    let mut offsets = Vec::with_capacity(num);
    for i in 0..num {
        let o = read_be(&r, table + i * off_size, off_size).ok_or_else(|| FormatError::Truncated("offset table".into()))?;
        let o = usize::try_from(o).map_err(|_| FormatError::Malformed("object offset".into()))?;
        if o >= table {
            return Err(FormatError::Malformed(format!("object {i} lies outside the object area")));
        }
        offsets.push(o);
    }
    let top = usize::try_from(top).map_err(|_| FormatError::Malformed("top object".into()))?;
    let mut b = Bin { r, offsets, ref_size, nodes: 0 };
    let mut stack = Vec::new();
    b.object(top, &mut stack)
}

fn read_be(r: &Reader<'_>, off: usize, n: usize) -> Option<u64> {
    let s = r.slice(off, n)?;
    Some(s.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
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
        let n = 1usize << (m & 0x0f).min(3);
        let v = read_be(&self.r, at + 2, n).ok_or_else(|| FormatError::Truncated("length".into()))?;
        let v = usize::try_from(v).map_err(|_| FormatError::Malformed("length".into()))?;
        Ok((v, at + 2 + n))
    }

    fn reference(&self, at: usize, i: usize) -> Result<usize, FormatError> {
        let off = i.checked_mul(self.ref_size).and_then(|x| x.checked_add(at)).ok_or_else(|| FormatError::Malformed("reference".into()))?;
        let v = read_be(&self.r, off, self.ref_size).ok_or_else(|| FormatError::Truncated("object reference".into()))?;
        usize::try_from(v).map_err(|_| FormatError::Malformed("reference".into()))
    }

    fn object(&mut self, idx: usize, stack: &mut Vec<usize>) -> Result<Value, FormatError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(FormatError::TooLarge(format!("more than {MAX_NODES} plist nodes")));
        }
        if stack.len() >= MAX_DEPTH {
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
                _ => Value::Null,
            },
            0x1 => {
                let n = 1usize << lo.min(4);
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
            0x3 => Value::Date(DateValue::Seconds(self.r.u64_be(at + 1).map(f64::from_bits).ok_or_else(|| FormatError::Truncated("date".into()))?)),
            0x4 => {
                let (n, s) = self.count(at, lo)?;
                Value::Data(self.r.slice(s, n).ok_or_else(|| FormatError::Truncated("data".into()))?.to_vec())
            }
            0x5 => {
                let (n, s) = self.count(at, lo)?;
                Value::String(String::from_utf8_lossy(self.r.slice(s, n).ok_or_else(|| FormatError::Truncated("string".into()))?).into_owned())
            }
            0x6 => {
                let (n, s) = self.count(at, lo)?;
                let bytes = self
                    .r
                    .slice(s, n.checked_mul(2).ok_or_else(|| FormatError::Malformed("string".into()))?)
                    .ok_or_else(|| FormatError::Truncated("string".into()))?;
                let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                Value::String(String::from_utf16_lossy(&units))
            }
            0x8 => {
                let n = usize::from(lo) + 1;
                Value::Uid(read_be(&self.r, at + 1, n).ok_or_else(|| FormatError::Truncated("uid".into()))?)
            }
            0xa | 0xc => {
                let (n, s) = self.count(at, lo)?;
                if n > self.offsets.len().saturating_mul(4).max(16) || s.saturating_add(n.saturating_mul(self.ref_size)) > self.r.len() {
                    return Err(FormatError::Malformed("array larger than the file".into()));
                }
                stack.push(idx);
                let mut out = Vec::with_capacity(n);
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
                stack.push(idx);
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    let k = self.reference(s, i)?;
                    let v = self.reference(s, i + n)?;
                    let key = match self.object(k, stack)? {
                        Value::String(s) => s,
                        other => format!("{:?}", other.to_json()),
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

