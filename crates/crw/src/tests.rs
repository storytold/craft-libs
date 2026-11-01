//! Unit tests: synthetic CIFF files built with a test-only encoder that is the inverse of the
//! frozen model, plus the frozen tables' structural properties. The decoder's behaviour on real
//! files is pinned in `tests/samples.rs`.

use super::*;

fn put16(v: u16, o: Order) -> [u8; 2] {
    match o {
        Order::Le => v.to_le_bytes(),
        Order::Be => v.to_be_bytes(),
    }
}
fn put32(v: u32, o: Order) -> [u8; 4] {
    match o {
        Order::Le => v.to_le_bytes(),
        Order::Be => v.to_be_bytes(),
    }
}

/// A heap block: values, record table, trailing offset-table pointer.
fn heap(records: &[(u16, Vec<u8>)], o: Order) -> Vec<u8> {
    let mut b = Vec::new();
    let mut entries = Vec::new();
    for (tc, v) in records {
        let off = b.len();
        b.extend_from_slice(v);
        if b.len() % 2 == 1 {
            b.push(0);
        }
        entries.push((*tc, v.len() as u32, off as u32));
    }
    let tbl = b.len() as u32;
    b.extend_from_slice(&put16(entries.len() as u16, o));
    for (tc, l, off) in entries {
        b.extend_from_slice(&put16(tc, o));
        b.extend_from_slice(&put32(l, o));
        b.extend_from_slice(&put32(off, o));
    }
    b.extend_from_slice(&put32(tbl, o));
    b
}

/// A complete CIFF file: 26-byte header + the root heap.
fn ciff(records: &[(u16, Vec<u8>)], o: Order) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(match o {
        Order::Le => b"II",
        Order::Be => b"MM",
    });
    f.extend_from_slice(&put32(26, o));
    f.extend_from_slice(b"HEAPCCDR");
    f.extend_from_slice(&put32(0x0001_0002, o));
    f.extend_from_slice(&[0; 8]);
    f.extend_from_slice(&heap(records, o));
    f
}

fn sensor_info(w: u16, h: u16, crop: Option<(u16, u16, u16, u16)>, o: Order) -> Vec<u8> {
    let (l, t, r, b) = crop.unwrap_or((0, 0, 0, 0));
    [34, w, h, 0, 0, l, t, r, b, 0, 0, 0, 0, 0, 0, 0, 0].iter().flat_map(|&v| put16(v, o)).collect()
}

fn decoder_table(sel: u32, o: Order) -> Vec<u8> {
    [sel, 0, 514, 0].iter().flat_map(|&v| put32(v, o)).collect()
}

/// Entropy-encode a 10-bit high plane with the model's inverse: per-block first/main tables,
/// (0, s) tokens only (the caller keeps mid-block diffs nonzero), JPEG magnitudes, carry,
/// parity chains reset at row starts; MSB-first packing, zero padding, ff00 stuffing.
fn encode_hi(hi: &[u16], w: usize, t: &tables::SelTables) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    let push_code = |bits: &mut Vec<bool>, tab: &[(u16, u8, u8, u8)], s: u32| {
        let &(c, l, ..) = tab.iter().find(|e| e.2 == 0 && e.3 as u32 == s).expect("(0, s) code exists");
        bits.extend((0..l).rev().map(|i| (c >> i) & 1 == 1));
    };
    let size_of = |d: i32| if d == 0 { 0 } else { 32 - d.unsigned_abs().leading_zeros() };
    let (mut ce, mut co) = (512i32, 512i32);
    let mut carry = 0i32;
    for (qi, &v) in hi.iter().enumerate() {
        if qi % w == 0 {
            ce = 512;
            co = 512;
        }
        let c = if qi % 2 == 0 { &mut ce } else { &mut co };
        let dd = v as i32 - *c;
        *c = v as i32;
        let (tab, d) = if qi % 64 == 0 {
            let d = dd - carry;
            carry = dd;
            (t.first, d)
        } else {
            assert_ne!(dd, 0, "test images avoid zero diffs mid-block");
            (t.main, dd)
        };
        let s = size_of(d);
        push_code(&mut bits, tab, s);
        if s > 0 {
            let m = if d > 0 { d } else { d + (1 << s) - 1 } as u32;
            bits.extend((0..s).rev().map(|i| (m >> i) & 1 == 1));
        }
    }
    pack_entropy(&bits)
}

