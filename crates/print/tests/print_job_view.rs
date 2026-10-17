//! `PrintJob::view` and `Snapshot::view`: the tracked print as a `craft_job::JobView`, field by
//! field and for every lifecycle state, through the public API with the exported fakes on an
//! injected clock.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use craft_print::testing::{FakeBackend, SubmitStep};
use craft_print::{
    BytesSource, DocumentFormat, ErrorKind, Job, JobId, JobKind, JobOptions, JobState, JobStatus, LifecycleState, ManualClock, NativeCode, Phase,
    PrintError, PrintJob, Progress, Registry, RetryAdvice, RetrySafety, Tag, TagPlacement, TagSupport, Timestamp, UserConfirmed, WatchSupport,
};

const PDF: &[u8] = b"%PDF-1.7";

fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

fn status(state: JobState) -> JobStatus {
    JobStatus::new(state)
}

fn busy() -> PrintError {
    PrintError::new(ErrorKind::Busy, "the printer is busy").with_safety(RetrySafety::Safe)
}

struct Rig {
    registry: Registry,
    clock: ManualClock,
}

fn rig(backend: &FakeBackend) -> Rig {
    let registry = Registry::new();
    registry.register(Box::new(backend.clone())).unwrap();
    Rig { registry, clock: ManualClock::new() }
}

/// A network-like backend (tag in a native attribute) that reports the scripted statuses.
fn network(script: Vec<Result<JobStatus, PrintError>>) -> FakeBackend {
    FakeBackend::ipp_like("net").with_printer("p1", true).with_status_script(script)
}

/// A spooler-like backend: no tag support.
fn local(script: Vec<Result<JobStatus, PrintError>>) -> FakeBackend {
    FakeBackend::new("net").with_printer("p1", true).with_watch(WatchSupport::Poll).with_status_script(script)
}

fn print_job(rig: &Rig) -> PrintJob {
    PrintJob::new(
        Job { printer: Some("net:p1".into()), title: "Quarterly report".into(), ..Job::default() },
        BytesSource::new(DocumentFormat::Pdf, PDF.to_vec()),
    )
    .with_clock(Arc::new(rig.clock.clone()))
    .with_wall_clock(|| at(1_000))
    .with_consent(UserConfirmed::interactive())
}

fn drive(job: &mut PrintJob, rig: &Rig) {
    for _ in 0..200 {
        match job.advance(&rig.registry).next_in {
            Some(wait) => rig.clock.advance(wait),
            None => return,
        }
    }
    panic!("the job never settled: {:?}", job.state());
}

#[test]
fn a_new_print_is_a_pending_job_with_a_local_id_and_nothing_else() {
    let rig = rig(&network(vec![]));
    let job = print_job(&rig);
    let view = job.view();
    assert_eq!(view.kind, JobKind::Print);
    assert_eq!(view.state, JobState::Pending);
    assert_eq!(view.title, "Quarterly report");
    assert_eq!((view.id.backend(), view.id.native()), ("local", job.id().to_string().as_str()));
    assert_eq!(view.attempts, 0);
    assert!(view.reasons.is_empty() && view.progress.is_none() && view.documents.is_empty());
    assert!(view.last_error.is_none() && view.tag.is_none());
    assert_eq!(view.device_name.as_deref(), Some("net:p1"), "the printer the app asked for");
    assert_eq!(view.times.created, Some(Timestamp::from_secs(1_000)), "the injected wall clock stamps the creation");
    assert_eq!((view.times.started, view.times.finished), (None, None));
    assert!(!view.is_terminal() && !view.needs_attention());
}

