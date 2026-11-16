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

/// Parse a big-endian opcode list with an explicit maximum record count.
pub fn parse(bytes: &[u8], max_opcodes: usize) -> Result<Vec<Opcode>, Malformed> {
    let mut at = 0usize;
    let read = |at: &mut usize| -> Result<u32, Malformed> {
        let end = at.checked_add(4).ok_or(Malformed)?;
        let value = bytes.get(*at..end).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes).ok_or(Malformed)?;
        *at = end;
        Ok(value)
    };
    let count = read(&mut at)? as usize;
    if count > bytes.len() / 16 || count > max_opcodes {
        return Err(Malformed);
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let id = read(&mut at)?;
        let version = read(&mut at)?;
        let flags = read(&mut at)?;
        let len = read(&mut at)? as usize;
        let end = at.checked_add(len).ok_or(Malformed)?;
        let params = bytes.get(at..end).ok_or(Malformed)?.to_vec();
        at = end;
        out.push(Opcode { id, version, flags, params });
    }
    Ok(out)
}

/// Encode records as a big-endian opcode list, preserving version and parameter bytes.
pub fn encode(list: &[Opcode]) -> Vec<u8> {
    let mut out = (list.len() as u32).to_be_bytes().to_vec();
    for op in list {
        for value in [op.id, op.version, op.flags, op.params.len() as u32] {
            out.extend(value.to_be_bytes());
        }
        out.extend_from_slice(&op.params);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_records_round_trip_and_limits_are_enforced() {
        let records = vec![Opcode { id: 900, version: 0x0107_0100, flags: 3, params: vec![1, 2, 3] }];
        let bytes = encode(&records);
        assert_eq!(parse(&bytes, 1).unwrap(), records);
        assert!(parse(&bytes, 0).is_err());
        assert!(parse(&bytes[..bytes.len() - 1], 1).is_err());
        assert!(records[0].optional() && records[0].preview_skippable());
    }
}
