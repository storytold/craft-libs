//! `PrintJob` through a registry that has a converter: the document is converted for the printer,
//! the tag is applied once, a retry reads the source and converts again, and the view and snapshot
//! describe what was sent. Also the same on the WS-Print fake with Photo quality and a printer that
//! lists only raster. No sleeping: an injected clock.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use craft_print::testing::{FakeBackend, SubmitStep};
use craft_print::{
    Document, DocumentConverter, DocumentFormat, ErrorKind, FnSource, Job, JobOptions, JobState, JobStatus, LifecycleState, ManualClock, Outcome,
    OwnedDocument, PrintError, PrintJob, Registry, Resolution, RetrySafety, UserConfirmed,
};

const PDF: &[u8] = b"%PDF-1.7";

/// Makes PWG raster out of a PDF and counts its calls.
#[derive(Default)]
struct ToRaster {
    calls: AtomicU32,
}

impl DocumentConverter for ToRaster {
    fn targets(&self, from: DocumentFormat) -> Vec<DocumentFormat> {
        if from == DocumentFormat::Pdf { vec![DocumentFormat::PwgRaster] } else { Vec::new() }
    }
    fn convert(&self, _: Document<'_>, target: DocumentFormat, _: &craft_print::ConvertContext) -> Result<OwnedDocument, PrintError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(OwnedDocument::new(target, format!("raster#{n}").into_bytes()))
    }
}

fn raster_only() -> FakeBackend {
    FakeBackend::ipp_like("net")
        .with_printer("p1", true)
        .with_status_script(vec![Ok(JobStatus::new(JobState::Completed))])
        .with_formats(vec![DocumentFormat::PwgRaster])
        .with_formats_per_printer(true)
        .with_printer_formats(vec![DocumentFormat::PwgRaster])
        .with_resolutions(vec![Resolution::square(150), Resolution::square(600)])
}

fn rig(backend: &FakeBackend, converter: &Arc<ToRaster>) -> (Registry, ManualClock) {
    let registry = Registry::new();
    registry.register(Box::new(backend.clone())).unwrap();
    registry.set_converter(converter.clone());
    (registry, ManualClock::new())
}

fn drive(job: &mut PrintJob, registry: &Registry, clock: &ManualClock) {
    for _ in 0..200 {
        match job.advance(registry).next_in {
            Some(wait) => clock.advance(wait),
            None => return,
        }
    }
    panic!("never settled: {:?}", job.state());
}

fn counting_source(reads: &Arc<AtomicU32>) -> FnSource<impl Fn() -> Result<Vec<u8>, PrintError> + Send + use<>> {
    let counted = Arc::clone(reads);
    FnSource::new(DocumentFormat::Pdf, move || {
        counted.fetch_add(1, Ordering::SeqCst);
        Ok(PDF.to_vec())
    })
}

#[test]
fn a_tracked_print_is_converted_for_the_printer_and_the_tag_travels_once() {
    let backend = raster_only();
    let converter = Arc::new(ToRaster::default());
    let (registry, clock) = rig(&backend, &converter);
    let reads = Arc::new(AtomicU32::new(0));
    let mut job = PrintJob::new(Job { printer: Some("net:p1".into()), title: "Report".into(), ..Job::default() }, counting_source(&reads))
        .with_options(JobOptions::new().with_expected_pages(3))
        .with_clock(Arc::new(clock.clone()))
        .with_consent(UserConfirmed::interactive());
    let tag = *job.tag();
    drive(&mut job, &registry, &clock);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));

    let sent = backend.submissions();
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].format, sent[0].bytes.as_slice()), (DocumentFormat::PwgRaster, &b"raster#1"[..]));
    assert_eq!(sent[0].tag, Some(tag), "the tag reached the backend exactly once");
    assert_eq!(sent[0].options.expected_pages, Some(3), "the job's options are forwarded");
    assert_eq!(sent[0].options.resolution, Some(Resolution::square(150)), "the raster's resolution is in the options");

    let snapshot = job.snapshot();
    let conversion = snapshot.conversion.clone().expect("conversion recorded");
    assert_eq!((conversion.from, conversion.to, conversion.bytes), (DocumentFormat::Pdf, DocumentFormat::PwgRaster, 8));
    assert_eq!(job.request().conversion, snapshot.conversion);
    let info = snapshot.document.unwrap();
    assert_eq!((info.format, info.len), (DocumentFormat::Pdf, PDF.len()), "the snapshot keeps the source as it was read");
    assert_eq!(job.request().options.resolution, None, "the caller's options are not rewritten");
    assert!(snapshot.tag_applied);

    let view = job.view();
    assert_eq!(view.documents.len(), 1);
    let document = view.documents.first().unwrap();
    assert_eq!((document.format.as_deref(), document.bytes), (Some("pwg-raster"), Some(8)));
    assert_eq!(document.pages, Some(3));
    assert_eq!(view.state, JobState::Completed);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn a_retry_after_a_safe_failure_reads_the_source_and_converts_again() {
    let busy = PrintError::new(ErrorKind::Busy, "the printer is busy").with_safety(RetrySafety::Safe);
    let backend = raster_only().with_submit_script(vec![SubmitStep::Fail(busy), SubmitStep::Accept]);
    let converter = Arc::new(ToRaster::default());
    let (registry, clock) = rig(&backend, &converter);
    let reads = Arc::new(AtomicU32::new(0));
    let mut job = PrintJob::new(Job { printer: Some("net:p1".into()), title: "Report".into(), ..Job::default() }, counting_source(&reads))
        .with_clock(Arc::new(clock.clone()))
        .with_consent(UserConfirmed::interactive());
    let tag = *job.tag();

    // The failed attempt converted, but nothing was accepted, so no conversion is reported yet.
    let first = job.advance(&registry);
    assert!(first.next_in.is_some(), "a Safe failure is retried on its own");
    assert_eq!(job.snapshot().conversion, None);
    drive(&mut job, &registry, &clock);
    assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));
    assert_eq!(reads.load(Ordering::SeqCst), 2, "the source is read for every attempt");
    assert_eq!(converter.calls.load(Ordering::SeqCst), 2, "and converted again");
    assert_eq!(job.snapshot().attempt, 2);

    let sent = backend.submissions();
    let last = sent.last().unwrap();
    assert_eq!(last.bytes, b"raster#2");
    assert_eq!(last.tag, Some(tag));
    assert!(sent.iter().all(|s| s.tag == Some(tag)), "every request carried the same tag");
    let view = job.view();
    assert_eq!(view.attempts, 2);
    assert_eq!(view.documents.first().unwrap().format.as_deref(), Some("pwg-raster"));
}

