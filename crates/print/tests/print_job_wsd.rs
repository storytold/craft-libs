//! `PrintJob` over the WS-Print backend and its in-memory printer: the tag travels in the job name
//! of the ticket, a lost `SendDocument` reply is reconciled by the tag instead of being resent, and
//! `list_jobs` fills the common remote-job base from `GetActiveJobs` and `GetJobHistory`.
#![cfg(all(feature = "wsd", not(target_arch = "wasm32")))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use craft_print::backends::Backend;
use craft_print::wsd::adapter::{DeviceUrl, Request, Response, Transport, TransportKind, WireError};
use craft_print::wsd::testing::{FakeConfig, FakePrinter};
use craft_print::wsd::{Reconciled as WsdReconciled, WsdBackend};
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

/// As `LossyReply`, and a printer that stores a shorter job name than it was sent: the ` [ref ...]`
/// suffix is gone from the name it lists. Also loses the first `SendDocument` reply.
struct CutsTheName {
    inner: LossyReply,
}

impl Transport for CutsTheName {
    fn post(&self, url: &DeviceUrl, mut request: Request) -> Result<Response, WireError> {
        if request.soap_action.ends_with("CreatePrintJob") {
            let text = String::from_utf8(request.body.clone()).unwrap();
            let at = text.find(" [ref ").expect("the ticket carries the tag");
            let end = at + text[at..].find(']').unwrap() + 1;
            request.body = format!("{}{}", &text[..at], &text[end..]).into_bytes();
        }
        self.inner.post(url, request)
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
fn a_title_with_markup_is_listed_back_intact() {
    let printer = FakePrinter::new(FakeConfig::default());
    let registry = registry_over(&printer, 0);
    foreign(&registry, "R&D <report>");
    let listed = registry.list_jobs(&format!("wsd:{URL}"), &JobFilter::new()).unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].title, "R&D <report>");
}

#[test]
fn a_lost_send_reply_is_reconciled_and_nothing_is_sent_twice() {
    let printer = FakePrinter::new(FakeConfig::default());
    foreign(&registry_over(&printer, 0), "Quarterly report"); // an earlier job with the very same title
    let rig = Rig { registry: registry_over(&printer, 1), printer, clock: ManualClock::new() };

    let mut job = print_job(&rig, "Quarterly report");
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::NeedsAttention, "{:?}", job.snapshot().last_error);
    assert_eq!(rig.printer.jobs(), 2, "the printer made the job although the reply was lost");
    let sends = || rig.printer.actions().iter().filter(|a| *a == "SendDocument").count();
    assert_eq!(sends(), 2);

    // The tag survived in the printer's listing, so it decides: the earlier job with the same
    // title is not a candidate and the answer is not ambiguous.
    let found = job.reconcile(&rig.registry).unwrap();
    let Reconciled::Found(remote) = found else { panic!("{found:?}") };
    assert!(remote.ours, "the tag came back, so the job is ours");
    assert_eq!(remote.id.to_string(), format!("wsd:{URL}#2"));
    assert_eq!(remote.user.as_deref(), Some("craft"));
    assert_eq!(remote.created, None, "WS-Print job summaries carry no creation time");

    job.accept(&rig.registry, &remote.id).unwrap();
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));
    assert_eq!(sends(), 2, "the second copy was never sent");

    // The same lookup by title alone cannot tell the two apart.
    let result = rig.registry.find_job(&format!("wsd:{URL}"), &Fingerprint::new("Quarterly report", Timestamp::default())).unwrap();
    assert!(matches!(&result, Reconciled::Ambiguous(jobs) if jobs.len() == 2 && jobs.iter().all(|j| !j.ours)), "{result:?}");
}

#[test]
fn a_printer_that_cuts_the_tag_off_the_job_name_is_still_reconciled_by_title_and_user() {
    let printer = FakePrinter::new(FakeConfig::default());
    let lossy = LossyReply { printer: Arc::clone(&printer), lose: "SendDocument", remaining: AtomicU32::new(1) };
    let backend = WsdBackend::new().with_transport(CutsTheName { inner: lossy }).with_poll_interval(Duration::from_millis(1));
    backend.add(URL).unwrap();
    let registry = Registry::new();
    registry.register(Box::new(backend)).unwrap();
    let rig = Rig { printer, registry, clock: ManualClock::new() };

    let mut job = print_job(&rig, "Quarterly report");
    drive(&mut job, &rig);
    assert_eq!(job.state(), LifecycleState::NeedsAttention, "{:?}", job.snapshot().last_error);
    assert_eq!(rig.printer.job(1).unwrap().name, "Quarterly report", "the printer dropped the tag");
    assert_eq!(rig.registry.backend_info(Some(&format!("wsd:{URL}"))).unwrap().tag_support.placement(), TagPlacement::JobNameSuffix);

    // A tag-only match would find nothing here and a caller that resends on "not found" would
    // print the document twice.
    let found = job.reconcile(&rig.registry).unwrap();
    let Reconciled::Found(remote) = found else { panic!("{found:?}") };
    assert_eq!(remote.id.to_string(), format!("wsd:{URL}#1"));
    assert!(!remote.ours, "the tag did not come back, so nothing says it is ours");
}

