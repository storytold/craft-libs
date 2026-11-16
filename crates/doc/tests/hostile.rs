//! Truncated, oversized and hostile input: the reader returns an error within its `Limits` and
//! never panics.

mod common;

use common::*;
use craft_doc::{Error, Limits, info, outline, renditions, sniff};
use proptest::prelude::*;

fn small() -> Limits {
    let mut l = Limits::default();
    l.max_json = 64 << 10;
    l.max_entry = 64 << 10;
    l.max_total = 256 << 10;
    l
}

fn limit_exceeded<T: std::fmt::Debug>(r: Result<T, Error>) -> bool {
    matches!(r, Err(Error::LimitExceeded(_)))
}

/// Every call on `bytes`; none may panic.
fn exercise(bytes: &[u8], limits: &Limits) {
    let _ = sniff(bytes);
    let _ = info(bytes, limits);
    let _ = renditions(bytes, limits);
    let _ = outline(bytes, limits);
}

fn samples() -> Vec<Vec<u8>> {
    let members = format!(r#""preview":{},"pdf":{}"#, inline("image/png", &png(2, 2, 3)), blob("application/pdf", 0, PDF.len()));
    let mut v4 = vectorcraft_json(4, &members).into_bytes();
    v4.extend(BLOB_MAGIC);
    v4.extend(PDF);
    vec![gzip(&v4), v4, pcraft(&png(3, 3, 1), &png(6, 6, 2)), ecproj(2).into_bytes()]
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    for s in samples() {
        for n in 0..s.len() {
            exercise(&s[..n], &small());
        }
    }
}

#[test]
fn a_blob_longer_than_max_entry_is_refused_before_it_is_copied() {
    let mut bytes = vectorcraft_json(4, &format!(r#""pdf":{}"#, blob("application/pdf", 0, 1 << 40))).into_bytes();
    bytes.extend(BLOB_MAGIC);
    bytes.extend(PDF);
    assert!(limit_exceeded(renditions(&bytes, &small())));
}

#[test]
fn a_blob_past_the_end_of_the_file_is_malformed() {
    for at in [[PDF.len() as u64, 1], [u64::MAX, 1], [1, PDF.len() as u64]] {
        let members = format!(r#""pdf":{{"mime":"application/pdf","blob":[{},{}]}}"#, at[0], at[1]);
        let mut bytes = vectorcraft_json(4, &members).into_bytes();
        bytes.extend(BLOB_MAGIC);
        bytes.extend(PDF);
        assert!(matches!(renditions(&bytes, &small()), Err(Error::Malformed(_))), "{at:?}");
    }
}

#[test]
fn base64_past_max_entry_is_refused() {
    let big = vec![1u8; 100 << 10];
    let mut limits = small();
    limits.max_json = 1 << 20;
    let bytes = vectorcraft_json(3, &format!(r#""pdf":{}"#, inline("application/pdf", &big))).into_bytes();
    assert!(limit_exceeded(renditions(&bytes, &limits)));
    limits.max_entry = 1 << 20;
    assert_eq!(renditions(&bytes, &limits).unwrap()[0].bytes, big);
    let bad = vectorcraft_json(3, r#""pdf":{"mime":"application/pdf","data":"QU*D"}"#).into_bytes();
    assert!(matches!(renditions(&bad, &limits), Err(Error::Malformed(_))));
}

#[test]
fn a_gzip_bomb_stops_at_the_limits() {
    // 8 MiB of padding in the JSON, compressed to a few KiB.
    let pad = "x".repeat(8 << 20);
    let json = vectorcraft_json(4, &format!(r#""pad":"{pad}""#));
    let gz = gzip(json.as_bytes());
    assert!(gz.len() < 64 << 10);
    assert!(limit_exceeded(info(&gz, &small())));
    assert!(limit_exceeded(outline(&gz, &small())));
    assert!(limit_exceeded(renditions(&gz, &small())));
    // The data after the JSON counts against max_total as far as it is read.
    let mut tail = vectorcraft_json(4, &format!(r#""pdf":{}"#, blob("application/pdf", 8 << 20, 1))).into_bytes();
    tail.extend(BLOB_MAGIC);
    tail.extend(vec![0u8; (8 << 20) + 1]);
    let gz = gzip(&tail);
    assert!(limit_exceeded(renditions(&gz, &small())));
    let mut more = small();
    more.max_total = 9 << 20;
    assert_eq!(renditions(&gz, &more).unwrap()[0].bytes, [0]);
    // info stops at the marker and reads the JSON only.
    assert_eq!(info(&gz, &small()).unwrap().version, 4);
}

#[test]
fn a_compressed_member_past_max_entry_is_refused_before_the_tail_is_inflated() {
    // A member declares 8 MiB; the data after the JSON is 8 MiB that compress to a few KiB.
    let mut file = vectorcraft_json(4, &format!(r#""pdf":{}"#, blob("application/pdf", 0, 8 << 20))).into_bytes();
    file.extend(BLOB_MAGIC);
    file.extend(vec![1u8; 8 << 20]);
    let gz = gzip(&file);
    let mut limits = small();
    // Enough for the tail, not for the member.
    limits.max_total = 64 << 20;
    match renditions(&gz, &limits) {
        Err(Error::LimitExceeded(m)) => assert!(m.contains("max_entry"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_compressed_member_whose_offset_passes_max_total_is_refused_before_the_tail_is_inflated() {
    // A 4-byte preview stated 200 MiB past the marker, in a file whose data after the JSON is
    // 1 MiB.
    let mut file = vectorcraft_json(4, &format!(r#""preview":{}"#, blob("image/png", 200 << 20, 4))).into_bytes();
    file.extend(BLOB_MAGIC);
    file.extend(vec![0u8; 1 << 20]);
    let gz = gzip(&file);
    let mut limits = small();
    limits.max_total = 64 << 20;
    match renditions(&gz, &limits) {
        Err(Error::LimitExceeded(m)) => assert!(m.contains("max_total"), "{m}"),
        other => panic!("{other:?}"),
    }
    // Within max_total, the stream is read as far as it goes, and it ends first.
    limits.max_total = 256 << 20;
    assert!(matches!(renditions(&gz, &limits), Err(Error::Malformed(_))));
}

#[test]
fn a_nul_outside_the_marker_is_malformed_without_reading_on() {
    // A NUL where the marker is not, then 8 MiB that compress to almost nothing.
    let mut file = vectorcraft_json(4, "").into_bytes();
    file.extend(b"\n\0NOTBLOBS\0");
    file.extend(vec![0u8; 8 << 20]);
    let mut limits = small();
    limits.max_total = 1 << 20;
    for bytes in [gzip(&file), file] {
        assert!(matches!(info(&bytes, &limits), Err(Error::Malformed(_))));
        assert!(matches!(renditions(&bytes, &limits), Err(Error::Malformed(_))));
        assert!(matches!(outline(&bytes, &limits), Err(Error::Malformed(_))));
    }
    // A NUL as the very first byte of the decompressed stream.
    let mut first = b"\0".to_vec();
    first.extend(vectorcraft_json(4, "").as_bytes());
    exercise(&gzip(&first), &limits);
}

#[test]
fn a_manifest_past_max_entry_is_refused() {
    let pc = pcraft(&png(1, 1, 0), &png(1, 1, 0));
    let mut limits = small();
    limits.max_entry = 64;
    assert!(limit_exceeded(info(&pc, &limits)));
}

#[test]
fn an_uncompressed_document_past_max_json_is_refused() {
    let json = vectorcraft_json(4, &format!(r#""pad":"{}""#, "x".repeat(100 << 10)));
    assert!(limit_exceeded(info(json.as_bytes(), &small())));
    let ec = ecproj(2).replacen(r#""settings": {}"#, &format!(r#""settings": {{"pad":"{}"}}"#, "x".repeat(100 << 10)), 1);
    assert!(limit_exceeded(outline(ec.as_bytes(), &small())));
}

#[test]
fn json_nested_past_max_depth_is_refused() {
    let deep = format!("{}{}", "[".repeat(2000), "]".repeat(2000));
    let vc = vectorcraft_json(4, &format!(r#""deep":{deep}"#));
    assert!(limit_exceeded(outline(vc.as_bytes(), &Limits::default())));
    let ec = ecproj(2).replacen(r#""settings": {}"#, &format!(r#""settings": {deep}"#), 1);
    assert!(limit_exceeded(info(ec.as_bytes(), &Limits::default())));
    let pc = zip(&[store("manifest.json", pcraft_manifest(1, &deep, "").as_bytes())]);
    assert!(limit_exceeded(outline(&pc, &Limits::default())));
    // At the limit (the file's object is one level), the document still reads.
    let n = Limits::default().max_depth as usize - 1;
    let at_limit = vectorcraft_json(4, &format!(r#""deep":{}{}"#, "[".repeat(n), "]".repeat(n)));
    assert_eq!(outline(at_limit.as_bytes(), &Limits::default()).unwrap().pages.len(), 2);
    let past = vectorcraft_json(4, &format!(r#""deep":{}{}"#, "[".repeat(n + 1), "]".repeat(n + 1)));
    assert!(limit_exceeded(outline(past.as_bytes(), &Limits::default())));
    // A raised limit stops at MAX_DEPTH: 2000 nested groups are refused, not parsed.
    let mut raised = Limits::default();
    raised.max_depth = u32::MAX;
    assert!(limit_exceeded(outline(past.as_bytes(), &raised)));
    let mut layer = pcraft_layer("Leaf", true, "Normal", 1.0, raster());
    for _ in 0..2000 {
        layer = pcraft_layer("G", true, "Normal", 1.0, &format!(r#"{{"kind":"group","expanded":false,"children":[{layer}]}}"#));
    }
    let pc = zip(&[deflated("manifest.json", pcraft_manifest(1, &format!("[{layer}]"), "").as_bytes())]);
    raised.max_json = 1 << 30;
    raised.max_entry = 1 << 30;
    assert!(limit_exceeded(outline(&pc, &raised)));
}

#[test]
fn lists_past_max_items_are_refused() {
    let with_max = |n: u64| {
        let mut l = Limits::default();
        l.max_items = n;
        l
    };
    // Four project items and two layers; layers of `{}` count as well.
    let ec = ecproj(2);
    assert_eq!(outline(ec.as_bytes(), &with_max(6)).unwrap().pages.len(), 2);
    assert!(limit_exceeded(outline(ec.as_bytes(), &with_max(5))));
    let empty = ec.replacen(r#""layers": []"#, &format!(r#""layers": [{}{{}}]"#, "{},".repeat(9)), 1);
    assert_eq!(outline(empty.as_bytes(), &with_max(16)).unwrap().pages[0].layers.len(), 10);
    assert!(limit_exceeded(outline(empty.as_bytes(), &with_max(15))));
    let null = ec.replacen(r#""layers": []"#, r#""layers": null"#, 1);
    assert!(outline(null.as_bytes(), &with_max(6)).unwrap().pages[0].layers.is_empty());
    // Two artboards and two layers.
    let vc = vectorcraft_json(4, "");
    assert_eq!(outline(vc.as_bytes(), &with_max(4)).unwrap().layers.len(), 2);
    assert!(limit_exceeded(outline(vc.as_bytes(), &with_max(3))));
    // An element past the limit is refused before it is read.
    let bad = vc.replacen(r#""artboards":["#, r#""artboards":[{"rect":0},"#, 1);
    assert!(matches!(outline(bad.as_bytes(), &with_max(4)), Err(Error::Malformed(_))));
    assert!(limit_exceeded(outline(bad.as_bytes(), &with_max(0))));
    // A group and its one layer.
    let leaf = pcraft_layer("Leaf", true, "Normal", 1.0, raster());
    let group = pcraft_layer("G", true, "Normal", 1.0, &format!(r#"{{"kind":"group","expanded":false,"children":[{leaf}]}}"#));
    let pc = zip(&[store("manifest.json", pcraft_manifest(1, &format!("[{group}]"), "").as_bytes())]);
    assert_eq!(outline(&pc, &with_max(2)).unwrap().layers[0].children.len(), 1);
    assert!(limit_exceeded(outline(&pc, &with_max(1))));
}

#[test]
fn only_the_json_of_a_compressed_file_is_counted_when_no_member_follows_it() {
    let mut file = vectorcraft_json(4, "").into_bytes();
    let json_len = file.len() as u64;
    file.extend(BLOB_MAGIC);
    file.extend((0..1u32 << 20).map(|i| (i % 251) as u8));
    let gz = gzip(&file);
    let mut limits = small();
    limits.max_total = json_len + BLOB_MAGIC.len() as u64;
    assert_eq!(info(&gz, &limits).unwrap().version, 4);
    assert_eq!(outline(&gz, &limits).unwrap().pages.len(), 2);
    assert_eq!(renditions(&gz, &limits).unwrap(), vec![]);
}

#[test]
fn a_compressed_member_is_counted_against_max_total_once() {
    // A 40 KiB PDF after the JSON, with max_total 64 KiB: it reads compressed as it does
    // uncompressed.
    let pdf: Vec<u8> = (0..40u32 << 10).map(|i| (i % 251) as u8).collect();
    let mut file = vectorcraft_json(4, &format!(r#""pdf":{}"#, blob("application/pdf", 0, pdf.len()))).into_bytes();
    file.extend(BLOB_MAGIC);
    file.extend(&pdf);
    let mut limits = small();
    limits.max_total = 64 << 10;
    for bytes in [file.clone(), gzip(&file)] {
        assert_eq!(renditions(&bytes, &limits).unwrap()[0].bytes, pdf);
    }
    // Without room for the JSON and the PDF, the compressed file is refused.
    limits.max_total = pdf.len() as u64;
    assert!(limit_exceeded(renditions(&gzip(&file), &limits)));
    // A preview naming the same bytes is a second copy, counted too.
    let mut twice = vectorcraft_json(4, &format!(r#""preview":{0},"pdf":{0}"#, blob("image/png", 0, pdf.len()))).into_bytes();
    twice.extend(BLOB_MAGIC);
    twice.extend(&pdf);
    for bytes in [twice.clone(), gzip(&twice)] {
        limits.max_total = 64 << 10;
        assert!(limit_exceeded(renditions(&bytes, &limits)));
        limits.max_total = 128 << 10;
        assert_eq!(renditions(&bytes, &limits).unwrap().len(), 2);
    }
}

#[test]
fn a_large_array_in_place_of_the_generator_or_a_preview_is_skipped() {
    // A million elements, where a buffered read would keep each one.
    let zeros = format!("[{}0]", "0,".repeat(1 << 20));
    let members = format!(r#""generator":{zeros},"preview":{{"mime":{zeros}}},"pdf":[{zeros}]"#);
    let vc = vectorcraft_json(4, &members).replacen(r#""generator":"VectorCraft 0.1.0","#, "", 1);
    assert_eq!(info(vc.as_bytes(), &Limits::default()).unwrap().generator, None);
    assert_eq!(renditions(vc.as_bytes(), &Limits::default()).unwrap(), vec![]);
    let manifest = pcraft_manifest(1, "[]", "").replacen(r#""generator":"photocraft-format 0.1.0""#, &format!(r#""generator":{zeros}"#), 1);
    let pc = zip(&[store("manifest.json", manifest.as_bytes())]);
    assert_eq!(info(&pc, &Limits::default()).unwrap().generator, None);
}

#[test]
fn a_zip_entry_past_max_entry_is_refused_before_it_is_inflated() {
    let big = vec![0u8; 1 << 20];
    let pc = zip(&[
        store("manifest.json", pcraft_manifest(1, "[]", r#","composite":"composite/preview.png""#).as_bytes()),
        deflated("composite/preview.png", &big),
    ]);
    assert!(limit_exceeded(renditions(&pc, &small())));
    let mut limits = small();
    limits.max_entry = 2 << 20;
    limits.max_total = 4 << 20;
    assert_eq!(renditions(&pc, &limits).unwrap()[0].bytes.len(), big.len());
}

/// Where the central directory entry of the `index`th entry starts.
fn central(zip: &[u8], index: usize) -> usize {
    zip.windows(4).enumerate().filter(|(_, w)| *w == [0x50, 0x4b, 0x01, 0x02]).nth(index).unwrap().0
}

#[test]
fn a_deflate_entry_that_inflates_past_its_size_is_malformed() {
    let manifest = pcraft_manifest(1, "[]", r#","composite":"c.png""#);
    let mut pc = zip(&[store("manifest.json", manifest.as_bytes()), deflated("c.png", &vec![5u8; 1 << 20])]);
    // Declare 16 bytes in the central directory.
    let c = central(&pc, 1);
    pc[c + 24..c + 28].copy_from_slice(&16u32.to_le_bytes());
    assert!(matches!(renditions(&pc, &small()), Err(Error::Malformed(_))));
}

#[test]
fn zip_features_outside_the_reader_are_unsupported() {
    let manifest = pcraft_manifest(1, "[]", "");
    let base = zip(&[store("manifest.json", manifest.as_bytes())]);
    let c = central(&base, 0);
    let mut encrypted = base.clone();
    encrypted[c + 8] |= 1;
    assert!(matches!(info(&encrypted, &small()), Err(Error::Unsupported(_))));
    let mut method = base.clone();
    method[c + 10] = 12;
    assert!(matches!(info(&method, &small()), Err(Error::Unsupported(_))));
    let mut zip64 = base.clone();
    zip64[c + 20..c + 24].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(info(&zip64, &small()), Err(Error::Unsupported(_))));
    // A ZIP64 end locator in front of the end record.
    let eocd = base.len() - 22;
    let mut locator = base[..eocd].to_vec();
    locator.extend(0x0706_4b50u32.to_le_bytes());
    locator.extend([0u8; 16]);
    locator.extend(&base[eocd..]);
    assert!(matches!(info(&locator, &small()), Err(Error::Unsupported(_))));
    let mut crc = base.clone();
    crc[c + 16] ^= 0xff;
    assert!(matches!(info(&crc, &small()), Err(Error::Malformed(_))));
}

#[test]
fn a_comment_holding_an_end_signature_does_not_hide_the_archive() {
    let mut pc = pcraft(&png(1, 1, 0), &png(1, 1, 0));
    let comment = b"PK\x05\x06 not the end record";
    let n = pc.len();
    pc[n - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    pc.extend(comment);
    assert_eq!(info(&pc, &small()).unwrap().version, 1);
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[test]
fn seeded_mutations_never_panic() {
    let mut rng = Rng(0x5eed);
    for s in samples() {
        for _ in 0..1500 {
            let mut d = s.clone();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(d.len());
                match rng.below(4) {
                    0 => d[at] ^= 1 << rng.below(8),
                    1 => d[at] = rng.next() as u8,
                    2 => {
                        let v = [0u32, 1, u32::MAX, 0x8000_0000][rng.below(4)];
                        let end = (at + 4).min(d.len());
                        d[at..end].copy_from_slice(&v.to_le_bytes()[..end - at]);
                    }
                    _ => {
                        let end = (at + 1 + rng.below(64)).min(d.len());
                        d.drain(at..end);
                        if d.is_empty() {
                            break;
                        }
                    }
                }
            }
            exercise(&d, &small());
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        exercise(&bytes, &small());
    }

    #[test]
    fn arbitrary_bytes_behind_each_magic_never_panic(prefix in 0usize..4, rest in proptest::collection::vec(any::<u8>(), 0..512)) {
        let heads: [&[u8]; 4] = [b"PK\x03\x04", &[0x1f, 0x8b, 8, 0], br#"{"format":"vectorcraft","#, br#"{"savedBy":"1","schema":"#];
        let mut bytes = heads[prefix].to_vec();
        bytes.extend(rest);
        exercise(&bytes, &small());
    }
}
