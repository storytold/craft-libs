//! USD Crate integer delta decoding, adapted from TextureCraft's bflatastic implementation.

/// Wire width of each integer and its common delta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// Signed 32-bit wrapping arithmetic, sign-extended to i64 on output.
    I32,
    /// Signed 64-bit wrapping arithmetic.
    I64,
}
impl Width {
    fn bytes(self) -> usize {
        match self {
            Self::I32 => 4,
            Self::I64 => 8,
        }
    }
}

/// Invalid packed data or a caller output-budget/allocation failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The declared count exceeds the caller's byte budget or addressable output.
    #[error("USD integer array exceeds output budget")]
    Limit,
    /// A common delta, code table or selected delta is incomplete.
    #[error("truncated USD integer array")]
    Truncated,
    /// The bounded output reservation failed.
    #[error("cannot allocate USD integer array")]
    Allocation,
}

fn signed(bytes: &[u8], at: usize, width: usize) -> Option<i64> {
    let part = bytes.get(at..at.checked_add(width)?)?;
    Some(match width {
        1 => i64::from(i8::from_le_bytes(part.try_into().ok()?)),
        2 => i64::from(i16::from_le_bytes(part.try_into().ok()?)),
        4 => i64::from(i32::from_le_bytes(part.try_into().ok()?)),
        8 => i64::from_le_bytes(part.try_into().ok()?),
        _ => return None,
    })
}

/// Decode `count` values, charging eight output bytes per value against `max_output_bytes`.
///
/// The common delta is followed by a packed two-bit code per value and variable-width
/// exceptional deltas. Arithmetic wraps at the specified wire width. Extra bytes after the
/// values are ignored, matching the original reader. Counts, the code table and the output
/// byte budget are checked before a fallible reservation. A zero count returns an empty vector.
pub fn decode(data: &[u8], count: usize, width: Width, max_output_bytes: usize) -> Result<Vec<i64>, Error> {
    let output_bytes = count.checked_mul(8).filter(|n| *n <= max_output_bytes && *n <= isize::MAX as usize).ok_or(Error::Limit)?;
    if output_bytes == 0 {
        return Ok(Vec::new());
    }
    let size = width.bytes();
    let common = signed(data, 0, size).ok_or(Error::Truncated)?;
    let code_bytes = count.div_ceil(4);
    let mut at = size.checked_add(code_bytes).ok_or(Error::Truncated)?;
    let codes = data.get(size..at).ok_or(Error::Truncated)?;
    let mut output = Vec::new();
    output.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
    let mut previous = 0i64;
    for i in 0..count {
        let code = (codes.get(i / 4).copied().ok_or(Error::Truncated)? >> (2 * (i % 4))) & 3;
        let delta = if code == 0 {
            common
        } else {
            let bytes = (size >> (3 - usize::from(code))).max(1);
            let value = signed(data, at, bytes).ok_or(Error::Truncated)?;
            at = at.checked_add(bytes).ok_or(Error::Truncated)?;
            value
        };
        previous = match width {
            Width::I32 => i64::from((previous as i32).wrapping_add(delta as i32)),
            Width::I64 => previous.wrapping_add(delta),
        };
        output.push(previous);
    }
    Ok(output)
}

