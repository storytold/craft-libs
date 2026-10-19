use super::*;
use crate::connection::Locality;
use crate::error::ErrorKind;
use crate::history::{EndRecord, HISTORY_SCHEMA, HistoryRecord, StartRecord, merge};
use crate::job::{CostEstimate, Usage};

const HOUR: u64 = 3_600_000;
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
fn the_offset_moves_the_day_boundary_both_ways() {
    let late = 1_767_225_599_000; // 2025-12-31T23:59:59Z
    assert_eq!(day_of(late, 60).to_string(), "2026-01-01");
    assert_eq!(day_of(late, -300).to_string(), "2025-12-31");
    let early = 1_767_225_600_000; // 2026-01-01T00:00:00Z
    assert_eq!(day_of(early, -1).to_string(), "2025-12-31");
    assert_eq!(day_of(0, -60).to_string(), "1969-12-31");
}

#[test]
fn iso_time_is_utc_and_padded() {
    assert_eq!(iso_utc(T0), "2025-10-09T08:53:20Z");
    assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
}

#[test]
fn amounts_are_exact_and_small_costs_are_not_zero() {
    assert_eq!(amount_text(31_000), "0.03");
    assert_eq!(amount_text(1_234_567), "1.23");
    assert_eq!(amount_text(1_500), "0.0015");
    assert_eq!(amount_text(0), "0.0000");
    assert_eq!(exact_amount(31_000), "0.031000");
    assert_eq!(exact_amount(2_000_001), "2.000001");
}

#[test]
fn the_filter_narrows_by_app_model_and_time_and_empty_shows_all() {
    let e = entries(&[
        ("a", T0, "photocraft", "m1", Some(Outcome::Done), Some(1)),
        ("b", T0 + HOUR, "wordcraft", "m1", Some(Outcome::Done), None),
        ("c", T0 + 2 * HOUR, "photocraft", "m2", None, None),
    ]);
    assert_eq!(HistoryFilter::default().apply(&e).len(), 3);
    let photo = HistoryFilter { app: Some("photocraft".into()), ..HistoryFilter::default() };
    assert_eq!(photo.apply(&e).len(), 2);
    let both = HistoryFilter { model: Some("m2".into()), ..photo.clone() };
    assert_eq!(both.apply(&e).len(), 1);
    let recent = HistoryFilter { since_ms: Some(T0 + HOUR), ..HistoryFilter::default() };
    let runs: Vec<_> = recent.apply(&e).iter().map(|x| x.start.run.clone()).collect();
    assert_eq!(runs, ["c", "b"], "inclusive of the boundary, newest first");
    assert_eq!(apps(&e), ["photocraft", "wordcraft"]);
    assert_eq!(models(&e), ["m1", "m2"]);
}

#[test]
fn totals_count_outcomes_and_sum_cost_per_currency() {
    let e = entries(&[
        ("a", T0, "p", "m", Some(Outcome::Done), Some(40_000)),
        ("b", T0 + 1, "p", "m", Some(Outcome::Done), Some(25_000)),
        ("c", T0 + 2, "p", "m", Some(Outcome::Done), None),
        ("d", T0 + 3, "p", "m", Some(Outcome::Failed { kind: ErrorKind::Quota }), None),
        ("e", T0 + 4, "p", "m", Some(Outcome::Cancelled), None),
        ("f", T0 + 5, "p", "m", None, None),
    ]);
    let t = totals(&e);
    assert_eq!(t.jobs, 6);
    assert_eq!((t.done, t.failed, t.cancelled, t.unfinished), (3, 1, 1, 1));
    assert_eq!(t.cost_micros.get("USD"), Some(&65_000));
    assert_eq!(t.without_cost, 1, "only a finished success counts as unpriced");
}

#[test]
fn a_huge_cost_saturates_instead_of_wrapping() {
    let e = entries(&[("a", T0, "p", "m", Some(Outcome::Done), Some(u64::MAX)), ("b", T0 + 1, "p", "m", Some(Outcome::Done), Some(10))]);
    assert_eq!(totals(&e).cost_micros.get("USD"), Some(&u64::MAX));
}

#[test]
fn days_and_months_group_newest_first() {
    let day = 24 * HOUR;
    let e = entries(&[
        ("a", T0, "p", "m", Some(Outcome::Done), Some(10_000)),
        ("b", T0 + HOUR, "p", "m", Some(Outcome::Done), Some(20_000)),
        ("c", T0 + day, "p", "m", Some(Outcome::Done), Some(5_000)),
        ("d", T0 + 30 * day, "p", "m", Some(Outcome::Done), Some(1_000)),
    ]);
    let refs: Vec<&HistoryEntry> = e.iter().collect();
    let days = by_day(&refs, 0);
    let labels: Vec<_> = days.iter().map(|(d, t)| (d.to_string(), t.jobs)).collect();
    assert_eq!(labels, [("2025-11-08".to_owned(), 1), ("2025-10-10".to_owned(), 1), ("2025-10-09".to_owned(), 2)]);
    let months = by_month(&refs, 0);
    let labels: Vec<_> = months.iter().map(|(m, t)| (m.to_string(), t.jobs, t.cost_micros.get("USD").copied())).collect();
    assert_eq!(labels, [("2025-11".to_owned(), 1, Some(1_000)), ("2025-10".to_owned(), 3, Some(35_000))]);
}

