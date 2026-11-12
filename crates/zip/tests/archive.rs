//! Reading archives within limits, duplicate names, and the writer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_zip::{Archive, Compression, Error, Limits, Writer, check_entry_count, zip64_entry_count_sample};

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
