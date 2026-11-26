// ---- from Craft Anything crates/formats/tests/tar_gnu_names.rs
mod tar_gnu_names {
    use crate::testkit as fx;
    use crate::{member_data, tar};

    fn checksum(header: &mut [u8]) {
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    }

    #[test]
    fn old_gnu_timestamps_are_not_posix_path_prefixes() {
        let mut archive = fx::tar(&[("program", b"payload")]);
        let header = &mut archive[..512];
        header[257..265].copy_from_slice(b"ustar  \0");
        header[345..357].copy_from_slice(b"00000000123\0");
        header[357..369].copy_from_slice(b"00000000456\0");
        checksum(header);
        let listing = tar(&archive, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(listing.members[0].name, "program");
        assert_eq!(member_data(&archive, &listing.members[0]), Some(b"payload".as_slice()));
    }

    #[test]
    fn posix_ustar_prefixes_are_still_joined() {
        let mut archive = fx::tar(&[("program", b"payload")]);
        let header = &mut archive[..512];
        header[345..352].copy_from_slice(b"usr/bin");
        checksum(header);
        assert_eq!(tar(&archive, 10).unwrap().members[0].name, "usr/bin/program");
    }
}

// ---- from Craft Anything crates/formats/tests/tar_local_pax_size.rs
mod tar_local_pax_size {
    use crate::testkit::tar_entries;
    use crate::{member_data, tar};

    /// Encode the decimal byte count that includes the PAX record itself.
    fn record(value: &str) -> String {
        let body = format!(" size={value}\n");
        let mut size = body.len() + 1;
        loop {
            let result = format!("{size}{body}");
            if result.len() == size {
                return result;
            }
            size = result.len();
        }
    }

    /// Invalid local sizes cannot silently fall back to a plausible member header.
    #[test]
    fn malformed_local_sizes_stop_before_the_affected_member() {
        for value in ["no", "-1", "+3", " 3", "3 ", "3.0", "18446744073709551616"] {
            let pax = record(value);
            let bytes = tar_entries(&[("keep", b'0', b"ok", ""), ("PaxHeader", b'x', pax.as_bytes(), ""), ("affected", b'0', b"abc", "")]);
            let report = tar(&bytes, 10).unwrap();
            assert!(!report.complete, "size={value}");
            assert_eq!(report.members.len(), 1);
            assert_eq!(report.members[0].name, "keep");
            assert_eq!(member_data(&bytes, &report.members[0]), Some(b"ok".as_slice()));
            assert!(report.warnings.iter().any(|w| w.contains("pax size")));
        }
    }

    /// Decimal sizes, zero, and the existing empty override behavior still work.
    #[test]
    fn valid_local_sizes_remain_readable() {
        for (size, body) in [("3", b"abc".as_slice()), ("0003", b"abc"), ("0", b""), ("", b"abc")] {
            let pax = record(size);
            let bytes = tar_entries(&[("PaxHeader", b'x', pax.as_bytes(), ""), ("file", b'0', body, "")]);
            let report = tar(&bytes, 10).unwrap();
            assert!(report.complete, "{:?}", report.warnings);
            assert_eq!(member_data(&bytes, &report.members[0]), Some(body));
        }
    }
}

// ---- from Craft Anything crates/formats/tests/tar_pax_size.rs
mod tar_pax_size {
    use crate::testkit as fx;
    use crate::{member_data, tar};

    fn clear_member_size(archive: &mut [u8], offset: usize) {
        let header = &mut archive[offset..offset + 512];
        header[124..136].copy_from_slice(b"00000000000\0");
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    }

