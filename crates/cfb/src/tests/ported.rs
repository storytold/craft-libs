//! Tests for what the other readers this crate replaces did and the base reader did not: a
//! spreadsheet reader (unpadded last sectors, looping stream chains, a stream size cap, a lenient
//! directory scan), a presentation reader (the same scan) and a word processor's guard in front
//! of a third-party reader (repeated FAT sectors, spare FAT sectors, zero-padded DIFAT sectors).
//! Plus the limits, the listing API and exhaustive truncation and bit flips.

use super::{bare_header, pattern, put_u32};
use crate::{
    CompoundFile, ENDOFCHAIN, EntryKind, Error, FATSECT, FREESECT, Limits, NOSTREAM, SIGNATURE, Version, Writer, directory_names, has_entry_named,
    write_streams,
};

/// A version 3 file written by a second, independent writer with a different layout from
/// [`Writer`]: FAT first, then the directory, the mini FAT, the mini stream and each large stream
/// last, and the root's children chained as right siblings in the order given (a degenerate,
/// unsorted, all-black tree, as simple writers produce). Up to 109 FAT sectors.
fn foreign_file(streams: &[(&str, &[u8])]) -> Vec<u8> {
    const SECTOR: usize = 512;
    const MINI: usize = 64;
    const CUTOFF: usize = 4096;
    let is_mini = |d: &[u8]| d.len() < CUTOFF;

    let mut container = vec![];
    let mut mini_fat: Vec<u32> = vec![];
    let mut mini_starts = vec![];
    for (_, d) in streams {
        if !is_mini(d) || d.is_empty() {
            mini_starts.push(ENDOFCHAIN);
            continue;
        }
        let first = mini_fat.len();
        let n = d.len().div_ceil(MINI);
        for k in 0..n {
            mini_fat.push(if k + 1 == n { ENDOFCHAIN } else { (first + k + 1) as u32 });
        }
        mini_starts.push(first as u32);
        container.extend_from_slice(d);
        container.resize((first + n) * MINI, 0);
    }

    // Regular-sector blobs, in order: directory, mini FAT, mini stream, then each large stream.
    let dir_len = (streams.len() + 1).div_ceil(4) * SECTOR;
    let mut blobs: Vec<Vec<u8>> = vec![vec![0; dir_len], mini_fat.iter().flat_map(|v| v.to_le_bytes()).collect(), container.clone()];
    blobs.extend(streams.iter().map(|(_, d)| if is_mini(d) { vec![] } else { d.to_vec() }));
    let counts: Vec<usize> = blobs.iter().map(|b| b.len().div_ceil(SECTOR)).collect();
    let body: usize = counts.iter().sum();
    let mut fat_sectors = 1;
    while fat_sectors * 128 < fat_sectors + body {
        fat_sectors += 1;
    }
    assert!(fat_sectors <= 109, "the test writer has no DIFAT sectors");
    let mut fat = vec![FREESECT; fat_sectors * 128];
    fat[..fat_sectors].fill(FATSECT);
    let mut starts = vec![];
    let mut next = fat_sectors;
    for &n in &counts {
        if n == 0 {
            starts.push(ENDOFCHAIN);
            continue;
        }
        starts.push(next as u32);
        for k in 0..n {
            fat[next + k] = if k + 1 == n { ENDOFCHAIN } else { (next + k + 1) as u32 };
        }
        next += n;
    }

    let put16 = |b: &mut [u8], at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    let dir = &mut blobs[0];
    for k in 0..dir_len / 128 {
        for at in [68, 72, 76] {
            put_u32(dir, k * 128 + at, NOSTREAM);
        }
    }
    let mut entry = |k: usize, name: &str, kind: u8, start: u32, size: usize| {
        let e = k * 128;
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        dir[e..e + units.len()].copy_from_slice(&units);
        put16(dir, e + 64, units.len() as u16 + 2);
        dir[e + 66] = kind;
        dir[e + 67] = 1; // black
        if kind == 5 && !streams.is_empty() {
            put_u32(dir, e + 76, 1);
        }
        if kind == 2 && k < streams.len() {
            put_u32(dir, e + 72, k as u32 + 1);
        }
        put_u32(dir, e + 116, start);
        put_u32(dir, e + 120, size as u32);
    };
    entry(0, "Root Entry", 5, starts[2], container.len());
    for (k, (name, data)) in streams.iter().enumerate() {
        let start = if is_mini(data) { mini_starts[k] } else { starts[3 + k] };
        entry(k + 1, name, 2, start, data.len());
    }

    let mut out = vec![0u8; SECTOR * (1 + fat_sectors + body)];
    out[..8].copy_from_slice(&SIGNATURE);
    put16(&mut out, 24, 0x3E); // minor version
    put16(&mut out, 26, 3); // major version
    put16(&mut out, 28, 0xFFFE); // byte order
    put16(&mut out, 30, 9); // 512-byte sectors
    put16(&mut out, 32, 6); // 64-byte mini sectors
    put_u32(&mut out, 44, fat_sectors as u32);
    put_u32(&mut out, 48, starts[0]);
    put_u32(&mut out, 56, CUTOFF as u32);
    put_u32(&mut out, 60, starts[1]); // first mini FAT sector (END when there is none)
    put_u32(&mut out, 64, counts[1] as u32);
    put_u32(&mut out, 68, ENDOFCHAIN); // no DIFAT sectors
    for i in 0..109 {
        put_u32(&mut out, 76 + i * 4, if i < fat_sectors { i as u32 } else { FREESECT });
    }
    for (i, v) in fat.iter().enumerate() {
        put_u32(&mut out, SECTOR + i * 4, *v);
    }
    for (blob, start) in blobs.iter().zip(&starts) {
        if !blob.is_empty() {
            let at = SECTOR * (1 + *start as usize);
            out[at..at + blob.len()].copy_from_slice(blob);
        }
    }
    out
}

