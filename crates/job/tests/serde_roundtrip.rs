//! The wire format of the optional `serde` feature: round trips and the names that are promised.
#![cfg(feature = "serde")]

use std::time::Duration;

use craft_job::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Serialises `value`, reads it back, checks the two are equal and returns the JSON text.
fn round<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> String {
    let text = serde_json::to_string(value).expect("serialise");
    let back: T = serde_json::from_str(&text).expect("deserialise");
    assert_eq!(&back, value, "{text}");
    text
}

#[test]
fn ids_are_strings() {
    assert_eq!(round(&JobId::new("wsd", "urn:uuid:1:2")), "\"wsd:urn:uuid:1:2\"");
    assert_eq!(round(&DeviceId::new("sane", "pixma:usb")), "\"sane:pixma:usb\"");
    assert_eq!(round(&BackendName::new("escl").expect("name")), "\"escl\"");
    assert!(serde_json::from_str::<JobId>("\"nocolon\"").is_err());
    assert!(serde_json::from_str::<JobId>("\"Bad:1\"").is_err());
    assert!(serde_json::from_str::<BackendName>("\"\"").is_err());
    assert_eq!(round(&Tag::from_seed(3)), format!("\"{}\"", Tag::from_seed(3)));
    assert!(serde_json::from_str::<Tag>("\"ABC\"").is_err());
}

