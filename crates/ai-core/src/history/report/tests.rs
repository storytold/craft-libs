use super::*;
use crate::connection::Locality;
use crate::history::{EndRecord, HISTORY_SCHEMA, HistoryRecord, StartRecord, merge};
use crate::job::Usage;

const T0: u64 = 1_760_000_000_000; // 2025-10-09T08:53:20Z

fn start(run: &str, at_ms: u64, app: &str, model: &str) -> StartRecord {
    StartRecord {
        v: HISTORY_SCHEMA,
        run: run.into(),
        at_ms,
        app: app.into(),
        role_id: "fill".into(),
        role_label: "Generative Fill".into(),
        capability: "image.inpaint".into(),
        connection: "c1".into(),
        connection_label: "Hosted".into(),
        model: model.into(),
        locality: Locality::Remote,
        destination: "https://api.example.com".into(),
        sent: Vec::new(),
        params: Vec::new(),
        provider_job: Some("gen-1".into()),
        estimate: None,
        prompt: None,
    }
}

fn finish(run: &str, at_ms: u64, outcome: Outcome, cost: Option<u64>) -> EndRecord {
    EndRecord {
        v: HISTORY_SCHEMA,
        run: run.into(),
        at_ms,
        outcome,
        usage: cost.map(|c| Usage { cost_micros: Some(c), currency: Some("USD".into()), ..Usage::default() }),
    }
}

/// (run, started, app, model, outcome, cost); a `None` outcome means unfinished.
type Spec<'a> = (&'a str, u64, &'a str, &'a str, Option<Outcome>, Option<u64>);

fn entries(spec: &[Spec<'_>]) -> Vec<HistoryEntry> {
    let mut records = Vec::new();
    for (run, at, app, model, outcome, cost) in spec {
        records.push(HistoryRecord::Start(start(run, *at, app, model)));
        if let Some(o) = outcome {
            records.push(HistoryRecord::End(finish(run, at + 2000, o.clone(), *cost)));
        }
    }
    merge(records, 0).entries
}

#[test]
fn days_are_right_across_leap_days_epoch_and_year_ends() {
    assert_eq!(day_of(0, 0).to_string(), "1970-01-01");
    assert_eq!(day_of(951_782_400_000, 0).to_string(), "2000-02-29");
    assert_eq!(day_of(1_709_164_800_000, 0).to_string(), "2024-02-29");
    assert_eq!(day_of(1_767_225_599_000, 0).to_string(), "2025-12-31");
    assert_eq!(day_of(1_767_225_600_000, 0).to_string(), "2026-01-01");
    assert_eq!(day_of(T0, 0).to_string(), "2025-10-09");
}

#[test]
fn csv_quotes_awkward_text_and_defuses_formulas() {
    let mut e = entries(&[("a", T0, "p", "m", Some(Outcome::Done), None)]);
    e[0].start.role_label = "Fill, \"quick\"".into();
    e[0].start.prompt = Some("=HYPERLINK(\"http://x\")\nsecond line".into());
    e[0].start.connection_label = "@home".into();
    let refs: Vec<&HistoryEntry> = e.iter().collect();
    let csv = to_csv(&refs);
    assert!(csv.contains("\"Fill, \"\"quick\"\"\""), "{csv}");
    assert!(csv.contains("\"'=HYPERLINK(\"\"http://x\"\")\nsecond line\""), "{csv}");
    assert!(csv.contains(",'@home,"), "{csv}");
    assert!(!csv.contains(",=HYPERLINK"), "{csv}");
}

#[test]
fn days_convert_to_and_from_the_epoch_count_both_ways() {
    for ms in [0, 951_782_400_000, 1_709_164_800_000, 1_767_225_599_000, T0] {
        let day = day_of(ms, 0);
        assert_eq!(Day::from_days(day.to_days()), day, "{day}");
    }
    assert_eq!(day_of(0, 0).to_days(), 0);
    assert_eq!(day_of(DAY_MS_FOR_TESTS, 0).to_days(), 1);
    assert_eq!(day_of(0, -60).to_days(), -1);
    // A day count past what chrono can represent reads as the 0000-01-01 fallback, not a stray
    // valid date: `i64::MAX` days would overflow the CE `i32` (which once became 0000-12-31).
    let fallback = Day { year: 0, month: 1, day: 1 };
    assert_eq!(Day::from_days(i64::MAX), fallback);
    assert_eq!(Day::from_days(i64::MIN), fallback);
    // Valid `i32` years past chrono's ±262143 still convert, both ways, as before chrono.
    for (day, days) in [
        (Day { year: 262_144, month: 1, day: 1 }, 95_026_602),
        (Day { year: -262_145, month: 3, day: 1 }, -96465964),
        (Day { year: i32::MAX, month: 12, day: 31 }, 784351576776),
        (Day { year: i32::MIN, month: 1, day: 1 }, -784353015833),
    ] {
        assert_eq!(day.to_days(), days, "{day}");
        assert_eq!(Day::from_days(days), day, "{days}");
    }
    assert_eq!(Day { year: 2025, month: 2, day: 30 }.to_days(), 0, "a day that doesn't exist");
}

const DAY_MS_FOR_TESTS: u64 = 86_400_000;