/// Byte offset of directory entry `i` in a file whose directory is contiguous (both writers').
fn dir_at(b: &[u8], i: usize) -> usize {
    let ss = 1usize << u16::from_le_bytes([b[30], b[31]]);
    (u32::from_le_bytes(b[48..52].try_into().unwrap()) as usize + 1) * ss + i * 128
}

fn invalid_containing(r: crate::Result<Vec<u8>>, what: &str) {
    match r {
        Err(Error::Invalid(m)) if m.contains(what) => {}
        other => panic!("expected an Invalid error containing {what:?}, got {other:?}"),
    }
}

// ---- an independent writer's files -------------------------------------------------------

/// The reader does not depend on the sibling tree being sorted or balanced: a right-linked list
/// in insertion order resolves every name, mini and regular streams alike.
#[test]
fn a_foreign_layout_with_an_unsorted_sibling_list_reads() {
    let streams: Vec<(String, Vec<u8>)> =
        [("Workbook", 9000), ("\u{5}SummaryInformation", 300), ("a", 0), ("Zeta", 4096), ("beta", 4095), ("CompObj", 1)]
            .iter()
            .enumerate()
            .map(|(i, (n, len))| (n.to_string(), pattern(*len, i as u8)))
            .collect();
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    let file = foreign_file(&refs);
    let cf = CompoundFile::open(&file).unwrap();
    for (name, data) in &streams {
        assert_eq!(&cf.read_path(&name.to_uppercase()).unwrap(), data, "{name}");
    }
    let listed: Vec<String> = cf.walk().into_iter().map(|id| cf.path(id).unwrap()).collect();
    assert_eq!(listed, streams.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(), "sibling-tree order");
}

// ---- unpadded last sector ------------------------------------------------------------------

/// One regular-sector stream of `len` bytes (FAT in sector 0, directory in sector 1, the stream
/// from sector 2), cut right after the stream's last byte, the way some writers leave the final
/// sector unpadded.
fn unpadded(len: usize) -> (Vec<u8>, Vec<u8>) {
    let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    let mut file = foreign_file(&[("Workbook", &data)]);
    file.truncate(512 * 3 + len);
    (file, data)
}

#[test]
fn an_unpadded_last_sector_still_reads() {
    let (file, data) = unpadded(5000);
    assert_ne!(file.len() % 512, 0);
    let cf = CompoundFile::open(&file).unwrap();
    assert_eq!(cf.read_path("Workbook").unwrap(), data);
    // Bytes the stream needs that the file doesn't hold are still damage.
    let cf = CompoundFile::open(&file[..file.len() - 1]).unwrap();
    invalid_containing(cf.read_path("Workbook"), "truncated");
}

#[test]
fn an_unpadded_last_mini_sector_still_reads() {
    let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    let mut file = foreign_file(&[("Workbook", &data)]);
    // Header, FAT, directory, mini FAT, then the mini stream holding the stream's bytes.
    file.truncate(512 * 4 + data.len());
    let cf = CompoundFile::open(&file).unwrap();
    assert_eq!(cf.read_path("Workbook").unwrap(), data);
    let cf = CompoundFile::open(&file[..file.len() - 1]).unwrap();
    invalid_containing(cf.read_path("Workbook"), "truncated");
}

