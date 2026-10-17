//! `PrintJob`, the tracked lifecycle of one print, driven through the public API with the exported
//! fakes on an injected clock: the state machine, automatic retry and its limits, the idempotency
//! tag, reconcile, cancel in every state and the common job-listing base. No sleeping anywhere.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use craft_print::testing::{FakeBackend, SubmitStep};
use craft_print::{
    AttemptOutcome, BytesSource, DocumentFormat, ErrorKind, Fingerprint, FnSource, Job, JobFilter, JobId, JobOptions, JobQueryCaps, JobState,
    JobStatus, LifecycleState, ManualClock, Outcome, Phase, PrintError, PrintJob, PrintJobConfig, Reconciled, Registry, RemoteJob, RetryAdvice,
    RetryPolicy, RetrySafety, SimpleState, Staleness, Tag, TagPlacement, TagPolicy, TagSupport, Timestamp, TrailEvent, UserConfirmed, WatchSupport,
};

const PDF: &[u8] = b"%PDF-1.7";
const T0: SystemTime = SystemTime::UNIX_EPOCH;

fn at(secs: u64) -> SystemTime {
    T0 + Duration::from_secs(secs)
}

fn source() -> BytesSource {
    BytesSource::new(DocumentFormat::Pdf, PDF.to_vec())
}

fn status(state: JobState) -> JobStatus {
    JobStatus::new(state)
}

fn busy() -> PrintError {
    PrintError::new(ErrorKind::Busy, "the printer is busy").with_safety(RetrySafety::Safe)
}

/// A send whose reply was lost: the request may have arrived.
fn lost() -> PrintError {
    PrintError::new(ErrorKind::Unavailable, "the connection closed with no reply").with_phase(Phase::Reply).with_safety(RetrySafety::OutcomeUnknown)
}

/// A local spooler-like backend that reports a job's status.
fn local(script: Vec<Result<JobStatus, PrintError>>) -> FakeBackend {
    FakeBackend::new("net").with_printer("p1", true).with_watch(WatchSupport::Poll).with_status_script(script)
}

