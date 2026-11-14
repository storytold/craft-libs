use super::*;

fn start(run: &str, at_ms: u64) -> StartRecord {
    StartRecord {
        v: HISTORY_SCHEMA,
        run: run.into(),
        at_ms,
        app: "photocraft".into(),
        role_id: "fill".into(),
        role_label: "Generative Fill".into(),
        capability: "image.inpaint".into(),
        connection: "conn1".into(),
        connection_label: "Hosted".into(),
        model: "m1".into(),
        locality: Locality::Remote,
        destination: "https://api.example.com".into(),
        sent: Vec::new(),
        params: vec!["seed".into()],
        provider_job: Some("gen-9".into()),
        estimate: None,
        prompt: None,
    }
}

fn end(run: &str, at_ms: u64, outcome: Outcome) -> EndRecord {
    EndRecord { v: HISTORY_SCHEMA, run: run.into(), at_ms, outcome, usage: None }
}

fn s(r: StartRecord) -> HistoryRecord {
    HistoryRecord::Start(r)
}

fn e(r: EndRecord) -> HistoryRecord {
    HistoryRecord::End(r)
}

#[test]
fn a_line_round_trips_and_leaves_out_what_is_absent() {
    let line = serde_json::to_string(&s(start("a", 5))).unwrap();
    assert!(!line.contains('\n'));
    assert!(!line.contains("prompt"), "{line}");
    assert!(!line.contains("estimate"), "{line}");
    assert_eq!(serde_json::from_str::<HistoryRecord>(&line).unwrap(), s(start("a", 5)));
    let failed = e(end("a", 9, Outcome::Failed { kind: ErrorKind::Quota }));
    let line = serde_json::to_string(&failed).unwrap();
    assert!(line.contains("\"phase\":\"end\""), "{line}");
    assert_eq!(serde_json::from_str::<HistoryRecord>(&line).unwrap(), failed);
}

#[test]
fn an_unknown_phase_does_not_parse_so_readers_can_skip_it() {
    assert!(serde_json::from_str::<HistoryRecord>(r#"{"phase":"pause","run":"a"}"#).is_err());
}

#[test]
fn start_and_end_join_by_run_and_the_list_is_newest_first() {
    let log = merge(vec![s(start("old", 10)), s(start("new", 30)), e(end("new", 45, Outcome::Done)), e(end("old", 12, Outcome::Cancelled))], 0);
    let runs: Vec<_> = log.entries.iter().map(|x| x.start.run.as_str()).collect();
    assert_eq!(runs, ["new", "old"]);
    assert_eq!(log.entries[0].duration_ms(), Some(15));
    assert_eq!(log.entries[1].outcome(), Some(&Outcome::Cancelled));
}

#[test]
fn a_start_with_no_end_stays_unfinished() {
    let log = merge(vec![s(start("a", 10))], 0);
    assert_eq!(log.entries.len(), 1);
    assert_eq!(log.entries[0].outcome(), None);
    assert_eq!(log.entries[0].duration_ms(), None);
}

#[test]
fn an_end_whose_start_was_rotated_away_is_dropped() {
    let log = merge(vec![e(end("gone", 5, Outcome::Done))], 2);
    assert!(log.entries.is_empty());
    assert_eq!(log.skipped, 2);
}

#[test]
fn the_first_end_wins_and_a_repeated_start_is_ignored() {
    let log = merge(vec![s(start("a", 10)), s(start("a", 99)), e(end("a", 20, Outcome::Done)), e(end("a", 30, Outcome::Cancelled))], 0);
    assert_eq!(log.entries.len(), 1);
    assert_eq!(log.entries[0].start.at_ms, 10);
    assert_eq!(log.entries[0].outcome(), Some(&Outcome::Done));
}

#[test]
fn a_clock_that_went_backwards_gives_no_duration() {
    let log = merge(vec![s(start("a", 50)), e(end("a", 40, Outcome::Done))], 0);
    assert_eq!(log.entries[0].duration_ms(), None);
}

#[test]
fn cost_is_only_what_the_provider_reported() {
    let mut finished = end("a", 20, Outcome::Done);
    let mut log = merge(vec![s(start("a", 10)), e(finished.clone())], 0);
    assert_eq!(log.entries[0].reported_cost(), None);

    finished.usage = Some(Usage { cost_micros: Some(40_000), currency: None, ..Usage::default() });
    log = merge(vec![s(start("a", 10)), e(finished.clone())], 0);
    assert_eq!(log.entries[0].reported_cost(), None, "no currency, no cost");

    finished.usage = Some(Usage { cost_micros: Some(40_000), currency: Some("USD".into()), ..Usage::default() });
    log = merge(vec![s(start("a", 10)), e(finished)], 0);
    assert_eq!(log.entries[0].reported_cost(), Some((40_000, "USD")));
}

#[test]
fn prompts_are_clipped_on_a_character_boundary() {
    let long = "é".repeat(MAX_PROMPT_CHARS + 50);
    let clipped = clip_prompt(&long);
    assert_eq!(clipped.chars().count(), MAX_PROMPT_CHARS);
    assert_eq!(clip_prompt("short"), "short");
}
