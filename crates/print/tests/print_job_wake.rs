//! A push channel through the registry and `PrintJob`, on the trait-level fake backend: a wake
//! brings the next status read forward, a healthy channel stretches the idle gap to the safety
//! poll, an unhealthy one changes nothing, and only a read ever ends the print.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::Duration;

use craft_print::testing::FakeBackend;
use craft_print::{
    BytesSource, DocumentFormat, Job, JobId, JobState, JobStatus, LifecycleState, ManualClock, Outcome, PrintJob, Registry, UserConfirmed,
    WaitPolicy, WatchSupport,
};

const PDF: &[u8] = b"%PDF-1.7";

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

struct Rig {
    fake: FakeBackend,
    registry: Registry,
    clock: ManualClock,
}

fn rig(script: Vec<Result<JobStatus, craft_print::backends::PrintError>>) -> Rig {
    let fake = FakeBackend::ipp_like("net").with_push().with_printer("p1", true).with_status_script(script);
    let registry = Registry::new();
    registry.register(Box::new(fake.clone())).unwrap();
    Rig { fake, registry, clock: ManualClock::new() }
}

fn status(state: JobState) -> Result<JobStatus, craft_print::backends::PrintError> {
    Ok(JobStatus::new(state))
}

fn print_job(rig: &Rig) -> PrintJob {
    PrintJob::new(Job { printer: Some("net:p1".into()), title: "t".into(), ..Job::default() }, BytesSource::new(DocumentFormat::Pdf, PDF.to_vec()))
        .with_clock(Arc::new(rig.clock.clone()))
        .with_consent(UserConfirmed::interactive())
}

/// Advance until the print is being watched and has read once.
fn watching(job: &mut PrintJob, rig: &Rig) {
    for _ in 0..10 {
        job.advance(&rig.registry);
        if matches!(job.state(), LifecycleState::Watching(_)) {
            return;
        }
    }
    panic!("{:?}", job.state());
}

#[test]
fn the_registry_hands_a_watch_the_backends_wake_channel() {
    let rig = rig(vec![status(JobState::Processing)]);
    let job = JobId::new("net", "p1#1");
    assert!(rig.registry.wake_source(&job).is_some());
    let watch = rig.registry.job_watch(&job, WaitPolicy::default()).unwrap();
    assert!(watch.is_pushed());
    assert!(rig.registry.wake_source(&JobId::new("nobody", "x")).is_none());
    let plain = FakeBackend::new("plain");
    let registry = Registry::new();
    registry.register(Box::new(plain)).unwrap();
    assert!(registry.wake_source(&JobId::new("plain", "x")).is_none());
    assert_eq!(rig.registry.watch_support(&job).unwrap(), WatchSupport::Poll);
}

#[test]
fn a_healthy_channel_slows_the_polling_and_a_wake_brings_a_read_forward() {
    let rig = rig(vec![status(JobState::Processing)]);
    let mut job = print_job(&rig);
    watching(&mut job, &rig);
    let reads = rig.fake.status_calls();
    assert!(reads >= 1);
    let next = job.advance(&rig.registry).next_in.unwrap();
    assert!(next >= secs(25), "the safety poll is about 30 seconds, not the 2 second poll: {next:?}");
    assert_eq!(rig.fake.push_watchers(), 1);

    assert_eq!(rig.fake.wake_watches(), 1);
    rig.clock.advance(secs(1));
    job.advance(&rig.registry);
    assert_eq!(rig.fake.status_calls(), reads + 1, "the wake caused one read");
    // The read said Processing: a wake alone never ends or changes the print.
    assert_eq!(job.state(), LifecycleState::Watching(JobState::Processing));
}

#[test]
fn an_unhealthy_channel_polls_as_usual() {
    let rig = rig(vec![status(JobState::Processing)]);
    rig.fake.set_push_healthy(false);
    let mut job = print_job(&rig);
    watching(&mut job, &rig);
    let next = job.advance(&rig.registry).next_in.unwrap();
    assert!(next <= secs(15), "{next:?}");
}

#[test]
fn losing_trust_midway_returns_to_the_ordinary_cadence_and_the_print_still_completes() {
    let rig = rig(vec![status(JobState::Processing)]);
    let mut job = print_job(&rig);
    watching(&mut job, &rig);
    assert!(job.advance(&rig.registry).next_in.unwrap() >= secs(25));
    rig.fake.set_push_healthy(false);
    assert!(job.advance(&rig.registry).next_in.unwrap() <= secs(15));
    rig.fake.set_status_script(vec![status(JobState::Completed)]);
    for _ in 0..50 {
        match job.advance(&rig.registry).next_in {
            Some(wait) => rig.clock.advance(wait),
            None => break,
        }
    }
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));
    // The finished watch released its receiver.
    assert_eq!(rig.fake.push_watchers(), 0);
}

#[test]
fn a_lost_wake_costs_only_latency_the_safety_poll_finds_the_end() {
    let rig = rig(vec![status(JobState::Processing)]);
    let mut job = print_job(&rig);
    watching(&mut job, &rig);
    rig.fake.set_status_script(vec![status(JobState::Completed)]); // finished, and nobody told us
    for _ in 0..50 {
        match job.advance(&rig.registry).next_in {
            Some(wait) => rig.clock.advance(wait),
            None => break,
        }
    }
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));
}