#[test]
fn every_printer_state_shows_through_the_lifecycle_and_progress_and_reasons_follow() {
    let mut held = status(JobState::Held);
    held.reasons = vec!["job-hold-until-specified".into()];
    let mut processing = status(JobState::Processing);
    processing.sheets_done = Some(1);
    processing.sheets_total = Some(4);
    processing.reasons = vec!["job-printing".into()];
    let mut stopped = status(JobState::NeedsAttention);
    stopped.sheets_done = Some(2);
    stopped.reasons = vec!["media-empty".into()];
    let fake = network(vec![Ok(status(JobState::Pending)), Ok(held), Ok(processing), Ok(stopped), Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);

    // Preparing and Submitting: nothing sent yet.
    job.advance(&rig.registry);
    assert_eq!((job.state(), job.view().state), (LifecycleState::Submitting, JobState::Pending));

    // The send itself: accepted, status not read yet. The id is now the printer's.
    job.advance(&rig.registry);
    assert_eq!(job.state(), LifecycleState::Accepted);
    let accepted = job.view();
    assert_eq!(accepted.state, JobState::Unknown, "accepted, outcome unknown until a reading arrives");
    assert_eq!(accepted.id.to_string(), "net:p1#1");
    assert_eq!(accepted.device_name.as_deref(), Some("net:p1"));
    assert_eq!(accepted.attempts, 1);

    let mut seen = Vec::new();
    for _ in 0..4 {
        rig.clock.advance(Duration::from_secs(60));
        job.advance(&rig.registry);
        seen.push(job.view());
    }
    let states: Vec<_> = seen.iter().map(|v| v.state).collect();
    assert_eq!(states, [JobState::Pending, JobState::Held, JobState::Processing, JobState::NeedsAttention]);
    assert_eq!(seen[1].reasons, ["job-hold-until-specified"]);
    assert_eq!(seen[2].progress, Some(Progress::of(1, 4)));
    assert_eq!(seen[2].reasons, ["job-printing"]);
    assert_eq!(seen[3].progress.map(|p| p.done), Some(2));
    assert!(seen[3].needs_attention(), "a stopped printer is the one that needs a person");
    assert!(seen.iter().all(|v| !v.is_terminal()));

    rig.clock.advance(Duration::from_secs(60));
    job.advance(&rig.registry);
    let done = job.view();
    assert_eq!(done.state, JobState::Completed);
    assert!(done.is_terminal() && !done.needs_attention());
}

#[test]
fn a_finished_print_carries_its_document_times_tag_and_remote_id() {
    let fake = network(vec![Ok(status(JobState::Processing)), Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig).with_options(JobOptions::new().with_expected_pages(3));
    drive(&mut job, &rig);

    let view = job.view();
    assert_eq!((view.state, view.kind), (JobState::Completed, JobKind::Print));
    assert_eq!(view.id, JobId::new("net", "p1#1"));
    assert_eq!(view.attempts, 1);

    let document = view.documents.first().unwrap();
    assert_eq!(document.name, "Quarterly report");
    assert_eq!(document.format.as_deref(), Some("pdf"));
    assert_eq!(document.bytes, Some(PDF.len() as u64));
    assert_eq!(document.pages, Some(3));
    assert_eq!(view.documents.len(), 1);

    assert_eq!(view.tag, Some(*job.tag()), "a native placement carries the tag");
    assert_eq!(view.title, "Quarterly report", "the title is the app's, without the tag suffix");

    let created = view.times.created.unwrap();
    assert_eq!(view.times.started, Some(created), "the wall clock is fixed in this test");
    let finished = view.times.finished.unwrap();
    assert_eq!(
        finished.millis() - created.millis(),
        job.snapshot().elapsed.as_millis() as u64,
        "the end is the creation plus the job clock's elapsed time"
    );
    assert!(view.times.elapsed().is_some());
    assert_eq!(view.progress, None, "this printer reported no counters and none were expected");
}

#[test]
fn expected_totals_fill_the_progress_total() {
    let mut reading = status(JobState::Processing);
    reading.sheets_done = Some(2);
    let rig = rig(&network(vec![Ok(reading)]));
    let mut job = print_job(&rig).with_options(JobOptions::new().with_expected_pages(5));
    job.advance(&rig.registry);
    job.advance(&rig.registry);
    rig.clock.advance(Duration::from_secs(60));
    job.advance(&rig.registry);
    assert_eq!(job.view().progress, Some(Progress::of(2, 5)));
}

#[test]
fn completed_with_warnings_is_completed_plus_the_warnings_as_reasons() {
    let mut reading = status(JobState::Completed);
    reading.reasons = vec!["job-completed-successfully".into()];
    reading.warnings = vec!["the printer completed the job with warnings".into()];
    let rig = rig(&network(vec![Ok(reading)]));
    let mut job = print_job(&rig);
    drive(&mut job, &rig);
    let view = job.view();
    assert_eq!(view.state, JobState::Completed);
    assert!(view.state.is_success());
    assert_eq!(view.reasons, ["job-completed-successfully", "the printer completed the job with warnings"]);
    assert!(view.last_error.is_none());
}

#[test]
fn a_forgotten_job_is_completed_with_the_reason_it_is_believed_to_have_printed() {
    let fake = network(vec![Ok(status(JobState::Processing)), Err(PrintError::new(ErrorKind::NotFound, "the printer forgot the job"))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    drive(&mut job, &rig);
    let view = job.view();
    assert_eq!(view.state, JobState::Completed);
    assert!(view.reasons.iter().any(|r| r.contains("most likely finished")), "{:?}", view.reasons);
}

#[test]
fn a_printer_side_abort_is_aborted_and_completed_with_errors_keeps_its_name() {
    let rig_aborted = rig(&network(vec![Ok(status(JobState::Aborted))]));
    let mut job = print_job(&rig_aborted);
    drive(&mut job, &rig_aborted);
    let view = job.view();
    assert_eq!(view.state, JobState::Aborted);
    assert!(view.is_terminal() && view.needs_attention());

    let rig_errors = rig(&network(vec![Ok(status(JobState::CompletedWithErrors))]));
    let mut job = print_job(&rig_errors);
    drive(&mut job, &rig_errors);
    assert_eq!(job.state(), LifecycleState::Terminal(craft_print::Outcome::Failed));
    let view = job.view();
    assert_eq!(view.state, JobState::CompletedWithErrors);
    assert!(view.needs_attention());
}

#[test]
fn a_refusal_is_aborted_with_the_error_its_advice_and_its_native_code() {
    let native = NativeCode::new("ipp", 0x0400, Some("client-error-bad-request"));
    let fake = network(vec![])
        .failing_submit(PrintError::new(ErrorKind::Rejected, "the printer refused the job").with_native(native).with_phase(Phase::Reply));
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    drive(&mut job, &rig);
    let view = job.view();
    assert_eq!(view.state, JobState::Aborted);
    let error = view.last_error.unwrap();
    assert_eq!(error.message, "the printer refused the job");
    assert_eq!(error.kind_label, "rejected");
    assert_eq!(error.retry_advice, Some(RetryAdvice::DoNotRetry));
    assert_eq!(error.native, Some(native));
    assert_eq!(view.attempts, 1);
    assert_eq!(view.id.backend(), "local", "no printer job was ever accepted");
}

#[test]
fn an_unknown_outcome_needs_attention_and_says_to_check_the_printer_first() {
    let lost = PrintError::new(ErrorKind::Unavailable, "the connection closed with no reply")
        .with_phase(Phase::Reply)
        .with_safety(RetrySafety::OutcomeUnknown);
    let rig = rig(&network(vec![]).failing_submit(lost));
    let mut job = print_job(&rig);
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    let view = job.view();
    assert_eq!(view.state, JobState::NeedsAttention);
    assert!(!view.is_terminal() && view.needs_attention());
    let error = view.last_error.unwrap();
    assert_eq!((error.kind_label.as_str(), error.retry_advice), ("unavailable", Some(RetryAdvice::CheckDeviceFirst)));
}

#[test]
fn a_scheduled_retry_is_pending_and_the_error_says_when() {
    let fake =
        network(vec![Ok(status(JobState::Completed))]).with_submit_script(vec![SubmitStep::Fail(busy().with_retry_after(Duration::from_secs(5)))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    job.advance(&rig.registry);
    job.advance(&rig.registry);
    assert!(matches!(job.state(), LifecycleState::RetryScheduled { .. }));
    let view = job.view();
    assert_eq!(view.state, JobState::Pending);
    assert_eq!(view.attempts, 1);
    let error = view.last_error.unwrap();
    assert_eq!(error.kind_label, "busy");
    assert_eq!(error.retry_advice, Some(RetryAdvice::RetryAfter(Duration::from_secs(5))));
    drive(&mut job, &rig);
    let view = job.view();
    assert_eq!((view.state, view.attempts), (JobState::Completed, 2));
    assert!(view.last_error.is_none(), "an accepted attempt clears the error");
}

#[test]
fn a_cancelled_print_is_cancelled_and_not_a_problem() {
    let rig = rig(&network(vec![Ok(status(JobState::Processing))]));
    let mut job = print_job(&rig);
    job.cancel(&rig.registry).unwrap();
    let view = job.view();
    assert_eq!(view.state, JobState::Cancelled);
    assert!(view.is_terminal() && !view.needs_attention());
    assert!(view.times.finished.is_some());
}

#[test]
fn a_backend_that_cannot_report_leaves_the_job_unknown_and_a_spooler_carries_no_tag() {
    let fake = FakeBackend::new("net").with_printer("p1", true);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Accepted);
    let view = job.view();
    assert_eq!(view.state, JobState::Unknown);
    assert!(view.tag.is_none(), "the policy leaves the tag off for a local backend");
    assert!(!view.is_terminal());
}

#[test]
fn a_tag_in_the_job_name_is_in_the_view_and_a_manual_retry_clears_the_remote_id_and_the_end_time() {
    let fake = local(vec![Ok(status(JobState::Aborted)), Ok(status(JobState::Completed))])
        .with_tag_support(TagSupport::Carried(TagPlacement::JobNameSuffix));
    let rig = rig(&fake);
    let mut job = print_job(&rig).with_tag(Tag::from_seed(7)).with_tag_policy(craft_print::TagPolicy::Always);
    drive(&mut job, &rig);
    let aborted = job.view();
    assert_eq!(aborted.state, JobState::Aborted);
    assert_eq!(aborted.tag, Some(Tag::from_seed(7)));
    assert_eq!(aborted.title, "Quarterly report");
    assert!(aborted.times.finished.is_some());
    assert_eq!(aborted.id.to_string(), "net:p1#1");

    job.retry().unwrap();
    let again = job.view();
    assert_eq!(again.state, JobState::Pending);
    assert_eq!(again.id.backend(), "local", "the old remote job is not this attempt's");
    assert_eq!(again.times.finished, None, "a print that is going again has not finished");
    assert_eq!(again.attempts, 1, "the attempt count carries on from the history");

    drive(&mut job, &rig);
    let done = job.view();
    assert_eq!((done.state, done.attempts), (JobState::Completed, 2));
    assert_eq!(done.id.to_string(), "net:p1#2");
}

#[test]
fn the_snapshot_and_the_job_give_the_same_view() {
    let rig = rig(&network(vec![Ok(status(JobState::Processing)), Ok(status(JobState::Completed))]));
    let mut job = print_job(&rig);
    for _ in 0..4 {
        job.advance(&rig.registry);
        rig.clock.advance(Duration::from_secs(1));
        assert_eq!(job.view(), job.snapshot().view());
    }
}
