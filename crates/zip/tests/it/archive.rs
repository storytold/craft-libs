//! Reading archives within limits, duplicate names, and the writer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_zip::{
    Archive, Compression, Error, Limits, SeekableArchive, SeekableLimits, Writer, check_entry_count, has_traversal_component, sanitize_path,
    zip64_entry_count_sample,
};

const ROOMY: Limits = Limits { max_entries: 1000, max_total_bytes: 1 << 20 };

fn archive(entries: &[(&str, &[u8], Compression)]) -> Vec<u8> {
    let mut w = Writer::new();
    for (name, data, compression) in entries {
        w.add(name, data, *compression).unwrap();
    }
    w.finish().unwrap()
}

fn find(bytes: &[u8], needle: &[u8], from: usize) -> usize {
    from + bytes[from..].windows(needle.len()).position(|w| w == needle).unwrap()
}

#[test]
fn entries_are_written_in_order_and_read_back() {
    let bytes = archive(&[
        ("mimetype", b"application/x-test", Compression::Stored),
        ("dir/a.xml", &[b'a'; 5000], Compression::Deflated),
        ("b.bin", b"\x00\x01", Compression::Level(9)),
    ]);
    // A stored first entry is readable in place: local header (30) + name (8).
    assert_eq!(&bytes[38..56], b"application/x-test");
    assert!(bytes.len() < 1000, "the deflated entry is compressed");
    let mut z = Archive::open(&bytes, ROOMY).unwrap();
    assert_eq!(z.names().collect::<Vec<_>>(), ["mimetype", "dir/a.xml", "b.bin"]);
    assert_eq!((z.len(), z.is_empty(), z.has_duplicate_names()), (3, false, false));
    assert!(z.contains("b.bin") && !z.contains("B.bin"));
    assert_eq!(z.declared_size("dir/a.xml"), Some(5000));
    assert_eq!(z.declared_size("nope"), None);
    assert_eq!(z.read("dir/a.xml", 5000).unwrap().unwrap(), vec![b'a'; 5000]);
    assert_eq!(z.read("b.bin", 10).unwrap().unwrap(), [0, 1]);
    assert_eq!(z.read("nope", 10).unwrap(), None);
    assert_eq!(z.budget().used(), 5002);
}

#[test]
fn the_output_is_deterministic() {
    let make = || archive(&[("a", b"one", Compression::Deflated), ("b", b"two", Compression::Stored)]);
    assert_eq!(make(), make());
}

#[test]
fn adding_a_name_twice_is_an_error() {
    let mut w = Writer::new();
    w.add("a", b"1", Compression::Stored).unwrap();
    assert!(matches!(w.add("a", b"2", Compression::Stored), Err(Error::Write(m)) if m.starts_with("a: ")));
}

#[test]
fn what_is_not_a_zip_is_refused() {
    assert!(matches!(Archive::open(b"not a zip", ROOMY).err(), Some(Error::NotZip(_))));
    assert!(matches!(Archive::open(b"", ROOMY).err(), Some(Error::NotZip(_))));
    let bytes = archive(&[("a", b"abc", Compression::Stored)]);
    assert!(Archive::open(&bytes[..bytes.len() - 5], ROOMY).is_err());
}

#[test]
fn an_entry_past_its_limit_is_refused_and_not_charged() {
    let bytes = archive(&[("a", &[b'a'; 100], Compression::Deflated)]);
    let mut z = Archive::open(&bytes, ROOMY).unwrap();
    assert_eq!(z.read("a", 99), Err(Error::EntryTooLarge { name: "a".into(), limit: 99 }));
    assert_eq!(z.budget().used(), 0);
    assert_eq!(z.read("a", 100).unwrap().map(|d| d.len()), Some(100));
}