#[test]
fn a_cut_sector_inside_a_chain_is_damage() {
    let (mut file, _) = unpadded(5000);
    // The cut sector (11) points on to the directory sector and the stream claims those bytes
    // too: the partial sector would splice 392 bytes into the middle of the stream.
    put_u32(&mut file, 512 + 11 * 4, 1);
    put_u32(&mut file, 1024 + 128 + 120, 5512);
    let cf = CompoundFile::open(&file).unwrap();
    invalid_containing(cf.read_path("Workbook"), "cut short inside a chain");
}

#[test]
fn a_cut_mini_sector_inside_a_chain_is_damage() {
    let small = [7u8; 100];
    let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    let mut file = foreign_file(&[("Small", &small), ("Workbook", &data)]);
    // "Small" holds mini sectors 0-1, "Workbook" 2-17; the mini stream starts at sector 3.
    file.truncate(512 * 4 + 128 + data.len());
    assert_eq!(CompoundFile::open(&file).unwrap().read_path("Workbook").unwrap(), data);
    // The cut mini sector (17) points on to sector 0 and the stream claims those bytes too.
    put_u32(&mut file, 1536 + 17 * 4, 0);
    put_u32(&mut file, 1024 + 2 * 128 + 120, 1064);
    let cf = CompoundFile::open(&file).unwrap();
    invalid_containing(cf.read_path("Workbook"), "cut short inside a chain");
}

/// FAT sectors are never read short: a file cut inside its only FAT sector does not open.
#[test]
fn a_cut_fat_sector_is_refused() {
    let file = foreign_file(&[("a", &[1; 10])]);
    assert!(CompoundFile::open(&file[..1000]).is_err());
}

// ---- loops shorter than the stream ---------------------------------------------------------

/// A chain that loops inside the stream's declared size used to read back as the same sectors
/// over and over. It is damage.
#[test]
fn a_stream_chain_that_loops_within_its_size_is_refused() {
    let data = pattern(5000, 1);
    let mut file = foreign_file(&[("Workbook", &data)]);
    assert_eq!(CompoundFile::open(&file).unwrap().read_path("Workbook").unwrap(), data);
    put_u32(&mut file, 512 + 3 * 4, 2); // FAT[3] = 2: sectors 2, 3, 2, 3, ...
    invalid_containing(CompoundFile::open(&file).unwrap().read_path("Workbook"), "sector chain loops: sector 2 is visited twice");
}

#[test]
fn a_mini_chain_that_loops_within_its_size_is_refused() {
    let data = pattern(200, 1);
    let mut file = foreign_file(&[("Small", &data)]);
    assert_eq!(CompoundFile::open(&file).unwrap().read_path("Small").unwrap(), data);
    put_u32(&mut file, 1536 + 4, 0); // mini FAT[1] = 0: mini sectors 0, 1, 0, 1
    invalid_containing(CompoundFile::open(&file).unwrap().read_path("Small"), "mini sector chain loops");
}

// ---- a damaged mini stream -----------------------------------------------------------------

/// A mini stream that cannot be read, or is shorter than the root entry says, does not stop the
/// file from opening: the streams in regular sectors still read.
#[test]
fn a_damaged_mini_stream_only_fails_the_streams_in_it() {
    let (small, big) = (pattern(100, 1), pattern(5000, 2));
    let mut file = foreign_file(&[("Small", &small), ("Workbook", &big)]);
    let root = dir_at(&file, 0);
    // the root entry claims a mini stream far longer than its chain: what is there still reads
    put_u32(&mut file, root + 120, 1 << 20);
    let cf = CompoundFile::open(&file).unwrap();
    assert_eq!(cf.read_path("Small").unwrap(), small);
    // the mini stream starts at a sector outside the file
    put_u32(&mut file, root + 116, 5000);
    let cf = CompoundFile::open(&file).unwrap();
    assert_eq!(cf.read_path("Workbook").unwrap(), big);
    invalid_containing(cf.read_path("Small"), "sector 5000 has no FAT entry");
}

// ---- stream size cap and the other limits --------------------------------------------------

