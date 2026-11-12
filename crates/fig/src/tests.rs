//! Tests on files built in code: a small schema in the compact text form, a message encoded with
//! craft-kiwi, compressed (deflate, zlib and zstd), framed and zipped.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::io::Write;
use std::sync::Arc;

use craft_kiwi::{Kv, Schema, encode};

use super::*;

const SCHEMA: &str = "
struct GUID uint sessionID, uint localID
message Image byte[] hash, string name, uint dataBlob
message Paint Image image
message NodeChange GUID guid, string name, Paint[] fillPaints
message Blob byte[] bytes
message Message NodeChange[] nodeChanges, Blob[] blobs
";

fn schema() -> Schema {
    Schema::parse(SCHEMA).unwrap()
}

fn o(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}

fn blob(bytes: Vec<u8>) -> Kv<'static> {
    o(vec![("bytes", Kv::Bytes(bytes))])
}

/// A node filled with image `hash` (20 bytes of it), its bytes in blob `data_blob` when given.
fn image_node(hash: u8, data_blob: Option<u64>) -> Kv<'static> {
    let mut image = vec![("hash", Kv::Bytes(vec![hash; 20]))];
    if let Some(i) = data_blob {
        image.push(("dataBlob", Kv::Uint(i)));
    }
    o(vec![("name", Kv::Str("n".into())), ("fillPaints", Kv::Array(vec![o(vec![("image", o(image))])]))])
}

fn message() -> Kv<'static> {
    o(vec![("nodeChanges", Kv::Array(vec![image_node(1, None), image_node(2, None)])), ("blobs", Kv::Array(vec![blob(vec![1, 2, 3])]))])
}

/// `canvas.fig`: magic, version, deflated schema, zstd message.
fn canvas(schema: &Schema, msg: &Kv) -> Vec<u8> {
    let root = schema.find("Message").unwrap();
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&encode::schema(schema)).unwrap();
    let schema_chunk = z.finish().unwrap();
    let msg_chunk = ruzstd::encoding::compress_to_vec(&encode::message(schema, root, msg).unwrap()[..], ruzstd::encoding::CompressionLevel::Fastest);
    let mut out = b"fig-kiwi".to_vec();
    out.extend_from_slice(&101u32.to_le_bytes());
    for c in [schema_chunk, msg_chunk] {
        out.extend_from_slice(&(c.len() as u32).to_le_bytes());
        out.extend_from_slice(&c);
    }
    out
}

fn name(hash: u8) -> String {
    format!("{hash:02x}").repeat(20)
}

/// Bytes that don't compress to nothing, different for each `hash`.
fn image(hash: u8) -> Vec<u8> {
    let mut r = Lcg(u64::from(hash));
    (0..70).map(|_| r.next() as u8).collect()
}