#[test]
fn entries_cannot_add_up_past_the_total() {
    let data = [b'a'; 100];
    let bytes = archive(&[("a", &data, Compression::Deflated), ("b", &data, Compression::Deflated), ("c", &data, Compression::Deflated)]);
    let mut z = Archive::open(&bytes, Limits { max_total_bytes: 250, ..ROOMY }).unwrap();
    assert!(z.read("a", 1000).unwrap().is_some());
    assert!(z.read("b", 1000).unwrap().is_some());
    assert_eq!(z.budget().remaining(), 50);
    let refused = z.read("c", 1000);
    assert_eq!(refused, Err(Error::BudgetSpent { name: "c".into(), limit: 250 }));
    assert!(refused.is_err_and(|e| e.is_limit()));
    // A caller with its own rule can charge more.
    z.budget_mut().charge(50).unwrap();
    assert_eq!(z.budget().remaining(), 0);
}

/// The size an entry declares is a claim. One that expands past it is refused whatever the
/// limit, and a small declared size does not get a large entry under a small limit.
#[test]
fn an_entry_that_expands_past_its_declared_size_is_refused() {
    let mut bytes = archive(&[("a", &[b'a'; 5000], Compression::Deflated)]);
    // The uncompressed size, in the central header (at 24) and the local header (at 22).
    let central = find(&bytes, b"PK\x01\x02", 0);
    for at in [central + 24, 22] {
        assert_eq!(bytes[at..at + 4], 5000u32.to_le_bytes());
        bytes[at..at + 4].copy_from_slice(&10u32.to_le_bytes());
    }
    let mut z = Archive::open(&bytes, ROOMY).unwrap();
    assert_eq!(z.declared_size("a"), Some(10));
    // `zip` notices first when it can; either way it is refused and nothing is charged.
    assert!(matches!(z.read("a", 1 << 20), Err(Error::EntryTooLarge { limit: 10, .. } | Error::Entry { .. })));
    assert_eq!(z.budget().used(), 0);
}

#[test]
fn a_broken_entry_is_an_entry_error() {
    let mut bytes = archive(&[("a", b"hello world", Compression::Stored)]);
    let at = find(&bytes, b"hello", 0);
    bytes[at] = b'j';
    let mut z = Archive::open(&bytes, ROOMY).unwrap();
    assert!(matches!(z.read("a", 100), Err(Error::Entry { name, .. }) if name == "a"));
    assert_eq!(z.budget().used(), 0);
}

#[test]
fn two_entries_with_one_name_are_told() {
    let mut bytes = archive(&[("a", b"first", Compression::Stored), ("b", b"second", Compression::Stored)]);
    assert!(!Archive::open(&bytes, ROOMY).unwrap().has_duplicate_names());
    // Rename `b` to `a` in its local header and in the central directory.
    let local = find(&bytes, b"PK\x03\x04", 4);
    bytes[local + 30] = b'a';
    let central = find(&bytes, b"PK\x01\x02", find(&bytes, b"PK\x01\x02", 0) + 4);
    assert_eq!(bytes[central + 46], b'b');
    bytes[central + 46] = b'a';
    let z = Archive::open(&bytes, ROOMY).unwrap();
    assert_eq!(z.len(), 1);
    assert!(z.has_duplicate_names());
}

#[test]
fn an_archive_declaring_too_many_entries_is_refused_before_it_is_indexed() {
    let bomb = zip64_entry_count_sample(20_001);
    assert_eq!(check_entry_count(&bomb, 20_000), Err(Error::TooManyEntries { declared: 20_001, limit: 20_000 }));
    assert_eq!(Archive::open(&bomb, Limits { max_entries: 20_000, ..ROOMY }).err(), Some(Error::TooManyEntries { declared: 20_001, limit: 20_000 }));
    let bytes = archive(&[("a", b"", Compression::Stored), ("b", b"", Compression::Stored), ("c", b"", Compression::Stored)]);
    assert_eq!(Archive::open(&bytes, Limits { max_entries: 2, ..ROOMY }).err(), Some(Error::TooManyEntries { declared: 3, limit: 2 }));
    assert!(Archive::open(&bytes, Limits { max_entries: 3, ..ROOMY }).is_ok());
}

