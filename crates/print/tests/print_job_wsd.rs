//! `PrintJob` over the WS-Print backend and its in-memory printer: the tag travels in the job name
//! of the ticket, a lost `SendDocument` reply is reconciled by the tag instead of being resent, and
//! `list_jobs` fills the common remote-job base from `GetActiveJobs` and `GetJobHistory`.
#![cfg(all(feature = "wsd", not(target_arch = "wasm32")))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use craft_print::backends::Backend;
use craft_print::wsd::WsdBackend;
use craft_print::wsd::adapter::{DeviceUrl, Request, Response, Transport, TransportKind, WireError};
use craft_print::wsd::testing::{FakeConfig, FakePrinter};
use craft_print::{
    BytesSource, Document, DocumentFormat, Fingerprint, Job, JobFilter, LifecycleState, ManualClock, Outcome, PrintJob, Reconciled, Registry,
    RetryAdvice, SubmitRequest, Tag, TagExt as _, TagPlacement, TagPolicy, Timestamp, UserConfirmed,
};

const URL: &str = "http://127.0.0.1:5357/print";
const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n";

/// Delivers every request to the fake printer and loses the reply to the named operation a number
/// of times: the printer did the work and the client saw the connection close.
struct LossyReply {
    printer: Arc<FakePrinter>,
    lose: &'static str,
    remaining: AtomicU32,
}

impl Transport for LossyReply {
    fn post(&self, _: &DeviceUrl, request: Request) -> Result<Response, WireError> {
        let (status, content_type, body) = self.printer.handle(&request.content_type, &request.body);
        if request.soap_action.ends_with(self.lose) && self.remaining.load(Ordering::SeqCst) > 0 {
            self.remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(WireError::transport(TransportKind::ConnectionClosed, "closed without a reply"));
        }
        Ok(Response::new(status, Some(content_type), body))
    }
}

struct Rig {
    printer: Arc<FakePrinter>,
    registry: Registry,
    clock: ManualClock,
}

/// A registry whose WS-Print backend talks to `printer` and loses the replies to the first
/// `lose_send_replies` `SendDocument` calls.
fn registry_over(printer: &Arc<FakePrinter>, lose_send_replies: u32) -> Registry {
    let transport = LossyReply { printer: Arc::clone(printer), lose: "SendDocument", remaining: AtomicU32::new(lose_send_replies) };
    let backend = WsdBackend::new().with_transport(transport).with_poll_interval(Duration::from_millis(1));
    backend.add(URL).unwrap();
    let registry = Registry::new();
    registry.register(Box::new(backend)).unwrap();
    registry
}

fn rig(lose_send_replies: u32) -> Rig {
    let printer = FakePrinter::new(FakeConfig::default());
    let registry = registry_over(&printer, lose_send_replies);
    Rig { printer, registry, clock: ManualClock::new() }
}

fn print_job(rig: &Rig, title: &str) -> PrintJob {
    let job = Job { printer: Some(format!("wsd:{URL}")), title: title.into(), ..Job::default() };
    PrintJob::new(job, BytesSource::new(DocumentFormat::Pdf, PDF.to_vec()))
        .with_clock(Arc::new(rig.clock.clone()))
        .with_consent(UserConfirmed::interactive())
}

fn drive(job: &mut PrintJob, rig: &Rig) {
    for _ in 0..200 {
        match job.advance(&rig.registry).next_in {
            Some(wait) => rig.clock.advance(wait),
            None => return,
        }
    }
    panic!("never settled: {:?}", job.state());
}

/// A job somebody else sent: same printer, same user name, no tag.
fn foreign(registry: &Registry, title: &str) {
    let job = Job { printer: Some(format!("wsd:{URL}")), title: title.into(), ..Job::default() };
    registry.submit_job(SubmitRequest::new(Document::pdf(PDF), job).with_consent(UserConfirmed::interactive())).unwrap();
}

#[test]
fn the_tag_is_a_suffix_of_the_ticket_job_name_and_the_snapshot_says_how_it_was_sent() {
    let rig = rig(0);
    let mut job = print_job(&rig, "Quarterly report");
    let tag = *job.tag();
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));
    let stored = rig.printer.job(1).unwrap();
    assert_eq!(stored.name, format!("Quarterly report [ref {tag}]"));
    assert_eq!(stored.user, "craft");
    let snapshot = job.snapshot();
    assert!(snapshot.tag_applied);
    let effective = snapshot.native.unwrap().effective.unwrap();
    assert!(effective.contains("ticket shape full"), "{effective}");
    assert_eq!(rig.registry.backend_info(Some(&format!("wsd:{URL}"))).unwrap().tag_support.placement(), TagPlacement::JobNameSuffix);
}

#[test]
fn a_long_title_is_cut_so_that_the_tag_survives_the_255_character_limit() {
    let rig = rig(0);
    let mut job = print_job(&rig, &"x".repeat(400));
    let tag = *job.tag();
    drive(&mut job, &rig);
    let name = rig.printer.job(1).unwrap().name;
    assert_eq!(name.chars().count(), 255);
    assert_eq!(Tag::find_in(&name), Some(tag));
}

