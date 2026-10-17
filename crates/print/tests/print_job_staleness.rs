//! The staleness rules of `PrintJob`: an accepted print that is not heard from needs attention (and
//! recovers on news), a stalled one needs a look, a very old one is abandoned, none of it is a
//! failure or a resend, and the wait after a lost send is the device's own. All on an injected
//! clock and the exported fakes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use craft_print::testing::{FakeBackend, SubmitStep};
use craft_print::{
    BackendClass, BytesSource, Clock, DocumentFormat, ErrorKind, Job, JobQueryCaps, JobState, JobStatus, LifecycleState, ManualClock, Outcome, Phase,
    PrintError, PrintJob, PrintJobConfig, Reconciled, Registry, RetryAdvice, RetrySafety, SimpleState, StaleReason, Staleness, TagPlacement,
    TagSupport, UserConfirmed, WaitPolicy, WatchSupport,
};

const PDF: &[u8] = b"%PDF-1.7";

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn mins(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

fn at(n: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + secs(n)
}

fn unreachable_printer() -> PrintError {
    PrintError::new(ErrorKind::Unavailable, "no answer").with_safety(RetrySafety::Safe)
}

fn lost() -> PrintError {
    PrintError::new(ErrorKind::Unavailable, "the connection closed with no reply").with_phase(Phase::Reply).with_safety(RetrySafety::OutcomeUnknown)
}

fn reading(state: JobState) -> Result<JobStatus, PrintError> {
    Ok(JobStatus::new(state))
}

fn with_sheets(state: JobState, done: u32) -> Result<JobStatus, PrintError> {
    let mut status = JobStatus::new(state);
    status.sheets_done = Some(done);
    Ok(status)
}

struct Rig {
    registry: Registry,
    clock: ManualClock,
}

fn rig(fake: &FakeBackend) -> Rig {
    let registry = Registry::new();
    registry.register(Box::new(fake.clone())).unwrap();
    Rig { registry, clock: ManualClock::new() }
}

fn ipp(script: Vec<Result<JobStatus, PrintError>>) -> FakeBackend {
    FakeBackend::ipp_like("net").with_printer("p1", true).with_status_script(script)
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

/// Advance the job, moving the clock by what it asks for, until `span` has passed.
fn run_for(job: &mut PrintJob, rig: &Rig, span: Duration) {
    let end = rig.clock.now() + span;
    for _ in 0..50_000 {
        let now = rig.clock.now();
        if now >= end {
            return;
        }
        let wait = job.advance(&rig.registry).next_in.unwrap_or(end - now).min(end - now);
        rig.clock.advance(wait);
    }
    panic!("no progress: {:?}", job.state());
}

/// Run a freshly made job to the point where the printer accepted it.
fn accepted(job: &mut PrintJob, rig: &Rig) {
    for _ in 0..10 {
        job.advance(&rig.registry);
        if !matches!(job.state(), LifecycleState::Preparing | LifecycleState::Submitting) {
            return;
        }
    }
    panic!("not accepted: {:?}", job.state());
}

#[test]
fn silence_needs_attention_with_the_last_good_read_and_recovers_on_news_without_a_resend() {
    let fake = ipp(vec![reading(JobState::Processing), Err(unreachable_printer())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, mins(2));
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Processing), "two quiet minutes are not silence yet");
    assert_eq!(job.snapshot().stale, None);

    run_for(&mut job, &rig, secs(70));
    let snapshot = job.snapshot();
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    assert_eq!((snapshot.stale, snapshot.simple), (Some(StaleReason::NoNews), SimpleState::NeedsAttention));
    assert_eq!(snapshot.advice, RetryAdvice::CheckDeviceFirst, "the device may still print");
    assert_eq!(snapshot.last_news.map(|t| t.secs()), Some(1_000));
    assert_eq!(job.view().state, JobState::NeedsAttention);
    assert!(job.view().reasons.iter().any(|r| r == "no-news-since:1000"), "{:?}", job.view().reasons);
    assert!(job.advance(&rig.registry).next_in.is_some(), "probing goes on");

    // The printer comes back.
    fake.set_status_script(vec![with_sheets(JobState::Processing, 1)]);
    run_for(&mut job, &rig, secs(40));
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Processing));
    assert_eq!(job.snapshot().stale, None);
    assert!(job.view().reasons.iter().all(|r| !r.starts_with("no-news")));
    assert_eq!(fake.submissions().len(), 1, "a timeout never resends");
    assert!(job.snapshot().attempt == 1);
}