#[test]
fn native_code_reads_back_with_static_strings() {
    let code = NativeCode::hresult(0x8021_0006, Some("WIA_ERROR_BUSY"));
    assert_eq!(round(&code), r#"{"domain":"hresult","code":2149646342,"name":"WIA_ERROR_BUSY"}"#);
    let odd = NativeCode::new("vendor-x", -4, None);
    round(&odd);
    let again: NativeCode = serde_json::from_str(r#"{"domain":"vendor-x","code":-4}"#).expect("name is optional");
    assert_eq!(again, odd);
    // Over-long text is not interned: the domain reads back as "other".
    let long = format!(r#"{{"domain":"{}","code":1,"name":"{}"}}"#, "d".repeat(200), "n".repeat(200));
    let read: NativeCode = serde_json::from_str(&long).expect("parses");
    assert_eq!((read.domain, read.name), ("other", None));
}

#[test]
fn enums_use_snake_case_and_unknown_values_fall_back() {
    assert_eq!(round(&JobKind::Scan), "\"scan\"");
    assert_eq!(round(&JobState::CompletedWithErrors), "\"completed_with_errors\"");
    assert_eq!(round(&JobState::NeedsAttention), "\"needs_attention\"");
    assert_eq!(round(&JobState::Abandoned), "\"abandoned\"");
    assert_eq!(serde_json::from_str::<JobState>("\"from_the_future\"").expect("falls back"), JobState::Unknown);
    assert_eq!(serde_json::from_str::<DeviceState>("\"hibernating\"").expect("falls back"), DeviceState::Unknown);
    assert_eq!(serde_json::from_str::<DeviceConnection>("\"bluetooth\"").expect("falls back"), DeviceConnection::Unknown);
    assert_eq!(serde_json::from_str::<Tri>("\"maybe\"").expect("falls back"), Tri::NotReported);
    assert_eq!(serde_json::from_str::<TagPlacement>("\"carrier_pigeon\"").expect("falls back"), TagPlacement::None);
    assert_eq!(round(&Tri::NotReported), "\"not_reported\"");
    assert_eq!(round(&Severity::Warning), "\"warning\"");
    assert_eq!(round(&RetrySafety::OutcomeUnknown), "\"outcome_unknown\"");
    assert!(serde_json::from_str::<RetrySafety>("\"sometimes\"").is_err(), "no fallback: a retry rule is not guessed");
    assert_eq!(round(&TagSupport::Carried(TagPlacement::JobNameSuffix)), r#"{"carried":"job_name_suffix"}"#);
    assert_eq!(round(&TagSupport::None), "\"none\"");
    assert_eq!(round(&PaperSource::Tray("A4".into())), r#"{"tray":"A4"}"#);
    assert_eq!(round(&PaperSource::Glass), "\"glass\"");
}

#[test]
fn advice_durations_are_milliseconds() {
    assert_eq!(round(&RetryAdvice::RetryNow), "\"retry_now\"");
    assert_eq!(round(&RetryAdvice::RetryAfter(Duration::from_millis(1500))), r#"{"retry_after":1500}"#);
    assert_eq!(round(&RetryAdvice::CheckDeviceFirst), "\"check_device_first\"");
    assert_eq!(round(&RetryAdvice::DoNotRetry), "\"do_not_retry\"");
    let sub = serde_json::to_string(&RetryAdvice::RetryAfter(Duration::from_micros(1999))).expect("ser");
    assert_eq!(sub, r#"{"retry_after":1}"#, "finer than a millisecond is dropped");
}

#[test]
fn indicator_keys_are_names_and_unknown_names_become_other() {
    assert_eq!(round(&IndicatorKey::MultiFeed), "\"multi_feed\"");
    assert_eq!(round(&IndicatorKey::Other("toner_door")), "\"toner_door\"");
    let key: IndicatorKey = serde_json::from_str("\"jam\"").expect("known");
    assert_eq!(key, IndicatorKey::Jam);
    let key: IndicatorKey = serde_json::from_str("\"fuser_hot\"").expect("unknown");
    assert_eq!(key, IndicatorKey::Other("fuser_hot"));
    let key: IndicatorKey = serde_json::from_str(&format!("\"{}\"", "k".repeat(500))).expect("long");
    assert_eq!(key, IndicatorKey::Other("other"));
}

#[test]
fn times_progress_documents_and_fingerprint() {
    assert_eq!(round(&Timestamp::from_millis(1234)), "1234");
    let times = JobTimes::new().with_created(Timestamp::from_secs(1)).with_finished(Timestamp::from_secs(3));
    assert_eq!(round(&times), r#"{"created":1000,"started":null,"finished":3000}"#);
    assert_eq!(serde_json::from_str::<JobTimes>("{}").expect("all optional"), JobTimes::new());
    assert_eq!(round(&Progress::of(2, 5)), r#"{"done":2,"total":5}"#);
    assert_eq!(serde_json::from_str::<Progress>(r#"{"done":2}"#).expect("total optional"), Progress::done_only(2));
    round(&ExpectedTotals::for_request(Some(5), true, 2));
    let docs = JobDocuments::new().with(DocumentRef::new("page-1").with_format("png").with_pages(1).with_bytes(10)).with(DocumentRef::new("page-2"));
    assert!(round(&docs).starts_with("[{\"name\":\"page-1\""));
    let fp = Fingerprint::new("Report", Timestamp::from_secs(9)).with_user("u").with_window(Duration::from_secs(60)).with_tag(Tag::from_seed(1));
    round(&fp);
    let bare: Fingerprint = serde_json::from_str(r#"{"title":"T","submitted":5}"#).expect("window defaults");
    assert_eq!(bare.window, Fingerprint::DEFAULT_WINDOW);
}

/// A job view with every field set, used to check the full wire shape.
fn sample_view() -> JobView {
    JobView::new(JobId::new("wia", "local-3"), JobKind::Scan, "Invoices", JobState::NeedsAttention)
        .with_reason("jam")
        .with_progress(Progress::of(12, 40))
        .with_times(JobTimes::new().with_created(Timestamp::from_secs(1)).with_started(Timestamp::from_secs(2)))
        .with_documents(JobDocuments::new().with(DocumentRef::new("page-1").with_format("jpeg").with_pages(1).with_bytes(99)))
        .with_attempts(2)
        .with_error(ErrorSummary::new("The scanner has a paper jam.", "jam").with_retry_advice(RetryAdvice::DoNotRetry).with_native(NativeCode::new(
            "http",
            503,
            Some("Service Unavailable"),
        )))
        .with_device_name("Desk scanner")
}

#[test]
fn a_job_view_round_trips_and_tolerates_missing_and_extra_fields() {
    let view = sample_view();
    let text = round(&view);
    assert!(text.contains("\"kind\":\"scan\"") && text.contains("\"state\":\"needs_attention\"") && text.contains("\"id\":\"wia:local-3\""));
    // Fields a later version adds are ignored; optional ones an earlier version lacked are defaulted.
    let minimal = r#"{"id":"wia:local-3","kind":"scan","title":"T","state":"completed","future_field":{"x":1}}"#;
    let read: JobView = serde_json::from_str(minimal).expect("minimal view");
    assert_eq!((read.state, read.attempts, read.progress), (JobState::Completed, 0, None));
    assert!(read.reasons.is_empty() && read.documents.is_empty() && read.last_error.is_none());
}

#[test]
fn a_device_view_round_trips() {
    let view = DeviceView::new(DeviceId::new("escl", "scanner-1"), "Hall scanner", JobKind::Scan)
        .with_connection(DeviceConnection::Network)
        .with_state(DeviceState::Attention)
        .with_capabilities(CapabilitiesSummary::new().with_feeder(Tri::On).with_flatbed(Tri::Off).with_honoured("dpi"))
        .with_indicator(Indicator::new(IndicatorKey::Jam, Tri::On).with_detail("front"))
        .with_indicator(Indicator::new(IndicatorKey::Other("fuser"), Tri::NotReported))
        .with_paper(PaperPathView::new().with_feeder_loaded(Tri::On).with_feeder_capacity(50).with_source_in_use(PaperSource::Feeder))
        .with_supply(Supply::new("toner", Tri::Off).with_level_percent(5));
    let text = round(&view);
    assert!(text.contains("\"key\":\"jam\"") && text.contains("\"severity\":\"error\""));
    let read: DeviceView = serde_json::from_str(r#"{"id":"escl:s","name":"N","kind":"scan"}"#).expect("minimal device");
    assert_eq!((read.state, read.connection), (DeviceState::Unknown, DeviceConnection::Unknown));
}