/// MSB-first packing into bytes, zero padding in the last byte, then ff00 stuffing.
fn pack_entropy(bits: &[bool]) -> Vec<u8> {
    let mut stuffed = Vec::new();
    for chunk in bits.chunks(8) {
        let mut b = 0u8;
        for (i, &bit) in chunk.iter().enumerate() {
            b |= (bit as u8) << (7 - i);
        }
        stuffed.push(b);
        if b == 0xff {
            stuffed.push(0x00);
        }
    }
    stuffed
}

/// One end-of-block token pair per 64-sample block: the block's first sample takes the carry
/// (zero here), the other 63 keep their chain values. A flat frame like this compresses to a
/// small fraction of its low-bits plane, the shape a lens-cap or blown-out frame has.
fn encode_eob(n: usize, t: &tables::SelTables) -> Vec<u8> {
    let code = |tab: &[(u16, u8, u8, u8)]| {
        let &(c, l, ..) = tab.iter().find(|e| e.2 == 0 && e.3 == 0).expect("an end-of-block code");
        (c, l)
    };
    let (fc, fl) = code(t.first);
    let (mc, ml) = code(t.main);
    let mut bits: Vec<bool> = Vec::new();
    for _ in 0..n.div_ceil(64) {
        for (c, l) in [(fc, fl), (mc, ml)] {
            bits.extend((0..l).rev().map(|i| (c >> i) & 1 == 1));
        }
    }
    pack_entropy(&bits)
}

/// A high plane whose diffs are all nonzero, bounded around 512: values follow
/// `(3 qi) mod 5 + 1`, so same-parity neighbours differ by 1 or −4 (never 0) and
/// row starts sit 1–5 above the reset value.
fn test_hi(n: usize) -> Vec<u16> {
    (0..n).map(|qi| 512 + (qi as u16 * 3) % 5 + 1).collect()
}

fn payload(low: Option<&[u8]>, entropy: &[u8]) -> Vec<u8> {
    let mut p = low.map(|l| l.to_vec()).unwrap_or_default();
    p.extend_from_slice(&[0; 514]);
    p.extend_from_slice(entropy);
    p.extend_from_slice(&[0, 0]);
    p
}

/// The real PowerShot record shape: the payload closes with `ff d9` and a pad byte, not `00 00`.
fn payload_with_trailer(low: Option<&[u8]>, entropy: &[u8]) -> Vec<u8> {
    let mut p = low.map(|l| l.to_vec()).unwrap_or_default();
    p.extend_from_slice(&[0; 514]);
    p.extend_from_slice(entropy);
    p.extend_from_slice(&[0xff, 0xd9, 0x00]);
    p
}

/// A file whose raw record is `payload` verbatim, with `extra` records beside the usual three.
fn file_with_payload(w: u16, h: u16, model: &str, payload: &[u8], extra: &[(u16, Vec<u8>)], o: Order) -> Vec<u8> {
    let mut props = vec![(SENSOR_INFO, sensor_info(w, h, None, o)), (DECODER_TABLE, decoder_table(0, o)), (MAKE_MODEL, make_model(model))];
    props.extend_from_slice(extra);
    ciff(&[(RAW_DATA, payload.to_vec()), (0x300a, heap(&props, o))], o)
}

fn make_model(model: &str) -> Vec<u8> {
    let mut v = b"Canon\0".to_vec();
    v.extend_from_slice(model.as_bytes());
    v.push(0);
    v
}

fn roundtrip_file(w: u16, h: u16, model: &str, low: Option<&[u8]>, hi: &[u16], o: Order) -> Vec<u8> {
    let tables = tables::tables(0).expect("selector 0");
    let entropy = encode_hi(hi, w as usize, &tables);
    // the realistic shape: raw data in the root, properties in a nested heap
    let props = heap(&[(SENSOR_INFO, sensor_info(w, h, None, o)), (DECODER_TABLE, decoder_table(0, o)), (MAKE_MODEL, make_model(model))], o);
    ciff(&[(RAW_DATA, payload(low, &entropy)), (0x300a, props)], o)
}

fn full(bytes: &[u8]) -> Result<Image> {
    decode(bytes, &Limits::default())
}

