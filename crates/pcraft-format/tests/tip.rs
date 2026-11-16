use craft_pcraft_format::tip::{MAX_SAMPLES, MAX_SIDE, decode, decode_with_max_side, encode};
use std::io::Write;

fn fixture(bits: u8, width: u32, height: u32, samples: &[u8]) -> Vec<u8> {
    let mut out = b"PCTIP1".to_vec();
    out.push(bits);
    out.extend(width.to_le_bytes());
    out.extend(height.to_le_bytes());
    let mut z = flate2::write::ZlibEncoder::new(out, flate2::Compression::new(3));
    z.write_all(samples).unwrap();
    z.finish().unwrap()
}

#[test]
fn legacy_eight_and_sixteen_bit_fixtures_and_roundtrips() {
    for (bits, raw, samples) in [(8, vec![0, 128, 255], vec![0, 32896, 65535]), (16, vec![1, 0, 52, 18, 255, 255], vec![1, 0x1234, 65535])] {
        let legacy = fixture(bits, 3, 1, &raw);
        let tip = decode(&legacy).unwrap();
        assert_eq!((tip.width, tip.height), (3, 1));
        assert_eq!(tip.data, samples);
        let encoded = encode(3, 1, &samples);
        assert_eq!(&encoded[..15], &legacy[..15]);
        assert_eq!(decode(&encoded).unwrap(), tip);
    }
}

#[test]
fn checks_headers_limits_and_sample_lengths() {
    let good = fixture(8, 1, 1, &[255]);
    for n in 0..good.len() {
        assert!(decode(&good[..n]).is_err(), "truncation at {n}");
    }
    let mut bad = good.clone();
    bad[0] = b'X';
    assert!(decode(&bad).is_err());
    for (bits, w, h) in [(7, 1, 1), (8, 0, 1), (8, 1, 0), (8, MAX_SIDE + 1, 1), (8, MAX_SIDE, MAX_SIDE)] {
        assert!(decode(&fixture(bits, w, h, &[])).is_err());
    }
    assert!(MAX_SAMPLES < u64::from(MAX_SIDE) * u64::from(MAX_SIDE));
    for raw in [vec![], vec![0, 1], vec![0; 10000]] {
        assert!(decode(&fixture(8, 1, 1, &raw)).is_err());
    }
    assert!(decode(&fixture(16, 1, 1, &[1])).is_err());
    let mut checksum = good;
    *checksum.last_mut().unwrap() ^= 1;
    assert!(decode(&checksum).is_err());
    assert!(encode(0, 1, &[]).is_empty());
    assert!(encode(1, 1, &[]).is_empty());
    assert!(encode(MAX_SIDE + 1, 1, &[0]).is_empty());
}

#[test]
fn preview_side_limit_is_checked_before_inflation() {
    let good = encode(2, 1, &[0, 65535]);
    assert!(decode_with_max_side(&good, 1).is_err());
    assert!(decode_with_max_side(&good, 2).is_ok());
    assert!(decode_with_max_side(&good, 0).is_err());
}
