//! Byte transforms shared by flat and deep OpenEXR blocks.

use std::io::{Error, ErrorKind, Result};

/// Expand a compressed RLE block to exactly `expected` bytes and undo its
/// byte prediction. The caller checks its chunk limit and handles blocks
/// stored uncompressed before calling this function.
pub fn decode_rle(mut packed: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some((&count, rest)) = packed.split_first() {
        let count = count as i8;
        let literal = count < 0;
        let run = if literal { usize::from(count.unsigned_abs()) } else { count as usize + 1 };
        let Some(end) = out.len().checked_add(run).filter(|&end| end <= expected) else {
            return Err(Error::new(ErrorKind::InvalidData, format!("RLE data expands past {expected} bytes")));
        };
        let (data, next) = rest
            .split_at_checked(if literal { run } else { 1 })
            .ok_or_else(|| Error::new(ErrorKind::UnexpectedEof, "RLE run points past the compressed data"))?;

        if end > out.capacity() {
            let target = out.capacity().saturating_mul(2).max(end).min(expected);
            out.try_reserve_exact(target - out.len())
                .map_err(|_| Error::new(ErrorKind::OutOfMemory, format!("cannot allocate {target} RLE bytes")))?;
        }
        if literal {
            out.extend_from_slice(data);
        } else if let Some(&value) = data.first() {
            out.resize(end, value);
        }
        packed = next;
    }
    if out.len() != expected {
        return Err(Error::new(ErrorKind::InvalidData, format!("RLE data expands to {} bytes, expected {expected}", out.len())));
    }
    undo_byte_prediction(out)
}

/// Undo RLE/ZIP's byte differences and even/odd split, returning the bytes
/// in their original order. The scratch allocation is fallible and no larger
/// than the already bounded decompressed block.
pub fn undo_byte_prediction(mut bytes: Vec<u8>) -> Result<Vec<u8>> {
    if let Some((&mut first, differences)) = bytes.split_first_mut() {
        let mut previous = first;
        for byte in differences {
            *byte = previous.wrapping_add(*byte).wrapping_sub(128);
            previous = *byte;
        }
    }

    let (even, odd) = bytes.split_at(bytes.len().div_ceil(2));
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.len()).map_err(|_| Error::new(ErrorKind::OutOfMemory, format!("cannot allocate {} elements", bytes.len())))?;
    for (&even, &odd) in even.iter().zip(odd) {
        out.extend([even, odd]);
    }
    out.extend_from_slice(&even[odd.len()..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_reverses_wrapping_prediction_and_an_odd_byte_split() {
        // Literal 50, repeated delta 128 three times, then three literals.
        let rle = [255, 50, 2, 128, 253, 100, 250, 129];
        let expected = [50, 22, 50, 144, 50, 145, 50];
        assert_eq!(decode_rle(&rle, 7).unwrap(), expected);
        assert_eq!(undo_byte_prediction(vec![50, 128, 128, 128, 100, 250, 129]).unwrap(), expected);
    }

    #[test]
    fn rle_handles_maximum_literal_and_repeat_counts() {
        let mut rle = vec![128];
        rle.extend([128; 128]);
        rle.extend([127, 128]);
        assert_eq!(decode_rle(&rle, 256).unwrap(), [128; 256]);
    }

    #[test]
    fn rle_refuses_short_input_and_wrong_output_sizes() {
        for block in [[255], [1]] {
            assert_eq!(decode_rle(&block, 2).unwrap_err().kind(), ErrorKind::UnexpectedEof);
        }
        assert_eq!(decode_rle(&[1, 128], 1).unwrap_err().kind(), ErrorKind::InvalidData);
        assert_eq!(decode_rle(&[0, 128], 2).unwrap_err().kind(), ErrorKind::InvalidData);
        // A truncated stream does not reserve the declared output size up front.
        assert_eq!(decode_rle(&[0, 128], usize::MAX).unwrap_err().kind(), ErrorKind::InvalidData);
    }
}
