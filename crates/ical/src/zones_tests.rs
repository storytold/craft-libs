//! `VTIMEZONE` reading and writing. The zones written are rule zones built here (the United States'
//! rules, the European Union's, Sydney's, a zone without changes, the United States' change of
//! rule in 2007), standing in for a caller's time-zone database.

use proptest::prelude::*;

use super::*;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn at(y: i32, m: u32, day: u32, h: u32, min: u32) -> NaiveDateTime {
    d(y, m, day).and_hms_opt(h, min, 0).unwrap()
}

fn utc(t: NaiveDateTime) -> i64 {
    to_timestamp(t, 0)
}

/// A zone by its list of changes.
struct Rules {
    before: i32,
    changes: Vec<Transition>,
}

impl ZoneRules for Rules {
    fn offset_at(&self, instant: i64) -> i32 {
        self.changes.iter().rev().find(|c| c.at <= instant).map_or(self.before, |c| c.after)
    }

    fn transitions(&self, from: i64, to: i64) -> Vec<Transition> {
        self.changes.iter().copied().filter(|c| c.at >= from && c.at < to).collect()
    }
}

/// The change on the `nth` `weekday` of `month` at `hour` UTC (`day_before`: the evening before).
fn change(year: i32, month: u32, nth: i8, hour: u32, day_before: bool, before: i32, after: i32) -> Transition {
    let day = nth_weekday(year, month, Weekday::Sun, nth).unwrap();
    let day = if day_before { day.pred_opt().unwrap() } else { day };
    Transition { at: utc(day.and_hms_opt(hour, 0, 0).unwrap()), before, after }
}

const H: i32 = 3600;

/// The United States' rule since 2007 (and, with `history`, the one before it up to 2006).
fn us_eastern(years: std::ops::RangeInclusive<i32>, history: bool) -> Rules {
    let mut changes = Vec::new();
    for y in years {
        if history && y < 2007 {
            changes.push(change(y, 4, 1, 7, false, -5 * H, -4 * H));
            changes.push(change(y, 10, -1, 6, false, -4 * H, -5 * H));
        } else {
            changes.push(change(y, 3, 2, 7, false, -5 * H, -4 * H));
            changes.push(change(y, 11, 1, 6, false, -4 * H, -5 * H));
        }
    }
    Rules { before: -5 * H, changes }
}

fn berlin(years: std::ops::RangeInclusive<i32>) -> Rules {
    let changes = years.flat_map(|y| [change(y, 3, -1, 1, false, H, 2 * H), change(y, 10, -1, 1, false, 2 * H, H)]).collect();
    Rules { before: H, changes }
}

fn sydney(years: std::ops::RangeInclusive<i32>) -> Rules {
    let changes = years.flat_map(|y| [change(y, 4, 1, 16, true, 11 * H, 10 * H), change(y, 10, 1, 16, true, 10 * H, 11 * H)]).collect();
    Rules { before: 11 * H, changes }
}

fn fixed(seconds: i32) -> Rules {
    Rules { before: seconds, changes: Vec::new() }
}

/// The zone as a big mail program writes it: today's rule, from the year 1601.
const EASTERN: &str = "BEGIN:VTIMEZONE\r\nTZID:Eastern Standard Time\r\nBEGIN:STANDARD\r\nDTSTART:16011104T020000\r\nRRULE:FREQ=YEARLY;BYDAY=1SU;BYMONTH=11\r\n\
TZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010311T020000\r\nRRULE:FREQ=YEARLY;BYDAY=2SU;BYMONTH=3\r\n\
TZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n";

fn only(text: &str) -> VTimezone {
    let zones = TimeZones::collect(text);
    assert_eq!(zones.iter().count(), 1, "{text}");
    zones.iter().next().unwrap().clone()
}

