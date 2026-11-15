//! Bounded USD Crate wire primitives. Not a complete USD scene reader or writer.
//!
//! The framing decoder is adapted from TextureCraft's `usd/codec.rs`, originally
//! bflatastic, commit bdbb52c9b66b221a55b370105b42f485a76e6fc7.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Bounded Crate integer delta arrays.
pub mod integers;

/// Failure to read a bounded Crate LZ4 frame.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Invalid or incomplete chunk framing.
    #[error("invalid USD Crate LZ4 framing: {0}")]
    Framing(&'static str),
    /// Invalid compressed data or output exceeding the supplied capacity.
    #[error("USD Crate LZ4 data: {0}")]
    Lz4(#[from] lz4_flex::block::DecompressError),
    /// The bounded output allocation could not be reserved.
    #[error("cannot allocate USD Crate LZ4 output")]
    Allocation,
}

/// Decode Crate's chunk-count byte and LZ4 blocks to at most `max_output` bytes.
///
/// A zero count means a single unprefixed block; 1..=127 means that many blocks,
/// each preceded by a signed little-endian 32-bit length. As in the original reader,
/// bytes following the declared chunks are ignored. The output allocation is fallible
/// and capped by both the caller's budget and the input's maximum LZ4 expansion.
/// This bounds the returned plane, not backend internal scratch memory.
pub fn decompress(input: &[u8], max_output: usize) -> Result<Vec<u8>, Error> {
    let (&chunks, mut rest) = input.split_first().ok_or(Error::Framing("empty block"))?;
    if chunks > 127 {
        return Err(Error::Framing("chunk count exceeds 127"));
    }
    let capacity = max_output.min(input.len().saturating_mul(256).saturating_add(64));
    let mut output = Vec::new();
    output.try_reserve_exact(capacity).map_err(|_| Error::Allocation)?;
    output.resize(capacity, 0);
    let written = if chunks == 0 {
        lz4_flex::block::decompress_into(rest, &mut output)?
    } else {
        let mut total = 0usize;
        for _ in 0..chunks {
            let length = rest.get(..4).and_then(|b| b.try_into().ok()).map(i32::from_le_bytes).ok_or(Error::Framing("missing chunk length"))?;
            let length = usize::try_from(length).map_err(|_| Error::Framing("negative chunk length"))?;
            let end = 4usize.checked_add(length).ok_or(Error::Framing("chunk length overflow"))?;
            let chunk = rest.get(4..end).ok_or(Error::Framing("truncated chunk"))?;
            rest = rest.get(end..).unwrap_or_default();
            let tail = output.get_mut(total..).ok_or(Error::Framing("output offset overflow"))?;
            let count = lz4_flex::block::decompress_into(chunk, tail)?;
            total = total.checked_add(count).ok_or(Error::Framing("output length overflow"))?;
        }
        total
    };
    output.truncate(written);
    Ok(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn single_block_matches_original_wire() {
        for n in [0, 1, 15, 16, 255, 4096] {
            let raw: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            let mut frame = vec![0];
            frame.extend(lz4_flex::block::compress(&raw));
            assert_eq!(decompress(&frame, raw.len()).unwrap(), raw);
            if n > 0 {
                assert!(decompress(&frame, n - 1).is_err());
            }
        }
    }

    #[test]
    fn counted_blocks_and_trailing_bytes_keep_existing_policy() {
        let mut frame = vec![2];
        for raw in [b"first".as_slice(), b"second".as_slice()] {
            let block = lz4_flex::block::compress(raw);
            frame.extend((block.len() as i32).to_le_bytes());
            frame.extend(block);
        }
        frame.extend(b"ignored");
        assert_eq!(decompress(&frame, 11).unwrap(), b"firstsecond");
        assert!(decompress(&frame, 10).is_err());
    }

    #[test]
    fn malformed_frames_fail_within_budget() {
        for data in [vec![], vec![128], vec![1], vec![1, 255, 255, 255, 255], vec![1, 5, 0, 0, 0, 1]] {
            assert!(decompress(&data, 32).is_err());
        }
    }
}
