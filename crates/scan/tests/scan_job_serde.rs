//! The journal and wire format of `ScanJob` under the `serde` feature: snapshots round-trip, a journal written
//! from a snapshot seeds a continuation in another "process", and the existing wire strings are unchanged.
#![cfg(feature = "serde")]

mod common;

use craft_job::{JobState, Timestamp};
use craft_scan::job::{Continuation, ExitClass, Outcome, PageInfo, ScanJob, ScanJobConfig, ScanJobJournal, ScanJobSnapshot, ScanJobState};
use craft_scan::{
    Backend, Color, Compression, Connection, ImageFormat, Intent, InterruptKind, NativeCode, PageSize, Phase, ScanError, ScanSettings, Source,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use common::sane_like::SaneLike;

fn t(secs: u64) -> Timestamp {
    Timestamp::from_secs(secs)
}

fn round<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> String {
    let text = serde_json::to_string(value).expect("serialise");
    let back: T = serde_json::from_str(&text).expect("deserialise");
    assert_eq!(&back, value, "{text}");
    text
}

#[test]
fn the_contract_enums_keep_their_wire_strings() {
    assert_eq!(round(&Source::Glass), "\"glass\"");
    assert_eq!(round(&Color::BlackAndWhite), "\"bw\"");
    assert_eq!(round(&Intent::Text), "\"text\"");
    assert_eq!(round(&PageSize::BusinessCard), "\"business-card\"");
    assert_eq!(round(&ImageFormat::Jpeg), "\"jpeg\"");
    assert_eq!(round(&Compression::ALL[0]), format!("\"{}\"", Compression::ALL[0].as_str()));
    assert_eq!(round(&Connection::Wsd), "\"wsd\"");
    assert_eq!(round(&InterruptKind::BusyOrFeederEmpty), "\"busy_or_feeder_empty\"");
    assert_eq!(round(&InterruptKind::Cancelled), "\"cancelled\"");
    assert_eq!(round(&Phase::MidPage), "\"mid_page\"");
    // Aliases the CLI accepts are accepted; an unknown source is an error; an unknown interrupt kind reads as Other.
    assert_eq!(serde_json::from_str::<Source>("\"flatbed\"").expect("alias"), Source::Glass);
    assert!(serde_json::from_str::<Source>("\"scanner_arm\"").is_err());
    assert_eq!(serde_json::from_str::<InterruptKind>("\"on_fire\"").expect("falls back"), InterruptKind::Other);
}

#[test]
fn job_state_names_are_stable() {
    assert_eq!(round(&ScanJobState::NeedsAttention), "\"needs_attention\"");
    assert_eq!(round(&ScanJobState::Terminal(Outcome::CancelledByDevice)), r#"{"terminal":"cancelled_by_device"}"#);
    assert_eq!(round(&Continuation::Append), "\"append\"");
    assert_eq!(round(&ExitClass::NeedsAttention), "\"needs_attention\"");
}

fn feeder(pages: u32) -> ScanSettings {
    ScanSettings { device: "sane:x".into(), source: Source::Feeder, pages, ..ScanSettings::default() }
}

fn interrupted_job() -> ScanJob {
    let mut job = ScanJob::new(feeder(40), ScanJobConfig::default().with_title("Archive").with_created(t(5)));
    let plan = job.begin_attempt(t(10)).expect("attempt");
    job.finish_attempt(SaneLike::new(12).acquire(&plan.settings), t(20)).expect("finish");
    job
}

#[test]
fn a_snapshot_round_trips() {
    let snap = interrupted_job().snapshot();
    let text = round(&snap);
    assert!(text.contains("\"state\":\"interrupted\"") && text.contains("\"kind\":\"feeder_empty\""), "{text}");
    assert!(!text.contains("\"bytes\":[") && text.len() < 16 * 1024, "no page bytes: {} bytes of JSON", text.len());
    let read: ScanJobSnapshot = serde_json::from_str(&text).expect("read back");
    assert_eq!(read.pages.len(), 12);
    assert_eq!(read.pages[0].format, "jpeg");
    assert_eq!(read.exit_class(), ExitClass::NeedsAttention);
}

#[test]
fn a_journal_written_from_a_snapshot_seeds_a_continuation_in_another_process() {
    let journal = interrupted_job().snapshot().into_journal();
    assert_eq!(journal.version, ScanJobJournal::VERSION);
    // The first process ends here; the journal is a file.
    let file = serde_json::to_string_pretty(&journal).expect("write journal");

    // `scan --continue DIR` in a new process: the journal and the original request.
    let read: ScanJobJournal = serde_json::from_str(&file).expect("read journal");
    assert_eq!(read, journal);
    let mut job = ScanJob::continuing_from(feeder(0), ScanJobConfig::default().with_title("Archive"), read);
    let view = job.view();
    assert_eq!(view.state, JobState::NeedsAttention);
    assert_eq!(view.progress.map(|p| (p.done, p.total)), Some((12, Some(40))), "the wanted count comes from the journal when --pages is not given");
    assert_eq!(view.attempts, 1);
    assert_eq!(view.last_error.as_ref().map(|e| e.kind_label.as_str()), Some("feeder_empty"));

    let plan = job.begin_attempt_with(t(100), Continuation::Append).expect("append");
    assert_eq!((plan.number, plan.settings.pages, plan.kept_pages), (2, 28, 12));
    job.finish_attempt(SaneLike::new(28).acquire(&plan.settings), t(120)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Completed));
    assert_eq!(job.page_count(), 40);
}

#[test]
fn an_unknown_outcome_survives_the_journal() {
    let mut job = ScanJob::new(feeder(10), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    for _ in 0..3 {
        plan.progress.push_page(craft_scan::Page {
            bytes: vec![0; 8],
            format: "jpeg",
            width: 16,
            height: 8,
            dpi: 200,
            dpi_y: 200,
            requested_dpi: 200,
            thresholded: false,
            length: Default::default(),
        });
    }
    let offline = ScanError::native_error(InterruptKind::Offline, NativeCode::new("http", 503, None), InterruptKind::Offline.sentence());
    job.finish_attempt(Err(offline), t(2)).expect("finish");
    let text = serde_json::to_string(&job.snapshot().into_journal()).expect("journal");
    let journal: ScanJobJournal = serde_json::from_str(&text).expect("read");
    let again = ScanJob::continuing_from(feeder(10), ScanJobConfig::default(), journal);
    assert_eq!(again.state(), ScanJobState::NeedsAttention);
    assert_eq!(again.retry_advice(), craft_job::RetryAdvice::CheckDeviceFirst);
    assert_eq!(again.page_count(), 3);
}

#[test]
fn a_journal_tolerates_unknown_and_missing_fields() {
    let text = r#"{"title":"T","state":"interrupted","future":[1,2],"pages":[
        {"number":1,"format":"tiff","attempt":1,"extra":true},{"number":2,"format":"holographic"}]}"#;
    let journal: ScanJobJournal = serde_json::from_str(text).expect("tolerant");
    assert_eq!((journal.version, journal.attempts_total, journal.expected_pages), (ScanJobJournal::VERSION, 0, None));
    assert_eq!(journal.pages.iter().map(|p| p.format).collect::<Vec<_>>(), ["tiff", "bin"], "an unknown page format reads as bin");
    let job = ScanJob::continuing_from(feeder(5), ScanJobConfig::default(), journal);
    assert_eq!(job.page_count(), 2);
    let info: PageInfo = serde_json::from_str(r#"{"number":4,"format":"png"}"#).expect("page");
    assert_eq!((info.number, info.bytes), (4, 0));
}

#[test]
fn the_page_length_wire_strings_and_the_per_page_record_survive_the_journal() {
    use craft_scan::{LengthRecord, LengthSource, PageLength};
    assert_eq!(round(&PageLength::AutoInfer), "\"infer\"");
    assert_eq!(round(&PageLength::Driver), "\"driver\"");
    assert_eq!(round(&LengthSource::Inferred), "\"inferred\"");
    let record = LengthRecord { source: LengthSource::Inferred, length_mils: Some(10_730), approximate: true };
    let text = round(&record);
    assert_eq!(text, r#"{"source":"inferred","length_mils":10730,"approximate":true}"#);
    let info =
        PageInfo { number: 1, attempt: 1, source: Some(Source::Feeder), format: "bmp", width: 10, height: 20, dpi: 200, bytes: 5, length: record };
    let again: PageInfo = serde_json::from_str(&serde_json::to_string(&info).expect("page")).expect("read");
    assert_eq!(again, info);
    // A journal written before the field existed reads as a driver length.
    let old: PageInfo = serde_json::from_str(r#"{"number":1,"format":"bmp","height":20,"dpi":200}"#).expect("old page");
    assert_eq!(old.length, LengthRecord::default());
    assert_eq!(old.length.source, LengthSource::Driver);
}

#[test]
fn a_job_plan_carries_the_page_length_request_unchanged() {
    use craft_scan::PageLength;
    let request = ScanSettings { page_length: PageLength::AutoInfer, ..feeder(3) };
    let mut job = ScanJob::new(request, ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    assert_eq!(plan.settings.page_length, PageLength::AutoInfer);
    assert_eq!(job.settings().page_length, PageLength::AutoInfer);
}