#[test]
fn a_definition_gives_each_time_its_offset() {
    let eastern = only(EASTERN);
    assert_eq!(eastern.tzid(), "Eastern Standard Time");
    assert_eq!(eastern.offset_for_wall(at(2026, 1, 15, 9, 0)), -5 * 3600);
    assert_eq!(eastern.offset_for_wall(at(2026, 7, 15, 9, 0)), -4 * 3600);
    // March 8, 2026: 1:59 is winter time, 3:00 summer time, and 2:30 (which no clock shows)
    // is still read by the offset before.
    assert_eq!(eastern.offset_for_wall(at(2026, 3, 8, 1, 59)), -5 * 3600);
    assert_eq!(eastern.offset_for_wall(at(2026, 3, 8, 2, 30)), -5 * 3600);
    assert_eq!(eastern.offset_for_wall(at(2026, 3, 8, 3, 0)), -4 * 3600);
    // November 1: 1:30 comes twice, and is the first of the two.
    assert_eq!(eastern.offset_for_wall(at(2026, 11, 1, 1, 30)), -4 * 3600);
    assert_eq!(eastern.offset_for_wall(at(2026, 11, 1, 2, 0)), -5 * 3600);
    // By instants, every half hour of two years agrees with the rules.
    let rules = us_eastern(2025..=2028, false);
    let mut wall = at(2026, 1, 1, 0, 0);
    while wall < at(2028, 1, 1, 0, 0) {
        let instant = utc(wall);
        assert_eq!(eastern.offset_at(instant), rules.offset_at(instant), "{wall}");
        wall += chrono::Duration::minutes(30);
    }
    assert_eq!(eastern.instant(at(2026, 7, 15, 9, 0)), utc(at(2026, 7, 15, 13, 0)));
    // Before there were any rules: what the first change changes from.
    assert_eq!(eastern.offset_for_wall(at(1500, 6, 1, 0, 0)), -5 * 3600);
}

#[test]
fn definitions_of_every_shape_are_read() {
    // Listed dates, a rule that ends, a rule by the day of the month, a part with no rule.
    let text = "BEGIN:VTIMEZONE\nTZID:Example/Odd\nX-LIC-LOCATION:Europe/Berlin\n\
BEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0100\nEND:STANDARD\n\
BEGIN:DAYLIGHT\nDTSTART:20200329T020000\nRDATE:20210328T020000,20220327T020000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0200\nEND:DAYLIGHT\n\
BEGIN:STANDARD\nDTSTART:20201025T030000\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU;UNTIL=20221030T010000Z\nTZOFFSETFROM:+0200\nTZOFFSETTO:+0100\nEND:STANDARD\n\
BEGIN:DAYLIGHT\nDTSTART:20240401T000000\nRRULE:FREQ=YEARLY;BYMONTHDAY=1;BYMONTH=4;COUNT=2\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0130\nEND:DAYLIGHT\n\
BEGIN:STANDARD\nDTSTART:20241001T000000\nRRULE:FREQ=YEARLY\nTZOFFSETFROM:+0130\nTZOFFSETTO:+0100\nEND:STANDARD\n\
END:VTIMEZONE\n";
    let odd = only(text);
    let offset = |wall: NaiveDateTime| odd.offset_for_wall(wall) / 60;
    assert_eq!(offset(at(2019, 7, 1, 12, 0)), 60);
    assert_eq!(offset(at(2020, 7, 1, 12, 0)), 120);
    assert_eq!(offset(at(2021, 1, 1, 12, 0)), 60);
    assert_eq!(offset(at(2022, 7, 1, 12, 0)), 120);
    assert_eq!(offset(at(2023, 7, 1, 12, 0)), 60);
    // Two Aprils of half an hour, and no third.
    assert_eq!(offset(at(2024, 7, 1, 12, 0)), 90);
    assert_eq!(offset(at(2025, 7, 1, 12, 0)), 90);
    assert_eq!(offset(at(2025, 12, 1, 12, 0)), 60);
    assert_eq!(offset(at(2026, 7, 1, 12, 0)), 60);
    // It says where it is, and is found by its name in any letter case.
    let zones = TimeZones::collect(text);
    assert_eq!(zones.get("Example/Odd").map(VTimezone::place), Some("Europe/Berlin"));
    assert!(zones.get("example/odd").is_some());
    assert!(zones.get("Example/Even").is_none());
}

