//! Submit and watch as separate steps, driven through the public API with fake backends: one that
//! reports a job's status and can cancel it, one that cannot report anything (a fire-and-forget
//! spooler), and the retry vocabulary of [`PrintError`].
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use craft_print::testing::FakeBackend;
use craft_print::{
    CancelToken, Document, ErrorKind, Job, JobId, JobOptions, JobState, JobStatus, JobWatch, ManualClock, ParseIdErrorKind, Phase, PrintError,
    Progress, Registry, RetrySafety, SubmitRequest, UserConfirmed, WaitPolicy, WatchOutcome, WatchStep, WatchSupport, wait_for_job,
};

/// A printer that reports a job's status (the script's answers, the last one repeating), can cancel
/// it, and names every job `<printer>#7`.
fn reporting(script: Vec<Result<JobStatus, PrintError>>) -> FakeBackend {
    FakeBackend::new("net")
        .with_printer("p1", true)
        .with_watch(WatchSupport::Poll)
        .with_poll_interval(Duration::from_millis(1500))
        .with_job_handle(|printer, _| Some(format!("{}#7", printer.unwrap_or_default())))
        .with_status_script(script)
}

/// Takes the document and says nothing more, as a fire-and-forget spooler does.
fn silent() -> FakeBackend {
    FakeBackend::new("lp")
        .with_printer("q", false)
        .with_message("request id is q-1")
        .with_job_handle(|_, _| Some("q-1".into()))
        .with_cancel_support(false)
}

fn jid(text: &str) -> JobId {
    text.parse().unwrap()
}

fn status(state: JobState) -> JobStatus {
    JobStatus::new(state)
}

fn request(printer: &str) -> SubmitRequest<'static> {
    SubmitRequest::new(Document::pdf(b"%PDF-1.7"), Job { printer: Some(printer.into()), ..Job::default() })
}

fn registry(backend: &FakeBackend) -> Registry {
    let registry = Registry::new();
    registry.register(Box::new(backend.clone())).unwrap();
    registry.register(Box::new(silent())).unwrap();
    registry
}

#[test]
fn submit_returns_the_accepted_job_without_waiting_and_the_registry_fills_in_the_rest() {
    let printer = reporting(vec![Ok(status(JobState::Processing))]);
    let registry = registry(&printer);
    let started = Instant::now();
    let submitted = registry.submit_job(request("net:p1")).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5), "submit never waits for the job: the printer keeps it Processing forever");
    assert_eq!(printer.status_calls(), 0, "submit asks nothing after acceptance");
    assert_eq!(submitted.backend, "net", "the registry sets the backend name");
    assert_eq!(submitted.job.as_ref().map(|j| (j.to_string(), j.native().to_owned())), Some(("net:p1#7".to_string(), "p1#7".to_string())));
    assert_eq!(submitted.watch, WatchSupport::Poll);
    assert_eq!(submitted.poll_interval, Some(Duration::from_millis(1500)));
    assert_eq!(submitted.receipt().job_id.as_deref(), Some("p1#7"));
    // The older receipt entry point is the same call.
    assert_eq!(registry.submit(request("net:p1")).unwrap().message, "accepted");
}

#[test]
fn status_and_cancel_route_by_the_job_ids_backend() {
    let printer = reporting(vec![Ok(status(JobState::Pending)), Ok(status(JobState::Processing))]);
    let registry = registry(&printer);
    let job = jid("net:p1#7");
    assert_eq!(registry.job_status(&job).unwrap().state, JobState::Pending);
    assert_eq!(registry.job_status(&job).unwrap().state, JobState::Processing);
    registry.cancel_job(&job).unwrap();
    assert_eq!(printer.cancels(), ["p1#7"]);
    assert_eq!(registry.watch_support(&job).unwrap(), WatchSupport::Poll);
    assert_eq!(registry.recommended_poll_interval(&job).unwrap(), Some(Duration::from_millis(1500)));
    let stranger = jid("nope:p1#7");
    assert_eq!(registry.job_status(&stranger).unwrap_err().kind(), ErrorKind::UnknownBackend);
    assert_eq!(registry.cancel_job(&stranger).unwrap_err().kind(), ErrorKind::UnknownBackend);
    assert!(registry.job_status(&stranger).unwrap_err().message().contains("job nope:p1#7"), "the message names a job, not a printer");
    assert_eq!(Registry::new().job_status(&jid("net:x")).unwrap_err().kind(), ErrorKind::Unsupported);
}

#[test]
fn a_submitted_job_id_round_trips_through_text_and_is_accepted_back() {
    let printer = reporting(vec![Ok(status(JobState::Processing))]);
    let registry = registry(&printer);
    let submitted = registry.submit_job(request("net:p1")).unwrap();
    let job = submitted.job.unwrap();
    assert_eq!((job.backend(), job.native()), ("net", "p1#7"));
    let kept: JobId = job.to_string().parse().unwrap();
    assert_eq!(kept, job);
    assert_eq!(registry.job_status(&kept).unwrap().state, JobState::Processing);
    assert!(registry.job_watch(&kept, WaitPolicy::default()).is_ok());
    assert_eq!("p1#7".parse::<JobId>().unwrap_err().kind(), ParseIdErrorKind::MissingSeparator);
}