#[test]
fn probe_needs_the_signature() {
    assert!(probe(b"II\x1a\0\0\0HEAPCCDR\0\0"));
    assert!(probe(b"MM\0\0\0\x1aHEAPCCDR"));
    assert!(!probe(b"II\x1a\0\0\0HEAP"));
    assert!(!probe(b"II*\0\x08\0\0\0"), "a TIFF");
    assert!(!probe(b""));
    assert_eq!(full(b"II*\0\x08\0\0\0"), Err(Error::NotCrw));
    // a signature with no heap behind it is corrupt, not a panic
    assert!(matches!(full(b"II    HEAPCCDR  "), Err(Error::Corrupt(_))));
}

#[test]
fn roundtrips_a_10_bit_frame() {
    let (w, h) = (16u16, 8u16);
    let hi = test_hi(128);
    let f = roundtrip_file(w, h, "Canon EOS D60", None, &hi, Order::Le);
    let img = full(&f).expect("decodes");
    assert_eq!((img.header.width, img.header.height, img.header.bits), (16, 8, 10));
    assert_eq!(img.samples, hi);
    assert_eq!(img.header.cfa, Cfa::Rggb);
    assert_eq!(img.header.selector, 0);
    assert_eq!(img.header.make.as_deref(), Some("Canon"));
    assert_eq!(img.header.model.as_deref(), Some("Canon EOS D60"));
    assert_eq!(img.header.exif_orientation, 1);
    assert_eq!(img.header.capture_unix, None);
    assert_eq!(img.header.wb_multipliers, None);
}

#[test]
fn roundtrips_a_12_bit_frame_with_a_low_bits_plane() {
    let (w, h) = (16u16, 8u16);
    let hi = test_hi(128);
    // low bits all 3: the plane is all 0xff bytes, which trips the presence rule
    let low = vec![0xffu8; 32];
    let f = roundtrip_file(w, h, "Canon EOS D60", Some(&low), &hi, Order::Le);
    let img = full(&f).expect("decodes");
    assert_eq!(img.header.bits, 12);
    let expected: Vec<u16> = hi.iter().map(|&v| (v << 2) | 3).collect();
    assert_eq!(img.samples, expected);
}

/// A low-bits plane without a single 0xff byte (real D60 files whose lowest bit is nearly
/// always 0) used to be taken for "no plane": the plane was read as entropy data and the
/// file came out as 10-bit garbage. The zero header after the plane gives it away.
#[test]
fn a_low_bits_plane_without_ff_bytes_is_still_a_plane() {
    let hi = test_hi(128);
    for byte in [0x22u8, 0x00] {
        let low = vec![byte; 32];
        let f = roundtrip_file(16, 8, "Canon EOS D60", Some(&low), &hi, Order::Le);
        let img = full(&f).expect("decodes");
        assert_eq!(img.header.bits, 12);
        let expected: Vec<u16> = hi.iter().enumerate().map(|(i, &v)| (v << 2) | u16::from((byte >> ((i & 3) * 2)) & 3)).collect();
        assert_eq!(img.samples, expected);
    }
}

#[test]
fn big_endian_ciff_decodes_too() {
    let hi = test_hi(128);
    let f = roundtrip_file(16, 8, "Canon EOS D60", None, &hi, Order::Be);
    let img = full(&f).expect("decodes");
    assert_eq!(img.samples, hi);
}

#[test]
fn header_reads_geometry_without_samples() {
    let hi = test_hi(128);
    let f = roundtrip_file(16, 8, "Canon EOS D60", None, &hi, Order::Le);
    let hdr = header(&f, &Limits::default()).expect("header decodes");
    assert_eq!((hdr.width, hdr.height, hdr.bits), (16, 8, 10));
    assert_eq!(hdr, full(&f).expect("decodes").header, "the header is the same either way");
}

#[test]
fn limits_cap_the_sample_count() {
    let hi = test_hi(128);
    let f = roundtrip_file(16, 8, "Canon EOS D60", None, &hi, Order::Le);
    let mut limits = Limits { max_samples: 127 };
    assert_eq!(decode(&f, &limits), Err(Error::LimitExceeded("image too large")));
    assert!(header(&f, &limits).is_err());
    limits.max_samples = 128;
    assert!(decode(&f, &limits).is_ok());
}