#[test]
fn stream_size_limit() {
    let data = pattern(5000, 3);
    let file = write_streams(Version::V3, &[("s", &data), ("t", &[1, 2, 3])]).unwrap();
    let over = |r: crate::Result<Vec<u8>>| assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    let cf = CompoundFile::open_with(&file, Limits { max_stream_size: 4999, ..Limits::default() }).unwrap();
    over(cf.read_path("s"));
    assert_eq!(cf.read_path("t").unwrap(), [1, 2, 3], "a stream under the limit");
    let cf = CompoundFile::open_with(&file, Limits { max_stream_size: 5000, ..Limits::default() }).unwrap();
    assert_eq!(cf.read_path("s").unwrap(), data);
    // the per-call cap
    over(cf.read_path_capped("s", 4999));
    over(cf.read_capped(cf.find("s").unwrap(), 0));
    assert_eq!(cf.read_path_capped("s", 5000).unwrap(), data);
    assert_eq!(cf.read_path_capped("s", u64::MAX).unwrap(), data, "a cap above the limit does not raise it");
}

/// The default limit is 1 GiB, and a size the file cannot hold is refused before any allocation
/// whatever the limit.
#[test]
fn a_huge_declared_size_is_refused_without_allocating() {
    assert_eq!(Limits::default(), Limits { max_stream_size: 1 << 30, max_dir_entries: 1 << 20 });
    let mut file = write_streams(Version::V4, &[("s", &pattern(5000, 3))]).unwrap();
    let e = dir_at(&file, 1);
    file[e + 120..e + 128].copy_from_slice(&((1u64 << 30) + 1).to_le_bytes());
    assert!(matches!(CompoundFile::open(&file).unwrap().read_path("s"), Err(Error::LimitExceeded(_))));
    let cf = CompoundFile::open_with(&file, Limits { max_stream_size: u64::MAX, ..Limits::default() }).unwrap();
    invalid_containing(cf.read_path("s"), "larger than the file");
    file[e + 120..e + 128].copy_from_slice(&u64::MAX.to_le_bytes());
    let cf = CompoundFile::open_with(&file, Limits { max_stream_size: u64::MAX, ..Limits::default() }).unwrap();
    invalid_containing(cf.read_path("s"), "larger than the file");
}

#[test]
fn directory_entry_limit() {
    let streams: Vec<(String, Vec<u8>)> = (0..10).map(|i| (format!("s{i}"), vec![i as u8])).collect();
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    // 11 entries: three 512-byte directory sectors, so 12 entries with the unused one
    let file = write_streams(Version::V3, &refs).unwrap();
    let with = |n| CompoundFile::open_with(&file, Limits { max_dir_entries: n, ..Limits::default() });
    assert!(matches!(with(11), Err(Error::LimitExceeded(_))));
    assert_eq!(with(12).unwrap().walk().len(), 10);
    assert_eq!(
        directory_names(&file, &Limits { max_dir_entries: 4, ..Limits::default() }).unwrap(),
        ["Root Entry", "s0", "s1", "s2"],
        "the scan reports what it read"
    );
}

/// Only 512- and 4096-byte sectors exist. Other shifts used to be accepted (down to 128-byte
/// sectors, which overlap the header).
#[test]
fn only_sector_shifts_9_and_12_are_accepted() {
    for (version, good) in [(Version::V3, 9u16), (Version::V4, 12)] {
        let mut file = write_streams(version, &[("s", &[1; 10])]).unwrap();
        assert!(CompoundFile::open(&file).is_ok());
        for shift in (0..=20).filter(|s| *s != 9 && *s != 12) {
            file[30..32].copy_from_slice(&(shift as u16).to_le_bytes());
            assert_eq!(CompoundFile::open(&file).unwrap_err(), Error::Invalid(format!("sector shift {shift}")));
            assert_eq!(directory_names(&file, &Limits::default()), None);
        }
        file[30..32].copy_from_slice(&good.to_le_bytes());
        file[32..34].copy_from_slice(&7u16.to_le_bytes());
        assert_eq!(CompoundFile::open(&file).unwrap_err(), Error::Invalid("mini sector shift 7".into()));
    }
}

#[test]
fn a_wrong_byte_order_mark_is_not_a_compound_file() {
    let mut file = write_streams(Version::V3, &[("s", &[1; 10])]).unwrap();
    file[28] = 0xFF;
    file[29] = 0xFE;
    assert_eq!(CompoundFile::open(&file).unwrap_err(), Error::NotCfb);
}

/// Before version 4 a size is 32 bits and writers may leave anything in the high half (§2.6.3).
#[test]
fn version_3_ignores_the_high_half_of_a_size() {
    let data = pattern(5000, 9);
    let mut file = foreign_file(&[("Workbook", &data)]);
    let e = dir_at(&file, 1);
    put_u32(&mut file, e + 124, 0xDEAD_BEEF);
    assert_eq!(CompoundFile::open(&file).unwrap().read_path("Workbook").unwrap(), data);
    // a version field below 3 is treated the same way
    file[26] = 2;
    assert_eq!(CompoundFile::open(&file).unwrap().read_path("Workbook").unwrap(), data);
}