/// What the strict preflight accepts, the writer writes.
#[test]
fn written_archives_pass_the_strict_preflight() {
    use craft_zip::preflight::{Limits, preflight};
    let bytes = archive(&[("mimetype", b"x", Compression::Stored), ("a/b.xml", &[b'a'; 300], Compression::Deflated)]);
    let directory = preflight(&bytes, Limits { max_entries: 10, max_central_directory_bytes: 1 << 16, max_name_bytes: 256 }).unwrap();
    assert_eq!(directory.entries, 2);
}

#[test]
fn a_stored_mimetype_is_read_in_place() {
    let bytes = archive(&[("mimetype", b"image/openraster", Compression::Stored), ("a", b"x", Compression::Deflated)]);
    assert_eq!(craft_zip::stored_mimetype(&bytes), Some(&b"image/openraster"[..]));
    assert_eq!(craft_zip::stored_mimetype(&archive(&[("mimetype", b"x", Compression::Deflated)])), None, "compressed");
    assert_eq!(craft_zip::stored_mimetype(&archive(&[("a", b"x", Compression::Stored), ("mimetype", b"x", Compression::Stored)])), None, "not first");
    assert_eq!(craft_zip::stored_mimetype(&archive(&[("mimetype", &[b'a'; 257], Compression::Stored)])), None, "too long");
    assert_eq!(craft_zip::stored_mimetype(b"PK\x03\x04"), None);
    assert_eq!(craft_zip::stored_mimetype(&[]), None);
}

#[test]
fn a_head_is_read_without_inflating_the_rest_and_charged() {
    let bytes = archive(&[("font", &[b"wOF2".as_slice(), &[0u8; 100_000]].concat(), Compression::Deflated), ("short", b"wO", Compression::Stored)]);
    let mut z = Archive::open(&bytes, Limits { max_total_bytes: 7, ..ROOMY }).unwrap();
    assert_eq!(z.compressed_size("short"), Some(2));
    assert!(z.compressed_size("font").unwrap() < 1000);
    assert_eq!(z.compressed_size("nope"), None);
    assert_eq!(z.read_head("font", 4).unwrap().as_deref(), Some(&b"wOF2"[..]));
    assert_eq!(z.read_head("short", 4).unwrap().as_deref(), Some(&b"wO"[..]), "shorter than asked");
    assert_eq!(z.read_head("nope", 4).unwrap(), None);
    assert_eq!(z.budget().used(), 6);
    assert_eq!(z.read_head("font", 4), Err(Error::BudgetSpent { name: "font".into(), limit: 7 }), "one byte left");
    assert_eq!(z.budget().used(), 6);
}

#[test]
fn a_clone_keeps_its_own_count() {
    let bytes = archive(&[("a", b"abc", Compression::Deflated)]);
    let mut z = Archive::open(&bytes, ROOMY).unwrap();
    let mut c = z.clone();
    assert_eq!(c.read("a", 10).unwrap().as_deref(), Some(&b"abc"[..]));
    assert_eq!((c.budget().used(), z.budget().used()), (3, 0));
    assert!(z.read("a", 10).unwrap().is_some());
}

#[test]
fn owned_archive_reads_and_clones_with_independent_budgets() {
    let bytes = archive(&[("a", b"abc", Compression::Deflated)]);
    let mut z = Archive::open_owned(bytes, Limits { max_total_bytes: 3, ..ROOMY }).unwrap();
    let mut clone = z.clone();
    assert_eq!(z.read("a", 3).unwrap().as_deref(), Some(&b"abc"[..]));
    assert!(matches!(z.read("a", 3), Err(Error::BudgetSpent { .. })));
    assert_eq!(clone.read("a", 3).unwrap().as_deref(), Some(&b"abc"[..]));
    assert_eq!(clone.budget().used(), 3);
}