/// A header declaring 2²⁴ samples over a 128-sample stream is refused by the default limits
/// before any sample-sized buffer exists: the cap must stay above every real frame (7.4 million)
/// and below anything that would let a crafted file demand gigabytes.
#[test]
fn default_limits_refuse_a_crafted_giant_frame() {
    let hi = test_hi(128);
    let f = roundtrip_file(4096, 4096, "Canon EOS D60", None, &hi, Order::Le);
    assert_eq!(decode(&f, &Limits::default()), Err(Error::LimitExceeded("image too large")));
    assert_eq!(header(&f, &Limits::default()), Err(Error::LimitExceeded("image too large")));
    assert!(Limits::default().max_samples >= 7_407_040, "the cap must admit the largest real frame");
}

#[test]
fn g1_is_gbrg() {
    let hi = test_hi(128);
    let f = roundtrip_file(16, 8, "Canon PowerShot G1", None, &hi, Order::Le);
    let img = full(&f).expect("decodes");
    assert_eq!(img.header.cfa, Cfa::Gbrg);
    assert_eq!(img.header.cfa.pattern(), [1, 2, 0, 1]);
    assert_eq!(Cfa::Rggb.pattern(), [0, 1, 1, 2]);
}

#[test]
fn g5_is_refused_with_its_anomaly_note() {
    let hi = test_hi(128);
    let f = roundtrip_file(16, 8, "Canon PowerShot G5", None, &hi, Order::Le);
    let Err(Error::Unsupported(why)) = full(&f) else { panic!("expected Unsupported") };
    assert!(why.contains("G5"), "{why}");
    assert!(matches!(header(&f, &Limits::default()), Err(Error::Unsupported(_))));
}

/// A D60 at ISO 100 clips near 4000, and a photo with a small blown area has no plateau:
/// the white level must still come down to where the data stops, not stay at 4095. A few
/// hot pixels, or a photo that never gets near the top, leave the full range.
#[test]
fn white_level_follows_a_small_clipped_area() {
    let mut data: Vec<u16> = (0..200_000).map(|i| 300 + (i % 3000) as u16).collect();
    assert_eq!(white_level(&data, 12), 4095.0, "nothing near the top");
    for v in data.iter_mut().take(100) {
        *v = 4002;
    }
    assert_eq!(white_level(&data, 12), (4002 - 15) as f32, "100 samples at the clip level");
    let mut hot: Vec<u16> = (0..200_000).map(|i| 300 + (i % 3000) as u16).collect();
    for v in hot.iter_mut().take(10) {
        *v = 4002;
    }
    assert_eq!(white_level(&hot, 12), 4095.0, "ten hot pixels are not a clip level");
}

#[test]
fn saturation_plateau_is_found_with_strays_above_it() {
    let mut d: Vec<u16> = (0..100_000).map(|i| (i % 10000) as u16).collect();
    assert_eq!(saturation_plateau(&d, 14), 16383.0);
    d.extend(std::iter::repeat_n(15000u16, 1000));
    let w = saturation_plateau(&d, 14);
    assert!((14900.0..=15000.0).contains(&w), "{w}");
    assert_eq!(saturation_plateau(&[], 12), 4095.0);
    let mut d: Vec<u16> = (0..100_000).map(|i| (2048 + i % 10000) as u16).collect();
    d.extend(std::iter::repeat_n(13583u16, 5000));
    d.extend([13700, 13900, 14100, 14426]);
    let w = saturation_plateau(&d, 14);
    assert!((13500.0..=13583.0).contains(&w), "{w}");
}

/// The D60 layout: sensor rows clamped to 0 above the image, dark columns left of it. The
/// clamped rows must not count towards the black level.
#[test]
fn black_level_ignores_clamped_rows() {
    let (w, h) = (64usize, 40usize);
    let active = Rect::new(24, 16, 40, 24);
    let mut data = vec![900u16; w * h];
    for y in 0..h {
        for x in 0..w {
            data[y * w + x] = if y < 7 {
                0
            } else if x < 24 {
                128
            } else {
                900
            };
        }
    }
    let black = black_from_borders(&data, w, h, active);
    assert!(black.values.iter().all(|v| (v - 128.0).abs() < 0.5), "{:?}", black.values);
    assert_eq!(black.at(active, 24, 16), black.values[0]);
    assert_eq!(black.at(active, 25, 16), black.values[1]);
    assert_eq!(black.at(active, 24, 17), black.values[2]);
    assert_eq!(black.at(active, 23, 15), black.values[3], "sites are counted from the active origin, modulo 2");
    assert_eq!(black_from_borders(&data, w, h, Rect::new(0, 0, w, h)), BlackLevel::uniform(0.0), "no border, no level");
}