#[test]
fn zero_copies_end_failed_with_do_not_retry_and_nothing_is_sent() {
    let rig = rig(0);
    let job = Job { printer: Some(format!("wsd:{URL}")), title: "Zero".into(), copies: 0, ..Job::default() };
    let mut job = PrintJob::new(job, BytesSource::new(DocumentFormat::Pdf, PDF.to_vec()))
        .with_clock(Arc::new(rig.clock.clone()))
        .with_consent(UserConfirmed::interactive());
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Failed), "{:?}", job.snapshot().last_error);
    assert_eq!(job.snapshot().advice, RetryAdvice::DoNotRetry);
    assert_eq!(rig.printer.jobs(), 0, "no job was created");
    let actions = rig.printer.actions();
    assert!(!actions.iter().any(|a| a == "CreatePrintJob" || a == "SendDocument"), "{actions:?}");
}

#[test]
fn a_lost_send_reply_is_reconciled_by_tag_and_nothing_is_sent_twice() {
    let printer = FakePrinter::new(FakeConfig::default());
    foreign(&registry_over(&printer, 0), "Quarterly report"); // an earlier job with the very same title
    let rig = Rig { registry: registry_over(&printer, 1), printer, clock: ManualClock::new() };

    let mut job = print_job(&rig, "Quarterly report");
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::NeedsAttention, "{:?}", job.snapshot().last_error);
    assert_eq!(rig.printer.jobs(), 2, "the printer made the job although the reply was lost");
    let sends = || rig.printer.actions().iter().filter(|a| *a == "SendDocument").count();
    assert_eq!(sends(), 2);

    let found = job.reconcile(&rig.registry).unwrap();
    let Reconciled::Found(remote) = found else { panic!("{found:?}") };
    assert!(remote.ours);
    assert_eq!(remote.id.to_string(), format!("wsd:{URL}#2"));
    assert_eq!(remote.user.as_deref(), Some("craft"));
    assert_eq!(remote.created, None, "WS-Print job summaries carry no creation time");
    assert_eq!(job.snapshot().advice, RetryAdvice::DoNotRetry);

    job.accept(&rig.registry, &remote.id).unwrap();
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));
    assert_eq!(sends(), 2, "the second copy was never sent");

    // The same lookup by title alone cannot tell the two apart.
    let result = rig.registry.find_job(&format!("wsd:{URL}"), &Fingerprint::new("Quarterly report", Timestamp::default())).unwrap();
    assert!(matches!(&result, Reconciled::Ambiguous(jobs) if jobs.len() == 2 && jobs.iter().all(|j| !j.ours)), "{result:?}");
}

#[test]
fn without_the_tag_a_lost_reply_reconciles_only_by_title_and_is_ambiguous_when_the_title_was_used_before() {
    let printer = FakePrinter::new(FakeConfig::default());
    foreign(&registry_over(&printer, 0), "Quarterly report");
    let rig = Rig { registry: registry_over(&printer, 1), printer, clock: ManualClock::new() };

    let mut job = print_job(&rig, "Quarterly report").with_tag_policy(TagPolicy::Never);
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::NeedsAttention);
    assert_eq!(rig.printer.job(2).unwrap().name, "Quarterly report", "no tag was sent");
    let found = job.reconcile(&rig.registry).unwrap();
    assert!(matches!(&found, Reconciled::Ambiguous(jobs) if jobs.len() == 2 && jobs.iter().all(|j| !j.ours)), "{found:?}");
    assert_eq!(job.snapshot().advice, RetryAdvice::CheckDeviceFirst);
}

#[test]
fn list_jobs_returns_active_and_finished_jobs_with_title_user_and_state() {
    let rig = rig(0);
    foreign(&rig.registry, "First");
    let mut mine = print_job(&rig, "Second");
    drive(&mut mine, &rig);
    let listed = rig.registry.list_jobs(&format!("wsd:{URL}"), &JobFilter::new().marking(*mine.tag())).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id.to_string(), format!("wsd:{URL}#2"), "newest first");
    assert!(listed[0].ours && !listed[1].ours);
    assert_eq!((listed[0].display_title(), listed[1].display_title()), ("Second", "First"));
    assert!(listed.iter().all(|j| j.user.as_deref() == Some("craft") && j.created.is_none()));
    let caps = rig.registry.backend_info(Some(&format!("wsd:{URL}"))).unwrap().query_caps;
    assert!(caps.list && caps.user && caps.tag && caps.history && !caps.created_time);

    let only_second = rig.registry.list_jobs(&format!("wsd:{URL}"), &JobFilter::new().titled("Second")).unwrap();
    assert_eq!(only_second.len(), 1);
    let backend = WsdBackend::new();
    assert_eq!(backend.job_query_caps(), caps);
}