#[test]
fn owned_archive_applies_directory_guards() {
    let mut bytes = archive(&[("a", b"1", Compression::Stored), ("b", b"2", Compression::Stored)]);
    assert!(matches!(Archive::open_owned(bytes.clone(), Limits { max_entries: 1, ..ROOMY }), Err(Error::TooManyEntries { .. })));
    let local = find(&bytes, b"PK\x03\x04", 4);
    bytes[local + 30] = b'a';
    let central = find(&bytes, b"PK\x01\x02", find(&bytes, b"PK\x01\x02", 0) + 4);
    bytes[central + 46] = b'a';
    assert!(Archive::open_owned(bytes, ROOMY).unwrap().has_duplicate_names());
    assert!(Archive::open_owned(b"invalid".to_vec(), ROOMY).is_err());
}

#[test]
fn path_traversal_detection_flags_hostile_vectors() {
    // Parent directory traversal
    assert!(has_traversal_component("../../etc/passwd"));
    assert!(has_traversal_component(r"..\..\windows\system32"));
    assert!(has_traversal_component("foo/../bar"));
    assert!(has_traversal_component(r"foo\..\bar"));
    assert!(has_traversal_component(".."));

    // Absolute and drive-letter prefixes
    assert!(has_traversal_component("C:/test.txt"));
    assert!(has_traversal_component(r"C:\test.txt"));
    assert!(has_traversal_component("c:/test.txt"));
    assert!(has_traversal_component(r"c:\windows\system32"));
    assert!(has_traversal_component("/etc/passwd"));
    assert!(has_traversal_component(r"\windows\system32"));

    // Null bytes
    assert!(has_traversal_component("foo\0bar"));

    // Sanitization flags or cleans them safely
    assert_eq!(sanitize_path("../../etc/passwd"), None);
    assert_eq!(sanitize_path(r"..\..\windows\system32"), None);
    assert_eq!(sanitize_path("C:/test.txt"), None);
    assert_eq!(sanitize_path("/etc/passwd"), Some("etc/passwd"));
    assert_eq!(sanitize_path(r"\windows\system32"), Some(r"windows\system32"));
    assert_eq!(sanitize_path("foo\0bar"), None);

    // Safe relative paths are allowed
    assert!(!has_traversal_component("safe/path.txt"));
    assert!(!has_traversal_component("mimetype"));
    assert_eq!(sanitize_path("safe/path.txt"), Some("safe/path.txt"));
    assert_eq!(sanitize_path("mimetype"), Some("mimetype"));
}