#[test]
fn nonsense_definitions_are_no_definitions() {
    for text in [
        "BEGIN:VTIMEZONE\nEND:VTIMEZONE\n",
        "BEGIN:VTIMEZONE\nTZID:Empty\nEND:VTIMEZONE\n",
        "BEGIN:VTIMEZONE\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE\n",
        // Offsets of days, and no offsets at all.
        "BEGIN:VTIMEZONE\nTZID:Far\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+9900\nTZOFFSETTO:-7200\nEND:STANDARD\nEND:VTIMEZONE\n",
        "BEGIN:VTIMEZONE\nTZID:Far\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+1801\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE\n",
        "BEGIN:VTIMEZONE\nTZID:None\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:soon\nTZOFFSETTO:later\nEND:STANDARD\nEND:VTIMEZONE\n",
        "BEGIN:VTIMEZONE\nTZID:NoStart\nBEGIN:STANDARD\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE\n",
        // A part outside a zone, and a zone that never ends.
        "BEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0100\nEND:STANDARD\n",
        "BEGIN:VTIMEZONE\nTZID:Open\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0100\nEND:STANDARD\n",
    ] {
        assert!(TimeZones::collect(text).is_empty(), "{text}");
    }
    assert_eq!(parse_utc_offset("+053000"), Some(5 * 3600 + 30 * 60));
    assert_eq!(parse_utc_offset("-00:30"), Some(-30 * 60));
    for bad in ["", "0100", "+1", "+01", "+0160", "+010060", "+2400", "+١٢٠٠", "++0100"] {
        assert_eq!(parse_utc_offset(bad), None, "{bad}");
    }
    assert_eq!((format_utc_offset(-570 * 60), format_utc_offset(0), format_utc_offset(16_562)), ("-0930".into(), "+0000".into(), "+043602".into()));
    // A megabyte of name, hundreds of zones, thousands of parts and dates: all bounded.
    let long = format!(
        "BEGIN:VTIMEZONE\nTZID:{}\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE\n",
        "Z".repeat(1 << 20)
    );
    let zones = TimeZones::collect(&long);
    assert!(only(&long).tzid().len() <= MAX_TZID);
    assert!(zones.get(&"Z".repeat(1 << 20)).is_some());
    let many: String = (0..500)
        .map(|n| format!("BEGIN:VTIMEZONE\nTZID:Zone{n}\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE\n"))
        .collect();
    assert_eq!(TimeZones::collect(&many).iter().count(), MAX_ZONES);
    let part = format!(
        "BEGIN:STANDARD\nDTSTART:19700101T000000\nRDATE:{}\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0100\nEND:STANDARD\n",
        "19800101T000000,".repeat(5_000)
    );
    let crowded = only(&format!("BEGIN:VTIMEZONE\nTZID:Crowded\n{}END:VTIMEZONE\n", part.repeat(1_000)));
    assert_eq!((crowded.parts.len(), crowded.parts[0].dates.len()), (MAX_PARTS, MAX_DATES));
    assert_eq!(crowded.cost(), (MAX_PARTS * (8 + MAX_DATES)) as u64);
    let _ = crowded.offset_for_wall(at(2026, 1, 1, 0, 0));
    // Hostile times ask nothing that panics.
    let eastern = only(EASTERN);
    for wall in [NaiveDateTime::MAX, NaiveDateTime::MIN, NaiveDateTime::default()] {
        let _ = (eastern.offset_for_wall(wall), eastern.instant(wall));
    }
    let _ = (eastern.offset_at(i64::MIN), eastern.offset_at(i64::MAX));
}

#[test]
fn the_budget_runs_out_for_good() {
    let zones = TimeZones::collect(EASTERN);
    assert_eq!(zones.stopped_after(), None);
    assert!(zones.spend(WORK_BUDGET - 10));
    assert!(zones.spend(10));
    assert!(!zones.spend(1));
    // Once out, out: even a spend of nothing is refused.
    assert!(!zones.spend(0));
    assert_eq!(zones.stopped_after(), Some(2));
}

/// What a written definition says, read back.
fn written(name: &str, zone: &dyn ZoneRules, from: i32, to: i32) -> (String, VTimezone) {
    let mut out = String::new();
    write_vtimezone(&mut out, name, zone, from, to);
    let read = only(&out);
    (out, read)
}

