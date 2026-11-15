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