/// A second backend over the same printer, to ask `reconcile_tagged` directly.
fn asker(printer: &Arc<FakePrinter>) -> WsdBackend {
    let transport = LossyReply { printer: Arc::clone(printer), lose: "SendDocument", remaining: AtomicU32::new(0) };
    let backend = WsdBackend::new().with_transport(transport);
    backend.add(URL).unwrap();
    backend
}

#[test]
fn reconcile_tagged_finds_a_job_whose_name_lost_the_tag_but_not_one_that_carries_another_tag() {
    let printer = FakePrinter::new(FakeConfig::default());
    let lossy = LossyReply { printer: Arc::clone(&printer), lose: "SendDocument", remaining: AtomicU32::new(0) };
    let backend = WsdBackend::new().with_transport(CutsTheName { inner: lossy }).with_poll_interval(Duration::from_millis(1));
    backend.add(URL).unwrap();
    let registry = Registry::new();
    registry.register(Box::new(backend)).unwrap();
    let rig = Rig { printer: Arc::clone(&printer), registry, clock: ManualClock::new() };
    let mut job = print_job(&rig, "Quarterly report");
    let tag = *job.tag();
    drive(&mut job, &rig);
    assert_eq!(printer.job(1).unwrap().name, "Quarterly report", "the printer dropped the tag");

    // The listed name has no tag at all, so the title and the user decide: the job is found, and
    // `NotFound` (which says that sending again is safe) is not the answer.
    let found = asker(&printer).reconcile_tagged(URL, "Quarterly report", Some(&tag)).unwrap();
    assert!(matches!(&found, WsdReconciled::Found(m) if m.name == "Quarterly report"), "{found:?}");

    // A listing that carries a different tag is somebody else's job with the same title.
    let printer = FakePrinter::new(FakeConfig::default());
    let other = Rig { printer: Arc::clone(&printer), registry: registry_over(&printer, 0), clock: ManualClock::new() };
    let mut theirs = print_job(&other, "Quarterly report");
    let their_tag = *theirs.tag();
    drive(&mut theirs, &other);
    assert!(printer.job(1).unwrap().name.contains(" [ref "), "the tag is on the listed name");
    let asked = asker(&printer);
    assert_eq!(asked.reconcile_tagged(URL, "Quarterly report", Some(&Tag::random())).unwrap(), WsdReconciled::NotFound);
    assert!(matches!(asked.reconcile_tagged(URL, "Quarterly report", Some(&their_tag)).unwrap(), WsdReconciled::Found(_)));

    // A job that still carries the tag is the answer even with an untagged job of the same title
    // listed beside it (newer or older): the title decides only when no listed name has the tag.
    foreign(&registry_over(&printer, 0), "Quarterly report");
    let found = asked.reconcile_tagged(URL, "Quarterly report", Some(&their_tag)).unwrap();
    assert!(matches!(&found, WsdReconciled::Found(m) if m.name.contains(" [ref ")), "{found:?}");
}

/// A printer whose job lists leave out the named fields of every summary (the ticket still has
/// them): `GetActiveJobs` and `GetJobHistory` answers are edited, everything else passes through.
/// Counts the `GetJobElements` lookups.
struct BlankSummaries {
    printer: Arc<FakePrinter>,
    fields: &'static [&'static str],
    lookups: Arc<AtomicU32>,
    /// The ticket does not name the user either.
    ticket_has_no_user: bool,
}

impl Transport for BlankSummaries {
    fn post(&self, _: &DeviceUrl, request: Request) -> Result<Response, WireError> {
        let (status, content_type, mut body) = self.printer.handle(&request.content_type, &request.body);
        if request.soap_action.ends_with("GetJobElements") {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            // This fake answers the status only; a real printer also returns the ticket.
            let asked = String::from_utf8(request.body.clone()).unwrap();
            let id: u32 = asked.split("<wprt:JobId>").nth(1).and_then(|r| r.split('<').next()).and_then(|n| n.parse().ok()).unwrap();
            let job = self.printer.job(id).unwrap();
            let user = if self.ticket_has_no_user { "" } else { job.user.as_str() };
            let ticket = format!(
                "<wprt:ElementData Name=\"wprt:PrintTicket\" Valid=\"true\"><wprt:PrintTicket><wprt:JobDescription><wprt:JobName>{}</wprt:JobName><wprt:JobOriginatingUserName>{}</wprt:JobOriginatingUserName></wprt:JobDescription></wprt:PrintTicket></wprt:ElementData>",
                job.name, user
            );
            body = String::from_utf8(body).unwrap().replace("</wprt:JobElements>", &format!("{ticket}</wprt:JobElements>")).into_bytes();
        }
        if request.soap_action.ends_with("GetActiveJobs") || request.soap_action.ends_with("GetJobHistory") {
            let mut text = String::from_utf8(body).unwrap();
            for field in self.fields {
                let (open, close) = (format!("<wprt:{field}>"), format!("</wprt:{field}>"));
                let mut from = 0;
                while let Some(at) = text[from..].find(&open).map(|at| at + from) {
                    let inner = at + open.len();
                    let end = inner + text[inner..].find(&close).unwrap();
                    text.replace_range(inner..end, "");
                    from = inner + close.len();
                }
            }
            body = text.into_bytes();
        }
        Ok(Response::new(status, Some(content_type), body))
    }
}

