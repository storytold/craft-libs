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
