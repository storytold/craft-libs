//! The names the printing stack imports from this crate, without the `serde` feature. A compile
//! check: removing or renaming any of them is a breaking change for craft-print.

use std::time::Duration;

use craft_job::{
    BackendName, DeviceId, DocumentRef, ErrorSummary, ExpectedTotals, Fingerprint, HRESULT_DOMAIN, Jitter, JobDocuments, JobId, JobKind, JobState,
    JobTimes, JobView, NativeCode, ParseIdError, ParseIdErrorKind, Progress, RetryAdvice, RetryFacts, RetryPolicy, RetrySafety, Tag, TagError,
    TagPlacement, TagSupport, Timestamp, advise, namespaced_id, split_id,
};

#[test]
fn the_names_print_imports_exist_and_have_the_methods_it_calls() {
    let id: JobId = "wsd:42".parse().expect("a namespaced id");
    let _device: DeviceId = "wsd:printer".parse().expect("a namespaced id");
    let _backend: BackendName = "wsd".parse().expect("a backend name");
    let error: ParseIdError = "nocolon".parse::<JobId>().expect_err("no backend prefix");
    let _kind: ParseIdErrorKind = error.kind();
    assert_eq!(split_id(&namespaced_id("wsd", "42")), Some(("wsd", "42")));

    assert!(JobState::Abandoned.is_terminal() && !JobState::Processing.is_terminal());
    assert_eq!(JobState::Abandoned.name(), "abandoned");
    let _ = (JobKind::Print, JobKind::Scan);
    let _ = (Progress::default(), ExpectedTotals::default());

    let _ = NativeCode::new(HRESULT_DOMAIN, 1, None);

    let facts = RetryFacts::new(true, RetrySafety::OutcomeUnknown).with_attempts(1);
    assert_eq!(advise(&facts, &RetryPolicy::default()), RetryAdvice::CheckDeviceFirst);
    let _ = Jitter::new(7);

    // A tag is exactly 8 lowercase hex characters; anything else is a `TagError`.
    let tag = Tag::parse("01234567").expect("a valid tag");
    assert_eq!(tag.as_str(), "01234567");
    let _: TagError = Tag::parse("0123456789abcdef").unwrap_err();
    let _: TagError = Tag::parse("0123456G").unwrap_err();
    let fingerprint = Fingerprint::new("title", Timestamp::from_millis(1)).with_user("user").with_window(Duration::from_secs(5)).with_tag(tag);
    let _ = (TagPlacement::None, TagSupport::None, fingerprint, JobTimes::default());

    let summary = ErrorSummary::new("jammed", "jam");
    assert_eq!(summary.kind_label, "jam");
    let mut view = JobView::new(id, JobKind::Print, "title", JobState::Pending);
    let _ = (&mut view, JobDocuments::default(), DocumentRef::new("a.pdf"));
}