/// The frozen stream-end rule: a truncated entropy stream leaves the remaining samples at
/// their chain values instead of failing the file.
#[test]
fn truncated_stream_keeps_chain_values() {
    let (w, h) = (16u16, 8u16);
    let hi = test_hi(128);
    let tables = tables::tables(0).expect("selector 0");
    let entropy = encode_hi(&hi, 16, &tables);
    let cut = entropy.len() / 2;
    let props = heap(
        &[(SENSOR_INFO, sensor_info(w, h, None, Order::Le)), (DECODER_TABLE, decoder_table(0, Order::Le)), (MAKE_MODEL, make_model("Canon EOS D60"))],
        Order::Le,
    );
    let f = ciff(&[(RAW_DATA, payload(None, &entropy[..cut])), (0x300a, props)], Order::Le);
    let img = full(&f).expect("truncated stream still decodes");
    assert_eq!(img.samples.len(), 128);
    assert_eq!(&img.samples[..32], &hi[..32], "the leading samples decode exactly");
}

/// Every record the decoder needs missing, bogus selectors, dimensions and offsets: errors,
/// never panics; and every truncation of a valid file survives.
#[test]
fn hostile_inputs_error_cleanly() {
    let o = Order::Le;
    let hi = test_hi(128);
    let good = roundtrip_file(16, 8, "Canon EOS D60", None, &hi, o);
    for n in 0..good.len() {
        let _ = full(&good[..n]);
        let _ = header(&good[..n], &Limits::default());
    }
    // single-byte mutations over the whole file
    for i in 0..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0xa5;
        let _ = full(&bad);
    }
    let props_no_sensor = heap(&[(DECODER_TABLE, decoder_table(0, o))], o);
    let f = ciff(&[(RAW_DATA, vec![0; 600]), (0x300a, props_no_sensor)], o);
    assert!(matches!(full(&f), Err(Error::Corrupt(_))));
    let props_bad_sel = heap(&[(SENSOR_INFO, sensor_info(16, 8, None, o)), (DECODER_TABLE, decoder_table(7, o))], o);
    let f = ciff(&[(RAW_DATA, vec![0; 600]), (0x300a, props_bad_sel)], o);
    assert!(matches!(full(&f), Err(Error::Unsupported(_))));
    let props_zero = heap(&[(SENSOR_INFO, sensor_info(0, 0, None, o)), (DECODER_TABLE, decoder_table(0, o))], o);
    let f = ciff(&[(RAW_DATA, vec![0; 600]), (0x300a, props_zero)], o);
    assert!(full(&f).is_err());
    // a heap that nests into itself must hit the depth bound, not recurse forever
    let mut self_heap = heap(&[(0x300a, vec![0; 10])], o);
    let len = self_heap.len() as u32;
    // point the nested record at the whole block
    let tbl = u32::from_le_bytes(self_heap[len as usize - 4..].try_into().unwrap()) as usize;
    self_heap[tbl + 4..tbl + 8].copy_from_slice(&len.to_le_bytes());
    self_heap[tbl + 8..tbl + 12].copy_from_slice(&0u32.to_le_bytes());
    let mut f = ciff(&[], o);
    f.truncate(26);
    f.extend_from_slice(&self_heap);
    assert!(full(&f).is_err());
}

/// The frozen tables are prefix-free within each table, so the 16-bit lookup is unambiguous.
#[test]
fn frozen_tables_are_prefix_free() {
    for sel in 0..3 {
        let t = tables::tables(sel).expect("selector");
        for (name, tab) in [("first", t.first), ("main", t.main)] {
            let mut lut = vec![false; 1 << 16];
            for &(code, len, ..) in tab {
                assert!((1..=16).contains(&len), "sel {sel} {name}: code length {len}");
                let base = (code as usize) << (16 - len);
                for cell in lut.iter_mut().skip(base).take(1 << (16 - len)) {
                    assert!(!*cell, "sel {sel} {name}: overlapping codes");
                    *cell = true;
                }
            }
        }
    }
    assert!(tables::tables(3).is_none());
}

