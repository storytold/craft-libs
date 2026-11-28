//! RunLengthDecode (ISO 32000-2 §7.4.5): a length byte L of 0–127 copies the next
//! L + 1 bytes, 129–255 repeats the next byte 257 − L times, 128 is EOD.
//! A missing EOD is tolerated; a truncated run is corrupt.

use crate::{Failure, Step};

const NAME: &str = "RunLengthDecode";

pub fn decode(data: &[u8], max: usize) -> Step {
    decode_packets(data, max, Mode::Whole).map(|(out, _)| out)
}

/// Decode one run-length stream and return its bytes and the number of input bytes consumed.
///
/// The count includes the end-of-data byte (128); any bytes after it remain unread. EOF between
/// packets is accepted without an end marker. Each packet must be complete before its output
/// limit is checked, so a truncated packet is corrupt even if its available bytes exceed `max`.
/// An earlier complete packet exceeding the limit still fails before any later packet is read.
/// On corruption, partial output contains only complete preceding packets.
///
/// Output grows as packets arrive instead of reserving space for the input, which may include
/// an arbitrarily long program after the end marker. The existing [`decode`] retains its
/// allocation and partial-literal limit policy.
pub fn decode_prefix(data: &[u8], max: usize) -> Result<(Vec<u8>, usize), Failure> {
    decode_packets(data, max, Mode::Prefix)
}

enum Mode {
    Whole,
    Prefix,
}

fn decode_packets(data: &[u8], max: usize, mode: Mode) -> Result<(Vec<u8>, usize), Failure> {
    let mut out = match mode {
        Mode::Whole => Vec::with_capacity(data.len().saturating_mul(2).min(max)),
        Mode::Prefix => Vec::new(),
    };
    let mut i = 0;
    while let Some(&l) = data.get(i) {
        i += 1;
        match l {
            128 => break,
            0..=127 => {
                let n = usize::from(l) + 1;
                let avail = n.min(data.len() - i);
                if matches!(mode, Mode::Prefix) && avail < n {
                    return Err(Failure::corrupt(NAME, "truncated literal run", out));
                }
                if out.len() + avail > max {
                    return Err(Failure::limit(max));
                }
                out.extend_from_slice(&data[i..i + avail]);
                i += avail;
                if avail < n {
                    return Err(Failure::corrupt(NAME, "truncated literal run", out));
                }
            }
            _ => {
                let Some(&b) = data.get(i) else {
                    return Err(Failure::corrupt(NAME, "truncated repeat run", out));
                };
                i += 1;
                let n = 257 - usize::from(l);
                if out.len() + n > max {
                    return Err(Failure::limit(max));
                }
                out.resize(out.len() + n, b);
            }
        }
    }
    Ok((out, i))
}

pub fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 128 + 2);
    encode_packets(data, &mut out, 2);
    out.push(128);
    out
}

/// Append a TIFF PackBits row without an end-of-data marker.
///
/// Runs of three or more bytes use repeat packets; pairs remain in literal packets.
/// Existing output is retained, permitting rows to be appended directly to a strip.
/// PostScript callers can append their own EOD byte after the final row.
pub fn encode_packbits_into(data: &[u8], out: &mut Vec<u8>) {
    encode_packets(data, out, 3);
}