#[test]
fn synthetic_hostile_zip_headers_are_safely_flagged_and_rejected() {
    use std::io::Write as _;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("../../etc/passwd", opts).unwrap();
    zip.write_all(b"root:x:0:0").unwrap();
    zip.start_file(r"..\..\windows\system32", opts).unwrap();
    zip.write_all(b"system32").unwrap();
    zip.start_file("C:/test.txt", opts).unwrap();
    zip.write_all(b"drive").unwrap();
    zip.start_file("safe/file.txt", opts).unwrap();
    zip.write_all(b"safe").unwrap();
    let bytes = zip.finish().unwrap().into_inner();

    // Archive level checks
    let mut archive = Archive::open(&bytes, ROOMY).unwrap();
    assert!(archive.has_traversal_component());

    // Enumeration and names() iterator omit traversal components
    let names = (0..archive.len()).filter_map(|i| archive.entry_name(i).unwrap()).collect::<Vec<_>>();
    assert_eq!(names, vec!["safe/file.txt"]);
    assert_eq!(archive.names().collect::<Vec<_>>(), vec!["safe/file.txt"]);

    let seekable =
        SeekableArchive::open(std::io::Cursor::new(&bytes), SeekableLimits { max_entries: 100, max_metadata_bytes: 16_384, max_total_bytes: 1000 })
            .unwrap();
    assert_eq!(seekable.names().collect::<Vec<_>>(), vec!["safe/file.txt"]);
    assert_eq!(archive.entry_name(3).unwrap().as_deref(), Some("safe/file.txt"));
    assert_eq!(archive.entry_name(0).unwrap(), None);
    assert_eq!(archive.entry_name(1).unwrap(), None);
    assert_eq!(archive.entry_name(2).unwrap(), None);

    // Raw entry name is still available for inspection
    assert_eq!(archive.raw_entry_name(0).unwrap().as_deref(), Some("../../etc/passwd"));
    assert_eq!(archive.raw_entry_name(1).unwrap().as_deref(), Some(r"..\..\windows\system32"));
    assert_eq!(archive.raw_entry_name(2).unwrap().as_deref(), Some("C:/test.txt"));

    // Virtual lookups reject traversal components
    assert!(!archive.contains("../../etc/passwd"));
    assert!(!archive.contains(r"..\..\windows\system32"));
    assert!(!archive.contains("C:/test.txt"));
    assert!(archive.contains("safe/file.txt"));
    assert_eq!(archive.declared_size("../../etc/passwd"), None);
    assert_eq!(archive.compressed_size(r"..\..\windows\system32"), None);
    assert_eq!(archive.is_stored("C:/test.txt"), None);
    assert_eq!(archive.declared_size("safe/file.txt"), Some(4));

    // Extractions reject traversal components
    assert_eq!(archive.read("../../etc/passwd", 100).unwrap(), None);
    assert_eq!(archive.read(r"..\..\windows\system32", 100).unwrap(), None);
    assert_eq!(archive.read("C:/test.txt", 100).unwrap(), None);
    assert_eq!(archive.read_head("../../etc/passwd", 10).unwrap(), None);
    assert_eq!(archive.read_index(0, 100).unwrap(), None);
    assert_eq!(archive.read_index(1, 100).unwrap(), None);
    assert_eq!(archive.read_index(2, 100).unwrap(), None);
    assert_eq!(archive.read("safe/file.txt", 100).unwrap().as_deref(), Some(&b"safe"[..]));
    assert_eq!(archive.read_index(3, 100).unwrap().as_deref(), Some(&b"safe"[..]));
}

#[test]
fn writer_rejects_path_traversal_components() {
    let mut writer = Writer::new();
    assert!(matches!(writer.add("../../etc/passwd", b"bad", Compression::Stored), Err(Error::Write(msg)) if msg.contains("path traversal")));
    assert!(matches!(writer.add(r"..\..\windows\system32", b"bad", Compression::Stored), Err(Error::Write(msg)) if msg.contains("path traversal")));
    assert!(matches!(writer.add("C:/test.txt", b"bad", Compression::Stored), Err(Error::Write(msg)) if msg.contains("path traversal")));
    assert!(matches!(writer.add_directory("../../etc/"), Err(Error::Write(msg)) if msg.contains("path traversal")));
}