#[cfg(all(feature = "wsd", not(target_arch = "wasm32")))]
mod wsd {
    use std::time::Duration;

    use craft_print::wsd::WsdBackend;
    use craft_print::wsd::testing::{FakeConfig, FakePrinter, MemTransport};
    use craft_print::{BytesSource, Quality};

    use super::*;

    const URL: &str = "http://127.0.0.1:5357/print";

    fn registry_over(printer: &Arc<FakePrinter>) -> Registry {
        let backend = WsdBackend::new().with_transport(MemTransport(Arc::clone(printer))).with_poll_interval(Duration::from_millis(1));
        backend.add(URL).unwrap();
        let registry = Registry::new();
        registry.register(Box::new(backend)).unwrap();
        registry
    }

    fn print(title: &str) -> PrintJob {
        PrintJob::new(
            Job { printer: Some(format!("wsd:{URL}")), title: title.into(), ..Job::default() },
            BytesSource::new(DocumentFormat::Pdf, PDF.to_vec()),
        )
        .with_consent(UserConfirmed::interactive())
    }

    #[test]
    fn photo_on_a_raster_only_printer_is_negotiated_converted_and_tracked() {
        let printer = FakePrinter::new(FakeConfig {
            formats: vec!["image/pwg-raster"],
            resolutions: vec![(150, 150), (600, 600)],
            qualities: vec!["Normal", "High", "Photo"],
            ..FakeConfig::default()
        });
        let registry = registry_over(&printer);
        let converter = Arc::new(ToRaster::default());
        registry.set_converter(converter.clone());
        let clock = ManualClock::new();
        let mut job = print("Holiday photo")
            .with_options(JobOptions::new().with_quality(Quality::Photo).with_resolution(Resolution::square(600)))
            .with_clock(Arc::new(clock.clone()));
        drive(&mut job, &registry, &clock);
        assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Completed));

        let stored = printer.job(1).unwrap();
        assert_eq!(stored.format.as_deref(), Some("image/pwg-raster"));
        assert_eq!(stored.document.as_deref(), Some(&b"raster#1"[..]));
        assert!(stored.ticket.contains(">Photo<") && stored.ticket.contains("600"), "{}", stored.ticket);
        assert_eq!(stored.name.matches("[ref ").count(), 1, "the tag is in the job name once: {}", stored.name);

        let snapshot = job.snapshot();
        let conversion = snapshot.conversion.unwrap();
        assert_eq!(
            (conversion.from, conversion.to, conversion.resolution),
            (DocumentFormat::Pdf, DocumentFormat::PwgRaster, Resolution::square(600))
        );
        assert_eq!(job.view().documents.first().unwrap().format.as_deref(), Some("pwg-raster"));
        assert_eq!(converter.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_printer_with_no_common_format_fails_the_print_before_anything_is_created() {
        let printer = FakePrinter::new(FakeConfig { formats: vec!["image/tiff"], ..FakeConfig::default() });
        let registry = registry_over(&printer);
        registry.set_converter(Arc::new(ToRaster::default()));
        let clock = ManualClock::new();
        let mut job = print("Nope").with_clock(Arc::new(clock.clone()));
        drive(&mut job, &registry, &clock);
        assert_eq!(job.state(), LifecycleState::Terminal(Outcome::Failed));
        assert_eq!(printer.jobs(), 0);
        assert_eq!(job.snapshot().last_error.unwrap().kind, ErrorKind::UnsupportedFormat);
        assert!(job.snapshot().conversion.is_none());
    }
}