#[test]
fn a_print_that_was_never_read_says_so_in_the_token() {
    let fake = ipp(vec![Err(unreachable_printer())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, mins(4));
    assert_eq!(job.snapshot().stale, Some(StaleReason::NoNews));
    assert!(job.view().reasons.iter().any(|r| r == "no-news-since:unknown"));
}

#[test]
fn progress_resets_the_stall_clock_and_an_unchanged_reading_stalls_after_ten_minutes() {
    let fake = ipp(vec![with_sheets(JobState::Processing, 0)]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, mins(9));
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Processing));
    fake.set_status_script(vec![with_sheets(JobState::Processing, 1)]);
    run_for(&mut job, &rig, mins(9));
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Processing), "a sheet done is progress: the clock started again");
    run_for(&mut job, &rig, mins(2));
    let snapshot = job.snapshot();
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    assert_eq!(snapshot.stale, Some(StaleReason::Stalled));
    assert_eq!(snapshot.advice, RetryAdvice::CheckDeviceFirst);
    assert!(job.view().reasons.iter().any(|r| r == "stalled"));

    // The same reading again is no news; a changed one is.
    run_for(&mut job, &rig, mins(2));
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    fake.set_status_script(vec![with_sheets(JobState::Processing, 2)]);
    run_for(&mut job, &rig, secs(40));
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Processing));
    assert_eq!(fake.submissions().len(), 1);
}

#[test]
fn a_queued_job_is_given_thirty_minutes_and_a_processing_one_ten() {
    let fake = ipp(vec![reading(JobState::Pending)]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, mins(29));
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Pending));
    run_for(&mut job, &rig, mins(2));
    assert_eq!(job.snapshot().stale, Some(StaleReason::Stalled));

    let fake = ipp(vec![reading(JobState::Processing)]);
    let rig = self::rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, mins(11));
    assert_eq!(job.snapshot().stale, Some(StaleReason::Stalled));
}

#[test]
fn a_held_or_stopped_job_never_stalls_and_every_row_gives_up_at_the_ceiling() {
    let fake = ipp(vec![reading(JobState::Held)]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, mins(179));
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Held), "an explicit hold has no timeout");
    assert_eq!(job.snapshot().stale, None);
    run_for(&mut job, &rig, mins(2));
    let snapshot = job.snapshot();
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Abandoned));
    assert_eq!(
        (snapshot.stale, snapshot.simple, snapshot.advice),
        (Some(StaleReason::Abandoned), SimpleState::Abandoned, RetryAdvice::CheckDeviceFirst)
    );
    let view = job.view();
    assert_eq!(view.state, JobState::Abandoned);
    assert!(view.is_terminal() && view.needs_attention() && !view.state.is_success());
    assert!(view.reasons.iter().any(|r| r == "abandoned-no-outcome"));
    assert!(job.advance(&rig.registry).next_in.is_none(), "the row is done");
    assert_eq!(fake.submissions().len(), 1);
    assert!(fake.cancels().is_empty(), "abandoning asks nothing of the device");

    let stopped = ipp(vec![reading(JobState::NeedsAttention)]);
    let rig = self::rig(&stopped);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, mins(120));
    assert_eq!(job.state(), LifecycleState::Watching(JobState::NeedsAttention), "a jam is the printer's to report, not ours to time out");
}

#[test]
fn a_person_can_dismiss_a_row_and_optionally_cancel_at_the_device() {
    let fake = ipp(vec![reading(JobState::Processing)]);
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, secs(30));
    job.dismiss();
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Abandoned));
    assert!(fake.cancels().is_empty());
    assert!(job.view().reasons.iter().any(|r| r == "abandoned-no-outcome"));
    assert_eq!(job.view().state, JobState::Abandoned);
    job.dismiss();
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Abandoned));

    let fake = ipp(vec![reading(JobState::Processing)]);
    let rig = self::rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, secs(30));
    job.dismiss_and_cancel(&rig.registry).unwrap();
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Abandoned));
    assert_eq!(fake.cancels(), ["p1#1"]);

    // A backend that cannot cancel: the error is returned and the row is dismissed all the same.
    let fake = ipp(vec![reading(JobState::Processing)]).with_cancel_support(false);
    let rig = self::rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, secs(30));
    assert!(job.dismiss_and_cancel(&rig.registry).is_err());
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Abandoned));
}

#[test]
fn a_backend_without_status_is_not_polled_and_never_times_out() {
    let fake = FakeBackend::cups_like("lp").with_printer("q", true);
    let rig = rig(&fake);
    let mut job = PrintJob::new(
        Job { printer: Some("lp:q".into()), title: "Fire and forget".into(), ..Job::default() },
        BytesSource::new(DocumentFormat::Pdf, PDF.to_vec()),
    )
    .with_clock(Arc::new(rig.clock.clone()));
    accepted(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Accepted);
    assert!(job.is_settled());
    assert_eq!(job.advance(&rig.registry).next_in, None);
    rig.clock.advance(mins(600));
    job.advance(&rig.registry);
    assert_eq!(job.state(), LifecycleState::Accepted, "sent, no status available: no ceiling either");
    let snapshot = job.snapshot();
    assert_eq!((snapshot.stale, snapshot.simple), (None, SimpleState::Sent));
    assert_eq!(fake.status_calls(), 0);
    assert_eq!(job.view().state, JobState::Unknown);
}

