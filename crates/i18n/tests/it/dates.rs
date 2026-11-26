//! Dates on jiff's civil types, behind the `dates` feature, and the local offset behind `local-time`.

use craft_i18n::{CalendarDate, DateFormat, DateStyle, DateTime, TimeZone, utc_stamp};

/// The seconds a `DateTime` is after 1970-01-01T00:00 on its own clock.
fn clock_seconds(at: DateTime) -> i64 {
    at.date.days() * 86_400 + i64::from(at.hour) * 3600 + i64::from(at.minute) * 60 + i64::from(at.second)
}

// Every moment, from the earliest to the latest a 64-bit count holds, has a valid calendar day on the UTC clock,
// and the days count back to the same moment's day: the 400-year cycle keeps years past jiff's -9999 to 9999.
#[test]
fn any_moment_has_a_valid_calendar_day() {
    for unix in [
        i64::MIN,
        i64::MIN / 2,
        -1_000_000_000_000_000,
        -377_705_116_800,
        -1,
        0,
        1_700_000_000,
        253_402_300_800,
        1_000_000_000_000_000,
        i64::MAX / 2,
        i64::MAX,
    ] {
        let at = DateTime::from_unix(unix, TimeZone::Utc);
        let date = at.date;
        assert_eq!(CalendarDate::new(date.year, date.month, date.day), Some(date), "{unix}: {date:?}");
        assert!(at.hour < 24 && at.minute < 60 && at.second < 60, "{unix}: {at:?}");
        // Saturating arithmetic at the ends of the range may fold the day, never panic; elsewhere the clock is exact.
        if unix.unsigned_abs() < 1 << 60 {
            assert_eq!(clock_seconds(at), unix, "{unix}");
        }
    }
}

// The local clock differs from UTC by less than a day, whatever zone the machine is in (its offsets before standard time
// were not whole minutes: Lisbon's was -0:36:45), and no moment, however far, stops it from answering.
#[test]
fn the_local_clock_is_a_zone_offset_from_utc() {
    for unix in
        [-1_000_000_000_000_000, -377_705_116_800, -2_208_988_800, 0, 1_700_000_000, 1_750_000_000, 253_402_300_800, 1_000_000_000_000_000, i64::MAX]
    {
        let utc = DateTime::from_unix(unix, TimeZone::Utc);
        let local = DateTime::from_unix(unix, TimeZone::Local);
        let offset = clock_seconds(local) - clock_seconds(utc);
        assert!(offset.abs() <= 15 * 3600, "{unix}: {offset} s");
        // Without the feature, the local clock is UTC.
        #[cfg(not(feature = "local-time"))]
        assert_eq!(offset, 0, "{unix}");
    }
}

// The stamp of a log line or a file name is UTC and the same in every language, with years past 9999 in full.
#[test]
fn a_stamp_is_utc_with_all_its_digits() {
    assert_eq!(utc_stamp(0, craft_i18n::TimePrecision::Minutes), "1970-01-01 00:00");
    assert_eq!(utc_stamp(1_700_000_000, craft_i18n::TimePrecision::Seconds), "2023-11-14 22:13:20");
    assert_eq!(utc_stamp(-1, craft_i18n::TimePrecision::Seconds), "1969-12-31 23:59:59");
    let far = utc_stamp(400 * 31_556_952 * 40, craft_i18n::TimePrecision::Minutes);
    assert!(far.split('-').next().is_some_and(|year| year.len() >= 5), "{far}");
}

// The CLDR date data loads: a German date is written the German way, not as the ISO fallback the formatter uses when
// its data blob does not load (`YYYY-MM-DD hh:mm`).
#[test]
fn the_date_data_loads_and_is_the_regions() {
    let at = DateTime { date: CalendarDate::new(2026, 10, 31).unwrap(), hour: 14, minute: 5, second: 11 };
    let german = DateFormat::from_tag("de-DE").format(at, DateStyle::DATE);
    let english = DateFormat::from_tag("en-US").format(at, DateStyle::DATE);
    assert!(german.contains("31.") && german.contains("2026"), "{german}");
    assert!(english.contains("Oct") && english.contains("31"), "{english}");
    assert_ne!(german, english);
    assert!(!german.starts_with("2026-"), "{german}");
}