// ---- the directory tree --------------------------------------------------------------------

/// Three streams a, b, c: `b` (entry 2) is the root of the sibling tree, `a` and `c` its leaves.
fn three() -> Vec<u8> {
    write_streams(Version::V3, &[("a", b"A"), ("b", b"B"), ("c", b"C")]).unwrap()
}

/// A sibling link to an entry that does not exist is ignored; the rest of the tree reads.
#[test]
fn a_dangling_sibling_link_is_ignored() {
    let mut file = three();
    let c = dir_at(&file, 3);
    put_u32(&mut file, c + 68, 1000); // c.left
    put_u32(&mut file, c + 72, 0x7FFF_FFFF); // c.right
    let cf = CompoundFile::open(&file).unwrap();
    for (name, data) in [("a", b"A"), ("b", b"B"), ("c", b"C")] {
        assert_eq!(cf.read_path(name).unwrap(), data);
    }
    // a dangling child link leaves an empty storage
    let root = dir_at(&file, 0);
    put_u32(&mut file, root + 76, 4);
    let cf = CompoundFile::open(&file).unwrap();
    assert!(cf.walk().is_empty());
}

/// A link to an entry already in the tree (a cycle or a shared subtree) is refused.
#[test]
fn an_entry_referenced_twice_is_refused() {
    for target in [0u32, 1, 2, 3] {
        let mut file = three();
        let c = dir_at(&file, 3);
        put_u32(&mut file, c + 72, target); // c.right: the root, a sibling, the tree root, itself
        assert_eq!(CompoundFile::open(&file).unwrap_err(), Error::Invalid(format!("directory entry {target} is referenced twice")));
    }
}

/// Entries the root does not reach are not walked, so a leftover storage entry that points into
/// the live tree (or at itself) does not make the file unreadable.
#[test]
fn an_orphan_entry_does_not_break_the_tree() {
    let mut file = write_streams(Version::V3, &[("a", b"A"), ("b", b"B")]).unwrap();
    let orphan = dir_at(&file, 3); // the unused fourth entry of the directory sector
    file[orphan] = b'z';
    file[orphan + 64] = 4;
    file[orphan + 66] = 1; // a storage
    for child in [1u32, 3] {
        put_u32(&mut file, orphan + 76, child);
        let cf = CompoundFile::open(&file).unwrap();
        assert_eq!(cf.read_path("a").unwrap(), b"A");
        assert_eq!(cf.walk().len(), 2);
        assert_eq!(cf.path(3), None, "not below the root");
        assert_eq!(cf.entries[3].parent, None);
        assert!(cf.entries[3].children.is_empty());
    }
}

// ---- listing -------------------------------------------------------------------------------

#[test]
fn walk_lists_storages_and_streams_with_paths() {
    let file = write_streams(
        Version::V4,
        &[("Data", b"d"), ("Macros/VBA/dir", b"x"), ("/Macros/VBA/ThisDocument", b"y"), ("Macros/PROJECT", b"z"), ("\u{1}CompObj", b"c")],
    )
    .unwrap();
    let cf = CompoundFile::open(&file).unwrap();
    let listed: Vec<(String, EntryKind)> = cf.walk().into_iter().map(|id| (cf.path(id).unwrap(), cf.entries[id].kind)).collect();
    use EntryKind::{Storage, Stream};
    // siblings in [MS-CFB] order: shorter names first, then case-insensitive code units
    let expect = [
        ("Data", Stream),
        ("Macros", Storage),
        ("Macros/VBA", Storage),
        ("Macros/VBA/dir", Stream),
        ("Macros/VBA/ThisDocument", Stream),
        ("Macros/PROJECT", Stream),
        ("\u{1}CompObj", Stream),
    ];
    assert_eq!(listed, expect.map(|(p, k)| (p.to_string(), k)));
    for id in cf.walk() {
        assert_eq!(cf.find(&cf.path(id).unwrap()).unwrap(), id, "find is the inverse of path");
    }
    assert_eq!(cf.path(0).as_deref(), Some(""));
    assert_eq!(cf.path(cf.entries.len()), None);
    assert!(cf.has_stream("macros/vba/DIR"));
    assert!(cf.has_stream("/Data"));
    assert!(!cf.has_stream("Macros"), "a storage");
    assert!(!cf.has_stream("Macros/nope"));
    assert!(!cf.has_stream(""), "the root");
    assert_eq!(cf.read_path("/Macros/VBA/ThisDocument").unwrap(), b"y");
}

