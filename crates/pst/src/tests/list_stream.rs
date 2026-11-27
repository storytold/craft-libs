//! A distribution list's stream ([MS-OXOCNTC] 2.2.2.2.4), as the writer makes it, and hostile.

use proptest::prelude::*;

use super::writer::{list_stream_bytes, one_off, wrapped_entry_id};
use crate::{Error, ListMember, MAX_LIST_ENTRY, list_stream};

fn members(n: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    (0..n)
        .map(|i| {
            let o = one_off(&format!("Member {i}"), "SMTP", &format!("m{i}@example.org"), true);
            if i % 2 == 0 { (o, Vec::new()) } else { (wrapped_entry_id(0xC3, &[i as u8; 46]), o) }
        })
        .collect()
}

#[test]
fn a_stream_reads_back_member_by_member() {
    let m = members(40);
    let b = list_stream_bytes(&m);
    let s = list_stream(&b, 1000).unwrap();
    assert_eq!((s.more, s.damaged, s.totals_differ), (0, 0, false));
    let want: Vec<ListMember> = m.iter().map(|(e, o)| ListMember { entry_id: e, one_off: o }).collect();
    assert_eq!(s.members, want);
}

#[test]
fn an_empty_list_has_no_members() {
    let b = list_stream_bytes(&[]);
    let s = list_stream(&b, 1000).unwrap();
    assert!(s.members.is_empty());
    // Without its terminators too.
    assert!(list_stream(&b[..28], 1000).unwrap().members.is_empty());
}

#[test]
fn past_the_cap_members_are_counted_not_kept() {
    let b = list_stream_bytes(&members(25));
    let s = list_stream(&b, 10).unwrap();
    assert_eq!((s.members.len(), s.more, s.damaged), (10, 15, 0));
}

#[test]
fn a_short_header_or_another_version_is_refused() {
    let b = list_stream_bytes(&members(2));
    assert!(matches!(list_stream(&b[..27], 10), Err(Error::Corrupt(_))));
    assert!(matches!(list_stream(&[], 10), Err(Error::Corrupt(_))));
    let mut v2 = b.clone();
    v2[0] = 2;
    assert!(matches!(list_stream(&v2, 10), Err(Error::Corrupt(_))));
}

#[test]
fn a_count_the_bytes_cannot_hold_is_refused() {
    let mut b = list_stream_bytes(&members(3));
    b[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(list_stream(&b, 10), Err(Error::Corrupt("a distribution list's stream counts more members than it has room for")));
}

#[test]
fn a_stream_cut_short_keeps_the_members_before_the_cut() {
    let m = members(6);
    let b = list_stream_bytes(&m);
    // Cut inside the fifth member.
    let fifth = 28 + m[..4].iter().map(|(e, o)| 12 + e.len() + o.len()).sum::<usize>();
    let s = list_stream(&b[..fifth + 20], 10).unwrap();
    assert_eq!((s.members.len(), s.damaged), (4, 2));
    assert!(!s.totals_differ);
}

#[test]
fn a_length_that_runs_past_the_end_stops_the_reading_there() {
    let m = members(3);
    let mut b = list_stream_bytes(&m);
    // The second member's entry id says it is longer than all there is.
    let second = 28 + 12 + m[0].0.len() + m[0].1.len();
    b[second..second + 4].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    let s = list_stream(&b, 10).unwrap();
    assert_eq!((s.members.len(), s.damaged), (1, 2));
    // And one longer than an entry may be, though it would fit.
    let big = vec![(vec![0u8; MAX_LIST_ENTRY + 1], Vec::new()), m[0].clone()];
    let b = list_stream_bytes(&big);
    let s = list_stream(&b, 10).unwrap();
    assert_eq!((s.members.len(), s.damaged), (0, 2));
}

#[test]
fn totals_that_lie_are_noticed_and_read_past() {
    let mut b = list_stream_bytes(&members(4));
    b[16..20].copy_from_slice(&7u32.to_le_bytes());
    let s = list_stream(&b, 10).unwrap();
    assert_eq!((s.members.len(), s.totals_differ), (4, true));
}

#[test]
fn extra_member_info_is_stepped_over() {
    let m = members(2);
    let mut b = list_stream_bytes(&m[..1]);
    b.truncate(b.len() - 12);
    b.extend(5u32.to_le_bytes());
    b.extend([9; 5]);
    let rest = list_stream_bytes(&m[1..]);
    b.extend_from_slice(&rest[28..]);
    b[12..16].copy_from_slice(&2u32.to_le_bytes());
    let s = list_stream(&b, 10).unwrap();
    assert_eq!(s.members.len(), 2);
    assert_eq!(s.members[1].one_off, &m[1].1[..]);
    assert!(s.totals_differ);
}

proptest! {
    #[test]
    fn any_bytes_never_panic(b in proptest::collection::vec(any::<u8>(), 0..600), cap in 0usize..20) {
        if let Ok(s) = list_stream(&b, cap) {
            prop_assert!(s.members.len() <= cap);
        }
    }

    #[test]
    fn a_header_then_any_bytes_never_panic(count in 0u32..64, tail in proptest::collection::vec(any::<u8>(), 0..800)) {
        let mut b = list_stream_bytes(&[]);
        b.truncate(28);
        b[12..16].copy_from_slice(&count.to_le_bytes());
        b.extend(tail);
        if let Ok(s) = list_stream(&b, 8) {
            prop_assert!(s.members.len() + s.more + s.damaged <= count as usize);
        }
    }

    #[test]
    fn what_is_written_reads_back(n in 0usize..30) {
        let m = members(n);
        let b = list_stream_bytes(&m);
        let s = list_stream(&b, 1000).unwrap();
        prop_assert_eq!(s.members.len(), n);
        prop_assert!(!s.totals_differ);
    }
}

/// The layout of [MS-OXOCNTC] 2.2.2.2.4 written out by hand, apart from the writer: one member
/// whose entry id is 3 bytes and whose one-off is 2.
#[test]
fn the_header_is_read_where_the_specification_puts_it() {
    #[rustfmt::skip]
    let b: &[u8] = &[
        0x01, 0x00, 0x00, 0x00, // StreamVersion, Reserved
        0x00, 0x71, 0x02, 0x00, // BuildVersion
        0x00, 0x00, 0x00, 0x00, // DistListStreamFlags
        0x01, 0x00, 0x00, 0x00, // CountOfEntries
        0x03, 0x00, 0x00, 0x00, // TotalEntryIDSize
        0x02, 0x00, 0x00, 0x00, // TotalOneOffSize
        0x00, 0x00, 0x00, 0x00, // TotalExtraSize
        0x03, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, // EntryIdSize, EntryIdData
        0x02, 0x00, 0x00, 0x00, 0xDD, 0xEE, // one-offEntryIdSize, one-offEntryIdData
        0x00, 0x00, 0x00, 0x00, // ExtraMemberInfoSize
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Terminator1, Terminator2
    ];
    let s = list_stream(b, 10).unwrap();
    assert_eq!(s.members, [ListMember { entry_id: &[0xAA, 0xBB, 0xCC], one_off: &[0xDD, 0xEE] }]);
    assert!(!s.totals_differ);
    // And the writer writes the same.
    assert_eq!(list_stream_bytes(&[(vec![0xAA, 0xBB, 0xCC], vec![0xDD, 0xEE])]), b);
}