#[test]
fn with_staleness_disabled_a_failed_watch_leaves_the_print_accepted_as_before() {
    let fake = ipp(vec![Err(unreachable_printer())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig).with_config(PrintJobConfig {
        staleness: Staleness::disabled(),
        watch: WaitPolicy::every(secs(1)).with_max_wait(secs(60)),
        ..PrintJobConfig::default()
    });
    run_for(&mut job, &rig, mins(30));
    assert_eq!(job.state(), LifecycleState::Accepted);
    assert!(job.is_settled());
    assert_eq!(job.snapshot().stale, None);
    assert_eq!(job.advance(&rig.registry).next_in, None);
    assert_eq!(fake.submissions().len(), 1);
}

#[test]
fn the_wait_after_a_lost_send_is_the_devices_own_window_before_a_resend_is_advised() {
    let backend = ipp(vec![]).with_submit_script(vec![SubmitStep::Fail(lost())]).with_open_job_timeout(secs(70));
    let rig = rig(&backend);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, secs(1));
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    assert_eq!(job.reconcile(&rig.registry).unwrap(), Reconciled::NotFound);
    assert_eq!(job.snapshot().advice, RetryAdvice::CheckDeviceFirst, "the device may still be holding the half-made job open");
    let left = job.reconcile_wait().unwrap();
    assert!(left > secs(60) && left <= secs(70), "{left:?}");
    rig.clock.advance(secs(60));
    assert_eq!(job.snapshot().advice, RetryAdvice::CheckDeviceFirst);
    rig.clock.advance(secs(10));
    assert_eq!((job.reconcile_wait(), job.snapshot().advice), (None, RetryAdvice::RetryNow));
    assert_eq!(backend.submissions().len(), 1, "nothing was resent on its own");

    // No hint from the backend: 65 seconds.
    let plain = FakeBackend::new("net")
        .with_class(BackendClass::Network)
        .with_printer("p1", true)
        .with_query_caps(JobQueryCaps::ALL)
        .with_tag_support(TagSupport::Carried(TagPlacement::NativeAttribute))
        .with_watch(WatchSupport::Poll)
        .with_submit_script(vec![SubmitStep::Fail(lost())]);
    let rig = self::rig(&plain);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, secs(1));
    let left = job.reconcile_wait().unwrap();
    assert!(left > secs(60) && left <= secs(65), "{left:?}");

    // The ipp-like fake says 240 seconds, the top of the IPP range.
    let long = ipp(vec![]).with_submit_script(vec![SubmitStep::Fail(lost())]);
    let rig = self::rig(&long);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, secs(1));
    assert!(job.reconcile_wait().unwrap() > secs(230));
}

#[test]
fn a_reconcile_that_finds_the_device_gave_up_advises_a_resend_and_a_live_job_is_adopted() {
    let lose = || ipp(vec![reading(JobState::Processing)]).with_submit_script(vec![SubmitStep::LoseReply(lost())]);

    let fake = lose();
    let rig = rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, secs(1));
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    fake.set_remote_state("p1#1", JobState::Aborted);
    fake.set_remote_reasons("p1#1", vec!["DocumentTimeoutError".into()]);
    let Reconciled::Found(found) = job.reconcile(&rig.registry).unwrap() else { panic!("not found") };
    assert_eq!(found.state, JobState::Aborted);
    assert_eq!(job.snapshot().advice, RetryAdvice::RetryNow, "the printer ended the empty job itself, even inside the window");

    // An aborted job with another reason is not that.
    fake.set_remote_reasons("p1#1", vec!["media-jam".into()]);
    job.reconcile(&rig.registry).unwrap();
    assert_eq!(job.snapshot().advice, RetryAdvice::DoNotRetry);

    let fake = lose();
    let rig = self::rig(&fake);
    let mut job = print_job(&rig);
    run_for(&mut job, &rig, secs(1));
    fake.set_remote_state("p1#1", JobState::Processing);
    let Reconciled::Found(found) = job.reconcile(&rig.registry).unwrap() else { panic!("not found") };
    job.accept(&rig.registry, &found.id).unwrap();
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Processing));
    assert_eq!(fake.submissions().len(), 1);
}