/// Every second table codes all 162 symbols: (r, s) for r 0..=15 and s 1..=10, end of block
/// and (15, 0). Selectors 0 and 1 lacked (7..=14, 10) and (14, 10), learned from too few
/// files, and 223 of 1,516 real D60 files failed with "undecodable entropy code".
#[test]
fn second_tables_code_every_symbol() {
    for sel in 0..3 {
        let t = tables::tables(sel).expect("selector");
        let have: std::collections::BTreeSet<(u8, u8)> = t.main.iter().map(|e| (e.2, e.3)).collect();
        let mut want: std::collections::BTreeSet<(u8, u8)> = (0..16).flat_map(|r| (1..=10).map(move |s| (r, s))).collect();
        want.extend([(0, 0), (15, 0)]);
        assert_eq!(have, want, "selector {sel}");
        assert_eq!(t.main.len(), 162, "selector {sel}: one code per symbol");
    }
}

#[test]
fn white_balance_comes_from_the_shot_info_slot() {
    let o = Order::Le;
    let hi = test_hi(128);
    // D60-shaped table: 7 slots + black levels; Daylight slot (1) = 1800, 800, 800, 960
    let mut cb: Vec<u16> = vec![66];
    cb.extend([2000u16, 760, 760, 830]);
    cb.extend([1800u16, 800, 800, 960]);
    for _ in 0..5 {
        cb.extend([1500u16, 900, 900, 1500]);
    }
    cb.extend([126u16, 127, 126, 126]);
    let cb: Vec<u8> = cb.iter().flat_map(|&v| put16(v, o)).collect();
    let mut shot: Vec<u16> = vec![54; 27];
    shot[7] = 1; // Daylight
    let shot: Vec<u8> = shot.iter().flat_map(|&v| put16(v, o)).collect();
    let tables = tables::tables(0).expect("selector 0");
    let props = heap(
        &[
            (SENSOR_INFO, sensor_info(16, 8, None, o)),
            (DECODER_TABLE, decoder_table(0, o)),
            (MAKE_MODEL, make_model("Canon EOS D60")),
            (COLOR_BALANCE, cb),
            (SHOT_INFO, shot),
        ],
        o,
    );
    let f = ciff(&[(RAW_DATA, payload(None, &encode_hi(&hi, 16, &tables))), (0x300a, props)], o);
    let img = full(&f).expect("decodes");
    let wb = img.header.wb_multipliers.expect("wb");
    assert!((wb[0] - 2.25).abs() < 1e-3 && wb[1] == 1.0 && (wb[2] - 1.2).abs() < 1e-3, "{wb:?}");
}

#[test]
fn rotation_maps_to_exif_orientation_and_the_time_stamp_is_unix_seconds() {
    let o = Order::Le;
    let hi = test_hi(128);
    let tables = tables::tables(0).expect("selector 0");
    // The record holds a signed degree count, so 270 may also be written as −90.
    for (deg, want) in [(0u32, 1u8), (90, 6), (180, 3), (270, 8), (-90i32 as u32, 8), (-270i32 as u32, 6)] {
        let image_info: Vec<u8> = [3072u32, 2048, 0, deg, 12, 12, 0].iter().flat_map(|&v| put32(v, o)).collect();
        let stamp: Vec<u8> = [1083410638u32, 0, 0].iter().flat_map(|&v| put32(v, o)).collect();
        let extra = [(IMAGE_INFO, image_info), (TIME_STAMP, stamp)];
        let f = file_with_payload(16, 8, "Canon EOS 300D DIGITAL", &payload(None, &encode_hi(&hi, 16, &tables)), &extra, o);
        let img = full(&f).expect("decodes");
        assert_eq!(img.header.exif_orientation, want, "rotation {deg}");
        assert_eq!(img.header.capture_unix, Some(1083410638));
    }
}

#[test]
fn sensor_info_crop_is_the_active_area() {
    let o = Order::Le;
    let hi = test_hi(128);
    let tables = tables::tables(0).expect("selector 0");
    let with = |crop| {
        let props =
            heap(&[(SENSOR_INFO, sensor_info(16, 8, crop, o)), (DECODER_TABLE, decoder_table(0, o)), (MAKE_MODEL, make_model("Canon EOS D60"))], o);
        let f = ciff(&[(RAW_DATA, payload(None, &encode_hi(&hi, 16, &tables))), (0x300a, props)], o);
        full(&f).expect("decodes").header.active_area
    };
    assert_eq!(with(Some((4, 1, 15, 7))), Rect::new(4, 1, 12, 7), "inclusive left, top, right, bottom");
    assert_eq!(with(None), Rect::new(0, 0, 16, 8), "no crop: the whole sensor");
    assert_eq!(with(Some((4, 1, 16, 7))), Rect::new(0, 0, 16, 8), "a crop beyond the sensor is ignored");
    assert_eq!(with(Some((5, 1, 4, 7))), Rect::new(0, 0, 16, 8), "an inverted crop is ignored");
}

