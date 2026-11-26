//! Bounded single-block USD Crate LZ4 writing.

/// Compression admission, allocation or backend failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Input or worst-case output exceeds the supported/caller budget.
    #[error("USD Crate LZ4 compression exceeds budget")]
    Limit,
    /// The bounded output reservation failed.
    #[error("cannot allocate USD Crate LZ4 output")]
    Allocation,
    /// The backend could not encode into the preflighted output buffer.
    #[error("USD Crate LZ4 compression: {0}")]
    Encode(#[from] lz4_flex::block::CompressError),
}

fn capacity(input: usize, max_output_bytes: usize) -> Result<usize, Error> {
    // Keep inputs below the LZ4 single-block limit and preflight the backend's
    // 20 + input*110/100 sizing arithmetic before calling it, including the count byte.
    if input > 0x7e00_0000 {
        return Err(Error::Limit);
    }
    input
        .checked_mul(110)
        .map(|n| n / 100)
        .and_then(|n| n.checked_add(21))
        .filter(|n| *n <= max_output_bytes && *n <= isize::MAX as usize)
        .ok_or(Error::Limit)
}

/// Write one Crate-framed LZ4 block, beginning with a zero chunk-count byte.
///
/// `max_output_bytes` bounds the reserved output including framing. Admission uses
/// the backend's worst-case size, so even highly compressible input must fit that bound.
/// Allocation is fallible; backend scratch memory is not included in the output cap.
/// Uses TextureCraft's original LZ4 backend/configuration and produces the same block bytes.
pub fn compress(input: &[u8], max_output_bytes: usize) -> Result<Vec<u8>, Error> {
    let capacity = capacity(input.len(), max_output_bytes)?;
    let mut output = Vec::new();
    output.try_reserve_exact(capacity).map_err(|_| Error::Allocation)?;
    output.resize(capacity, 0);
    let target = output.get_mut(1..).ok_or(Error::Limit)?;
    let written = lz4_flex::block::compress_into(input, target)?;
    output.truncate(written.checked_add(1).ok_or(Error::Limit)?);
    Ok(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn original_wire_bytes_and_roundtrip() {
        for size in [0, 1, 15, 16, 255, 4096, 65536] {
            for repeated in [false, true] {
                let input: Vec<u8> = (0..size).map(|i| if repeated { 17 } else { (i % 251) as u8 }).collect();
                let mut old = vec![0];
                old.extend(lz4_flex::block::compress(&input));
                let encoded = compress(&input, 1 << 20).unwrap();
                assert_eq!(encoded, old);
                assert_eq!(crate::decompress(&encoded, size).unwrap(), input);
            }
        }
    }

    #[test]
    fn checks_worst_case_before_allocation() {
        let input = [0; 100];
        assert!(matches!(compress(&input, 130), Err(Error::Limit)));
        assert!(compress(&input, 131).is_ok());
        assert!(capacity(usize::MAX, usize::MAX).is_err());
        assert!(capacity(0x7e00_0001, usize::MAX).is_err());
        assert_eq!(capacity(0, 21).unwrap(), 21);
    }
}