#[test]
fn stream_at_rules() {
    let mut w = Writer::default();
    let s = w.storage(Writer::ROOT, "Existing", [9; 16]).unwrap();
    w.stream_at("existing/inner", vec![1]).unwrap();
    assert!(matches!(w.stream_at("Existing/INNER", vec![2]), Err(Error::BadName(_))), "a duplicate");
    assert!(matches!(w.stream_at("Existing/inner/below", vec![2]), Err(Error::Invalid(_))), "a stream is not a storage");
    assert!(matches!(w.stream_at("", vec![]), Err(Error::BadName(_))));
    assert!(matches!(w.stream_at("a/b:c", vec![]), Err(Error::BadName(_))));
    assert!(matches!(write_streams(Version::V3, &[("x", b"1"), ("X", b"2")]), Err(Error::BadName(_))));
    let _ = s;
    let file = w.finish();
    let cf = CompoundFile::open(&file).unwrap();
    assert_eq!(cf.entries[cf.find("Existing").unwrap()].clsid, [9; 16], "the existing storage was reused");
    assert_eq!(cf.read_path("Existing/inner").unwrap(), [1]);
}

// ---- the FAT sector list -------------------------------------------------------------------

/// A file of about 1 MB whose DIFAT chain lists one FAT sector 2048 x 127 times. Built naively,
/// its FAT is about 130 MB.
fn repeating_difat_sample(claimed_fat_sectors: u32) -> Vec<u8> {
    const S: usize = 512;
    const CHAIN: u32 = 2048; // DIFAT sectors 1..=CHAIN, sector 0 is the "FAT"
    let mut f = vec![0u8; S * (CHAIN as usize + 2)];
    f[..512].copy_from_slice(&bare_header(9));
    put_u32(&mut f, 0x2C, claimed_fat_sectors);
    put_u32(&mut f, 0x44, 1);
    put_u32(&mut f, 0x48, CHAIN);
    f[0x4C..512].fill(0); // the 109 header entries are all 0; each chain sector's entries too
    for d in 1..=CHAIN {
        put_u32(&mut f, (d as usize + 2) * S - 4, if d == CHAIN { ENDOFCHAIN } else { d + 1 });
    }
    f
}

#[test]
fn a_repeating_difat_is_refused() {
    let err = |n| CompoundFile::open(&repeating_difat_sample(n)).unwrap_err();
    // more FAT sectors than the file has sectors
    assert_eq!(err(u32::MAX), Error::Invalid(format!("4294967295 FAT sectors in a {}-byte file", 512 * 2050)));
    assert_eq!(err(2052), Error::Invalid(format!("2052 FAT sectors in a {}-byte file", 512 * 2050)));
    // a count the file could hold: the list is still one sector over and over
    assert_eq!(err(2050), Error::Invalid("FAT sector 0 is listed twice".into()));
    assert_eq!(err(2), Error::Invalid("FAT sector 0 is listed twice".into()));
    // the lenient scan never builds a FAT: it walks the DIFAT chain once and finds no directory
    assert_eq!(directory_names(&repeating_difat_sample(2050), &Limits::default()), Some(vec![]));
}

/// Append `spare` free FAT sectors to a file with fewer than 109 and list them in the header.
fn with_spare_fat_sectors(mut file: Vec<u8>, spare: u32) -> Vec<u8> {
    let n_fat = u32::from_le_bytes(file[44..48].try_into().unwrap());
    let sectors = (file.len() / 512 - 1) as u32;
    for i in 0..spare {
        put_u32(&mut file, 76 + (n_fat + i) as usize * 4, sectors + i);
        file.extend_from_slice(&[0xFF; 512]);
    }
    put_u32(&mut file, 44, n_fat + spare);
    file
}

