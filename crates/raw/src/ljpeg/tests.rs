use super::*;

/// Independent 4×4, 16-bit Sony 4:2:0 stream. Hand-specified differences in MCU order.
pub(crate) fn fixture_420() -> Vec<u8> {
    fixture_subsampled(4, 4, 0x22, &[-31768, 1, 1000, 1, -16384, -16284, 1, 1, 1, 1, 1, 2, 100, 1, 1000, 1, 100, 100, 1, 1, 1, 1, 1, 2])
}

/// A 16-bit three-component frame of `width × height` pixels with the given luma `sampling` (`0x21` / `0x22`)
/// and one shared 5-bit Huffman table, whose differences are written as given (MCU order).
pub(crate) fn fixture_subsampled(width: u8, height: u8, sampling: u8, differences: &[i32]) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xc3, 0, 17, 16, 0, height, 0, width, 3, 1, sampling, 0, 2, 0x11, 0, 3, 0x11, 0];
    out.extend_from_slice(&[0xff, 0xc4, 0, 36, 0]);
    let mut counts = [0; 16];
    counts[4] = 17;
    out.extend_from_slice(&counts);
    out.extend(0..=16);
    out.extend_from_slice(&[0xff, 0xda, 0, 12, 3, 1, 0, 2, 0, 3, 0, 1, 0, 0]);
    let mut bw = BitWriter { out, acc: 0, n: 0 };
    for &d in differences {
        let s = ssss_of(d);
        bw.put(s as u32, 5);
        if s > 0 && s < 16 {
            let v = if d < 0 { d - 1 } else { d };
            bw.put(v as u32 & ((1 << s) - 1), s as u32);
        }
    }
    bw.flush();
    bw.out.extend_from_slice(&[0xff, 0xd9]);
    bw.out
}

#[test]
fn horizontal_only_subsampling_preserves_rows_and_chroma() {
    // Independent 4×2, 16-bit 4:2:2 stream: two luma samples, Cb, Cr per MCU.
    let enc = fixture_subsampled(4, 2, 0x21, &[-31768, 1, -16384, -16284, 1, 1, 1, 2, 100, 1, 100, 100, 1, 1, 1, 2]);
    let frame = decode_subsampled(&enc, 16, Prediction::Geometric).unwrap();
    assert_eq!((frame.width, frame.height, frame.vertical_subsampling), (4, 2, 1));
    assert_eq!(frame.planes[0], [1000, 1001, 1002, 1003, 1100, 1101, 1102, 1103]);
    assert_eq!(frame.planes[1], [16384, 16385, 16484, 16485]);
    assert_eq!(frame.planes[2], [16484, 16486, 16584, 16586]);
    assert!(decode(&enc, 24).is_err());
    assert!(matches!(decode_subsampled(&enc, 15, Prediction::Geometric), Err(RawError::Limit(_))));
    assert!(decode_subsampled(&enc[..enc.len() - 6], 16, Prediction::Geometric).is_err());
}

#[test]
fn subsampled_mcu_order_preserves_each_plane() {
    let enc = fixture_420();
    let frame = decode_subsampled(&enc, 24, Prediction::Geometric).unwrap();
    assert_eq!((frame.width, frame.height), (4, 4));
    assert_eq!(frame.planes[0], [1000, 1001, 1002, 1003, 2000, 2001, 2002, 2003, 1100, 1101, 1102, 1103, 2100, 2101, 2102, 2103]);
    assert_eq!(frame.planes[1], [16384, 16385, 16484, 16485]);
    assert_eq!(frame.planes[2], [16484, 16486, 16584, 16586]);
    assert!(decode(&enc, 48).is_err()); // CFA API must keep rejecting subsampling.
    assert!(matches!(decode_subsampled(&enc, 23, Prediction::Geometric), Err(RawError::Limit(_))));
    assert!(decode_subsampled(&enc[..enc.len() - 6], 24, Prediction::Geometric).is_err());
    let mut bad = enc.clone();
    bad[10] = 3; // odd frame width
    assert!(frame_info_subsampled(&bad).is_err());
    let sos = bad.windows(2).position(|w| w == [0xff, 0xda]).unwrap();
    bad = enc.clone();
    bad[sos + 11] = 2; // unsupported predictor
    assert!(frame_info_subsampled(&bad).is_err());
}

