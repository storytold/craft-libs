//! Unit and property tests for the bitstream helpers.

use super::*;
use proptest::prelude::*;

#[test]
fn exp_golomb_known() {
    // 1 | 010 | 011 | 00100 | 00101 -> 0,1,2,3,4
    let mut w = BitWriter::new();
    for v in 0..5 {
        w.write_ue(v);
    }
    let b = w.finish();
    let mut r = BitReader::new(&b);
    for v in 0..5 {
        assert_eq!(r.read_ue().unwrap(), v);
    }
    let mut w = BitWriter::new();
    for v in [0, 1, -1, 2, -2, 1000, -1000] {
        w.write_se(v);
    }
    let b = w.finish();
    let mut r = BitReader::new(&b);
    for v in [0, 1, -1, 2, -2, 1000, -1000] {
        assert_eq!(r.read_se().unwrap(), v);
    }
}

proptest! {
    #[test]
    fn roundtrip(vals in proptest::collection::vec((0u32..u32::MAX, 1u32..=32), 0..200), ues in proptest::collection::vec(0u32..u32::MAX - 1, 0..50)) {
        let mut w = BitWriter::new();
        for &(v, n) in &vals { w.write_bits(v, n); }
        for &u in &ues { w.write_ue(u); }
        let b = w.finish();
        let mut r = BitReader::new(&b);
        for &(v, n) in &vals {
            let mask = if n == 32 { u32::MAX } else { (1 << n) - 1 };
            prop_assert_eq!(r.read_bits(n).unwrap(), v & mask);
        }
        for &u in &ues { prop_assert_eq!(r.read_ue().unwrap(), u); }
    }

    #[test]
    fn escape_roundtrip(data in proptest::collection::vec(0u8..4, 0..300)) {
        let e = escape_rbsp(&data);
        prop_assert_eq!(unescape_rbsp(&e), data);
        for w in e.windows(3) { prop_assert!(!(w[0] == 0 && w[1] == 0 && w[2] < 3)); }
    }
}
