use craft_godot_pck::{FormatError, find, header, pack_end, parse, trailer_start};

fn fixture(format: u32, flags: u32) -> Vec<u8> {
    let directory = match format {
        1 => 84,
        2 => 96,
        _ => 40,
    };
    let mut out = vec![0u8; directory + 4];
    out[..4].copy_from_slice(b"GDPC");
    out[4..8].copy_from_slice(&format.to_le_bytes());
    out[8..12].copy_from_slice(&4u32.to_le_bytes());
    if format >= 2 {
        out[20..24].copy_from_slice(&flags.to_le_bytes());
    }
    if format >= 3 {
        out[32..40].copy_from_slice(&(directory as u64).to_le_bytes());
    }
    out[directory..directory + 4].copy_from_slice(&1u32.to_le_bytes());
    let path = b"res://file\0\0";
    out.extend_from_slice(&(path.len() as u32).to_le_bytes());
    out.extend_from_slice(path);
    let body = out.len() as u64 + 32 + if format >= 2 { 4 } else { 0 };
    out.extend_from_slice(&body.to_le_bytes());
    out.extend_from_slice(&3u64.to_le_bytes());
    out.extend_from_slice(&[0xab; 16]);
    if format >= 2 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(b"abc");
    out
}

#[test]
fn all_three_tables_preserve_offsets_and_hashes() {
    for version in [1, 2, 3] {
        let data = fixture(version, 2);
        let pack = parse(&data).unwrap();
        assert_eq!(pack.header.format, version);
        assert_eq!(pack.declared_files, Some(1));
        assert!(pack.table_read && !pack.truncated);
        let member = &pack.members[0];
        assert_eq!(member.path, "file");
        assert_eq!(member.md5.as_deref(), Some("abababababababababababababababab"));
        let offset = member.offset.unwrap() as usize;
        assert_eq!(&data[offset..offset + member.size as usize], b"abc");
    }
}

#[test]
fn embedded_relative_offsets_and_trailer_boundary_are_preserved() {
    for version in [2, 3] {
        let pck = fixture(version, 2);
        let mut exe = vec![0x7f; 32];
        exe.extend_from_slice(&pck);
        exe.extend_from_slice(&(pck.len() as u64).to_le_bytes());
        exe.extend_from_slice(b"GDPC");
        let pack = parse(&exe).unwrap();
        assert_eq!(find(&exe), Some((32, true)));
        assert_eq!(trailer_start(&exe, exe.len() as u64), Some(32));
        assert_eq!(pack_end(exe.len(), true), 32 + pck.len());
        assert_eq!(pack.members[0].offset, Some((32 + pck.len() - 3) as u64));
        let table = if version == 2 { 96 } else { 40 };
        // Move the entire directory to the trailer: the count must not come from it.
        if version == 3 {
            exe[64..72].copy_from_slice(&(pck.len() as u64).to_le_bytes());
            assert!(matches!(parse(&exe), Err(FormatError::Truncated(_))));
        } else {
            // Declare a second member without supplying its record.
            exe[32 + table..36 + table].copy_from_slice(&2u32.to_le_bytes());
            let pack = parse(&exe).unwrap();
            assert!(pack.truncated);
            assert_eq!(pack.members.len(), 1);
        }
    }
}

#[test]
fn unknown_sparse_and_encrypted_tables_remain_unread() {
    for flags in [1, 4, 128] {
        let data = fixture(3, flags);
        let pack = parse(&data).unwrap();
        assert!(!pack.table_read);
        assert!(pack.members.is_empty());
        assert!(!pack.notes.is_empty());
    }
    let mut later = fixture(3, 0);
    later[4..8].copy_from_slice(&4u32.to_le_bytes());
    let pack = parse(&later).unwrap();
    assert!(!pack.table_read && pack.members.is_empty());
}

#[test]
fn every_prefix_and_oversized_offset_is_bounded() {
    for version in [1, 2, 3] {
        let data = fixture(version, 2);
        for length in 0..data.len() {
            let _ = parse(&data[..length]);
            let _ = header(&data[..length]);
            let _ = find(&data[..length]);
        }
    }
    let mut data = fixture(3, 2);
    data[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(parse(&data).is_err());
}