#[test]
fn a_backend_that_cannot_report_is_accepted_outcome_unknown_and_is_not_polled() {
    let printer = reporting(vec![Ok(status(JobState::Processing))]);
    let registry = registry(&printer);
    let submitted = registry.submit_job(request("lp:q")).unwrap();
    assert_eq!(submitted.watch, WatchSupport::None);
    assert_eq!(submitted.poll_interval, None);
    let status = registry.job_status(submitted.job.as_ref().unwrap()).unwrap();
    assert_eq!(status.state, JobState::Unknown);
    assert!(!status.terminal(), "unknown is 'accepted, outcome unknown', not an ending");
    let mut watch = JobWatch::for_submitted(&submitted, WaitPolicy::default()).unwrap();
    assert_eq!(watch.tick(&registry), WatchStep::Done(WatchOutcome::NoStatus));
    assert_eq!(registry.cancel_job(&jid("lp:q-1")).unwrap_err().kind(), ErrorKind::Unsupported, "a backend with no cancel says so");
}

#[test]
fn a_watch_with_an_injected_clock_follows_the_job_to_a_terminal_state() {
    let printer = reporting(vec![
        Ok(status(JobState::Pending)),
        Ok(JobStatus { sheets_done: Some(1), sheets_total: Some(2), ..status(JobState::Processing) }),
        Ok(status(JobState::Completed)),
    ]);
    let registry = registry(&printer);
    let submitted = registry.submit_job(request("net:p1")).unwrap();
    let clock = ManualClock::new();
    let mut watch =
        JobWatch::for_submitted(&submitted, WaitPolicy { growth_percent: 100, ..WaitPolicy::default() }).unwrap().with_clock(Arc::new(clock.clone()));
    let mut seen = Vec::new();
    let outcome = watch.run(&registry, &mut |step| {
        if let WatchStep::Update(s) = step {
            seen.push((s.state, s.sheet_fraction()));
        }
    });
    assert!(matches!(outcome, WatchOutcome::Terminal(ref s) if s.state == JobState::Completed), "{outcome:?}");
    assert_eq!(seen, [(JobState::Pending, None), (JobState::Processing, Some(0.5))]);
    assert_eq!(clock.sleeps().iter().sum::<Duration>(), Duration::from_millis(3000), "two gaps of the backend's recommended 1.5 s");
    assert_eq!(printer.status_calls(), 3);
}