/// A `.fig` zip: the canvas, `meta.json` and deflated `images/` members.
fn file(members: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("canvas.fig", deflated).unwrap();
    z.write_all(&canvas(&schema(), &message())).unwrap();
    z.start_file("meta.json", deflated).unwrap();
    z.write_all(br#"{"file_name":"Fixture"}"#).unwrap();
    for (hash, bytes) in members {
        z.start_file(format!("images/{}", name(*hash)), deflated).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn open(zip: &[u8], max_image_total: u64) -> Archive<'_> {
    Archive::open_with(zip, Limits { max_image_total, ..Limits::default() }).unwrap()
}

/// Overwrite a 32-bit field of `hash`'s central directory record (16: CRC, 20: compressed size,
/// 24: size).
fn change_member_field(zip: &mut [u8], hash: u8, offset: usize, value: u32) {
    let start = {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&*zip)).unwrap();
        z.by_name(&format!("images/{}", name(hash))).unwrap().central_header_start() as usize
    };
    zip[start + offset..start + offset + 4].copy_from_slice(&value.to_le_bytes());
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn an_archive_opens_and_its_canvas_decodes() {
    let members = [(1, image(1)), (2, image(2))];
    let zip = file(&members);
    assert!(is_fig(&zip));
    let archive = Archive::open(&zip).unwrap();
    assert_eq!(archive.file_name.as_deref(), Some("Fixture"));
    assert!(!archive.is_board());
    let (sc, msg) = decode_canvas(&archive.canvas).unwrap();
    let root = sc.find("Message").unwrap();
    assert_eq!(sc.decode_message_within(root, &msg, value_budget(archive.canvas.len())).unwrap(), message());
    assert!(archive.has_image(&name(1)) && !archive.has_image(&name(9)));
    assert_eq!(archive.image(&name(1)), Some(members[0].1.clone()));
    assert_eq!(archive.image(&name(2)), Some(members[1].1.clone()));
    // Names that aren't a hash never reach the archive.
    for bad in ["", "../canvas.fig", "zz", &"0".repeat(2 * MAX_IMAGE_HASH + 1)] {
        assert_eq!(archive.image(bad), None, "{bad}");
    }
    assert_eq!(archive.images_skipped(), 0);
}

#[test]
fn bare_canvas_and_zlib_chunks_open() {
    let sc = schema();
    let c = canvas(&sc, &message());
    assert!(is_fig(&c));
    let archive = Archive::open(&c).unwrap();
    assert_eq!((archive.file_name.clone(), archive.has_image(&name(1)), archive.image(&name(1))), (None, false, None));
    assert!(decode_canvas(&archive.canvas).is_ok());
    // A zlib-wrapped schema chunk reads too.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&encode::schema(&sc)).unwrap();
    let zl = z.finish().unwrap();
    let parts = chunks(&c).unwrap();
    let mut out = c[..12].to_vec();
    for ch in [zl.as_slice(), parts[1]] {
        out.extend_from_slice(&(ch.len() as u32).to_le_bytes());
        out.extend_from_slice(ch);
    }
    assert!(decode_canvas(&out).is_ok());
    // A whiteboard's header.
    let mut jam = c.clone();
    jam[..8].copy_from_slice(JAM_HEADER);
    assert!(Archive::open(&jam).unwrap().is_board());
    assert!(decode_canvas(&jam).is_ok());
}

#[test]
fn a_written_file_reads_back() {
    let sc = schema();
    let root = sc.find("Message").unwrap();
    let c = encode_canvas(&encode::schema(&sc), &encode::message(&sc, root, &message()).unwrap()).unwrap();
    assert!(c.starts_with(DESIGN_HEADER));
    assert_eq!(c[8..12], VERSION.to_le_bytes());
    // A PNG's signature, IHDR length and name, then width 7 and height 5.
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&[0, 0, 0, 7, 0, 0, 0, 5]);
    let (a, b) = (name(1), name(2));
    let images = [(a.as_str(), &[1u8, 2, 3][..]), (b.as_str(), &[4u8][..])];
    let package = Package { file_name: "Written", background: [1.0, 0.5, 0.0, 1.0], canvas: &c, images: &images, thumbnail: Some(&png) };
    let zip = write_archive(&package).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&zip[..])).unwrap();
    let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect();
    assert_eq!(names, ["canvas.fig".to_string(), "thumbnail.png".into(), "meta.json".into(), format!("images/{a}"), format!("images/{b}")]);
    let mut meta = String::new();
    std::io::Read::read_to_string(&mut z.by_name("meta.json").unwrap(), &mut meta).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(
        meta,
        serde_json::json!({"client_meta": {"background_color": {"r": 1.0, "g": 0.5, "b": 0.0, "a": 1.0}, "thumbnail_size": {"width": 7, "height": 5}}, "file_name": "Written"})
    );
    let archive = Archive::open(&zip).unwrap();
    assert_eq!(archive.file_name.as_deref(), Some("Written"));
    assert_eq!(archive.image(&a), Some(vec![1, 2, 3]));
    let (sc2, msg) = decode_canvas(&archive.canvas).unwrap();
    assert_eq!(sc2.decode_message(sc2.find("Message").unwrap(), &msg).unwrap(), message());
    // No thumbnail, or one that isn't a PNG header: no size. An image not named by a hash is refused.
    for thumbnail in [None, Some(&b"short"[..])] {
        let zip = write_archive(&Package { thumbnail, images: &[], ..package }).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&zip[..])).unwrap();
        let meta: serde_json::Value = serde_json::from_reader(z.by_name("meta.json").unwrap()).unwrap();
        assert!(meta["client_meta"]["thumbnail_size"].is_null());
    }
    assert!(write_archive(&Package { images: &[("../x", &[1u8][..])], ..package }).is_err());
}