/// Writers reserve FAT sectors for growth: a real encrypted package of 78 sectors lists 4, far
/// more than its size needs. Only a list longer than the file, or a repeat, is refused.
#[test]
fn spare_fat_sectors_are_accepted() {
    let data = pattern(38_000, 4);
    let file = with_spare_fat_sectors(write_streams(Version::V3, &[("EncryptedPackage", &data)]).unwrap(), 3);
    assert_eq!(file.len() / 512 - 1, 80);
    assert_eq!(u32::from_le_bytes(file[44..48].try_into().unwrap()), 4);
    assert_eq!(CompoundFile::open(&file).unwrap().read_path("EncryptedPackage").unwrap(), data);
    // the same sector listed for two of them is a repeat
    let mut twice = file.clone();
    let last = u32::from_le_bytes(twice[76 + 12..76 + 16].try_into().unwrap());
    put_u32(&mut twice, 76 + 8, last);
    assert_eq!(CompoundFile::open(&twice).unwrap_err(), Error::Invalid(format!("FAT sector {last} is listed twice")));
    // more FAT sectors claimed than the file has sectors
    let mut many = file;
    put_u32(&mut many, 44, 83);
    assert!(matches!(CompoundFile::open(&many), Err(Error::Invalid(m)) if m.starts_with("83 FAT sectors")));
}

/// Some writers pad the last DIFAT sector with zeros, not FREESECT. Sector 0 is a real sector
/// (here the first of the stream), so the padding must not be read as FAT sector numbers: the
/// header's FAT sector count says where the list ends.
#[test]
fn a_zero_padded_difat_sector_is_accepted() {
    let big = pattern(7_500_000, 5);
    let mut file = write_streams(Version::V3, &[("big", &big)]).unwrap();
    assert_eq!(u32::from_le_bytes(file[72..76].try_into().unwrap()), 1, "one DIFAT sector");
    let difat = (u32::from_le_bytes(file[68..72].try_into().unwrap()) as usize + 1) * 512;
    let mut padded = 0;
    for i in 0..127 {
        if file[difat + i * 4..difat + i * 4 + 4] == [0xFF; 4] {
            put_u32(&mut file, difat + i * 4, 0);
            padded += 1;
        }
    }
    assert!(padded > 0);
    assert_eq!(CompoundFile::open(&file).unwrap().read_path("big").unwrap(), big);
}

// ---- the lenient directory scan ------------------------------------------------------------

/// A minimal file (512-byte sectors: FAT at sector 0, directory at sector 1) whose directory
/// holds a root entry plus the given stream names, linked to nothing: no storage's tree reaches
/// them, and they have no contents.
fn unlinked(streams: &[&str]) -> Vec<u8> {
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|n| (*n, &[][..])).collect();
    let mut b = foreign_file(&refs);
    let root = dir_at(&b, 0);
    put_u32(&mut b, root + 76, NOSTREAM);
    b
}

#[test]
fn scan_reads_directory_names() {
    let limits = Limits::default();
    let b = unlinked(&["EncryptionInfo", "EncryptedPackage"]);
    assert_eq!(directory_names(&b, &limits).unwrap(), ["Root Entry", "EncryptionInfo", "EncryptedPackage"]);
    assert!(CompoundFile::open(&b).unwrap().walk().is_empty(), "the tree reaches none of them");
    assert!(has_entry_named(&b, "EncryptedPackage", &limits));
    assert!(has_entry_named(&b, "encryptedpackage", &limits), "compared as sibling names are");
    assert!(!has_entry_named(&b, "Encrypted", &limits), "a whole name, not a prefix");
    let plain = unlinked(&["Workbook", "\u{5}SummaryInformation"]);
    assert!(!has_entry_named(&plain, "EncryptedPackage", &limits));
    assert!(has_entry_named(&plain, "\u{5}SummaryInformation", &limits));
    // not a compound file: no answer, and no byte search either
    assert_eq!(directory_names(b"PK\x03\x04", &limits), None);
    assert!(!has_entry_named(b"PK\x03\x04", "EncryptedPackage", &limits));
    let mut zip_with_the_name = b"PK\x03\x04".to_vec();
    zip_with_the_name.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
    assert!(!has_entry_named(&zip_with_the_name, "EncryptedPackage", &limits));
    // nested entries are listed too
    let nested = write_streams(Version::V4, &[("a/b/c", b"1")]).unwrap();
    assert_eq!(directory_names(&nested, &limits).unwrap(), ["Root Entry", "a", "b", "c"]);
}

/// A file whose directory cannot be followed still says whether it mentions the name.
#[test]
fn scan_falls_back_to_a_byte_search() {
    let limits = Limits::default();
    let b = unlinked(&["EncryptionInfo", "EncryptedPackage"]);
    for damaged in [&b[..1024 + 300], &b[..1024 + 128 + 32]] {
        assert_eq!(directory_names(damaged, &limits), None);
        assert!(has_entry_named(damaged, "EncryptionInfo", &limits));
        assert!(!has_entry_named(damaged, "Workbook", &limits));
    }
    assert!(!has_entry_named(&b[..1024], "EncryptionInfo", &limits), "cut before the directory");
    assert!(!has_entry_named(&b[..300], "", &limits));
}

