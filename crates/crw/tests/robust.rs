//! Malformed CRW files must decode to an error, never panic: a CIFF with the records the decoder
//! reads and a pseudo-random payload, mutated by bit flips and truncation.

use craft_crw::{Limits, decode, header, probe};
use proptest::prelude::*;

const W: usize = 48;
const H: usize = 20;

/// A Canon CRW: CIFF header, a raw-data payload of pseudo-random bytes, and a nested properties
/// heap with the records the decoder reads.
fn crw() -> Vec<u8> {
    let le16 = |v: u16| v.to_le_bytes();
    let le32 = |v: u32| v.to_le_bytes();
    let heap = |records: &[(u16, Vec<u8>)]| -> Vec<u8> {
        let mut b = Vec::new();
        let mut entries = Vec::new();
        for (tc, v) in records {
            entries.push((*tc, v.len() as u32, b.len() as u32));
            b.extend_from_slice(v);
            if b.len() % 2 == 1 {
                b.push(0);
            }
        }
        let tbl = b.len() as u32;
        b.extend_from_slice(&le16(entries.len() as u16));
        for (tc, l, off) in entries {
            b.extend_from_slice(&le16(tc));
            b.extend_from_slice(&le32(l));
            b.extend_from_slice(&le32(off));
        }
        b.extend_from_slice(&le32(tbl));
        b
    };
    let sensor: Vec<u8> = [34u16, W as u16, H as u16, 0, 0, 2, 2, W as u16 - 3, H as u16 - 3].iter().flat_map(|&v| le16(v)).collect();
    let dtable: Vec<u8> = [0u32, 0, 514, 0].iter().flat_map(|&v| le32(v)).collect();
    let image_info: Vec<u8> = [W as u32, H as u32, 0, 270, 12, 12, 0].iter().flat_map(|&v| le32(v)).collect();
    let mut color_balance: Vec<u16> = vec![66];
    color_balance.extend((0..8).flat_map(|i| [1800 + i * 10, 800, 800, 960 + i * 5]));
    let color_balance: Vec<u8> = color_balance.iter().flat_map(|&v| le16(v)).collect();
    let shot: Vec<u8> = [54u16; 27].iter().flat_map(|&v| le16(v)).collect();
    let props = heap(&[
        (0x1031, sensor),
        (0x1835, dtable),
        (0x080a, b"Canon\0Canon EOS D60\0".to_vec()),
        (0x1810, image_info),
        (0x180e, [1083410638u32, 0, 0].iter().flat_map(|&v| le32(v)).collect()),
        (0x10a9, color_balance),
        (0x102a, shot),
    ]);
    let payload: Vec<u8> = (0..514 + 256).map(|i| (i * 41 % 253) as u8).collect();
    let root = heap(&[(0x2005, payload), (0x300a, props)]);
    let mut f = b"II".to_vec();
    f.extend_from_slice(&le32(26));
    f.extend_from_slice(b"HEAPCCDR");
    f.extend_from_slice(&le32(0x0001_0002));
    f.extend_from_slice(&[0; 8]);
    f.extend_from_slice(&root);
    f
}

#[test]
fn the_synthetic_file_is_recognised() {
    let f = crw();
    assert!(probe(&f));
    let hdr = header(&f, &Limits::default()).expect("header");
    assert_eq!((hdr.width, hdr.height), (W, H));
    assert_eq!(hdr.exif_orientation, 8);
    assert_eq!(hdr.capture_unix, Some(1083410638));
    assert!(hdr.wb_multipliers.is_some());
    // pseudo-random entropy data decodes or is refused, never panics
    let _ = decode(&f, &Limits::default());
}

#[test]
fn every_truncation_and_single_byte_mutation_survives() {
    let good = crw();
    for n in 0..good.len() {
        let _ = header(&good[..n], &Limits::default());
        let _ = decode(&good[..n], &Limits::default());
    }
    for i in 0..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0xa5;
        let _ = decode(&bad, &Limits::default());
        bad[i] = 0xff;
        let _ = decode(&bad, &Limits::default());
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, .. ProptestConfig::default() })]

    #[test]
    fn mutated_files_never_panic(flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..16), cut in any::<usize>(), max_samples in 1usize..4096) {
        let mut data = crw();
        let n = data.len();
        for (i, v) in flips {
            data[i % n] ^= v;
        }
        data.truncate(cut % (n + 1));
        let mut limits = Limits::default();
        limits.max_samples = max_samples;
        let _ = header(&data, &limits);
        let _ = decode(&data, &limits);
        let _ = decode(&data, &Limits::default());
    }
}