#[test]
fn csv_has_a_header_one_row_per_entry_and_matching_column_counts() {
    let mut e = entries(&[("a", T0, "p", "m", Some(Outcome::Done), Some(31_000)), ("b", T0 + 5, "p", "m", None, None)]);
    e[0].start.estimate = Some(CostEstimate { micros: 30_000, currency: "USD".into(), billing: crate::Billing::Metered });
    let refs: Vec<&HistoryEntry> = e.iter().collect();
    let csv = to_csv(&refs);
    let lines: Vec<&str> = csv.split("\r\n").collect();
    assert_eq!(lines.len(), 4, "header, two rows, trailing empty");
    assert!(lines[0].starts_with("started_utc,app,feature"));
    assert_eq!(lines[0].split(',').count(), CSV_HEADER.len());
    // Newest first: the unfinished job (b) is the first row.
    assert!(lines[1].contains(",unfinished,"), "{}", lines[1]);
    assert!(lines[1].contains("0.030000,USD"), "estimate: {}", lines[1]);
    assert!(lines[2].contains("0.031000,USD"), "{}", lines[2]);
    assert!(lines[2].contains(",done,"), "{}", lines[2]);
    assert!(lines[2].contains(",2.000,"), "two seconds: {}", lines[2]);
    assert_eq!(lines[1].split(',').count(), CSV_HEADER.len());
    assert_eq!(lines[2].split(',').count(), CSV_HEADER.len());
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
fn a_failed_job_exports_its_error_kind() {
    let e = entries(&[("a", T0, "p", "m", Some(Outcome::Failed { kind: ErrorKind::RateLimited }), None)]);
    let refs: Vec<&HistoryEntry> = e.iter().collect();
    assert!(to_csv(&refs).contains(",failed,rate_limited,"));
}

#[test]
fn list_times_and_day_starts_follow_the_offset() {
    assert_eq!(time_text(T0, 0), "2025-10-09 08:53");
    assert_eq!(time_text(T0, 120), "2025-10-09 10:53");
    assert_eq!(time_text(T0, -540), "2025-10-08 23:53");
    // 2025-10-09T08:53:20Z; the UTC day began at 00:00Z.
    assert_eq!(start_of_day_ms(T0, 0), 1_759_968_000_000);
    // At UTC+2 the local day began at 22:00Z the evening before.
    assert_eq!(start_of_day_ms(T0, 120), 1_759_968_000_000 - 2 * HOUR);
    // At UTC-9 the local day (Oct 8) began at 09:00Z on Oct 8.
    assert_eq!(start_of_day_ms(T0, -540), 1_759_968_000_000 - 24 * HOUR + 9 * HOUR);
    assert_eq!(start_of_day_ms(0, 0), 0);
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
}

const DAY_MS_FOR_TESTS: u64 = 86_400_000;

#[test]
fn the_daily_series_fills_empty_days_and_keeps_the_most_recent() {
    let day = DAY_MS_FOR_TESTS;
    let e = entries(&[
        ("a", T0, "p", "m", Some(Outcome::Done), Some(10_000)),
        ("b", T0 + 3 * day, "p", "m", Some(Outcome::Done), Some(5_000)),
        ("c", T0 + 3 * day + 1, "p", "m", None, None),
    ]);
    let refs: Vec<&HistoryEntry> = e.iter().collect();
    let all = daily(&refs, 0, 31);
    let shape: Vec<(String, usize)> = all.iter().map(|(d, t)| (d.to_string(), t.jobs)).collect();
    assert_eq!(shape, [("2025-10-09".to_owned(), 1), ("2025-10-10".to_owned(), 0), ("2025-10-11".to_owned(), 0), ("2025-10-12".to_owned(), 2)]);
    let recent = daily(&refs, 0, 2);
    let shape: Vec<String> = recent.iter().map(|(d, _)| d.to_string()).collect();
    assert_eq!(shape, ["2025-10-11", "2025-10-12"]);
    assert!(daily(&[], 0, 31).is_empty());
}

#[test]
fn models_and_apps_group_busiest_first_and_average_only_successful_timed_jobs() {
    let e = entries(&[
        ("a", T0, "photocraft", "flux", Some(Outcome::Done), Some(50_000)),
        ("b", T0 + 10, "photocraft", "flux", Some(Outcome::Done), Some(50_000)),
        ("c", T0 + 20, "wordcraft", "gpt", Some(Outcome::Done), Some(1_000)),
        ("d", T0 + 30, "wordcraft", "gpt", Some(Outcome::Failed { kind: ErrorKind::Network }), None),
        ("e", T0 + 40, "wordcraft", "gpt", None, None),
    ]);
    let refs: Vec<&HistoryEntry> = e.iter().collect();
    let models = by_model(&refs);
    assert_eq!(models.iter().map(|(k, t)| (k.as_str(), t.jobs)).collect::<Vec<_>>(), [("gpt", 3), ("flux", 2)]);
    let apps = by_app(&refs);
    assert_eq!(apps[0].0, "wordcraft");
    // Each finished job took two seconds in the fixture; the failed and unfinished ones do not count.
    assert_eq!(models[0].1.average_ms(), Some(2_000));
    assert_eq!(models[0].1.timed, 1);
    assert_eq!(Totals::default().average_ms(), None);
}