#[test]
fn other_files_are_not_fig() {
    assert!(!is_fig(b"{\"document\": {}}"));
    assert!(!is_fig(b""));
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file("document.json", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(b"{}").unwrap();
    let other = z.finish().unwrap().into_inner();
    assert!(!is_fig(&other));
    assert!(Archive::open(&other).is_err());
}

#[test]
fn the_canvas_limits_hold() {
    let c = canvas(&schema(), &message());
    // No header, a cut length, a length past the end.
    assert!(chunks(b"fig-kiwi").is_err());
    assert!(chunks(b"PK\x03\x04aaaaaaaaaaaa").is_err());
    assert!(chunks(&c[..14]).is_err());
    assert!(chunks(&c[..c.len() - 1]).is_err());
    // One chunk is not a canvas.
    let first = chunks(&c).unwrap()[0].len();
    assert!(decode_canvas(&c[..12 + 4 + first]).is_err());
    // Chunks past the sixteenth are not read.
    let mut many = c[..12].to_vec();
    for _ in 0..MAX_CHUNKS + 4 {
        many.extend_from_slice(&0u32.to_le_bytes());
    }
    assert_eq!(chunks(&many).unwrap().len(), MAX_CHUNKS);
    // A chunk that unpacks past its cap is refused, deflate and zstd alike; at the cap it reads.
    let zeros = vec![0u8; 100_000];
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&zeros).unwrap();
    let deflated = z.finish().unwrap();
    let zstd = ruzstd::encoding::compress_to_vec(&zeros[..], ruzstd::encoding::CompressionLevel::Fastest);
    for packed in [&deflated, &zstd] {
        assert_eq!(inflate(packed, 100_000).unwrap().len(), 100_000);
        assert!(inflate(packed, 99_999).is_err());
    }
    assert!(inflate(&[0x28, 0xB5, 0x2F, 0xFD, 1, 2, 3], 1 << 20).is_err());
    assert!(inflate(b"\xff\xff\xff\xff", 1 << 20).is_err());
    // A schema chunk past MAX_SCHEMA is refused before it is decoded.
    let big = vec![0u8; MAX_SCHEMA as usize + 1];
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(&big).unwrap();
    let packed = z.finish().unwrap();
    let mut bomb = c[..12].to_vec();
    for ch in [packed.as_slice(), chunks(&c).unwrap()[1]] {
        bomb.extend_from_slice(&(ch.len() as u32).to_le_bytes());
        bomb.extend_from_slice(ch);
    }
    assert_eq!(decode_canvas(&bomb).unwrap_err(), Error::Corrupt("damaged or oversized deflate chunk".into()));
}

#[test]
fn large_files_get_a_larger_value_budget() {
    // Small files keep the fixed floor: a few bytes can't expand into a huge tree.
    assert_eq!(value_budget(0), 20_000_000);
    assert_eq!(value_budget(1 << 20), 20_000_000);
    // A real 400 000-layer file: an 83.5 MB canvas whose message holds 30.5 million values.
    assert!(value_budget(83_491_232) > 30_514_899);
    // A real 194 MB design-system file: a 39.5 MB canvas holding 29.7 million values.
    assert!(value_budget(39_528_420) > 29_703_730);
    // Still bounded by the largest canvas read.
    assert!(value_budget(MAX_CANVAS as usize) <= 128 << 20);
    // A dense file may reach about 2.4 values per compressed byte: below the ceiling the budget
    // covers that, the floor still holds up to 5 MB of canvas, and 32 MiB is enough for the ceiling.
    assert!(value_budget(8 << 20) >= (8 << 20) * 12 / 5);
    #[cfg(not(target_pointer_width = "32"))]
    assert!(value_budget(30_000_000) >= 30_000_000 * 12 / 5);
    assert_eq!(value_budget(5_000_000), 20_000_000);
    assert_eq!(value_budget(32 << 20), value_budget(MAX_CANVAS as usize));
}