/// A network IPP-like printer: tag in a native attribute, listing with creation time.
fn ipp(script: Vec<Result<JobStatus, PrintError>>) -> FakeBackend {
    FakeBackend::ipp_like("net").with_printer("p1", true).with_status_script(script).with_created_at(|| at(1_000))
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

fn job_for(printer: &str) -> Job {
    Job { printer: Some(printer.into()), title: "Quarterly report".into(), ..Job::default() }
}

fn print_job(rig: &Rig, printer: &str) -> PrintJob {
    PrintJob::new(job_for(printer), source())
        .with_clock(Arc::new(rig.clock.clone()))
        .with_wall_clock(|| at(1_000))
        .with_config(PrintJobConfig { staleness: Staleness::disabled(), ..PrintJobConfig::default() })
        .with_consent(UserConfirmed::interactive())
}

/// Advance until nothing is due, moving the injected clock by what the job asks for.
fn drive(job: &mut PrintJob, rig: &Rig) -> usize {
    for n in 1..=200 {
        match job.advance(&rig.registry).next_in {
            Some(wait) => rig.clock.advance(wait),
            None => return n,
        }
    }
    panic!("the job never settled: {:?}", job.snapshot().state);
}

fn states(job: &PrintJob) -> Vec<LifecycleState> {
    job.snapshot()
        .trail()
        .iter()
        .filter_map(|entry| match entry.event {
            TrailEvent::State(state) => Some(state),
            _ => None,
        })
        .collect()
}

// ---- the state machine ----

#[test]
fn a_print_walks_through_the_lifecycle_and_ends_completed() {
    let fake = local(vec![Ok(status(JobState::Pending)), Ok(status(JobState::Processing)), Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");

    assert_eq!(job.state(), LifecycleState::Preparing);
    let first = job.advance(&rig.registry);
    assert!(first.changed);
    assert_eq!((job.state(), first.next_in), (LifecycleState::Submitting, Some(Duration::ZERO)), "preparing is one cheap step with no I/O");
    assert!(fake.submissions().is_empty(), "nothing is sent before the submitting step");
    assert_eq!(job.snapshot().simple, SimpleState::Sending);

    drive(&mut job, &rig);
    assert_eq!(
        states(&job),
        [
            LifecycleState::Submitting,
            LifecycleState::Accepted,
            LifecycleState::Watching(JobState::Pending),
            LifecycleState::Watching(JobState::Processing),
            LifecycleState::Terminal(Outcome::Completed),
        ]
    );
    let snapshot = job.snapshot();
    assert_eq!((snapshot.simple, snapshot.settled, snapshot.advice), (SimpleState::Done, true, RetryAdvice::DoNotRetry));
    assert_eq!(fake.submissions().len(), 1);
    assert_eq!(snapshot.attempts().len(), 1);
    assert!(matches!(&snapshot.attempts()[0].outcome, AttemptOutcome::Accepted(Some(id)) if id.native() == "p1#1"));
    assert_eq!(snapshot.attempts()[0].remote_end, Some(JobState::Completed));
    assert_eq!(job.advance(&rig.registry).next_in, None, "a finished print does nothing more");
}

#[test]
fn a_backend_that_cannot_report_leaves_the_print_accepted_and_settled() {
    let fake = FakeBackend::cups_like("cups").with_printer("q", true);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "cups:q");
    drive(&mut job, &rig);
    let snapshot = job.snapshot();
    assert_eq!((snapshot.state, snapshot.simple, snapshot.settled), (LifecycleState::Accepted, SimpleState::Sent, true));
    assert_eq!(snapshot.advice, RetryAdvice::DoNotRetry, "it was accepted: resending would print twice");
    assert_eq!(fake.status_calls(), 0, "nobody polls a backend that cannot report");
}

#[test]
fn the_default_printer_is_pinned_at_the_first_step_so_retries_go_to_the_same_one() {
    let fake = local(vec![Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let mut job = PrintJob::new(Job::default(), source()).with_clock(Arc::new(rig.clock.clone()));
    job.advance(&rig.registry);
    assert_eq!(job.snapshot().printer.as_deref(), Some("net:p1"));
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));
}

#[test]
fn status_readings_are_deduplicated_and_the_trail_is_bounded() {
    let script =
        vec![Ok(status(JobState::Processing)), Ok(status(JobState::Processing)), Ok(status(JobState::Processing)), Ok(status(JobState::Completed))];
    let fake = local(script);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    let readings: Vec<_> = job
        .snapshot()
        .trail()
        .iter()
        .filter_map(|entry| match &entry.event {
            TrailEvent::Printer { state, .. } => Some(*state),
            _ => None,
        })
        .collect();
    assert_eq!(readings, [JobState::Processing, JobState::Completed], "three identical readings are one line");
    let times: Vec<_> = job.snapshot().trail().iter().map(|entry| entry.at).collect();
    assert!(times.windows(2).all(|pair| pair[0] <= pair[1]), "timestamps never go back");

    let rig2 = rig_with(&local(vec![Ok(status(JobState::Completed))]));
    let config = PrintJobConfig { trail_cap: 3, ..PrintJobConfig::default() };
    let mut bounded = print_job(&rig2, "net:p1").with_config(config);
    drive(&mut bounded, &rig2);
    let snapshot = bounded.snapshot();
    assert_eq!(snapshot.trail().len(), 3);
    assert!(snapshot.trail_dropped > 0);
}

fn rig_with(backend: &FakeBackend) -> Rig {
    rig(backend)
}

// ---- automatic retry ----

#[test]
fn safe_transient_errors_are_retried_up_to_three_attempts_then_the_print_fails() {
    let fake =
        local(vec![]).with_submit_script(vec![SubmitStep::Fail(busy()), SubmitStep::Fail(busy()), SubmitStep::Fail(busy()), SubmitStep::Accept]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);

    let snapshot = job.snapshot();
    assert_eq!(fake.submissions().len(), 3, "three attempts and no more");
    assert_eq!(snapshot.state, LifecycleState::Terminal(Outcome::Failed));
    assert_eq!(snapshot.attempts().len(), 3);
    assert!(
        snapshot
            .attempts()
            .iter()
            .all(|a| matches!(&a.outcome, AttemptOutcome::Error(e) if e.kind == ErrorKind::Busy && e.safety == RetrySafety::Safe))
    );
    let waits = states(&job).iter().filter(|s| matches!(s, LifecycleState::RetryScheduled { .. })).count();
    assert_eq!(waits, 2, "a wait between the attempts");
    assert_eq!(snapshot.advice, RetryAdvice::RetryNow, "the failure is known not to have printed");
    assert_eq!(snapshot.last_error.as_ref().map(|e| e.kind), Some(ErrorKind::Busy));
}

#[test]
fn a_retry_that_succeeds_ends_the_print_normally_and_keeps_the_failed_attempts() {
    let fake = local(vec![Ok(status(JobState::Completed))]).with_submit_script(vec![SubmitStep::Fail(busy()), SubmitStep::Fail(busy())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    let snapshot = job.snapshot();
    assert_eq!(snapshot.state, LifecycleState::Terminal(Outcome::Completed));
    assert_eq!(snapshot.attempts().iter().map(|a| a.number).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(matches!(snapshot.attempts()[2].outcome, AttemptOutcome::Accepted(Some(_))));
    assert_eq!(snapshot.last_error, None, "a later success clears the current error; the attempts keep theirs");
}

#[test]
fn the_wait_honours_the_devices_hint_and_runs_on_the_injected_clock() {
    let fake = local(vec![]).with_submit_script(vec![SubmitStep::Fail(busy().with_retry_after(Duration::from_secs(30)))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    job.advance(&rig.registry);
    job.advance(&rig.registry);
    let LifecycleState::RetryScheduled { attempt, next_at } = job.state() else { panic!("{:?}", job.state()) };
    assert_eq!(attempt, 2);
    assert!(next_at >= Duration::from_secs(30), "{next_at:?}");
    assert!(matches!(job.snapshot().advice, RetryAdvice::RetryAfter(wait) if wait >= Duration::from_secs(30)));
    assert_eq!(job.advance(&rig.registry).next_in, Some(next_at), "not due yet");
    assert_eq!(fake.submissions().len(), 1, "nothing is sent before the wait is over");
    rig.clock.advance(next_at);
    job.advance(&rig.registry);
    assert_eq!(job.state(), LifecycleState::Submitting);
}

#[test]
fn automatic_retry_can_be_turned_off_and_a_hard_refusal_is_never_retried() {
    let fake = local(vec![]).with_submit_script(vec![SubmitStep::Fail(busy())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1").with_retry(None);
    drive(&mut job, &rig);
    assert_eq!((job.state(), fake.submissions().len()), (LifecycleState::Terminal(Outcome::Failed), 1));

    let refusing = local(vec![]).failing_submit(PrintError::new(ErrorKind::Rejected, "the printer refused the job"));
    let rig = self::rig(&refusing);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    assert_eq!((job.state(), refusing.submissions().len()), (LifecycleState::Terminal(Outcome::Failed), 1));
    assert_eq!(job.snapshot().advice, RetryAdvice::DoNotRetry, "it will not get better by itself");

    let custom = RetryPolicy { max_attempts: 2, ..RetryPolicy::default() };
    let fake = local(vec![]).failing_submit(busy());
    let rig = self::rig(&fake);
    let mut job = print_job(&rig, "net:p1").with_retry(Some(custom));
    drive(&mut job, &rig);
    assert_eq!(fake.submissions().len(), 2, "the policy of the job decides");
}

#[test]
fn an_unknown_outcome_is_never_retried_automatically() {
    let fake = ipp(vec![]).with_submit_script(vec![SubmitStep::LoseReply(lost())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    assert!(drive(&mut job, &rig) <= 3);
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    for _ in 0..5 {
        rig.clock.advance(Duration::from_secs(3600));
        assert_eq!(job.advance(&rig.registry).next_in, None);
    }
    assert_eq!(fake.submissions().len(), 1, "one send, however long we wait");
    let snapshot = job.snapshot();
    assert_eq!((snapshot.simple, snapshot.settled, snapshot.advice), (SimpleState::NeedsAttention, true, RetryAdvice::CheckDeviceFirst));
    assert_eq!(snapshot.last_error.as_ref().map(|e| e.safety), Some(RetrySafety::OutcomeUnknown));
    assert_eq!(snapshot.attempts()[0].phase, Some(Phase::Reply));
}

#[test]
fn a_timeout_without_a_declared_safety_is_also_an_unknown_outcome() {
    let fake = local(vec![]).failing_submit(PrintError::new(ErrorKind::Timeout, "no reply"));
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    assert_eq!((job.state(), fake.submissions().len()), (LifecycleState::NeedsAttention, 1));
}

// ---- the printer's end states: manual retry only ----

#[test]
fn a_printer_side_abort_is_a_manual_retry_with_a_new_remote_job_and_the_history_kept() {
    let fake = local(vec![Ok(status(JobState::Aborted)), Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Aborted));
    assert_eq!(job.snapshot().advice, RetryAdvice::CheckDeviceFirst, "look at the printer before sending again");
    for _ in 0..3 {
        rig.clock.advance(Duration::from_secs(600));
        job.advance(&rig.registry);
    }
    assert_eq!(fake.submissions().len(), 1, "never retried on its own");

    job.retry().unwrap();
    assert_eq!(job.state(), LifecycleState::Submitting);
    drive(&mut job, &rig);
    let snapshot = job.snapshot();
    assert_eq!(snapshot.state, LifecycleState::Terminal(Outcome::Completed));
    assert_eq!(fake.submissions().len(), 2);
    let ids: Vec<_> = snapshot
        .attempts()
        .iter()
        .map(|a| match &a.outcome {
            AttemptOutcome::Accepted(Some(id)) => id.to_string(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(ids, ["net:p1#1", "net:p1#2"], "a new attempt gets a new remote job id");
    assert_eq!(snapshot.attempts()[0].remote_end, Some(JobState::Aborted), "and the first attempt remembers how it ended");
    assert_eq!(snapshot.remote.map(|j| j.to_string()).as_deref(), Some("net:p1#2"));
    assert_eq!(snapshot.attempt, 2);
}

#[test]
fn a_completed_print_cannot_be_retried_and_an_active_one_has_nothing_to_retry() {
    let fake = local(vec![Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    assert_eq!(job.retry().unwrap_err().kind(), ErrorKind::InvalidRequest);
    drive(&mut job, &rig);
    assert_eq!(job.retry().unwrap_err().kind(), ErrorKind::InvalidRequest, "it already printed");
}

#[test]
fn completed_with_errors_fails_the_print_and_a_forgotten_job_most_likely_printed() {
    let fake = local(vec![Ok(status(JobState::CompletedWithErrors))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    assert_eq!((job.state(), job.snapshot().advice), (LifecycleState::Terminal(Outcome::Failed), RetryAdvice::CheckDeviceFirst));

    let forgetting = local(vec![Ok(status(JobState::Processing)), Err(PrintError::new(ErrorKind::NotFound, "the printer forgot the job"))]);
    let rig = self::rig(&forgetting);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    let snapshot = job.snapshot();
    assert_eq!(snapshot.state, LifecycleState::Terminal(Outcome::CompletedWithWarnings));
    assert!(snapshot.warnings.iter().any(|w| w.contains("most likely finished")), "{:?}", snapshot.warnings);
}

#[test]
fn a_missing_printer_is_a_configuration_error_and_a_purged_job_is_still_forgotten() {
    let wrong_path = local(vec![Ok(status(JobState::Processing)), Err(PrintError::printer_not_found("the printer was not found at ipp://x/bogus"))]);
    let rig = rig(&wrong_path);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    let snapshot = job.snapshot();
    assert_eq!((snapshot.state, snapshot.advice), (LifecycleState::Terminal(Outcome::Failed), RetryAdvice::DoNotRetry));
    assert!(snapshot.warnings.iter().any(|w| w.contains("printer was not found")), "{:?}", snapshot.warnings);
    assert!(!snapshot.warnings.iter().any(|w| w.contains("most likely finished")), "{:?}", snapshot.warnings);

    // With stale-job probing on, the same error still ends the print: no probing on, no ceiling.
    let again = local(vec![Err(PrintError::printer_not_found("the printer was not found at ipp://x/bogus"))]);
    let rig = self::rig(&again);
    let mut job = PrintJob::new(job_for("net:p1"), source())
        .with_clock(Arc::new(rig.clock.clone()))
        .with_wall_clock(|| at(1_000))
        .with_consent(UserConfirmed::interactive());
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Failed));

    let purged = local(vec![Ok(status(JobState::Processing)), Err(PrintError::job_not_found("the printer purged the job"))]);
    let rig = self::rig(&purged);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::CompletedWithWarnings));
}

#[test]
fn a_printer_side_cancel_ends_the_print_cancelled_without_a_retry() {
    let fake = local(vec![Ok(status(JobState::Cancelled))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    assert_eq!((job.state(), job.snapshot().advice), (LifecycleState::Terminal(Outcome::Cancelled), RetryAdvice::DoNotRetry));
    assert_eq!(fake.submissions().len(), 1);
}

// ---- the tag ----

#[test]
fn the_tag_is_generated_once_and_appended_exactly_once_to_every_attempts_job_name() {
    let fake = FakeBackend::ipp_like("net")
        .with_printer("p1", true)
        .with_tag_support(TagSupport::Carried(TagPlacement::JobNameSuffix))
        .with_status_script(vec![Ok(status(JobState::Completed))])
        .with_submit_script(vec![SubmitStep::Fail(busy()), SubmitStep::Fail(busy())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    let tag = *job.tag();
    assert_eq!(tag.as_str().len(), 8);
    drive(&mut job, &rig);

    let submissions = fake.submissions();
    assert_eq!(submissions.len(), 3);
    for submission in &submissions {
        assert_eq!(submission.job.title, format!("Quarterly report [ref {tag}]"));
        assert_eq!(submission.job.title.matches("[ref ").count(), 1, "appended once, not once per attempt");
        assert_eq!(submission.tag.as_ref(), Some(&tag));
    }
    let snapshot = job.snapshot();
    assert_eq!((snapshot.tag, snapshot.tag_applied), (tag, true));
    assert_eq!(snapshot.title, "Quarterly report", "the app's title is kept as given");
    assert_eq!(fake.remote_jobs().last().map(|j| j.title.clone()), Some(format!("Quarterly report [ref {tag}]")));

    let other = PrintJob::new(job_for("net:p1"), source());
    assert_ne!(other.tag(), &tag, "each print has its own tag");
}

#[test]
fn a_native_placement_leaves_the_title_alone_and_a_fixed_tag_can_be_supplied() {
    let fake = ipp(vec![Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let fixed = Tag::parse("abc123de").unwrap();
    let mut job = print_job(&rig, "net:p1").with_tag(fixed);
    drive(&mut job, &rig);
    let submission = &fake.submissions()[0];
    assert_eq!(submission.job.title, "Quarterly report");
    assert_eq!(submission.tag, Some(fixed));
    assert_eq!(fake.remote_jobs()[0].tag, Some(fixed));
}

#[test]
fn the_tag_defaults_on_for_network_backends_and_off_for_local_ones() {
    let support = TagSupport::Carried(TagPlacement::JobNameSuffix);
    let network = FakeBackend::ipp_like("net").with_printer("p1", true).with_tag_support(support);
    let rig_net = rig(&network);
    drive(&mut print_job(&rig_net, "net:p1"), &rig_net);
    assert!(network.submissions()[0].tag.is_some(), "network: on");

    let spooler = FakeBackend::cups_like("lp").with_printer("q", true).with_tag_support(support);
    let rig_lp = rig(&spooler);
    drive(&mut print_job(&rig_lp, "lp:q"), &rig_lp);
    let sent = &spooler.submissions()[0];
    assert_eq!((sent.tag, sent.job.title.as_str()), (None, "Quarterly report"), "local: off, the title is untouched");

    let rig_lp2 = rig(&spooler);
    let mut forced = print_job(&rig_lp2, "lp:q").with_tag_policy(TagPolicy::Always);
    drive(&mut forced, &rig_lp2);
    assert!(spooler.submissions()[1].tag.is_some(), "the app can override: on for a local spooler");

    let never = FakeBackend::ipp_like("net").with_printer("p1", true).with_tag_support(support);
    let rig_never = rig(&never);
    let mut job = print_job(&rig_never, "net:p1").with_tag_policy(TagPolicy::Never);
    drive(&mut job, &rig_never);
    assert_eq!((never.submissions()[0].tag, job.snapshot().tag_applied), (None, false));
}

#[test]
fn a_backend_that_cannot_carry_a_tag_is_never_handed_one() {
    let fake = FakeBackend::new("net").with_class(craft_print::BackendClass::Network).with_printer("p1", true);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    assert_eq!(fake.submissions()[0].tag, None);
    assert!(!job.snapshot().tag_applied);
}

// ---- reconcile ----

#[test]
fn reconcile_finds_the_job_a_lost_reply_created_by_its_tag_and_the_print_adopts_it() {
    let fake = ipp(vec![Ok(status(JobState::Processing)), Ok(status(JobState::Completed))]).with_submit_script(vec![SubmitStep::LoseReply(lost())]);
    // A foreign job with the very same title and user, created before ours.
    fake.seed_job(RemoteJob::new(JobId::new("net", "p1#900"), "Quarterly report", JobState::Completed).with_user("fake-user").with_created(at(990)));
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::NeedsAttention);

    let found = job.reconcile(&rig.registry).unwrap();
    let Reconciled::Found(remote) = found else { panic!("{found:?}") };
    assert!(remote.ours, "matched by the tag");
    assert_eq!((remote.id.to_string(), remote.created), ("net:p1#1".to_owned(), Some(at(1_000))));
    assert_eq!(job.snapshot().advice, RetryAdvice::DoNotRetry, "it exists: adopt it, do not send a second copy");

    // Adopting something reconcile did not find is refused.
    let stranger = JobId::new("net", "p1#900");
    assert_eq!(job.accept(&rig.registry, &stranger).unwrap_err().kind(), ErrorKind::InvalidRequest);

    job.accept(&rig.registry, &remote.id).unwrap();
    drive(&mut job, &rig);
    let snapshot = job.snapshot();
    assert_eq!(snapshot.state, LifecycleState::Terminal(Outcome::Completed));
    assert_eq!(fake.submissions().len(), 1, "exactly one copy was ever sent");
    assert!(matches!(&snapshot.attempts()[0].outcome, AttemptOutcome::Accepted(Some(id)) if id == &remote.id));
    assert_eq!(snapshot.last_error, None);
}

#[test]
fn without_tag_support_reconcile_matches_by_title_user_and_time_and_can_be_ambiguous() {
    let lose = || vec![SubmitStep::LoseReply(lost())];
    // The tag policy is off, so only the title can match.
    let fake = ipp(vec![]).with_submit_script(lose());
    fake.seed_job(RemoteJob::new(JobId::new("net", "p1#800"), "Quarterly report", JobState::Completed).with_user("fake-user").with_created(at(100)));
    fake.seed_job(RemoteJob::new(JobId::new("net", "p1#801"), "Quarterly report", JobState::Completed).with_user("fake-user").with_created(at(950)));
    fake.seed_job(RemoteJob::new(JobId::new("net", "p1#802"), "Another title", JobState::Completed).with_user("fake-user").with_created(at(990)));
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1").with_tag_policy(TagPolicy::Never);
    drive(&mut job, &rig);
    let result = job.reconcile(&rig.registry).unwrap();
    let Reconciled::Ambiguous(jobs) = &result else { panic!("{result:?}") };
    let ids: Vec<_> = jobs.iter().map(|j| j.id.native().to_owned()).collect();
    assert_eq!(ids, ["p1#1", "p1#801"], "newest first; the job from long before the print is outside the window; another title never matches");
    assert!(jobs.iter().all(|j| !j.ours), "a title match is never claimed as ours");
    assert_eq!(job.snapshot().advice, RetryAdvice::CheckDeviceFirst);
}

#[test]
fn reconcile_reports_not_found_and_advises_a_retry_only_where_finished_jobs_are_listed() {
    // The request never arrived: the fake fails without creating a job.
    let fake = ipp(vec![]).with_submit_script(vec![SubmitStep::Fail(lost())]);
    let rig_a = rig(&fake);
    let mut job = print_job(&rig_a, "net:p1");
    drive(&mut job, &rig_a);
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    assert_eq!(job.reconcile(&rig_a.registry).unwrap(), Reconciled::NotFound);
    assert_eq!(job.snapshot().advice, RetryAdvice::RetryNow, "the printer lists finished jobs too, so nothing was printed");
    job.retry().unwrap();
    drive(&mut job, &rig_a);
    assert_eq!(fake.submissions().len(), 2, "the person chose to resend");

    // The same answer where finished jobs vanish proves less.
    let forgetful = FakeBackend::ipp_like("net")
        .with_printer("p1", true)
        .with_query_caps(JobQueryCaps { history: false, ..JobQueryCaps::ALL })
        .with_submit_script(vec![SubmitStep::Fail(lost())]);
    let rig_b = rig(&forgetful);
    let mut job = print_job(&rig_b, "net:p1");
    drive(&mut job, &rig_b);
    assert_eq!(job.reconcile(&rig_b.registry).unwrap(), Reconciled::NotFound);
    assert_eq!(job.snapshot().advice, RetryAdvice::CheckDeviceFirst, "it may have printed and been forgotten");
}

#[test]
fn a_backend_that_cannot_list_jobs_leaves_only_check_the_printer_first() {
    let fake = FakeBackend::cups_like("cups").with_printer("q", true).with_submit_script(vec![SubmitStep::LoseReply(lost())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "cups:q");
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    assert_eq!(job.reconcile(&rig.registry).unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(job.snapshot().advice, RetryAdvice::CheckDeviceFirst);
    assert_eq!(fake.submissions().len(), 1);
    assert_eq!(rig.registry.list_jobs("cups:q", &JobFilter::new()).unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(rig.registry.backend_info(Some("cups:q")).unwrap().query_caps, JobQueryCaps::NONE);
}

#[test]
fn reconcile_before_anything_was_sent_is_refused() {
    let fake = ipp(vec![]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    assert_eq!(job.reconcile(&rig.registry).unwrap_err().kind(), ErrorKind::InvalidRequest);
}

// ---- the common listing base ----

#[test]
fn listings_include_foreign_jobs_marked_not_ours_and_the_registry_stamps_the_backend() {
    let fake = ipp(vec![Ok(status(JobState::Processing))]);
    fake.seed_job(RemoteJob::new(JobId::new("whatever", "p1#50"), "Someone elses", JobState::Processing).with_user("other"));
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);

    let listed = rig.registry.list_jobs("net:p1", &JobFilter::new().marking(*job.tag())).unwrap();
    assert_eq!(listed.len(), 2);
    let ours: Vec<_> = listed.iter().map(|j| (j.id.to_string(), j.ours)).collect();
    assert_eq!(ours, [("net:p1#1".to_owned(), true), ("net:p1#50".to_owned(), false)], "newest first; the foreign job is listed and never ours");
    assert_eq!(listed[0].created, Some(at(1_000)));
    assert_eq!(listed[1].created, None, "not every source says when");

    let only_states = rig.registry.list_jobs("net:p1", &JobFilter::new().in_states(vec![JobState::Completed])).unwrap();
    assert!(only_states.is_empty());
    let by_user = rig.registry.list_jobs("net:p1", &JobFilter::new().for_user("other")).unwrap();
    assert_eq!(by_user.iter().map(|j| j.id.native()).collect::<Vec<_>>(), ["p1#50"], "a user filter keeps only that user's jobs");
    let found = rig.registry.find_job("net:p1", &Fingerprint::new("Quarterly report", Timestamp::default()).with_tag(*job.tag())).unwrap();
    assert!(matches!(&found, Reconciled::Found(j) if j.ours && j.id.backend() == "net"));
    let caps = rig.registry.backend_info(Some("net:p1")).unwrap();
    assert_eq!(caps.tag_support, TagSupport::Carried(TagPlacement::NativeAttribute));
}

// ---- cancel in every state ----

#[test]
fn cancel_before_the_document_is_sent_sends_nothing() {
    let fake = local(vec![]);
    let rig = rig(&fake);
    let mut preparing = print_job(&rig, "net:p1");
    preparing.cancel(&rig.registry).unwrap();
    assert_eq!(preparing.state(), LifecycleState::Terminal(Outcome::Cancelled));
    preparing.advance(&rig.registry);

    let mut submitting = print_job(&rig, "net:p1");
    submitting.advance(&rig.registry);
    assert_eq!(submitting.state(), LifecycleState::Submitting);
    submitting.cancel(&rig.registry).unwrap();
    assert_eq!(submitting.state(), LifecycleState::Terminal(Outcome::Cancelled));
    assert_eq!(submitting.advance(&rig.registry).next_in, None);

    let mut by_token = print_job(&rig, "net:p1");
    by_token.advance(&rig.registry);
    by_token.cancel_token().cancel();
    by_token.advance(&rig.registry);
    assert_eq!(by_token.state(), LifecycleState::Terminal(Outcome::Cancelled), "an app thread can cancel through the token");
    assert!(fake.submissions().is_empty());
}

#[test]
fn cancel_during_a_retry_wait_ends_the_print_and_nothing_is_resent() {
    let fake = local(vec![]).with_submit_script(vec![SubmitStep::Fail(busy())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    job.advance(&rig.registry);
    job.advance(&rig.registry);
    assert!(matches!(job.state(), LifecycleState::RetryScheduled { .. }));
    job.cancel(&rig.registry).unwrap();
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Cancelled));
    rig.clock.advance(Duration::from_secs(60));
    job.advance(&rig.registry);
    assert_eq!(fake.submissions().len(), 1);
}

#[test]
fn cancel_after_acceptance_asks_the_backend_and_follows_the_job_to_its_end() {
    let fake = local(vec![Ok(status(JobState::Processing)), Ok(status(JobState::Cancelled))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    while !matches!(job.state(), LifecycleState::Watching(JobState::Processing)) {
        let step = job.advance(&rig.registry);
        rig.clock.advance(step.next_in.unwrap_or_default());
    }
    job.cancel(&rig.registry).unwrap();
    assert_eq!(fake.cancels(), ["p1#1"]);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Cancelled));
    assert_eq!(job.snapshot().advice, RetryAdvice::DoNotRetry);
    job.cancel(&rig.registry).unwrap();
    assert_eq!(fake.cancels().len(), 1, "cancelling an ended print asks for nothing");
}

#[test]
fn cancel_when_the_backend_cannot_report_ends_cancelled_and_when_it_cannot_cancel_it_says_so() {
    let fake = FakeBackend::cups_like("cups").with_printer("q", true);
    let rig_a = rig(&fake);
    let mut job = print_job(&rig_a, "cups:q");
    drive(&mut job, &rig_a);
    assert_eq!(job.state(), LifecycleState::Accepted);
    job.cancel(&rig_a.registry).unwrap();
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Cancelled));
    assert!(job.snapshot().warnings.iter().any(|w| w.contains("cannot report")));

    let stubborn = FakeBackend::cups_like("cups").with_printer("q", true).with_cancel_support(false);
    let rig_b = rig(&stubborn);
    let mut job = print_job(&rig_b, "cups:q");
    drive(&mut job, &rig_b);
    assert_eq!(job.cancel(&rig_b.registry).unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(job.state(), LifecycleState::Accepted, "the state is unchanged when the backend refuses");

    let handleless = FakeBackend::cups_like("cups").with_printer("q", true).with_job_handle(|_, _| None);
    let rig_c = rig(&handleless);
    let mut job = print_job(&rig_c, "cups:q");
    drive(&mut job, &rig_c);
    assert_eq!(job.cancel(&rig_c.registry).unwrap_err().kind(), ErrorKind::Unsupported, "a spooler with no job handle has nothing to cancel by");
}

#[test]
fn cancel_while_the_outcome_is_unknown_cancels_only_jobs_that_are_provably_ours() {
    let tagged = ipp(vec![]).with_submit_script(vec![SubmitStep::LoseReply(lost())]);
    let rig_a = rig(&tagged);
    let mut job = print_job(&rig_a, "net:p1");
    drive(&mut job, &rig_a);
    job.cancel(&rig_a.registry).unwrap();
    assert_eq!(tagged.cancels(), ["p1#1"], "the job the lost reply created is cancelled");
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Cancelled));

    let by_title = ipp(vec![]).with_submit_script(vec![SubmitStep::LoseReply(lost())]);
    let rig_b = rig(&by_title);
    let mut job = print_job(&rig_b, "net:p1").with_tag_policy(TagPolicy::Never);
    drive(&mut job, &rig_b);
    assert!(job.cancel(&rig_b.registry).is_err(), "a title match could be someone else's job");
    assert_eq!((job.state(), by_title.cancels().len()), (LifecycleState::NeedsAttention, 0));

    let blind = FakeBackend::cups_like("cups").with_printer("q", true).with_submit_script(vec![SubmitStep::LoseReply(lost())]);
    let rig_c = rig(&blind);
    let mut job = print_job(&rig_c, "cups:q");
    drive(&mut job, &rig_c);
    job.cancel(&rig_c.registry).unwrap();
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Cancelled));
    assert!(job.snapshot().warnings.iter().any(|w| w.contains("may still be printing")));
}

// ---- totals, snapshots, documents ----

#[test]
fn expected_totals_still_flow_into_the_progress_and_a_reported_total_wins() {
    let mut reading = status(JobState::Processing);
    reading.sheets_done = Some(1);
    let fake = local(vec![Ok(reading.clone())]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1").with_options(JobOptions::new().with_expected_pages(4));
    for _ in 0..4 {
        let step = job.advance(&rig.registry);
        rig.clock.advance(step.next_in.unwrap_or_default());
    }
    let progress = job.snapshot().progress;
    assert_eq!((progress.sheets_done, progress.sheets_total, progress.pages_total), (Some(1), Some(4), Some(4)));
    assert_eq!(progress.fraction(), Some(0.25));

    reading.sheets_total = Some(10);
    let reporting = local(vec![Ok(reading)]);
    let rig = self::rig(&reporting);
    let mut job = print_job(&rig, "net:p1").with_options(JobOptions::new().with_expected_pages(4));
    for _ in 0..4 {
        let step = job.advance(&rig.registry);
        rig.clock.advance(step.next_in.unwrap_or_default());
    }
    assert_eq!(job.snapshot().progress.sheets_total, Some(10));
}

#[test]
fn a_snapshot_is_immutable_cheap_and_makes_no_requests() {
    let fake = local(vec![Ok(status(JobState::Processing)), Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    job.advance(&rig.registry);
    job.advance(&rig.registry);
    let before = job.snapshot();
    let calls = fake.status_calls();
    let again = job.snapshot();
    for _ in 0..1000 {
        let _ = job.snapshot();
    }
    assert_eq!((fake.status_calls(), fake.submissions().len()), (calls, 1), "snapshots never touch the backend");
    assert_eq!(before, again);
    assert!(std::ptr::eq(before.attempts().as_ptr(), again.attempts().as_ptr()), "the attempt list is shared, not copied");
    assert!(std::ptr::eq(before.trail().as_ptr(), again.trail().as_ptr()), "and so is the trail");

    drive(&mut job, &rig);
    assert_eq!(before.state, LifecycleState::Accepted, "an earlier snapshot does not change");
    assert_eq!(before.trail().len(), 2);
    assert_eq!(job.snapshot().state, LifecycleState::Terminal(Outcome::Completed));
}

#[test]
fn the_document_is_read_again_for_every_attempt_and_never_stored() {
    let reads = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&reads);
    let source = FnSource::new(DocumentFormat::Pdf, move || {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        Ok(format!("%PDF-1.7 revision {n}").into_bytes())
    });
    let fake = local(vec![Ok(status(JobState::Completed))]).with_submit_script(vec![SubmitStep::Fail(busy())]);
    let rig = rig(&fake);
    let mut job = PrintJob::new(job_for("net:p1"), source).with_clock(Arc::new(rig.clock.clone()));
    drive(&mut job, &rig);
    assert_eq!(reads.load(Ordering::SeqCst), 2, "once per attempt");
    let snapshot = job.snapshot();
    let document = snapshot.document.unwrap();
    assert_eq!((document.format, document.len), (DocumentFormat::Pdf, "%PDF-1.7 revision 1".len()));
    assert!(snapshot.warnings.iter().any(|w| w.contains("changed between attempts")), "{:?}", snapshot.warnings);
    assert_eq!(fake.submissions()[1].bytes, b"%PDF-1.7 revision 1", "the backend got the document the source gave for that attempt");
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("revision"), "the snapshot never carries the document");
}

#[test]
fn a_source_that_fails_is_an_attempt_that_failed_before_anything_was_sent() {
    let source = FnSource::new(DocumentFormat::Pdf, || Err(PrintError::new(ErrorKind::Io, "the file is gone")));
    let fake = local(vec![]);
    let rig = rig(&fake);
    let mut job = PrintJob::new(job_for("net:p1"), source).with_clock(Arc::new(rig.clock.clone()));
    drive(&mut job, &rig);
    let snapshot = job.snapshot();
    assert_eq!(snapshot.state, LifecycleState::Terminal(Outcome::Failed));
    assert!(fake.submissions().is_empty());
    let error = snapshot.last_error.unwrap();
    assert_eq!((error.kind, error.safety, error.phase), (ErrorKind::Io, RetrySafety::Safe, Some(Phase::Prepare)));
}

#[test]
fn a_network_print_without_consent_fails_at_once_and_a_bad_printer_id_does_too() {
    let fake = ipp(vec![]);
    let rig = rig(&fake);
    let mut job = PrintJob::new(job_for("net:p1"), source()).with_clock(Arc::new(rig.clock.clone()));
    drive(&mut job, &rig);
    assert_eq!(job.snapshot().last_error.map(|e| e.kind), Some(ErrorKind::ConsentRequired));
    assert_eq!((job.state(), fake.submissions().len()), (LifecycleState::Terminal(Outcome::Failed), 0));

    let mut unknown = PrintJob::new(job_for("nowhere:p1"), source()).with_clock(Arc::new(rig.clock.clone()));
    drive(&mut unknown, &rig);
    let snapshot = unknown.snapshot();
    assert_eq!((snapshot.state, snapshot.last_error.map(|e| e.kind)), (LifecycleState::Terminal(Outcome::Failed), Some(ErrorKind::UnknownBackend)));
    assert_eq!(snapshot.advice, RetryAdvice::DoNotRetry);
}

#[test]
fn the_snapshot_reports_debug_details_without_protocol_or_bytes() {
    let fake = local(vec![Ok(status(JobState::Completed))]);
    let rig = rig(&fake);
    let mut job = print_job(&rig, "net:p1");
    drive(&mut job, &rig);
    let native = job.snapshot().native.unwrap();
    assert_eq!(native.remote_native.as_deref(), Some("p1#1"));
}