#[test]
fn raw_bytes_with_traversal_and_invalid_utf8_are_flagged() {
    let mut bytes = archive(&[("xxxx", b"1", Compression::Stored)]);
    let local = find(&bytes, b"PK\x03\x04", 0);
    bytes[local + 30..local + 34].copy_from_slice(b"../\xff");
    let central = find(&bytes, b"PK\x01\x02", 0);
    bytes[central + 46..central + 50].copy_from_slice(b"../\xff");

    let mut archive = Archive::open(&bytes, ROOMY).unwrap();
    assert!(archive.has_traversal_component());
    assert_eq!(archive.names().collect::<Vec<_>>(), Vec::<std::borrow::Cow<'_, str>>::new());
    assert_eq!((0..archive.len()).filter_map(|i| archive.entry_name(i).unwrap()).collect::<Vec<_>>(), Vec::<String>::new());
    assert_eq!(archive.entry_name(0).unwrap(), None);
    assert_eq!(archive.entry_sizes(0).unwrap(), None);
    assert_eq!(archive.read_index(0, 10).unwrap(), None);

    let mut seekable =
        SeekableArchive::open(std::io::Cursor::new(bytes), SeekableLimits { max_entries: 100, max_metadata_bytes: 16_384, max_total_bytes: 1000 })
            .unwrap();
    assert!(seekable.has_traversal_component());
    assert_eq!(seekable.names().collect::<Vec<_>>(), Vec::<std::borrow::Cow<'_, str>>::new());
    assert_eq!((0..seekable.len()).filter_map(|i| seekable.entry_name(i).unwrap()).collect::<Vec<_>>(), Vec::<String>::new());
    assert_eq!(seekable.entry_name(0).unwrap(), None);
    assert_eq!(seekable.entry_sizes(0).unwrap(), None);
    assert_eq!(seekable.read_index(0, 10).unwrap(), None);
}

#[test]
fn virtual_names_strip_a_leading_separator_and_still_refuse_traversal() {
    use std::io::Write as _;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("/docProps/core.xml", &b"core"[..]), ("../evil.xml", b"evil"), ("C:/drive.xml", b"drive"), ("/", b"")] {
        zip.start_file(name, opts).unwrap();
        zip.write_all(data).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    let mut archive = Archive::open(&bytes, ROOMY).unwrap();

    // The named API stays closed to a leading separator; the virtual one reads it without it.
    assert_eq!(archive.entry_name(0).unwrap(), None);
    assert_eq!(archive.read("/docProps/core.xml", 100).unwrap(), None);
    assert_eq!(archive.virtual_entry_name(0).unwrap().as_deref(), Some("docProps/core.xml"));
    assert_eq!(archive.virtual_entry_size(0).unwrap(), Some(4));
    assert_eq!(archive.read_virtual_index(0, 100).unwrap().as_deref(), Some(&b"core"[..]));

    // Traversal, drive prefixes and an empty name stay unreadable either way.
    for index in 1..archive.len() {
        assert_eq!(archive.virtual_entry_name(index).unwrap(), None, "entry {index}");
        assert_eq!(archive.virtual_entry_size(index).unwrap(), None, "entry {index}");
        assert_eq!(archive.read_virtual_index(index, 100).unwrap(), None, "entry {index}");
    }
}

#[test]
fn virtual_names_come_from_the_central_directory_and_must_be_utf8() {
    use std::io::Write as _;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("good.xml", &b"good"[..]), ("Xcp437.xml", b"cp437"), ("bad.xml", b"bad")] {
        zip.start_file(name, opts).unwrap();
        zip.write_all(data).unwrap();
    }
    let mut bytes = zip.finish().unwrap().into_inner();
    // A byte that is not UTF-8: `zip` would decode it as CP437 `é`.
    let mut at = 0;
    while let Some(found) = bytes[at..].windows(10).position(|w| w == b"Xcp437.xml") {
        bytes[at + found] = 0x82;
        at += found + 10;
    }
    // Break the local header of `bad.xml`; its central-directory record stays intact.
    let local = find(&bytes, b"bad.xml", 0) - 30;
    assert_eq!(&bytes[local..local + 4], b"PK\x03\x04");
    bytes[local] = b'X';
    let mut archive = Archive::open(&bytes, ROOMY).unwrap();

    assert_eq!(archive.virtual_entry_name(0).unwrap().as_deref(), Some("good.xml"));
    assert_eq!(archive.virtual_entry_name(1).unwrap(), None);
    assert_eq!(archive.virtual_entry_size(1).unwrap(), None);
    assert_eq!(archive.read_virtual_index(1, 100).unwrap(), None);

    // The damaged entry is still named and sized; only reading it fails.
    assert_eq!(archive.virtual_entry_name(2).unwrap().as_deref(), Some("bad.xml"));
    assert_eq!(archive.virtual_entry_size(2).unwrap(), Some(3));
    assert!(archive.read_virtual_index(2, 100).is_err());
    assert_eq!(archive.read_virtual_index(0, 100).unwrap().as_deref(), Some(&b"good"[..]));
}