/// A block whose every record is a sub-heap pointing back at the block itself: depth alone
/// would allow N^MAX_DEPTH visits (here 64^17), the record budget ends the walk promptly.
#[test]
fn self_referencing_sub_heaps_terminate() {
    const N: usize = 64;
    for o in [Order::Le, Order::Be] {
        let len = (2 + N * 10 + 4) as u32;
        let mut f = Vec::new();
        f.extend_from_slice(match o {
            Order::Le => b"II",
            Order::Be => b"MM",
        });
        f.extend_from_slice(&put32(26, o));
        f.extend_from_slice(b"HEAPCCDR");
        f.extend_from_slice(&put32(0x0001_0002, o));
        f.extend_from_slice(&[0; 8]);
        f.extend_from_slice(&put16(N as u16, o)); // record table at block offset 0
        for _ in 0..N {
            f.extend_from_slice(&put16(5 << 11 | 1, o)); // storage class 5: sub-heap
            f.extend_from_slice(&put32(len, o)); // ... covering the whole parent block
            f.extend_from_slice(&put32(0, o));
        }
        f.extend_from_slice(&put32(0, o)); // trailing offset-table pointer
        let started = std::time::Instant::now();
        assert!(matches!(header(&f, &Limits::default()), Err(Error::Corrupt(_))));
        assert!(matches!(full(&f), Err(Error::Corrupt(_))));
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "walk did not stay within budget");
        let mut rec = BTreeMap::new();
        let mut budget = MAX_RECORDS;
        walk(&f, 26, len as usize, o, 0, &mut budget, &mut rec);
        assert_eq!(budget, 0);
        assert!(rec.is_empty());
    }
}

/// A no-plane G1 frame that compresses well closes with the record's real `ff d9 00` trailer and
/// is still shorter than plane + header — the shape of a lens-cap or blown-out frame. Rule (a)
/// read that `ff d9` as a plane byte, reported 12 bits and then refused the file.
#[test]
fn a_short_powershot_payload_closing_with_the_trailer_has_no_plane() {
    let o = Order::Le;
    let (w, h) = (64u16, 64u16);
    let n = w as usize * h as usize;
    let tables = tables::tables(0).expect("selector 0");
    let entropy = encode_eob(n, &tables);
    let p = payload_with_trailer(None, &entropy);
    let span = n.div_ceil(4) + 514;
    assert!(p.len() < span, "the payload must be shorter than plane + header: {} vs {span}", p.len());
    assert!(p.windows(2).any(|q| q[0] == 0xff && q[1] != 0x00), "the trailer is the unstuffed ff pair");

    let f = file_with_payload(w, h, "Canon PowerShot G1", &p, &[], o);
    let hdr = header(&f, &Limits::default()).expect("header");
    assert_eq!(hdr.bits, 10, "the trailer is not a low-bits plane");
    // Every block is one end-of-block token, so the chain values stand (512 at each row start):
    // the frame decodes instead of being refused for a plane it does not have.
    let img = full(&f).expect("decodes");
    assert_eq!((img.header.bits, img.samples.iter().filter(|&&v| v == 512).count()), (10, n));

    // The same frame without the trailer was already read as 10-bit.
    let f = file_with_payload(w, h, "Canon PowerShot G1", &payload(None, &entropy), &[], o);
    assert_eq!(header(&f, &Limits::default()).expect("header").bits, 10);
}

/// An end-of-block token at the head of a block emits the carry and leaves the other 63 samples
/// at their chain values: the runs the sample hashes cover, on a file the unit tests can read.
#[test]
fn end_of_block_keeps_the_chain_values() {
    let o = Order::Le;
    let (w, h) = (64u16, 64u16);
    let n = w as usize * h as usize;
    let tables = tables::tables(0).expect("selector 0");
    let f = file_with_payload(w, h, "Canon EOS D30", &payload(None, &encode_eob(n, &tables)), &[], o);
    let img = full(&f).expect("decodes");
    assert_eq!(img.samples.len(), n);
    assert!(img.samples.iter().all(|&v| v == 512), "every sample keeps its row's chain value");
}