#[test]
fn written_definitions_are_the_usual_two_rules_where_a_zone_has_them() {
    let (text, _) = written("America/New_York", &us_eastern(2025..=2027, false), 2026, 2026);
    assert_eq!(
        text.replace("\r\n", "\n"),
        "BEGIN:VTIMEZONE\nTZID:America/New_York\n\
BEGIN:DAYLIGHT\nDTSTART:20250309T020000\nTZOFFSETFROM:-0500\nTZOFFSETTO:-0400\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\nEND:DAYLIGHT\n\
BEGIN:STANDARD\nDTSTART:20251102T020000\nTZOFFSETFROM:-0400\nTZOFFSETTO:-0500\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\nEND:STANDARD\n\
END:VTIMEZONE\n"
    );
    let (text, _) = written("Europe/Berlin", &berlin(2025..=2031), 2026, 2030);
    assert!(text.contains("RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU") && text.contains("RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU"), "{text}");
    assert!(text.contains("DTSTART:20250330T020000") && text.contains("DTSTART:20251026T030000"), "{text}");
    // The south: summer time begins in October.
    let (text, _) = written("Australia/Sydney", &sydney(2025..=2027), 2026, 2026);
    assert!(text.contains("RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=1SU") && text.contains("RRULE:FREQ=YEARLY;BYMONTH=4;BYDAY=1SU"), "{text}");
    // No changes: one part. An offset is a zone too.
    let (text, _) = written("Asia/Kolkata", &fixed(5 * H + 1800), 2026, 2026);
    assert_eq!(
        text.replace("\r\n", "\n"),
        "BEGIN:VTIMEZONE\nTZID:Asia/Kolkata\nBEGIN:STANDARD\nDTSTART:16010101T000000\nTZOFFSETFROM:+0530\nTZOFFSETTO:+0530\nEND:STANDARD\nEND:VTIMEZONE\n"
    );
    let (text, _) = written("UTC-09:30", &fixed(-570 * 60), 2026, 2026);
    assert!(text.contains("TZID:UTC-09:30") && text.contains("TZOFFSETTO:-0930"), "{text}");
    // The year the United States changed its rule: written out change by change.
    let history = us_eastern(2004..=2009, true);
    let (text, read) = written("America/New_York", &history, 2006, 2007);
    assert!(!text.contains("RRULE") && text.contains("DTSTART:20060402T020000") && text.contains("DTSTART:20070311T020000"), "{text}");
    assert_eq!(read.instant(at(2006, 3, 20, 9, 0)), utc(at(2006, 3, 20, 14, 0)));
    assert_eq!(read.instant(at(2007, 3, 20, 9, 0)), utc(at(2007, 3, 20, 13, 0)));
    // Hostile years are years, and a hostile name can't start a line.
    for (from, to) in [(i32::MIN, i32::MAX), (9_999, 1), (0, 0)] {
        let _ = written("Europe/Berlin", &berlin(2020..=2030), from, to);
    }
    let (text, read) = written("Evil\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT", &fixed(0), 2026, 2026);
    assert_eq!(read.tzid(), "EvilEND:VTIMEZONEBEGIN:VEVENT");
    assert_eq!(text.matches("VTIMEZONE").count(), 3, "{text}");
}

fn any_zone() -> impl Strategy<Value = (usize, i32)> {
    (0usize..6, -12 * 60i32..=14 * 60)
}

fn zone_of(which: usize, minutes: i32) -> Rules {
    match which {
        0 => us_eastern(1960..=2100, false),
        1 => us_eastern(1960..=2100, true),
        2 => berlin(1960..=2100),
        3 => sydney(1960..=2100),
        4 => fixed(minutes * 60),
        _ => fixed(0),
    }
}

proptest! {
    #[test]
    fn a_written_definition_reads_as_the_zone_it_was_written_for((which, minutes_east) in any_zone(), year in 1971i32..2090, span in 0i32..4, minutes in 0i64..365 * 24 * 60) {
        let zone = zone_of(which, minutes_east);
        let (text, read) = written("Example/Zone", &zone, year, year + span);
        // Any time of those years is the instant the rules say.
        let wall = at(year, 1, 1, 0, 0) + chrono::Duration::minutes(minutes) + chrono::Duration::days(i64::from(span) * 365);
        let instant = utc(wall);
        prop_assert_eq!(read.offset_at(instant), zone.offset_at(instant), "{} at {}\n{}", which, wall, text);
        // And a wall-clock time is the instant it shows, where the clock shows it once.
        let wall_instant = read.instant(wall);
        if read.offset_at(wall_instant) == zone.offset_at(wall_instant) && zone.offset_at(wall_instant - 2 * i64::from(H)) == zone.offset_at(wall_instant + 2 * i64::from(H)) {
            prop_assert_eq!(wall_instant + i64::from(zone.offset_at(wall_instant)), civil_seconds(wall));
        }
    }

    #[test]
    fn any_text_is_definitions_or_none(text in proptest::prelude::any::<String>(), parts in proptest::collection::vec("(BEGIN|END|DTSTART|TZOFFSETFROM|TZOFFSETTO|RRULE|RDATE|TZID):[-+A-Z0-9=;,]{0,24}", 0..40), wall in proptest::num::i64::ANY) {
        let _ = TimeZones::collect(&text);
        let zones = TimeZones::collect(&format!("BEGIN:VTIMEZONE\nTZID:X\n{}\nEND:VTIMEZONE\n", parts.join("\n")));
        let wall = from_civil_seconds(wall);
        if let Some(definition) = zones.get("X") {
            let _ = (definition.offset_for_wall(wall), definition.offset_at(utc(wall)), definition.instant(wall));
        }
        let _ = zones.get(&text);
    }
}
