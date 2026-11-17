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