/// Zero sensor dimensions are damage, not a request for too much memory.
#[test]
fn zero_sensor_dimensions_are_corrupt_not_a_limit() {
    let f = roundtrip_file(0, 0, "Canon EOS D60", None, &[], Order::Le);
    assert!(matches!(full(&f), Err(Error::Corrupt(_))));
    assert!(matches!(header(&f, &Limits::default()), Err(Error::Corrupt(_))));
}

/// A hostile stream can run the chain past the bit depth (here a "10-bit" body writing 1535).
/// `samples` is documented as `bits` deep, so the surplus never reaches a `1 << bits` LUT.
#[test]
fn samples_are_clamped_to_the_declared_depth() {
    let f = roundtrip_file(2, 1, "Canon PowerShot G1", None, &[512, 1535], Order::Le);
    let img = full(&f).expect("decodes");
    assert_eq!((img.header.bits, img.samples.as_slice()), (10, &[512u16, 1023][..]));
    // the 12-bit merge of the same chain cannot leave 0..=4095 either
    let low = vec![0xffu8; 1];
    let f = roundtrip_file(2, 1, "Canon EOS D60", Some(&low), &[512, 1535], Order::Le);
    let img = full(&f).expect("decodes");
    assert_eq!(img.header.bits, 12);
    assert!(img.samples.iter().all(|&v| v <= 4095), "{:?}", img.samples);
}

/// The two ColorBalance layouts and every Canon setting that picks a slot. Every corpus file is
/// set to Auto, so a wrong arm would still pass on the samples alone.
#[test]
fn white_balance_slot_arms_follow_the_camera_setting() {
    let o = Order::Le;
    // Quadruple k is (100+k, 200, 200, 300+k): the multipliers name the slot chosen, and the last
    // quadruple is black levels, which is never a slot.
    let wb = |usable: u16, setting: u16| {
        let mut vals = vec![(usable + 1) * 8];
        for k in 0..=usable {
            vals.extend([100 + k, 200, 200, 300 + k]);
        }
        let mut bytes: Vec<u8> = vals.iter().flat_map(|&v| put16(v, o)).collect();
        let cb_len = bytes.len();
        let si_off = bytes.len();
        let mut shot = [0u16; 8];
        shot[7] = setting;
        bytes.extend(shot.iter().flat_map(|&v| put16(v, o)));
        let mut rec = BTreeMap::new();
        rec.insert(COLOR_BALANCE, (0usize, cb_len));
        rec.insert(SHOT_INFO, (si_off, 16usize));
        wb_from_color_balance(&bytes, &rec, o)
    };
    let expect = |k: u16| Some([(100 + k) as f32 / 200.0, 1.0, (300 + k) as f32 / 200.0]);
    // 10D/300D: Auto, Daylight, Shade, Cloudy, Tungsten, Fluorescent, Flash, Custom, Kelvin
    for (setting, slot) in [(0u16, 0u16), (1, 1), (8, 2), (2, 3), (3, 4), (4, 5), (5, 6), (6, 7), (9, 8), (7, 0)] {
        assert_eq!(wb(9, setting), expect(slot), "nine-slot setting {setting}");
    }
    // D60: Auto, Daylight, Cloudy, Tungsten, Fluorescent, Flash, Custom. Shade and Kelvin have no
    // slot of their own and fall back to Auto.
    for (setting, slot) in [(0u16, 0u16), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6), (8, 0), (9, 0)] {
        assert_eq!(wb(7, setting), expect(slot), "seven-slot setting {setting}");
    }
}

#[test]
fn errors_display_their_cause() {
    assert_eq!(Error::NotCrw.to_string(), "not a Canon CRW (CIFF) file");
    assert_eq!(corrupt("no raw data record").to_string(), "CRW: no raw data record");
    assert_eq!(Error::Unsupported("decoder table 7".into()).to_string(), "unsupported CRW: decoder table 7");
    assert_eq!(Error::LimitExceeded("image too large").to_string(), "CRW exceeds a limit: image too large");
}