#[test]
fn scan_survives_hostile_headers() {
    let limits = Limits::default();
    let mut b = unlinked(&["Workbook"]);
    for at in [30usize, 48, 68, 76, 512, 516, 1024 + 64] {
        for v in [0u32, 1, 7, 0xFFFF_FFFA, 0xFFFF_FFFF, 0x7FFF_FFFF] {
            let mut c = b.clone();
            put_u32(&mut c, at, v);
            let _ = has_entry_named(&c, "EncryptedPackage", &limits);
            for cut in [0usize, 8, 100, 600, 1100] {
                let _ = has_entry_named(&c[..cut.min(c.len())], "EncryptedPackage", &limits);
            }
        }
    }
    // A directory chain that loops back on itself stops at the bound and reports what it read.
    put_u32(&mut b, 516, 1);
    assert!(!has_entry_named(&b, "EncryptedPackage", &limits));
    assert!(has_entry_named(&b, "Workbook", &limits));
    // A DIFAT sector that points at itself stops at the bound too.
    let mut b = unlinked(&["Workbook"]);
    put_u32(&mut b, 68, 1);
    assert!(directory_names(&b, &limits).is_some());
}

// ---- exhaustive damage ---------------------------------------------------------------------

/// Everything the crate offers on bytes that may be anything. Streams that read back from a file
/// that is only cut short must be complete.
fn exercise(bytes: &[u8], intact: Option<&[(&str, &[u8])]>) {
    let limits = Limits::default();
    let _ = directory_names(bytes, &limits);
    let _ = has_entry_named(bytes, "big", &limits);
    let Ok(cf) = CompoundFile::open(bytes) else { return };
    for id in cf.walk() {
        let _ = cf.path(id);
    }
    for id in 0..cf.entries.len() {
        let _ = cf.read(id);
        let _ = cf.path(id);
    }
    for (path, data) in intact.unwrap_or(&[]) {
        if let Ok(d) = cf.read_path(path) {
            assert_eq!(&d, data, "{path} read back from a cut file");
        }
    }
}

type Streams = Vec<(&'static str, Vec<u8>)>;

fn small_samples() -> Vec<(Vec<u8>, Streams)> {
    let streams = vec![("a/b", pattern(100, 1)), ("big", pattern(5000, 2)), ("c", pattern(10, 3)), ("a/empty", vec![])];
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    let roots: Vec<(&str, &[u8])> = refs.iter().filter(|(n, _)| !n.contains('/')).copied().collect();
    vec![
        (write_streams(Version::V3, &refs).unwrap(), streams.clone()),
        (foreign_file(&roots), streams.into_iter().filter(|(n, _)| !n.contains('/')).collect()),
    ]
}

#[test]
fn truncation_at_every_length_never_panics() {
    for (file, streams) in small_samples() {
        let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (*n, d.as_slice())).collect();
        for cut in 0..=file.len() {
            exercise(&file[..cut], Some(&refs));
        }
    }
    // version 4, where one sector holds what several did
    let file = write_streams(Version::V4, &[("a/b", &pattern(100, 1)), ("big", &pattern(5000, 2))]).unwrap();
    for cut in (0..=file.len()).step_by(7) {
        exercise(&file[..cut], None);
    }
}

#[test]
fn every_single_bit_flip_never_panics_or_hangs() {
    for (mut file, _) in small_samples() {
        for bit in 0..file.len() * 8 {
            file[bit / 8] ^= 1 << (bit % 8);
            exercise(&file, None);
            file[bit / 8] ^= 1 << (bit % 8);
        }
    }
}

/// Every header and directory field, and the FAT (the first sector in one sample, the last in the
/// other), set to each of the values that mean something special.
#[test]
fn special_values_in_every_field_never_panic() {
    for (file, _) in small_samples() {
        let dir = dir_at(&file, 0);
        let fields =
            (0..512).step_by(4).chain((dir..dir + 5 * 128).step_by(4)).chain((512..1024).step_by(4)).chain((file.len() - 512..file.len()).step_by(4));
        for at in fields {
            for v in [0u32, 1, 2, 3, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFA, 0xFFFF_FFFB, 0xFFFF_FFFC, 0xFFFF_FFFD, 0xFFFF_FFFE, 0xFFFF_FFFF] {
                let mut g = file.clone();
                put_u32(&mut g, at, v);
                exercise(&g, None);
            }
        }
    }
}