#[test]
fn the_blocking_convenience_is_built_on_the_polls_and_gives_up_at_max_wait() {
    let printer = reporting(vec![Ok(status(JobState::Processing))]);
    let registry = registry(&printer);
    let submitted = registry.submit_job(request("net:p1")).unwrap();
    let job = submitted.job.unwrap();
    let started = Instant::now();
    let policy = WaitPolicy::every(Duration::from_millis(20)).with_max_wait(Duration::from_millis(200));
    let outcome = wait_for_job(&registry, &job, policy).unwrap();
    assert!(matches!(outcome, WatchOutcome::TimedOut { last: Some(ref s) } if s.state == JobState::Processing), "{outcome:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(printer.status_calls() >= 2);
    assert_eq!(wait_for_job(&registry, &JobId::new("nope", "1"), WaitPolicy::default()).unwrap_err().kind(), ErrorKind::UnknownBackend);
    // Finishing is also seen by the convenience.
    let done = reporting(vec![Ok(status(JobState::Processing)), Ok(JobStatus { warnings: vec!["warning".into()], ..status(JobState::Completed) })]);
    let registry = self::registry(&done);
    let outcome = wait_for_job(&registry, &JobId::new("net", "p1#7"), WaitPolicy::every(Duration::from_millis(10))).unwrap();
    assert!(outcome.is_success());
}

#[test]
fn a_watch_can_cancel_through_the_registry() {
    let printer = reporting(vec![Ok(status(JobState::Processing)), Ok(status(JobState::Cancelled))]);
    let registry = registry(&printer);
    let cancel = CancelToken::new();
    cancel.cancel();
    let mut watch = registry
        .job_watch(&JobId::new("net", "p1#7"), WaitPolicy::every(Duration::from_millis(5)))
        .unwrap()
        .with_cancel(cancel)
        .with_clock(Arc::new(ManualClock::new()));
    let outcome = watch.run(&registry, &mut |_| {});
    assert_eq!(printer.cancels(), ["p1#7"]);
    assert_eq!(outcome.error().map(|e| e.kind()), Some(ErrorKind::Cancelled));
}

#[test]
fn error_kinds_carry_a_default_retry_safety_a_backend_can_override() {
    let table = [
        (ErrorKind::Unsupported, false, RetrySafety::Safe),
        (ErrorKind::UnknownBackend, false, RetrySafety::Safe),
        (ErrorKind::NotFound, false, RetrySafety::Safe),
        (ErrorKind::InvalidRequest, false, RetrySafety::Safe),
        (ErrorKind::UnsupportedFormat, false, RetrySafety::Safe),
        (ErrorKind::ConsentRequired, false, RetrySafety::Safe),
        (ErrorKind::Cancelled, false, RetrySafety::Safe),
        (ErrorKind::Unavailable, true, RetrySafety::Safe),
        (ErrorKind::Busy, true, RetrySafety::Safe),
        (ErrorKind::Timeout, true, RetrySafety::OutcomeUnknown),
        (ErrorKind::Rejected, false, RetrySafety::Safe),
        (ErrorKind::Io, false, RetrySafety::Safe),
        (ErrorKind::Failed, false, RetrySafety::OutcomeUnknown),
    ];
    for (kind, retryable, safety) in table {
        let e = PrintError::new(kind, "x");
        assert_eq!((e.is_retryable(), e.retry_safety(), e.phase(), e.retry_after()), (retryable, safety, None, None), "{kind:?}");
        assert_eq!(e.outcome_unknown(), safety == RetrySafety::OutcomeUnknown);
    }
    let e = PrintError::new(ErrorKind::Timeout, "x")
        .with_safety(RetrySafety::Idempotent)
        .with_phase(Phase::Poll)
        .with_retry_after(Duration::from_secs(3));
    assert_eq!((e.retry_safety(), e.phase(), e.retry_after()), (RetrySafety::Idempotent, Some(Phase::Poll), Some(Duration::from_secs(3))));
    assert!(e.is_retryable() && !e.outcome_unknown());
    assert!(UserConfirmed::attest(Some(true)).is_ok());
}

#[test]
fn expected_pages_become_the_total_only_where_the_backend_reports_none() {
    // The backend reports sheets done and never a total (a network printer's way).
    let printer = reporting(vec![
        Ok(JobStatus { sheets_done: Some(1), ..status(JobState::Processing) }),
        Ok(JobStatus { sheets_done: Some(3), ..status(JobState::Processing) }),
        Ok(JobStatus { sheets_done: Some(4), sheets_total: Some(4), ..status(JobState::Completed) }),
    ]);
    let registry = registry(&printer);
    let options = JobOptions::new().with_expected_pages(6);
    let duplex = Job { printer: Some("net:p1".into()), duplex: Some(craft_print::Duplex::LongEdge), ..Job::default() };
    let submitted = registry.submit_job(SubmitRequest::new(Document::pdf(b"%PDF-1.7"), duplex).with_options(options)).unwrap();
    assert_eq!((submitted.expected.pages, submitted.expected.sheets), (Some(6), Some(3)));
    let mut watch =
        JobWatch::for_submitted(&submitted, WaitPolicy::every(Duration::from_millis(10))).unwrap().with_clock(Arc::new(ManualClock::new()));
    let mut fractions = Vec::new();
    let outcome = watch.run(&registry, &mut |step| {
        if let WatchStep::Update(s) = step {
            fractions.push((s.sheets_done, s.sheets_total, s.pages_total, s.sheet_fraction()));
        }
    });
    assert_eq!(fractions, [(Some(1), Some(3), Some(6), Some(1.0 / 3.0)), (Some(3), Some(3), Some(6), Some(1.0))]);
    assert!(matches!(outcome, WatchOutcome::Terminal(ref s) if s.sheets_total == Some(4)), "the reported total wins over the estimate: {outcome:?}");
}

#[test]
fn progress_events_get_the_expected_sheets_as_their_total() {
    let run = |events: Vec<Progress>, pages: Option<u32>| {
        let registry = Registry::new();
        registry.register(Box::new(FakeBackend::new("rep").with_printer("p", true).with_progress_events(events))).unwrap();
        let mut seen = Vec::new();
        let mut callback = |event: Progress| seen.push(event);
        let mut options = JobOptions::new();
        options.expected_pages = pages;
        let request = SubmitRequest::new(Document::pdf(b"%PDF"), Job::default()).with_options(options).with_progress(&mut callback);
        registry.submit_job(request).unwrap();
        seen
    };
    let unknown = vec![Progress::done_only(1), Progress::done_only(2)];
    assert_eq!(run(unknown.clone(), Some(5)), [Progress::of(1, 5), Progress::of(2, 5)]);
    assert_eq!(run(unknown, None), [Progress::done_only(1), Progress::done_only(2)], "no page count, no invented total");
    assert_eq!(run(vec![Progress::of(1, 9)], Some(5)), [Progress::of(1, 9)], "a backend-reported total wins");
}