fn encode_packets(data: &[u8], out: &mut Vec<u8>, repeat_min: usize) {
    let n = data.len();
    let mut i = 0;
    while i < n {
        let mut run = 1;
        while i + run < n && run < 128 && data[i + run] == data[i] {
            run += 1;
        }
        if run >= repeat_min {
            out.push((257 - run) as u8);
            out.push(data[i]);
            i += run;
            continue;
        }
        // Literal: extend until a repeat begins or 128 bytes.
        let start = i;
        let mut j = i + 1;
        while j < n && j - start < 128 && !data.get(j..).and_then(|rest| rest.get(..repeat_min)).is_some_and(|run| run.iter().all(|b| *b == data[j]))
        {
            j += 1;
        }
        out.push((j - start - 1) as u8);
        out.extend_from_slice(&data[start..j]);
        i = j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodecError;

    fn dec(d: &[u8]) -> Result<Vec<u8>, CodecError> {
        decode(d, 10_000).map_err(|f| f.error)
    }

    #[test]
    fn vectors() {
        assert_eq!(dec(&[2, b'a', b'b', b'c', 254, b'x', 128]).unwrap(), b"abcxxx");
        assert_eq!(dec(&[129, 7, 128]).unwrap(), [7; 128]);
        assert!(dec(&[127]).is_err());
        assert_eq!(dec(&[0, b'q']).unwrap(), b"q"); // missing EOD
        assert_eq!(dec(&[128, 0, b'q']).unwrap(), b""); // data after EOD ignored
        assert_eq!(dec(&[]).unwrap(), b"");
    }

    #[test]
    fn truncated() {
        let f = decode(&[4, b'a', b'b'], 100).err().unwrap();
        assert!(matches!(f.error, CodecError::Corrupt { .. }));
        assert_eq!(f.partial, b"ab");
        assert!(matches!(dec(&[0, b'a', 200]), Err(CodecError::Corrupt { .. })));
    }

    #[test]
    fn limit() {
        assert!(matches!(decode(&[129, 0, 129, 0], 200).map_err(|f| f.error), Err(CodecError::LimitExceeded(200))));
        assert!(matches!(decode(&[3, 1, 2, 3, 4], 3).map_err(|f| f.error), Err(CodecError::LimitExceeded(3))));
        assert_eq!(decode(&[129, 0, 129, 0], 256).ok().unwrap().len(), 256);
    }

    #[test]
    fn prefix_stops_after_eod_without_reading_the_program_tail() {
        let bytes = [2, b'a', b'b', b'c', 254, b'x', 128, 127];
        let (out, used) = decode_prefix(&bytes, 6).ok().unwrap();
        assert_eq!(out, b"abcxxx");
        assert_eq!(&bytes[used..], &[127]); // Truncated data after EOD is not decoded.
        assert_eq!(used, 7);
        assert_eq!(decode_prefix(&[128], 0).ok().unwrap(), (vec![], 1));
    }

    #[test]
    fn prefix_accepts_empty_input_and_eof_between_packets() {
        assert_eq!(decode_prefix(&[], 0).ok().unwrap(), (vec![], 0));
        let bytes = [0, b'a', 255, b'b'];
        assert_eq!(decode_prefix(&bytes, 3).ok().unwrap(), (b"abb".to_vec(), bytes.len()));
    }

    #[test]
    fn prefix_checks_packet_completeness_before_its_output_limit() {
        for bytes in [&[2, b'a', b'b'][..], &[129][..]] {
            let failure = decode_prefix(bytes, 0).err().unwrap();
            assert!(matches!(failure.error, CodecError::Corrupt { .. }));
            assert!(failure.partial.is_empty());
        }
        let failure = decode_prefix(&[0, b'q', 2, b'a', b'b'], 1).err().unwrap();
        assert!(matches!(failure.error, CodecError::Corrupt { .. }));
        assert_eq!(failure.partial, b"q");
    }

    #[test]
    fn prefix_checks_each_complete_packet_before_later_corruption() {
        for bytes in [&[0, b'a', 127][..], &[255, b'a', 127][..]] {
            let failure = decode_prefix(bytes, 0).err().unwrap();
            assert!(matches!(failure.error, CodecError::LimitExceeded(0)));
            assert!(failure.partial.is_empty());
        }
        assert_eq!(decode_prefix(&[0, b'a', 128], 1).ok().unwrap(), (vec![b'a'], 3));
        assert_eq!(decode_prefix(&[255, b'a', 128], 2).ok().unwrap(), (vec![b'a'; 2], 3));
    }

    #[test]
    fn prefix_does_not_reserve_for_the_undecoded_tail() {
        let mut bytes = vec![128];
        bytes.resize(1 << 20, 127);
        let (out, used) = decode_prefix(&bytes, usize::MAX).ok().unwrap();
        assert_eq!(used, 1);
        assert_eq!(out.capacity(), 0);
        let out = decode(&bytes, 256).ok().unwrap();
        assert!(out.capacity() >= 256, "whole decoding keeps its input-based reservation");
    }

    #[test]
    fn whole_decode_keeps_its_partial_literal_and_limit_precedence() {
        let bytes = [2, b'a', b'b'];
        let failure = decode(&bytes, 1).err().unwrap();
        assert!(matches!(failure.error, CodecError::LimitExceeded(1)));
        assert!(failure.partial.is_empty());
        let failure = decode(&bytes, 2).err().unwrap();
        assert!(matches!(failure.error, CodecError::Corrupt { .. }));
        assert_eq!(failure.partial, b"ab");
    }

    #[test]
    fn encoding() {
        assert_eq!(encode(b"abcxxx"), [2, b'a', b'b', b'c', 254, b'x', 128]);
        assert_eq!(encode(b""), [128]);
        assert_eq!(encode(&[5; 300]), [129, 5, 129, 5, 213, 5, 128]);
        let lit: Vec<u8> = (0..200u8).collect();
        let e = encode(&lit);
        assert_eq!((e[0], e[129]), (127, 71));
        assert_eq!(dec(&e).unwrap(), lit);
    }
    #[test]
    fn packbits_appends_rows_without_eod_and_keeps_pairs_literal() {
        let mut out = vec![42];
        encode_packbits_into(b"aabbccc", &mut out);
        assert_eq!(out, [42, 3, b'a', b'a', b'b', b'b', 254, b'c']);
        let before = out.clone();
        encode_packbits_into(b"", &mut out);
        assert_eq!(out, before);
        assert_eq!(encode(b"aa"), [255, b'a', 128]);
        for n in [1, 2, 3, 127, 128, 129, 255, 256, 300] {
            for row in [vec![7; n], (0..n).map(|i| (i % 251) as u8).collect()] {
                let mut encoded = Vec::new();
                encode_packbits_into(&row, &mut encoded);
                assert_eq!(decode(&encoded, n).map_err(|failure| failure.error).unwrap(), row);
            }
        }
    }
}
