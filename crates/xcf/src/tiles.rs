//! One tile's bytes: uncompressed, zlib-coded or RLE-coded.

use crate::{Compression, Result, invalid};

/// What one tile's bytes decode to, for a drawable of `bpp` bytes per pixel: `pixels × bpp`
/// interleaved bytes.
pub(crate) fn decode(input: &[u8], compression: Compression, bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let expected = pixels.checked_mul(bpp).ok_or_else(|| invalid("tile size overflow"))?;
    match compression {
        Compression::None => input.get(..expected).map(<[u8]>::to_vec).ok_or_else(|| invalid("an uncompressed tile is cut short")),
        Compression::Zlib => {
            // Reader work policy, not a restriction imposed by the XCF spec:
            // allow 64 KiB of framing plus twice the logical tile size. This
            // admits ordinary/incompressible and small edge tiles, but bounds
            // empty-block padding that produces no chargeable decoded samples.
            // Later file records may follow the stream; cap what we inspect,
            // never require the entire supplied slice to belong to this tile.
            let input_limit =
                expected.checked_mul(2).and_then(|bytes| bytes.checked_add(64 * 1024)).ok_or_else(|| invalid("zlib work limit overflow"))?;
            let input = input.get(..input.len().min(input_limit)).ok_or_else(|| invalid("invalid zlib input bound"))?;
            // A sample-sized Read::take stops before checking the end marker and
            // Adler trailer. One extra output byte detects oversize without ever
            // inflating an input-sized buffer, while StreamEnd verifies the trailer.
            let capacity = expected.checked_add(1).ok_or_else(|| invalid("tile size overflow"))?;
            let mut out = Vec::new();
            out.try_reserve_exact(capacity).map_err(|_| invalid("not enough memory for a zlib tile"))?;
            out.resize(capacity, 0);
            let mut decoder = flate2::Decompress::new(true);
            loop {
                let read = usize::try_from(decoder.total_in()).map_err(|_| invalid("zlib input position overflow"))?;
                let written = usize::try_from(decoder.total_out()).map_err(|_| invalid("zlib output position overflow"))?;
                let input = input.get(read..).ok_or_else(|| invalid("invalid zlib input position"))?;
                let output = out.get_mut(written..).ok_or_else(|| invalid("invalid zlib output position"))?;
                let status = decoder.decompress(input, output, flate2::FlushDecompress::Finish).map_err(|e| invalid(format!("zlib tile: {e}")))?;
                let produced = usize::try_from(decoder.total_out()).map_err(|_| invalid("zlib output position overflow"))?;
                if produced > expected {
                    return Err(invalid("a zlib tile exceeds its decoded size"));
                }
                if status == flate2::Status::StreamEnd {
                    if produced != expected {
                        return Err(invalid("a zlib tile is cut short"));
                    }
                    out.truncate(expected);
                    // Tile pointers can expose later records too. Preserve those
                    // bytes: only this first complete zlib stream belongs to us.
                    return Ok(out);
                }
                if decoder.total_in() == read as u64 && decoder.total_out() == written as u64 {
                    return Err(invalid("a zlib tile is cut short or has no complete trailer"));
                }
            }
        }
        Compression::Rle => rle_decode(input, bpp, pixels),
    }
}

