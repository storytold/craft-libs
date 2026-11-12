//! How an appointment repeats ([MS-OXOCAL] 2.2.1.44), as the writer makes it, and hostile.

use proptest::prelude::*;

use super::writer::{recurrence_bytes, time_zone_bytes};
use crate::recur::{END_DATE, FREQ_WEEKLY, PATTERN_WEEK};
use crate::{appointment_recurrence, time_zone_key};

fn weekly() -> Vec<u8> {
    let changed = vec![(2_000_600, 2_000_660, 2_000_540, Some("Moved caf\u{e9}".to_string()))];
    recurrence_bytes(FREQ_WEEKLY, PATTERN_WEEK, 1, &[0b10], (END_DATE, 4, 2_030_000), &[2_000_000, 2_010_000], 1_990_000, (540, 600), &changed)
}

#[test]
fn a_series_reads_back() {
    let r = appointment_recurrence(&weekly()).unwrap();
    assert_eq!((r.frequency, r.pattern, r.period, r.weekdays), (FREQ_WEEKLY, PATTERN_WEEK, 1, 0b10));
    assert_eq!((r.end_type, r.count, r.start_date, r.end_date), (END_DATE, 4, 1_990_000, 2_030_000));
    assert_eq!(r.deleted, vec![2_000_000, 2_010_000]);
    assert_eq!((r.start_offset, r.end_offset), (540, 600));
    let x = &r.exceptions[0];
    assert_eq!((x.start, x.end, x.original), (2_000_600, 2_000_660, 2_000_540));
    assert_eq!(x.subject.as_deref(), Some("Moved caf\u{e9}"));
}

#[test]
fn a_count_past_the_bytes_is_an_error_and_allocates_nothing() {
    let mut b = weekly();
    // DeletedInstanceCount, after 22 bytes of header, the one specific word and three more.
    b[38..42].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(appointment_recurrence(&b).is_err());
}

#[test]
fn a_cut_extended_part_keeps_the_rest() {
    let mut b = weekly();
    b.truncate(b.len() - 10);
    let r = appointment_recurrence(&b).unwrap();
    assert_eq!(r.exceptions.len(), 1);
    assert_eq!(r.exceptions[0].subject_8bit.as_deref(), Some("Moved caf\u{e9}".as_bytes()));
}

#[test]
fn a_time_zone_names_its_key() {
    assert_eq!(time_zone_key(&time_zone_bytes("W. Europe Standard Time")).unwrap(), "W. Europe Standard Time");
    assert!(time_zone_key(&[1, 1, 0, 0]).is_err());
    assert!(time_zone_key(&time_zone_bytes("")).is_err());
}

proptest! {
    #[test]
    fn any_bytes_never_panic(b in prop::collection::vec(any::<u8>(), 0..600)) {
        let _ = appointment_recurrence(&b);
        let _ = time_zone_key(&b);
    }

    #[test]
    fn a_damaged_series_never_panics(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..12)) {
        let mut b = weekly();
        for (at, v) in flips {
            let i = at.index(b.len());
            b[i] = v;
        }
        let _ = appointment_recurrence(&b);
    }
}