#[test]
fn sequential_prediction_chains_each_component_in_decoding_order() {
    // Canon sRAW / mRAW: the same stream as `subsampled_mcu_order_preserves_each_plane`, but each component
    // continues from its previous sample in decoding order (the second luma row of an MCU follows the first),
    // and a new MCU row starts from the first sample of the previous one. Worked out by hand from init 32768.
    let enc = fixture_420();
    let frame = decode_subsampled(&enc, 24, Prediction::Sequential).unwrap();
    assert_eq!((frame.width, frame.height, frame.vertical_subsampling), (4, 4, 2));
    assert_eq!(frame.planes[0], [1000, 1001, 2003, 2004, 2001, 2002, 2005, 2006, 1100, 1101, 2103, 2104, 2101, 2102, 2105, 2106]);
    assert_eq!(frame.planes[1], [16384, 16385, 16484, 16485]);
    assert_eq!(frame.planes[2], [16484, 16486, 16584, 16586]);
    // 4:2:2 has one luma row per MCU row, so both predictions agree
    let enc = fixture_subsampled(4, 2, 0x21, &[-31768, 1, -16384, -16284, 1, 1, 1, 2, 100, 1, 100, 100, 1, 1, 1, 2]);
    assert_eq!(
        decode_subsampled(&enc, 16, Prediction::Sequential).unwrap().planes,
        decode_subsampled(&enc, 16, Prediction::Geometric).unwrap().planes
    );
}

fn noise(n: usize, bits: u32, seed: u64) -> Vec<u16> {
    let mut s = seed;
    (0..n)
        .map(|i| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let smooth = ((i % 97) as u32 * 37) % (1 << bits);
            let jitter = ((s >> 33) as u32) % 64;
            ((smooth + jitter) % (1 << bits)) as u16
        })
        .collect()
}

#[test]
fn roundtrip_all_predictors_components_precisions() {
    for pred in 1..=7u8 {
        for comps in 1..=4usize {
            for bits in [8u8, 12, 14, 16] {
                let (w, h) = (13, 7);
                let data = noise(w * h * comps, bits as u32, (pred as u64) * 31 + comps as u64);
                let enc = encode(&data, w, h, comps, bits, pred, 0);
                let f = decode(&enc, usize::MAX).unwrap();
                assert_eq!((f.width, f.height, f.components, f.precision, f.predictor), (w, h, comps, bits, pred));
                assert_eq!(f.data, data, "pred {pred} comps {comps} bits {bits}");
            }
        }
    }
}

#[test]
fn roundtrip_extremes_and_restarts() {
    // alternating 0 / 65535 forces SSSS = 16 differences
    let data: Vec<u16> = (0..64).map(|i| if i % 2 == 0 { 0 } else { 65535 }).collect();
    for pred in 1..=7 {
        let enc = encode(&data, 8, 8, 1, 16, pred, 0);
        assert_eq!(decode(&enc, usize::MAX).unwrap().data, data);
    }
    let data = noise(20 * 9 * 2, 12, 5);
    for rst in [1usize, 7, 20, 40] {
        let enc = encode(&data, 20, 9, 2, 12, 6, rst);
        assert_eq!(decode(&enc, usize::MAX).unwrap().data, data, "restart {rst}");
    }
}

#[test]
fn constant_image_single_symbol_table() {
    let data = vec![1234u16; 16 * 4];
    let enc = encode(&data, 16, 4, 1, 14, 1, 0);
    assert_eq!(decode(&enc, usize::MAX).unwrap().data, data);
}

#[test]
fn rejects_bad_streams() {
    assert!(decode(&[], 100).is_err());
    assert!(decode(&[0xff, 0xd8, 0xff, 0xd9], 100).is_err());
    let data = noise(64, 12, 1);
    let enc = encode(&data, 8, 8, 1, 12, 1, 0);
    assert!(matches!(decode(&enc, 10), Err(RawError::Limit(_))));
    for n in 0..enc.len() {
        let _ = decode(&enc[..n], usize::MAX);
    }
    let mut bad = enc.clone();
    for i in (20..bad.len()).step_by(3) {
        bad[i] ^= 0x5a;
    }
    let _ = decode(&bad, usize::MAX);
}