/// RLE tile data: one run-length stream per byte position of the pixel, each of `pixels` bytes.
pub(crate) fn rle_decode(input: &[u8], bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let short = || invalid("an RLE tile is cut short");
    let expected = pixels.checked_mul(bpp).ok_or_else(|| invalid("tile size overflow"))?;
    if expected == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    out.try_reserve_exact(expected).map_err(|_| invalid("not enough memory for an RLE tile"))?;
    out.resize(expected, 0);
    let mut pos = 0usize;
    let mut plane = Vec::new();
    plane.try_reserve_exact(pixels).map_err(|_| invalid("not enough memory for an RLE plane"))?;
    for b in 0..bpp {
        plane.clear();
        while plane.len() < pixels {
            let op = *input.get(pos).ok_or_else(short)?;
            pos += 1;
            let mut long = || -> Result<usize> {
                let (p, q) = (input.get(pos).copied().ok_or_else(short)?, input.get(pos + 1).copied().ok_or_else(short)?);
                pos += 2;
                Ok(usize::from(p) * 256 + usize::from(q))
            };
            let (run, verbatim) = match op {
                0..=126 => (usize::from(op) + 1, false),
                127 => (long()?, false),
                128 => (long()?, true),
                _ => (256 - usize::from(op), true),
            };
            if run > pixels.saturating_sub(plane.len()) {
                return Err(invalid("an RLE run crosses the end of its tile"));
            }
            // A long run of length 0 consumes input without producing output: a tile of them
            // (reachable from many tile pointers) is work the pixel budget never charges.
            if run == 0 {
                return Err(invalid("an RLE run has no length"));
            }
            if verbatim {
                let end = pos.checked_add(run).ok_or_else(short)?;
                let data = input.get(pos..end).ok_or_else(short)?;
                pos = end;
                plane.extend_from_slice(data);
            } else {
                let v = *input.get(pos).ok_or_else(short)?;
                pos += 1;
                plane.resize(plane.len() + run, v);
            }
        }
        for (slot, v) in out.iter_mut().skip(b).step_by(bpp.max(1)).zip(&plane) {
            *slot = *v;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zlib(bytes: &[u8]) -> Vec<u8> {
        let mut writer = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap()
    }

    #[test]
    fn zlib_tiles_reject_decoded_output_larger_than_the_declared_tile() {
        let encoded = zlib(&[17, 31, 57, 255, 99]);
        assert!(decode(&encoded, Compression::Zlib, 4, 1).is_err());
    }

    #[test]
    fn zlib_tiles_require_the_complete_trailer_after_all_samples() {
        let encoded = zlib(&[17, 31, 57, 255]);
        for missing in 1..=4 {
            assert!(decode(&encoded[..encoded.len() - missing], Compression::Zlib, 4, 1).is_err(), "missing {missing} trailer bytes");
        }
    }

    #[test]
    fn zlib_tiles_reject_corrupt_trailers_even_when_the_sample_output_is_complete() {
        let mut encoded = zlib(&[17, 31, 57, 255]);
        let last = encoded.len() - 1;
        encoded[last] ^= 1;
        assert!(decode(&encoded, Compression::Zlib, 4, 1).is_err());
    }

    #[test]
    fn complete_zlib_tiles_preserve_samples_and_allow_following_file_bytes() {
        let expected = [17, 31, 57, 255];
        let mut encoded = zlib(&expected);
        assert_eq!(decode(&encoded, Compression::Zlib, 4, 1).unwrap(), expected);
        encoded.extend_from_slice(&zlib(&[0; 4]));
        encoded.extend_from_slice(b"later XCF records");
        assert_eq!(decode(&encoded, Compression::Zlib, 4, 1).unwrap(), expected, "only the first complete tile stream is consumed");
        assert!(decode(&zlib(&expected[..3]), Compression::Zlib, 4, 1).is_err());
    }

    // Original RFC1950/1951 fixture: empty nonfinal stored blocks, then one
    // final stored block. Even an arbitrarily padded stream is valid zlib.
    fn padded_zlib(bytes: &[u8], empty_blocks: usize) -> Vec<u8> {
        let length = u16::try_from(bytes.len()).unwrap();
        let mut encoded = vec![0x78, 0x01];
        for _ in 0..empty_blocks {
            encoded.extend([0, 0, 0, 255, 255]);
        }
        encoded.push(1);
        encoded.extend(length.to_le_bytes());
        encoded.extend((!length).to_le_bytes());
        encoded.extend(bytes);
        let (a, b) = bytes.iter().fold((1u32, 0u32), |(a, b), byte| {
            let a = (a + u32::from(*byte)) % 65_521;
            (a, (b + a) % 65_521)
        });
        encoded.extend(((b << 16) | a).to_be_bytes());
        encoded
    }

    #[test]
    fn zlib_tile_work_stops_at_the_reader_allowance_and_keeps_later_records() {
        use std::io::Read;
        let expected = [17, 31, 57, 255];
        let allowance = 64 * 1024 + 2 * expected.len();
        let last_admitted = (allowance - 15) / 5;
        let mut admitted = padded_zlib(&expected, last_admitted);
        assert!(admitted.len() <= allowance && admitted.len() + 5 > allowance);
        assert_eq!(decode(&admitted, Compression::Zlib, 4, 1).unwrap(), expected);
        admitted.extend(vec![99; allowance * 2]);
        assert_eq!(decode(&admitted, Compression::Zlib, 4, 1).unwrap(), expected, "later records are not part of the tile stream");
        let refused = padded_zlib(&expected, last_admitted + 1);
        let mut independent = Vec::new();
        flate2::read::ZlibDecoder::new(refused.as_slice()).read_to_end(&mut independent).unwrap();
        assert_eq!(independent, expected, "refusal is reader work policy, not malformed zlib");
        assert!(decode(&refused, Compression::Zlib, 4, 1).is_err());
    }

    #[test]
    fn ordinary_incompressible_full_precision_zlib_tiles_fit_the_work_allowance() {
        let mut state = 0x9e37_79b9u32;
        let bytes: Vec<_> = (0..64 * 64 * 32)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        assert_eq!(decode(&zlib(&bytes), Compression::Zlib, 32, 64 * 64).unwrap(), bytes);
    }

    #[test]
    fn rle_tile_size_overflow_returns_an_error_before_allocation() {
        assert!(rle_decode(&[], 2, usize::MAX).is_err());
    }
}