/// Encode integers with their original signed wrapping and largest-delta tie-break policy.
///
/// `max_bytes` bounds the combined worst-case output reservation and sorted delta scratch.
/// Both reservations are fallible. Sorting uses no extra heap storage; input storage and
/// allocator bookkeeping are outside this budget. Empty input emits no bytes.
pub fn encode(values: &[i64], width: Width, max_bytes: usize) -> Result<Vec<u8>, Error> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let size = width.bytes();
    let codes_len = values.len().div_ceil(4);
    let capacity = values.len().checked_mul(size).and_then(|n| n.checked_add(size)).and_then(|n| n.checked_add(codes_len)).ok_or(Error::Limit)?;
    let scratch = values.len().checked_mul(8).ok_or(Error::Limit)?;
    if capacity > isize::MAX as usize || scratch > isize::MAX as usize || capacity.checked_add(scratch).is_none_or(|n| n > max_bytes) {
        return Err(Error::Limit);
    }
    let wrap = |v: i64| match width {
        Width::I32 => i64::from(v as i32),
        Width::I64 => v,
    };
    let mut deltas = Vec::new();
    deltas.try_reserve_exact(values.len()).map_err(|_| Error::Allocation)?;
    let mut previous = 0i64;
    for &value in values {
        deltas.push(wrap(value.wrapping_sub(previous)));
        previous = value;
    }
    deltas.sort_unstable();
    let mut best = (0usize, 0i64);
    let mut at = 0;
    while at < deltas.len() {
        let value = deltas[at];
        let start = at;
        while at < deltas.len() && deltas[at] == value {
            at += 1;
        }
        best = best.max((at - start, value));
    }
    let common = best.1;
    let mut output = Vec::new();
    output.try_reserve_exact(capacity).map_err(|_| Error::Allocation)?;
    output.extend_from_slice(&common.to_le_bytes()[..size]);
    output.resize(size + codes_len, 0);
    previous = 0;
    for (i, &value) in values.iter().enumerate() {
        let delta = wrap(value.wrapping_sub(previous));
        previous = value;
        let small = |bits: u32| delta >= -(1i64 << (bits - 1)) && delta < (1i64 << (bits - 1));
        let code: u8 = if delta == common {
            0
        } else if small(size as u32 * 2) {
            1
        } else if small(size as u32 * 4) {
            2
        } else {
            3
        };
        if code != 0 {
            let bytes = (size >> (3 - usize::from(code))).max(1);
            output.extend_from_slice(&delta.to_le_bytes()[..bytes]);
        }
        output[size + i / 4] |= code << (2 * (i % 4));
    }
    Ok(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn common_deltas_and_output_budget() {
        let mut data = 1i32.to_le_bytes().to_vec();
        data.push(0);
        assert_eq!(decode(&data, 4, Width::I32, 32).unwrap(), [1, 2, 3, 4]);
        assert!(matches!(decode(&data, 4, Width::I32, 31), Err(Error::Limit)));
        assert!(matches!(decode(&data[..4], 4, Width::I32, 32), Err(Error::Truncated)));
        assert!(matches!(decode(&[], usize::MAX, Width::I64, usize::MAX), Err(Error::Limit)));
        assert!(decode(&[], 0, Width::I64, 0).unwrap().is_empty());
    }

    #[test]
    fn narrow_exception_codes_sign_extend() {
        for (width, size) in [(Width::I32, 4), (Width::I64, 8)] {
            let mut data = vec![0; size];
            data.push(1 | (2 << 2));
            let first_size = (size >> 2).max(1);
            let second_size = size >> 1;
            data.extend_from_slice(&(-3i64).to_le_bytes()[..first_size]);
            data.extend_from_slice(&7i64.to_le_bytes()[..second_size]);
            assert_eq!(decode(&data, 2, width, 16).unwrap(), [-3, 4]);
        }
    }

    #[test]
    fn exceptional_deltas_and_wrapping() {
        for (width, size) in [(Width::I32, 4), (Width::I64, 8)] {
            // Full-width positive maximum, followed by common +1, wraps to signed minimum.
            let mut data = 1i64.to_le_bytes()[..size].to_vec();
            data.push(3);
            let maximum = if width == Width::I32 { i64::from(i32::MAX) } else { i64::MAX };
            data.extend_from_slice(&maximum.to_le_bytes()[..size]);
            let minimum = if width == Width::I32 { i64::from(i32::MIN) } else { i64::MIN };
            assert_eq!(decode(&data, 2, width, 16).unwrap(), [maximum, minimum]);
            assert!(decode(&data[..data.len() - 1], 2, width, 16).is_err());
            data.extend(b"ignored tail");
            assert_eq!(decode(&data, 2, width, 16).unwrap(), [maximum, minimum]);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod encode_tests {
    use super::*;
    fn oracle(values: &[i64], width: usize) -> Vec<u8> {
        if values.is_empty() {
            return Vec::new();
        }
        let wrap = |v: i64| if width == 4 { i64::from(v as i32) } else { v };
        let mut prev = 0i64;
        let deltas: Vec<i64> = values
            .iter()
            .map(|&v| {
                let d = wrap(v.wrapping_sub(prev));
                prev = v;
                d
            })
            .collect();
        let mut counts = std::collections::HashMap::new();
        for d in &deltas {
            *counts.entry(*d).or_insert(0usize) += 1;
        }
        let common = counts.into_iter().max_by_key(|&(v, c)| (c, v)).map_or(0, |(v, _)| v);
        let mut out = Vec::with_capacity(width + values.len().div_ceil(4) + values.len() * width);
        out.extend_from_slice(&common.to_le_bytes()[..width]);
        let codes_at = out.len();
        out.resize(codes_at + values.len().div_ceil(4), 0);
        for (i, &d) in deltas.iter().enumerate() {
            let small = |bits: u32| d >= -(1i64 << (bits - 1)) && d < (1i64 << (bits - 1));
            let code: u8 = if d == common {
                0
            } else if small(width as u32 * 2) {
                1
            } else if small(width as u32 * 4) {
                2
            } else {
                3
            };
            if code > 0 {
                let w = (width >> (3 - code as usize)).max(1);
                out.extend_from_slice(&d.to_le_bytes()[..w]);
            }
            if let Some(b) = out.get_mut(codes_at + i / 4) {
                *b |= code << (2 * (i % 4));
            }
        }
        out
    }

    #[test]
    fn matches_original_bytes_and_wrapping() {
        for width in [Width::I32, Width::I64] {
            for n in [0, 1, 2, 3, 4, 5, 127, 1024] {
                let mut seed = 17u64;
                let values: Vec<i64> = (0..n)
                    .map(|_| {
                        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                        seed as i64
                    })
                    .collect();
                let bytes = encode(&values, width, 32768).unwrap();
                assert_eq!(bytes, oracle(&values, width.bytes()));
                let expected: Vec<i64> = values.iter().map(|&v| if width == Width::I32 { i64::from(v as i32) } else { v }).collect();
                assert_eq!(decode(&bytes, n, width, 32768).unwrap(), expected);
            }
            for values in [vec![1, 3, 4, 6], vec![i64::MAX, i64::MIN, 0], vec![7; 32]] {
                assert_eq!(encode(&values, width, 4096).unwrap(), oracle(&values, width.bytes()));
            }
        }
    }
    #[test]
    fn combined_reservation_budget_is_enforced() {
        assert_eq!(encode(&[], Width::I64, 0).unwrap(), Vec::<u8>::new());
        assert!(matches!(encode(&[1], Width::I32, 16), Err(Error::Limit)));
        assert!(encode(&[1], Width::I32, 17).is_ok());
    }
}