// --- selection value 8 (Hasselblad 3FR / FFF) ---

/// SOI, SOF3 (16 bits, `height` x `width`, one component), a DHT and an SOS with selection value 8, then
/// `entropy` and EOI.
fn hasselblad_stream(width: u16, height: u16, counts: &[u8; 16], values: &[u8], entropy: &[u8]) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xc3, 0, 11, 16];
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[1, 0, 0x11, 0]);
    out.extend_from_slice(&[0xff, 0xc4]);
    out.extend_from_slice(&((2 + 17 + values.len()) as u16).to_be_bytes());
    out.push(0);
    out.extend_from_slice(counts);
    out.extend_from_slice(values);
    out.extend_from_slice(&[0xff, 0xda, 0, 8, 1, 0, 0, 8, 0, 0]);
    out.extend_from_slice(entropy);
    out.extend_from_slice(&[0xff, 0xd9]);
    out
}

/// Codes: SSSS 0 = `00`, 1 = `01`, 2 = `100`, 15 = `101`, 16 = `110`.
const HAND_COUNTS: [u8; 16] = [0, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const HAND_VALUES: [u8; 5] = [0, 1, 2, 15, 16];

/// A 4 x 2 frame written out bit by bit:
/// - row 0, pair 1: codes `110` `110` (SSSS 16 twice), then 16 bits each (`FFFF`, `7FFF`) that do not count:
///   both samples are 32768 - 32768 = 0;
/// - row 0, pair 2: codes `01` `100`, bits `1` (+1) and `01` (-2): 0 + 1 = 1 and 0 - 2 = 65534 (modulo 2^16);
/// - row 1, pair 1: codes `00` `01`, bit `0` (-1): both predictors start again at 32768, so 32768 and 32767;
/// - row 1, pair 2: codes `100` `00`, bits `11` (+3): 32771 and 32767.
///
/// 58 bits, padded with zeros to two 32-bit words `DBFFFDFF` `FD9450C0`, stored little-endian. The `FF FD`
/// they start with would be a marker to a T.81 reader: this coding has no byte stuffing.
const HAND_WORDS: [u8; 8] = [0xff, 0xfd, 0xff, 0xdb, 0xc0, 0x50, 0x94, 0xfd];
const HAND_SAMPLES: [u16; 8] = [0, 0, 1, 65534, 32768, 32767, 32771, 32767];

#[test]
fn selection_value_8_known_answer() {
    let enc = hasselblad_stream(4, 2, &HAND_COUNTS, &HAND_VALUES, &HAND_WORDS);
    assert_eq!(frame_info(&enc).unwrap(), (4, 2, 1, 16));
    let f = decode(&enc, 8).unwrap();
    assert_eq!((f.width, f.height, f.components, f.predictor), (4, 2, 1, 8));
    assert_eq!(f.data, HAND_SAMPLES);
    assert_eq!(decode_hasselblad(&enc, 8).unwrap().data, HAND_SAMPLES);
    assert!(matches!(decode(&enc, 7), Err(RawError::Limit(_))));
}

/// The same bits in plain byte order (not 32-bit little-endian words) are a different stream.
#[test]
fn selection_value_8_reads_little_endian_words() {
    let mut plain = HAND_WORDS;
    plain[..4].reverse();
    plain[4..].reverse();
    let enc = hasselblad_stream(4, 2, &HAND_COUNTS, &HAND_VALUES, &plain);
    assert_ne!(decode(&enc, 8).map(|f| f.data).ok(), Some(HAND_SAMPLES.to_vec()));
}

#[test]
fn selection_value_8_rejects_bad_streams() {
    // one word is not enough for the second row
    let short = hasselblad_stream(4, 2, &HAND_COUNTS, &HAND_VALUES, &HAND_WORDS[..4]);
    assert!(decode(&short, 8).is_err());
    // a category above 16
    let enc = hasselblad_stream(4, 2, &HAND_COUNTS, &[0, 1, 2, 15, 17], &HAND_WORDS);
    assert!(decode(&enc, 8).is_err());
    // layouts never seen with selection value 8 are not guessed at
    let mut odd = hasselblad_stream(3, 2, &HAND_COUNTS, &HAND_VALUES, &HAND_WORDS);
    assert!(matches!(frame_info(&odd), Err(RawError::Unsupported(_))));
    assert!(matches!(decode(&odd, 8), Err(RawError::Unsupported(_))));
    odd = hasselblad_stream(4, 2, &HAND_COUNTS, &HAND_VALUES, &HAND_WORDS);
    odd[6] = 14; // precision
    assert!(matches!(decode(&odd, 8), Err(RawError::Unsupported(_))));
    // selection values above 8 stay unknown
    let mut nine = hasselblad_stream(4, 2, &HAND_COUNTS, &HAND_VALUES, &HAND_WORDS);
    let sos = nine.windows(2).position(|w| w == [0xff, 0xda]).unwrap();
    nine[sos + 7] = 9;
    assert!(matches!(decode(&nine, 8), Err(RawError::Unsupported(w)) if w.contains("selection value 9")));
    // every prefix fails cleanly
    let enc = hasselblad_stream(4, 2, &HAND_COUNTS, &HAND_VALUES, &HAND_WORDS);
    for n in 0..enc.len() {
        let _ = decode(&enc[..n], 8);
    }
}

/// Test-only writer of the selection-value-8 coding (the inverse of the rule in [`decode_hasselblad`]), with
/// a 5-bit code for every category. Category-16 differences carry `FFFF` as their ignored bits.
pub(crate) fn encode_hasselblad(data: &[u16], width: usize, height: usize) -> Vec<u8> {
    assert!(width.is_multiple_of(2) && data.len() == width * height);
    let mut bits: Vec<bool> = vec![];
    let put = |bits: &mut Vec<bool>, v: u32, n: u32| {
        for i in (0..n).rev() {
            bits.push((v >> i) & 1 == 1);
        }
    };
    for row in data.chunks_exact(width) {
        let mut last = [32768i32, 32768];
        for pair in row.as_chunks::<2>().0 {
            let mut d = [0i32; 2];
            for k in 0..2 {
                let mut v = (pair[k] as i32 - last[k]) & 0xffff;
                if v >= 32768 {
                    v -= 65536;
                }
                d[k] = v;
                last[k] = pair[k] as i32;
            }
            let s = [ssss_of(d[0]), ssss_of(d[1])];
            put(&mut bits, s[0] as u32, 5);
            put(&mut bits, s[1] as u32, 5);
            for k in 0..2 {
                match s[k] {
                    0 => {}
                    16 => put(&mut bits, 0xffff, 16),
                    n => {
                        let v = if d[k] < 0 { d[k] - 1 } else { d[k] };
                        put(&mut bits, v as u32 & ((1 << n) - 1), n as u32);
                    }
                }
            }
        }
    }
    let mut entropy = vec![];
    for word in bits.chunks(32) {
        let w = word.iter().enumerate().fold(0u32, |a, (i, &b)| a | ((b as u32) << (31 - i)));
        entropy.extend_from_slice(&w.to_le_bytes());
    }
    let mut counts = [0u8; 16];
    counts[4] = 17;
    let values: Vec<u8> = (0..=16).collect();
    hasselblad_stream(width as u16, height as u16, &counts, &values, &entropy)
}

#[test]
fn selection_value_8_round_trips_through_the_test_writer() {
    // extremes force SSSS 16 and wrap-around; noise exercises every other category
    let mut data = noise(10 * 7, 16, 9);
    data[0] = 0;
    data[1] = 65535;
    data[2] = 65535;
    data[3] = 0;
    let enc = encode_hasselblad(&data, 10, 7);
    assert_eq!(decode(&enc, 70).unwrap().data, data);
}

#[test]
fn code_lengths_are_limited_to_16() {
    // Fibonacci-like frequencies produce very long codes before adjustment
    let mut f = [0u32; 17];
    let (mut a, mut b) = (1u32, 1u32);
    for x in f.iter_mut() {
        *x = a;
        let c = a.saturating_add(b);
        a = b;
        b = c;
    }
    let (counts, values) = code_lengths(&f);
    assert_eq!(counts.iter().map(|&c| c as usize).sum::<usize>(), 17);
    assert_eq!(values.len(), 17);
    assert!(Huffman::new(&counts, &values).is_ok());
}
