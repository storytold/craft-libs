//! Robustness tests that need no oracle.

use super::*;

#[test]
fn garbage_never_panics() {
    let mut x = 0x9E37_79B9u32;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for len in [0usize, 1, 3, 4, 5, 17, 100, 1000] {
        for _ in 0..50 {
            let mut data: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            if len > 8 {
                data[..4].copy_from_slice(&[0, 0, 1, 0xB6]);
            }
            let mut d = Decoder::new();
            let _ = d.decode(&data, 0);
            let _ = d.flush();
        }
    }
}

#[test]
fn start_codes_are_found() {
    let data = [0, 0, 1, 0xB0, 3, 0, 0, 0, 1, 0xB6, 0, 0, 1];
    assert_eq!(start_codes(&data), vec![(0, 0xB0), (6, 0xB6)]);
}

#[test]
fn vop_before_vol_is_an_error() {
    let mut d = Decoder::new();
    assert!(d.decode(&[0, 0, 1, 0xB6, 0x10, 0x40], 0).is_err());
    assert_eq!(d.errors(), 1);
}
