//! DNG opcode-list envelopes, retaining unrecognized records and their parameter bytes.

/// An opcode with its version, flags and unmodified parameter bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Opcode {
    /// Opcode identifier.
    pub id: u32,
    /// Four-byte DNG version, stored as a big-endian integer.
    pub version: u32,
    /// Opcode flags.
    pub flags: u32,
    /// Unmodified parameter bytes.
    pub params: Vec<u8>,
}
impl Opcode {
    /// Unknown or unsupported optional opcodes may be skipped.
    pub fn optional(&self) -> bool {
        self.flags & 1 != 0
    }
    /// The opcode may be skipped for preview quality.
    pub fn preview_skippable(&self) -> bool {
        self.flags & 2 != 0
    }
}

/// A damaged opcode list or one exceeding the caller's count limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

/// The records of a big-endian opcode list in order, as `(id, version, flags, params)`, ending at
/// the declared count or at the first truncated record. The envelope walk shared by [`parse`] and
/// the typed [`crate::opcodes::parse_list`].
pub(crate) struct Records<'a> {
    bytes: &'a [u8],
    at: usize,
    /// Records the list header still promises.
    pub(crate) left: u32,
}

impl<'a> Records<'a> {
    /// The record walker of a list; `None` when even the count is missing.
    pub(crate) fn new(bytes: &'a [u8]) -> Option<Self> {
        let left = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?);
        Some(Self { bytes, at: 4, left })
    }
    fn u32(&mut self) -> Option<u32> {
        let end = self.at.checked_add(4)?;
        let value = u32::from_be_bytes(self.bytes.get(self.at..end)?.try_into().ok()?);
        self.at = end;
        Some(value)
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = (u32, u32, u32, &'a [u8]);
    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        let (id, version, flags, len) = (self.u32()?, self.u32()?, self.u32()?, self.u32()? as usize);
        let end = self.at.checked_add(len)?;
        let params = self.bytes.get(self.at..end)?;
        self.at = end;
        self.left -= 1;
        Some((id, version, flags, params))
    }
}

/// Parse a big-endian opcode list with an explicit maximum record count.
pub fn parse(bytes: &[u8], max_opcodes: usize) -> Result<Vec<Opcode>, Malformed> {
    let records = Records::new(bytes).ok_or(Malformed)?;
    let count = records.left as usize;
    if count > bytes.len() / 16 || count > max_opcodes {
        return Err(Malformed);
    }
    let out: Vec<Opcode> = records.map(|(id, version, flags, params)| Opcode { id, version, flags, params: params.to_vec() }).collect();
    if out.len() == count { Ok(out) } else { Err(Malformed) }
}

/// An opcode list cannot be represented in the DNG wire format or allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeError;

fn wire_length(length: usize) -> Result<u32, EncodeError> {
    u32::try_from(length).map_err(|_| EncodeError)
}

/// Encode records, rejecting unrepresentable counts and parameter lengths before allocation.
pub fn encode(list: &[Opcode]) -> Result<Vec<u8>, EncodeError> {
    let count = wire_length(list.len())?;
    let mut total = 4usize;
    for op in list {
        wire_length(op.params.len())?;
        total = total.checked_add(16).and_then(|n| n.checked_add(op.params.len())).ok_or(EncodeError)?;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| EncodeError)?;
    out.extend_from_slice(&count.to_be_bytes());
    for op in list {
        for value in [op.id, op.version, op.flags, wire_length(op.params.len())?] {
            out.extend(value.to_be_bytes());
        }
        out.extend_from_slice(&op.params);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_lengths_are_checked_without_large_allocations() {
        assert_eq!(wire_length(0), Ok(0));
        assert_eq!(wire_length(u32::MAX as usize), Ok(u32::MAX));
        if let Some(too_large) = (u32::MAX as usize).checked_add(1) {
            assert_eq!(wire_length(too_large), Err(EncodeError));
        }
        assert_eq!(encode(&[]).unwrap(), vec![0, 0, 0, 0]);
    }

    #[test]
    fn unknown_records_round_trip_and_limits_are_enforced() {
        let records = vec![Opcode { id: 900, version: 0x0107_0100, flags: 3, params: vec![1, 2, 3] }];
        let bytes = encode(&records).unwrap();
        assert_eq!(parse(&bytes, 1).unwrap(), records);
        assert!(parse(&bytes, 0).is_err());
        assert!(parse(&bytes[..bytes.len() - 1], 1).is_err());
        assert!(records[0].optional() && records[0].preview_skippable());
    }

    #[test]
    fn typed_lists_share_the_envelope() {
        use crate::opcodes::{self, Opcode as Typed};
        let typed = vec![Typed::TrimBounds { top: 1, left: 2, bottom: 3, right: 4 }, Typed::Unknown { id: 900, flags: 1, params: vec![5, 6] }];
        let bytes = opcodes::write_list(&typed);
        let raw = parse(&bytes, 8).unwrap();
        assert_eq!(raw.iter().map(|o| (o.id, o.version)).collect::<Vec<_>>(), [(6, 0x0103_0000), (900, 0x0103_0000)]);
        assert_eq!(encode(&raw).unwrap(), bytes);
        assert_eq!(opcodes::parse_list(&bytes), typed);
        // a truncated last record: the typed reader keeps what came before, the raw one refuses
        assert_eq!(opcodes::parse_list(&bytes[..bytes.len() - 1]), typed[..1]);
        assert!(parse(&bytes[..bytes.len() - 1], 8).is_err());
        assert!(opcodes::parse_list(&[0, 0]).is_empty());
        assert!(parse(&[0, 0], 8).is_err());
    }
}