/// A printer holding one tagged job of the default user, and a backend that sees its job lists with
/// `fields` left out of every summary.
fn blanked(fields: &'static [&'static str]) -> (WsdBackend, Arc<AtomicU32>, Tag) {
    blanked_with(fields, false, true)
}

/// As `blanked`; `ticket_has_no_user` leaves the user out of the ticket too, and `tagged: false`
/// sends the job without a tag, so the listed name is the bare title.
fn blanked_with(fields: &'static [&'static str], ticket_has_no_user: bool, tagged: bool) -> (WsdBackend, Arc<AtomicU32>, Tag) {
    let printer = FakePrinter::new(FakeConfig::default());
    let rig = Rig { printer: Arc::clone(&printer), registry: registry_over(&printer, 0), clock: ManualClock::new() };
    let mut job = print_job(&rig, "Quarterly report");
    if !tagged {
        job = job.with_tag_policy(TagPolicy::Never);
    }
    let tag = *job.tag();
    drive(&mut job, &rig);
    assert_eq!(printer.job(1).unwrap().name.contains(" [ref "), tagged);
    let lookups = Arc::new(AtomicU32::new(0));
    let backend = WsdBackend::new().with_transport(BlankSummaries { printer, fields, lookups: Arc::clone(&lookups), ticket_has_no_user });
    backend.add(URL).unwrap();
    (backend, lookups, tag)
}

#[test]
fn reconcile_tagged_completes_a_summary_without_a_name_from_the_ticket() {
    let (backend, lookups, tag) = blanked(&["JobName"]);
    let found = backend.reconcile_tagged(URL, "Quarterly report", Some(&tag)).unwrap();
    assert!(matches!(&found, WsdReconciled::Found(m) if m.name.contains(" [ref ")), "{found:?}");
    assert!(lookups.load(Ordering::SeqCst) >= 1, "the name came from the ticket");
}

#[test]
fn reconcile_tagged_completes_a_summary_without_a_user_from_the_ticket() {
    // The name is there and the user is not: an empty user must not reject the job the tag matches.
    let (backend, lookups, tag) = blanked(&["JobOriginatingUserName"]);
    let found = backend.reconcile_tagged(URL, "Quarterly report", Some(&tag)).unwrap();
    assert!(matches!(&found, WsdReconciled::Found(m) if m.name.contains(" [ref ")), "{found:?}");
    assert_eq!(lookups.load(Ordering::SeqCst), 1, "one bounded lookup for the one summary");
}

#[test]
fn reconcile_tagged_completes_a_summary_without_a_name_or_a_user_with_one_lookup() {
    let (backend, lookups, tag) = blanked(&["JobName", "JobOriginatingUserName"]);
    let found = backend.reconcile_tagged(URL, "Quarterly report", Some(&tag)).unwrap();
    assert!(matches!(&found, WsdReconciled::Found(m) if m.name.contains(" [ref ")), "{found:?}");
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
}

#[test]
fn a_user_the_printer_never_reveals_does_not_reject_the_exact_name_but_does_reject_a_title_only_match() {
    let users = &["JobOriginatingUserName"];
    // The summary and the ticket both lack the user: the name as sent still finds the job.
    let (backend, lookups, tag) = blanked_with(users, true, true);
    let found = backend.reconcile_tagged(URL, "Quarterly report", Some(&tag)).unwrap();
    assert!(matches!(&found, WsdReconciled::Found(m) if m.name.contains(" [ref ")), "{found:?}");
    assert_eq!(lookups.load(Ordering::SeqCst), 1, "the ticket was asked and had no user");

    // The listed name is only the title (no tag): the title matches, and the user decides, but it
    // is unknown, so the job is not claimed.
    let (backend, lookups, tag) = blanked_with(users, true, false);
    let found = backend.reconcile_tagged(URL, "Quarterly report", Some(&tag)).unwrap();
    assert_eq!(found, WsdReconciled::NotFound);
    assert_eq!(lookups.load(Ordering::SeqCst), 1);

    // With the user in the ticket the same title-only listing is found.
    let (backend, _, tag) = blanked_with(users, false, false);
    let found = backend.reconcile_tagged(URL, "Quarterly report", Some(&tag)).unwrap();
    assert!(matches!(&found, WsdReconciled::Found(m) if m.name == "Quarterly report"), "{found:?}");
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