    #[test]
    fn pax_size_survives_a_gnu_long_name_record() {
        let mut archive = fx::tar_entries(&[
            ("PaxHeader", b'x', b"10 size=3\n", ""),
            ("././@LongLink", b'L', b"renamed\0", ""),
            ("short", b'0', b"abc", ""),
            ("next", b'0', b"later", ""),
        ]);
        clear_member_size(&mut archive, 2048);
        let listing = tar(&archive, 10).unwrap();
        assert!(listing.complete, "{:?}", listing.warnings);
        assert_eq!(listing.members.len(), 2);
        assert_eq!(listing.members[0].name, "renamed");
        assert_eq!(listing.members[0].size, 3);
        assert_eq!(member_data(&archive, &listing.members[0]), Some(b"abc".as_slice()));
        assert_eq!(listing.members[1].size, 5);
        assert_eq!(member_data(&archive, &listing.members[1]), Some(b"later".as_slice()));
    }

    #[test]
    fn pax_size_without_intervening_metadata_still_applies_once() {
        let mut archive = fx::tar_entries(&[("PaxHeader", b'x', b"10 size=3\n", ""), ("short", b'0', b"abc", ""), ("next", b'0', b"later", "")]);
        clear_member_size(&mut archive, 1024);
        let listing = tar(&archive, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(listing.members.iter().map(|m| m.size).collect::<Vec<_>>(), vec![3, 5]);
    }

    fn pax_record(key: &str, value: &str) -> String {
        let content = format!(" {key}={value}\n");
        let mut len = content.len() + 1;
        loop {
            let record = format!("{len}{content}");
            if record.len() == len {
                return record;
            }
            len = record.len();
        }
    }

    #[test]
    fn malformed_pax_framing_stops_the_listing() {
        for invalid in [b"10 size=3!".as_slice(), b"10 size=3!\n", b"20 size=3\n", b"+12 path=no\n", b"5 =x\n", b"12 path=a=b\n11 path=bad"] {
            let archive = fx::tar_entries(&[("keep", b'0', b"one", ""), ("PaxHeader", b'x', invalid, ""), ("after", b'0', b"two", "")]);
            let listing = tar(&archive, 10).unwrap();
            assert!(!listing.complete);
            assert_eq!(listing.members.len(), 1);
            assert_eq!(listing.members[0].name, "keep");
            assert!(listing.warnings.iter().any(|s| s.contains("pax")));
        }
    }

    #[test]
    fn pax_path_can_contain_an_equals_sign() {
        let archive = fx::tar_entries(&[("PaxHeader", b'x', b"12 path=a=b\n", ""), ("short", b'0', b"ok", "")]);
        let listing = tar(&archive, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(listing.members[0].name, "a=b");
    }

    #[test]
    fn the_name_limit_applies_only_to_pax_fields_kept_as_names() {
        let exact = "a".repeat(4096);
        let large = "b".repeat(100_000);
        let pax = pax_record("comment", &large) + &pax_record("path", &exact);
        let archive = fx::tar_entries(&[("PaxHeader", b'x', pax.as_bytes(), ""), ("short", b'0', b"ok", "")]);
        let listing = tar(&archive, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(listing.members[0].name, exact);
        for key in ["path", "linkpath"] {
            let pax = pax_record(key, &large);
            let archive = fx::tar_entries(&[("PaxHeader", b'x', pax.as_bytes(), ""), ("short", b'2', b"", "link")]);
            let listing = tar(&archive, 10).unwrap();
            assert!(!listing.complete);
            assert!(listing.members.is_empty());
            assert!(listing.warnings.iter().any(|s| s.contains("4096")));
        }
    }

    #[test]
    fn empty_pax_values_remove_an_earlier_override() {
        let pax = pax_record("path", "other") + &pax_record("path", "") + &pax_record("linkpath", "first") + &pax_record("linkpath", "");
        let archive = fx::tar_entries(&[("PaxHeader", b'x', pax.as_bytes(), ""), ("original", b'2', b"", "target")]);
        let listing = tar(&archive, 10).unwrap();
        assert!(listing.complete);
        assert!(listing.members[0].name.is_empty());
        assert!(listing.members[0].link_target.is_none());
    }

    #[test]
    fn tar_sizes_allow_outer_padding_but_do_not_join_separated_digits() {
        for (field, valid) in [(b" 000000001\0 ", true), (b" 0000 0001\0 ", false), (b" 0000\x000001\0 ", false)] {
            let mut archive = fx::tar_entries(&[("one", b'0', b"1", ""), ("two", b'0', b"2", "")]);
            let header = &mut archive[..512];
            header[124..136].copy_from_slice(field);
            header[148..156].fill(b' ');
            let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
            header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
            let listing = tar(&archive, 10).unwrap();
            assert_eq!(listing.complete, valid);
            assert_eq!(listing.members.len(), if valid { 2 } else { 0 });
        }
    }

    #[test]
    fn excess_pax_records_do_not_hide_a_later_override() {
        let mut records = pax_record("comment", "").repeat(256);
        for excess in [false, true] {
            if excess {
                records.push_str(&pax_record("size", "9999"));
            }
            let archive = fx::tar_entries(&[("PaxHeader", b'x', records.as_bytes(), ""), ("member", b'0', b"ok", "")]);
            let listing = tar(&archive, 10).unwrap();
            assert_eq!(listing.complete, !excess);
            assert_eq!(listing.members.len(), usize::from(!excess));
        }
    }
}

// ---- from Craft Anything crates/formats/tests/tar_global_pax.rs
mod tar_global_pax {
    use crate::testkit as fx;
    use crate::{MemberKind, member_data, tar};

    fn record(key: &str, value: &str) -> String {
        let tail = format!(" {key}={value}\n");
        let mut len = tail.len() + 1;
        loop {
            let record = format!("{len}{tail}");
            if record.len() == len {
                return record;
            }
            len = record.len();
        }
    }

    fn clear_size(data: &mut [u8], at: usize) {
        let header = &mut data[at..at + 512];
        header[124..136].copy_from_slice(b"00000000000\0");
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    }

    #[test]
    fn global_path_and_size_persist_across_members_and_metadata() {
        let global = record("path", "global-name") + &record("size", "3");
        let local = record("comment", "unrelated");
        let mut data = fx::tar_entries(&[
            ("global", b'g', global.as_bytes(), ""),
            ("first", b'0', b"abc", ""),
            ("metadata", b'x', local.as_bytes(), ""),
            ("second", b'0', b"def", ""),
        ]);
        clear_size(&mut data, 1024);
        clear_size(&mut data, 3072);
        let listing = tar(&data, 10).unwrap();
        assert!(listing.complete, "{:?}", listing.warnings);
        assert_eq!(listing.members.len(), 2);
        assert!(listing.members.iter().all(|member| member.name == "global-name" && member.size == 3));
        assert_eq!(member_data(&data, &listing.members[0]), Some(b"abc".as_slice()));
        assert_eq!(member_data(&data, &listing.members[1]), Some(b"def".as_slice()));
    }

    #[test]
    fn local_attributes_override_only_one_member() {
        let global = record("path", "inherited") + &record("size", "3");
        let local = record("path", "one-member") + &record("size", "0");
        let data = fx::tar_entries(&[
            ("global", b'g', global.as_bytes(), ""),
            ("local", b'x', local.as_bytes(), ""),
            ("first", b'0', b"", ""),
            ("second", b'0', b"abc", ""),
        ]);
        let listing = tar(&data, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(listing.members.iter().map(|m| (m.name.as_str(), m.size)).collect::<Vec<_>>(), [("one-member", 0), ("inherited", 3)]);
    }

    #[test]
    fn later_global_records_update_and_delete_only_their_keys() {
        let first = record("path", "first-global") + &record("size", "3");
        let second = record("path", "second-global");
        let remove = record("path", "") + &record("size", "");
        let data = fx::tar_entries(&[
            ("global", b'g', first.as_bytes(), ""),
            ("one", b'0', b"abc", ""),
            ("global", b'g', second.as_bytes(), ""),
            ("two", b'0', b"def", ""),
            ("global", b'g', remove.as_bytes(), ""),
            ("plain", b'0', b"normal", ""),
        ]);
        let listing = tar(&data, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(
            listing.members.iter().map(|m| (m.name.as_str(), m.size)).collect::<Vec<_>>(),
            [("first-global", 3), ("second-global", 3), ("plain", 6)]
        );
    }

    #[test]
    fn global_link_names_persist_until_deleted() {
        let global = record("linkpath", "shared-target");
        let local = record("linkpath", "local-target");
        let remove = record("linkpath", "");
        let data = fx::tar_entries(&[
            ("global", b'g', global.as_bytes(), ""),
            ("one", b'2', b"", "fallback"),
            ("local", b'x', local.as_bytes(), ""),
            ("two", b'1', b"", "fallback"),
            ("three", b'2', b"", "fallback"),
            ("global", b'g', remove.as_bytes(), ""),
            ("four", b'2', b"", "original"),
        ]);
        let listing = tar(&data, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(
            listing.members.iter().map(|m| m.link_target.as_deref()).collect::<Vec<_>>(),
            [Some("shared-target"), Some("local-target"), Some("shared-target"), Some("original")]
        );
    }

    #[test]
    fn malformed_or_excess_global_records_do_not_claim_complete_members() {
        for metadata in [String::from("10 size=3!"), record("comment", "").repeat(257), record("path", &"a".repeat(4097))] {
            let data = fx::tar_entries(&[("keep", b'0', b"ok", ""), ("global", b'g', metadata.as_bytes(), ""), ("after", b'0', b"later", "")]);
            let listing = tar(&data, 10).unwrap();
            assert!(!listing.complete);
            assert_eq!(listing.members.len(), 1);
            assert_eq!(listing.members[0].name, "keep");
            assert!(listing.warnings.iter().any(|warning| warning.contains("pax")));
        }
    }

    #[test]
    fn global_sparse_metadata_keeps_stored_extents_out_of_regular_files() {
        let global = record("GNU.sparse.map", "0,3");
        let remove = record("GNU.sparse.map", "");
        let data = fx::tar_entries(&[
            ("global", b'g', global.as_bytes(), ""),
            ("one", b'0', b"abc", ""),
            ("two", b'0', b"def", ""),
            ("global", b'g', remove.as_bytes(), ""),
            ("plain", b'0', b"ghi", ""),
        ]);
        let listing = tar(&data, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(listing.members.iter().map(|m| m.kind).collect::<Vec<_>>(), [MemberKind::Other, MemberKind::Other, MemberKind::File]);
        assert_eq!(listing.warnings.iter().filter(|warning| warning.contains("sparse")).count(), 2);
    }

    #[test]
    fn unrelated_global_fields_preserve_local_and_gnu_names() {
        let global = record("comment", "not a member name");
        let local = record("path", "local");
        let data = fx::tar_entries(&[
            ("local", b'x', local.as_bytes(), ""),
            ("global", b'g', global.as_bytes(), ""),
            ("first", b'0', b"one", ""),
            ("././@LongLink", b'L', b"gnu-name\0", ""),
            ("second", b'0', b"two", ""),
        ]);
        let listing = tar(&data, 10).unwrap();
        assert!(listing.complete && listing.warnings.is_empty());
        assert_eq!(listing.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["local", "gnu-name"]);
    }

    #[test]
    fn global_names_obey_the_existing_member_limit() {
        let path = "a".repeat(4096);
        let global = record("path", &path);
        let data = fx::tar_entries(&[("global", b'g', global.as_bytes(), ""), ("one", b'0', b"abc", ""), ("two", b'0', b"def", "")]);
        let listing = tar(&data, 1).unwrap();
        assert!(listing.truncated && !listing.complete);
        assert_eq!(listing.members.len(), 1);
        assert_eq!(listing.members[0].name, path);
    }

    #[test]
    fn global_pax_names_override_and_consume_gnu_name_records() {
        let global = record("path", "global") + &record("linkpath", "global-target");
        let remove = record("path", "") + &record("linkpath", "");
        let data = fx::tar_entries(&[
            ("global", b'g', global.as_bytes(), ""),
            ("LongLink", b'L', b"gnu-name\0", ""),
            ("LongLink", b'K', b"gnu-target\0", ""),
            ("first", b'2', b"", "fallback"),
            ("global", b'g', remove.as_bytes(), ""),
            ("plain", b'2', b"", "original"),
        ]);
        let listing = tar(&data, 10).unwrap();
        assert!(listing.complete);
        assert_eq!(
            listing.members.iter().map(|m| (m.name.as_str(), m.link_target.as_deref())).collect::<Vec<_>>(),
            [("global", Some("global-target")), ("plain", Some("original"))]
        );
    }

    #[test]
    fn nonempty_global_sizes_must_be_unsigned_decimal_numbers() {
        for size in ["+3", "-3", "3x", " 3", "18446744073709551616"] {
            let global = record("size", size);
            let data = fx::tar_entries(&[("keep", b'0', b"abc", ""), ("global", b'g', global.as_bytes(), ""), ("after", b'0', b"def", "")]);
            let listing = tar(&data, 10).unwrap();
            assert!(!listing.complete);
            assert_eq!(listing.members.len(), 1);
            assert!(listing.warnings.iter().any(|warning| warning.contains("global pax size")));
        }
    }
}

// Ported from Craft Anything gnu_sparse_extensions.rs (RRFZ).
mod gnu_sparse_extensions {
    use crate::testkit::tar_entries;
    use crate::{MemberKind, member_data, tar};

    /// Build an old GNU sparse header followed by the requested number of extension blocks.
    fn archive(extension_blocks: usize) -> Vec<u8> {
        let mut archive = tar_entries(&[("sparse.dat", b'S', &[b'x'; 512], ""), ("after.txt", b'0', b"after", "")]);
        archive[257..265].copy_from_slice(b"ustar  \0");
        archive[482] = u8::from(extension_blocks != 0);
        archive[148..156].fill(b' ');
        let sum: u32 = archive[..512].iter().map(|&byte| u32::from(byte)).sum();
        archive[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut extensions = vec![0; 512 * extension_blocks];
        for block in extensions.as_chunks_mut::<512>().0.iter_mut().take(extension_blocks.saturating_sub(1)) {
            block[504] = 1;
        }
        archive.splice(512..512, extensions);
        archive
    }

    /// Actual file data starts after the entire sparse map. Later members remain readable.
    #[test]
    fn skips_old_gnu_sparse_extension_blocks() {
        for blocks in [1, 2] {
            let bytes = archive(blocks);
            let listing = tar(&bytes, 8).unwrap();
            assert!(listing.complete, "{blocks} extensions: {:?}", listing.warnings);
            assert_eq!(listing.members.len(), 2);
            assert_eq!(listing.members[0].kind, MemberKind::Other);
            assert_eq!(listing.members[0].data_offset, (512 * (blocks + 1)) as u64);
            assert_eq!(listing.members[1].name, "after.txt");
            assert_eq!(member_data(&bytes, &listing.members[1]), Some(b"after".as_slice()));
        }
    }

    /// A missing sparse extension cannot turn padding into a complete archive.
    #[test]
    fn refuses_a_truncated_old_gnu_sparse_extension() {
        let mut bytes = archive(2);
        bytes.truncate(512 + 512 + 17);
        let listing = tar(&bytes, 8).unwrap();
        assert!(!listing.complete);
        assert!(listing.members.is_empty());
        assert!(listing.warnings.iter().any(|warning| warning.contains("sparse extension")));
    }

    /// A sparse member with no extensions still leaves the next header at its usual offset.
    #[test]
    fn keeps_the_next_member_when_no_sparse_extensions_follow() {
        let bytes = archive(0);
        let listing = tar(&bytes, 8).unwrap();
        assert!(listing.complete);
        assert_eq!(listing.members[0].kind, MemberKind::Other);
        assert_eq!(listing.members[1].name, "after.txt");
    }
}

// Ported from Craft Anything cpio_symlink_crc.rs (RRFZ).
mod cpio_symlink_crc {
    use crate::cpio;
    use crate::testkit::cpio_newc;

    /// Convert a single-member newc fixture to CRC and set the member checksum.
    fn archive(mode: u32, checksum: u32) -> Vec<u8> {
        let mut bytes = cpio_newc(&[("entry", b"target", mode)]);
        bytes[..6].copy_from_slice(b"070702");
        bytes[102..110].copy_from_slice(format!("{checksum:08x}").as_bytes());
        let trailer = (110 + "entry".len() + 1 + 3) & !3;
        let trailer = (trailer + "target".len() + 3) & !3;
        bytes[trailer..trailer + 6].copy_from_slice(b"070702");
        bytes
    }

    /// GNU-style CRC symlinks carry a zero placeholder and keep the recorded target.
    #[test]
    fn accepts_a_symlink_with_no_stored_checksum() {
        let listing = cpio(&archive(0o120777, 0), 10).unwrap();
        assert!(listing.complete);
        assert_eq!(listing.members[0].link_target.as_deref(), Some("target"));
        assert!(listing.warnings.is_empty(), "{:?}", listing.warnings);
    }

    /// A nonzero symlink checksum and even a zero regular-file checksum are checked.
    #[test]
    fn still_checks_nonzero_symlink_and_zero_regular_file_checksums() {
        assert!(cpio(&archive(0o120777, 647), 10).unwrap().warnings.is_empty());
        for bytes in [archive(0o120777, 646), archive(0o100644, 0)] {
            assert!(cpio(&bytes, 10).unwrap().warnings.iter().any(|w| w.contains("checksum mismatch")));
        }
    }
}

// Ported from Craft Anything tar_pax_size_precedence.rs (RRFZ).
mod tar_pax_size_precedence {
    use crate::testkit::tar_entries;
    use crate::{member_data, tar};

    /// Make an octal size unreadable while leaving the enclosing TAR header checksum valid.
    fn corrupt_size(archive: &mut [u8], offset: usize) {
        let header = &mut archive[offset..offset + 512];
        header[124] = b'9';
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&byte| u32::from(byte)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    }

    /// Local and inherited size records override the bad field and keep the following header aligned.
    #[test]
    fn valid_pax_size_controls_the_member_extent() {
        for extension in *b"xg" {
            let mut archive = tar_entries(&[("PaxHeader", extension, b"10 size=3\n", ""), ("file", b'0', b"abc", ""), ("next", b'0', b"def", "")]);
            corrupt_size(&mut archive, 1024);
            let list = tar(&archive, 10).unwrap();
            assert!(list.complete, "{:?}", list.warnings);
            assert_eq!(list.members.len(), 2);
            assert_eq!(member_data(&archive, &list.members[0]), Some(b"abc".as_slice()));
            assert_eq!(member_data(&archive, &list.members[1]), Some(b"def".as_slice()));
        }
    }

    /// A local zero overrides a global size and does not consume the next header.
    #[test]
    fn a_local_zero_size_overrides_the_global_value() {
        let mut archive = tar_entries(&[
            ("Global", b'g', b"12 size=600\n", ""),
            ("Local", b'x', b"10 size=0\n", ""),
            ("empty", b'0', b"", ""),
            ("next", b'0', &[b'a'; 600], ""),
        ]);
        corrupt_size(&mut archive, 2048);
        let list = tar(&archive, 10).unwrap();
        assert!(list.complete, "{:?}", list.warnings);
        assert_eq!(list.members.iter().map(|m| m.size).collect::<Vec<_>>(), [0, 600]);
    }

    /// An ordinary header without an override and a PAX header itself still need readable sizes.
    #[test]
    fn rejects_bad_sizes_without_an_applicable_override() {
        let mut plain = tar_entries(&[("file", b'0', b"abc", "")]);
        corrupt_size(&mut plain, 0);
        let mut with_global = tar_entries(&[("Global", b'g', b"10 size=3\n", ""), ("Local", b'x', b"10 size=0\n", ""), ("file", b'0', b"", "")]);
        corrupt_size(&mut with_global, 1024);
        for archive in [plain, with_global] {
            let list = tar(&archive, 10).unwrap();
            assert!(!list.complete);
            assert!(list.members.is_empty());
            assert!(list.warnings.iter().any(|warning| warning.contains("unreadable size")));
        }
    }
}