#[test]
fn mangled_files_never_panic() {
    let zip = file(&[(1, image(1))]);
    let c = canvas(&schema(), &message());
    let read = |bytes: &[u8]| {
        let _ = is_fig(bytes);
        if let Ok(archive) = Archive::open(bytes) {
            let _ = archive.image(&name(1));
            if let Ok((sc, msg)) = decode_canvas(&archive.canvas)
                && let Some(root) = sc.find("Message")
                && let Ok(kv) = sc.decode_message(root, &msg)
            {
                let _ = embedded_copies(&kv, 1 << 20);
            }
        }
    };
    let mut r = Lcg(7);
    for src in [&zip, &c] {
        for cut in (0..src.len()).step_by(7) {
            read(&src[..cut]);
        }
        for _ in 0..300 {
            let mut m = src.clone();
            for _ in 0..1 + r.next() % 8 {
                let i = (r.next() as usize) % m.len();
                m[i] ^= 1 << (r.next() % 8);
            }
            read(&m);
        }
    }
    for _ in 0..500 {
        let junk: Vec<u8> = (0..(r.next() % 200)).map(|_| r.next() as u8).collect();
        let mut framed = b"fig-kiwi\0\0\0\0".to_vec();
        framed.extend_from_slice(&junk);
        read(&framed);
        let _ = network::decode(&junk);
        let _ = path::decode(&junk);
    }
}

#[test]
fn images_carried_in_the_blobs_are_found() {
    let msg = o(vec![
        ("nodeChanges", Kv::Array(vec![image_node(1, Some(1)), image_node(2, Some(7)), image_node(3, None), image_node(4, Some(2))])),
        ("blobs", Kv::Array(vec![blob(vec![9]), blob(vec![5, 6, 7]), blob(vec![0; 40]), blob(vec![])])),
    ]);
    // A blob index past the blobs, no index, and an empty or oversized blob name no image.
    assert_eq!(embedded_images(&msg, 39), [(name(1), 1)].into());
    assert_eq!(embedded_images(&msg, 40), [(name(1), 1), (name(4), 2)].into());
    assert_eq!(*embedded_copies(&msg, 39)[&name(1)], [5, 6, 7]);
    // A hash longer than 64 bytes is not a name.
    let long = o(vec![("image", o(vec![("hash", Kv::Bytes(vec![1; MAX_IMAGE_HASH + 1])), ("dataBlob", Kv::Uint(0))]))]);
    let msg = o(vec![("nodeChanges", Kv::Array(vec![long])), ("blobs", Kv::Array(vec![blob(vec![9])]))]);
    assert!(embedded_images(&msg, 100).is_empty());
}

/// A file can point any number of image hashes at one large blob; each shares one copy of it.
#[test]
fn hashes_naming_one_blob_share_one_copy_of_it() {
    let msg = o(vec![
        ("blobs", Kv::Array(vec![blob(vec![3; 64]), blob(vec![7; 16])])),
        ("nodeChanges", Kv::Array((1..=50).map(|h| image_node(h, Some(0))).chain([image_node(99, Some(1)), image_node(98, Some(2))]).collect())),
    ]);
    let copies = embedded_copies(&msg, 1 << 20);
    assert_eq!(copies.len(), 51, "the hash naming no blob is left out");
    let first = &copies[&name(1)];
    assert_eq!(**first, [3; 64]);
    assert_eq!(copies.values().filter(|c| Arc::ptr_eq(c, first)).count(), 50);
    assert_eq!(*copies[&name(99)], [7; 16]);
    // Every alias maps to the first hash; a hash `images/` also holds keeps its own name.
    let aliases = embedded_aliases(&copies, |_| false);
    assert_eq!(aliases.len(), 49);
    assert!(aliases.values().all(|c| *c == name(1)));
    assert!(!embedded_aliases(&copies, |h| h == name(5)).contains_key(&name(5)));
}

mod image_budget {
    use super::*;

    #[test]
    fn exact_total_limit_keeps_all_images_and_each_open_gets_its_own_budget() {
        let members = [(1, image(1)), (2, image(2))];
        let zip = file(&members);
        for _ in 0..2 {
            let archive = open(&zip, 140);
            assert_eq!(archive.image(&name(1)), Some(members[0].1.clone()));
            assert_eq!(archive.image(&name(2)), Some(members[1].1.clone()));
            assert_eq!((archive.image_bytes_left(), archive.images_skipped()), (0, 0));
        }
    }

    #[test]
    fn individually_acceptable_images_overflow_the_total() {
        let members = [(1, image(1)), (2, image(2))];
        let zip = file(&members);
        let archive = open(&zip, 139);
        assert!(archive.image(&name(1)).is_some());
        assert_eq!(archive.image(&name(2)), None);
        assert_eq!(archive.images_skipped(), 1);
    }

    #[test]
    fn rejecting_a_member_before_reading_leaves_room_for_a_smaller_one() {
        let members = [(1, vec![0; 71]), (2, image(2))];
        let zip = file(&members);
        let archive = open(&zip, 70);
        assert_eq!(archive.image(&name(1)), None);
        assert_eq!(archive.image(&name(2)), Some(members[1].1.clone()));
        assert_eq!(archive.images_skipped(), 1);
    }

    #[test]
    fn failed_crc_reads_still_consume_the_bytes_read() {
        let members = [(1, image(1)), (2, image(2))];
        let mut zip = file(&members);
        change_member_field(&mut zip, 1, 16, 0);
        let archive = open(&zip, 71);
        assert_eq!(archive.image(&name(1)), None);
        assert_eq!(archive.image_bytes_left(), 1);
        assert_eq!(archive.image(&name(2)), None);
        assert_eq!(archive.images_skipped(), 1);
    }

    #[test]
    fn a_truncated_deflate_read_keeps_its_partial_charge() {
        let members = [(1, image(1)), (2, image(2))];
        let mut zip = file(&members);
        // Shorten the first member's compressed size, cutting its deflate stream mid-image.
        change_member_field(&mut zip, 1, 20, 15);
        let archive = open(&zip, 70);
        assert!(archive.image(&name(1)).is_none());
        let remaining = archive.image_bytes_left();
        assert!(remaining < 70, "a partial read left {remaining} of 70 bytes");
        assert!(archive.image(&name(2)).is_none());
        assert_eq!(archive.image_bytes_left(), remaining);
    }

    #[test]
    fn the_individual_image_cap_is_kept_without_reading_a_large_member() {
        let members = [(1, image(1)), (2, image(2))];
        let mut zip = file(&members);
        change_member_field(&mut zip, 1, 24, (Limits::default().max_image + 1) as u32);
        let archive = open(&zip, 70);
        assert_eq!(archive.image(&name(1)), None);
        assert_eq!(archive.image(&name(2)), Some(members[1].1.clone()));
        assert_eq!(archive.images_skipped(), 0);
        // A cap of the caller's own.
        let zip = file(&members);
        let small = Archive::open_with(&zip, Limits { max_image: 69, ..Limits::default() }).unwrap();
        assert_eq!(small.image(&name(1)), None);
        assert_eq!(small.image_bytes_left(), Limits::default().max_image_total);
    }

    #[test]
    fn crc_is_checked_even_when_the_read_ends_exactly_at_the_budget() {
        let members = [(1, image(1))];
        let mut zip = file(&members);
        change_member_field(&mut zip, 1, 16, 0);
        let archive = open(&zip, 70);
        assert_eq!(archive.image(&name(1)), None);
        assert_eq!(archive.images_skipped(), 0);
    }

    #[test]
    fn an_understated_size_cannot_read_past_the_remaining_budget() {
        let members = [(1, image(1)), (2, image(2))];
        let mut zip = file(&members);
        // The directory says one byte; the stream holds 70. Whether the ZIP reader refuses the
        // mismatch or reads on, the member is not returned and the budget is never overdrawn.
        change_member_field(&mut zip, 1, 24, 1);
        let archive = open(&zip, 69);
        assert!(archive.image(&name(1)).is_none());
        assert!(archive.image_bytes_left() <= 69);
        assert!(archive.image(&name(2)).is_none());
        assert!(archive.images_skipped() >= 1);
    }

    #[test]
    fn missing_members_do_not_consume_the_budget() {
        let members = [(1, image(1)), (2, vec![0; 128])];
        let zip = file(&members);
        let archive = open(&zip, 70);
        assert_eq!(archive.image(&name(9)), None);
        assert_eq!(archive.image(&name(1)), Some(members[0].1.clone()));
        assert_eq!(archive.images_skipped(), 0);
    }
}

#[test]
fn a_vector_network_round_trips_and_its_limits_hold() {
    use network::{Network, Region, Segment, Vertex, decode, encode};
    let net = Network {
        vertices: vec![Vertex { style: 0, x: 0.0, y: 0.0 }, Vertex { style: 2, x: 10.0, y: 0.0 }, Vertex { style: 0, x: 5.0, y: 8.5 }],
        segments: vec![
            Segment { style: 0, a: 0, ta: (0.0, 0.0), b: 1, tb: (0.0, 0.0) },
            Segment { style: 1, a: 1, ta: (1.0, 2.0), b: 2, tb: (-1.0, 0.5) },
            Segment { style: 0, a: 2, ta: (0.0, 0.0), b: 0, tb: (0.0, 0.0) },
        ],
        regions: vec![Region { style: 3, non_zero: true, loops: vec![vec![0, 1, 2]] }, Region { style: 0, non_zero: false, loops: vec![] }],
    };
    let blob = encode(&net);
    assert_eq!(blob.len(), 12 + 3 * 12 + 3 * 28 + (8 + 4 + 12) + 8);
    // The first region's word: style 3 shifted once, the low bit set for non-zero.
    assert_eq!(blob[12 + 36 + 84..][..4], 7u32.to_le_bytes());
    assert_eq!(decode(&blob), Some(net));
    for cut in 0..blob.len() {
        assert_eq!(decode(&blob[..cut]), None, "cut at {cut}");
    }
    let with = |at: usize, v: u32| {
        let mut b = blob.clone();
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        b
    };
    // A count the blob can't hold or past the cap, a vertex or segment index out of range, NaN.
    assert_eq!(decode(&with(0, 4)), None);
    assert_eq!(decode(&with(0, network::MAX_ITEMS as u32 + 1)), None);
    assert_eq!(decode(&with(12 + 36 + 4, 3)), None);
    assert_eq!(decode(&with(blob.len() - 8 - 4, 3)), None);
    assert_eq!(decode(&with(12 + 4, f32::NAN.to_bits())), None);
    // Loop entries are capped in total, however many loops share them.
    let mut many = Vec::new();
    for v in [0u32, 0, 1, 0, 2] {
        many.extend_from_slice(&v.to_le_bytes());
    }
    for _ in 0..2 {
        many.extend_from_slice(&(network::MAX_ITEMS as u32 / 2 + 1).to_le_bytes());
        many.resize(many.len() + 4 * (network::MAX_ITEMS / 2 + 1), 0);
    }
    assert_eq!(decode(&many), None);
}

#[test]
fn a_geometry_blob_round_trips_and_its_limits_hold() {
    use path::{Command, decode, encode};
    let commands = vec![
        Command::Move([0.0, 0.0]),
        Command::Line([10.0, 0.5]),
        Command::Quad([1.0, 2.0], [3.0, 4.0]),
        Command::Cubic([5.0, 6.0], [7.0, 8.0], [9.0, -1.0]),
        Command::Close,
    ];
    let blob = encode(&commands);
    assert_eq!(blob.len(), 9 + 9 + 17 + 25 + 1);
    assert_eq!(blob[..9], [1, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(decode(&blob), Some(commands));
    assert_eq!(decode(&[]), Some(vec![]));
    // Cut inside a point, an unknown command, a value that isn't finite.
    assert_eq!(decode(&blob[..blob.len() - 2]), None);
    assert_eq!(decode(&[5]), None);
    let mut nan = vec![1];
    nan.extend_from_slice(&f32::INFINITY.to_le_bytes());
    nan.extend_from_slice(&0f32.to_le_bytes());
    assert_eq!(decode(&nan), None);
    // The command cap.
    assert_eq!(decode(&vec![0; path::MAX_COMMANDS]).map(|c| c.len()), Some(path::MAX_COMMANDS));
    assert_eq!(decode(&vec![0; path::MAX_COMMANDS + 1]), None);
}

/// A ZIP64 end record can declare millions of entries in a small file: refused on the raw bytes
/// before the archive is indexed, and such a file is not taken for a `.fig`.
#[test]
fn an_archive_declaring_too_many_entries_is_refused_before_it_is_indexed() {
    let bomb = craft_zip::zip64_entry_count_sample(MAX_ENTRIES + 1);
    let e = Archive::open(&bomb).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains(&format!("declares {} entries", MAX_ENTRIES + 1)), "{e}");
    assert!(!is_fig(&bomb));
}

/// Two entries of one name: `zip` keeps one, and which one the writer meant is not known.
#[test]
fn two_entries_of_one_name_are_refused() {
    let zip = file(&[(1, image(1)), (2, image(2))]);
    assert!(Archive::open(&zip).is_ok());
    let (one, two) = (name(1), name(2));
    let mut renamed = zip.clone();
    let mut i = 0;
    while let Some(at) = renamed[i..].windows(two.len()).position(|w| w == two.as_bytes()) {
        renamed[i + at..i + at + two.len()].copy_from_slice(one.as_bytes());
        i += at + two.len();
    }
    let e = Archive::open(&renamed).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("two entries of the archive share a name"), "{e}");
}
