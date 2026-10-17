//! Format negotiation and conversion through the registry, driven with the exported
//! `craft_print::testing` fakes (the way an app's own tests would).

use std::sync::{Arc, Mutex};

use DocumentFormat::{Jpeg, Pdf, PwgRaster};
use craft_print::testing::FakeBackend;
use craft_print::{
    ConvertContext, Document, DocumentConverter, DocumentFormat, ErrorKind, Job, JobOptions, OwnedDocument, Phase, PrintError, PrinterDetails,
    Registry, Resolution, RetrySafety, SubmitRequest,
};

/// Makes `target` out of a PDF; remembers what it was asked to do; can be told to fail.
#[derive(Default)]
struct Rasteriser {
    targets: Vec<DocumentFormat>,
    fail: Option<PrintError>,
    seen: Mutex<Vec<(DocumentFormat, Resolution, bool)>>,
}

impl Rasteriser {
    fn making(targets: &[DocumentFormat]) -> Arc<Self> {
        Arc::new(Self { targets: targets.to_vec(), ..Self::default() })
    }
}

impl DocumentConverter for Rasteriser {
    fn targets(&self, from: DocumentFormat) -> Vec<DocumentFormat> {
        if from == Pdf { self.targets.clone() } else { Vec::new() }
    }

    fn convert(&self, document: Document<'_>, target: DocumentFormat, context: &ConvertContext) -> Result<OwnedDocument, PrintError> {
        self.seen.lock().unwrap().push((target, context.resolution, context.colour));
        if let Some(error) = &self.fail {
            return Err(error.clone());
        }
        // A real converter renders `document.bytes`; this one wraps them so the test can see them.
        let mut bytes = format!("{}:", target.mime()).into_bytes();
        bytes.extend_from_slice(document.bytes);
        Ok(OwnedDocument::new(target, bytes))
    }
}

fn setup(fake: &FakeBackend, converter: Option<Arc<Rasteriser>>) -> Registry {
    let registry = Registry::new();
    registry.register(Box::new(fake.clone())).unwrap();
    if let Some(converter) = converter {
        registry.set_converter(converter);
    }
    registry
}

fn request<'a>() -> SubmitRequest<'a> {
    SubmitRequest::new(Document::pdf(b"%PDF-1.7"), Job::default())
}

#[test]
fn a_document_the_printer_accepts_is_sent_as_it_is() {
    let fake = FakeBackend::new("fake").with_printer("p", true).with_formats(vec![Pdf, PwgRaster]);
    let converter = Rasteriser::making(&[PwgRaster]);
    let submitted = setup(&fake, Some(converter.clone())).submit_job(request()).unwrap();
    assert_eq!(submitted.conversion, None);
    assert!(converter.seen.lock().unwrap().is_empty());
    let sent = &fake.submissions()[0];
    assert_eq!((sent.format, sent.bytes.as_slice()), (Pdf, &b"%PDF-1.7"[..]));
    assert_eq!(fake.detail_calls(), 0);
}

#[test]
fn a_document_the_printer_does_not_accept_is_converted_and_the_choice_is_reported() {
    let fake = FakeBackend::new("fake")
        .with_printer("p", true)
        .with_formats(vec![PwgRaster, Jpeg])
        .with_resolutions(vec![Resolution::square(150), Resolution::square(600)]);
    let converter = Rasteriser::making(&[Jpeg, PwgRaster]);
    let submitted = setup(&fake, Some(converter.clone())).submit_job(request()).unwrap();

    assert_eq!(*converter.seen.lock().unwrap(), [(PwgRaster, Resolution::square(150), true)]);
    let sent = &fake.submissions()[0];
    assert_eq!((sent.format, sent.bytes.as_slice()), (PwgRaster, &b"image/pwg-raster:%PDF-1.7"[..]));
    assert_eq!(sent.options.resolution, Some(Resolution::square(150)), "the raster's resolution travels in the job");
    let conversion = submitted.conversion.unwrap();
    assert_eq!((conversion.from, conversion.to, conversion.resolution), (Pdf, PwgRaster, Resolution::square(150)));
}

#[test]
fn the_callers_own_resolution_and_the_printers_list_decide_the_dpi() {
    let fake = FakeBackend::new("fake").with_printer("p", true).with_formats(vec![PwgRaster]).with_resolutions(vec![
        Resolution::square(300),
        Resolution::square(600),
        Resolution::square(1200),
    ]);
    let converter = Rasteriser::making(&[PwgRaster]);
    let registry = setup(&fake, Some(converter.clone()));
    registry.submit_job(request()).unwrap();
    registry.submit_job(request().with_options(JobOptions::new().with_resolution(Resolution::square(1200)))).unwrap();
    let seen = converter.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().map(|(_, r, _)| *r).collect::<Vec<_>>(), [Resolution::square(300), Resolution::square(1200)]);
}

#[test]
fn per_printer_formats_are_read_once_and_a_known_list_is_used_without_asking() {
    let printer_formats = vec![PwgRaster];
    let fake = FakeBackend::new("fake")
        .with_printer("p", true)
        .with_formats(vec![Pdf, PwgRaster])
        .with_formats_per_printer(true)
        .with_printer_formats(printer_formats.clone());
    let converter = Rasteriser::making(&[PwgRaster]);
    let registry = setup(&fake, Some(converter.clone()));
    assert_eq!(registry.submit_job(request()).unwrap().conversion.unwrap().to, PwgRaster);
    assert_eq!(fake.detail_calls(), 1);

    let details = PrinterDetails { formats: printer_formats, ..PrinterDetails::default() };
    registry.submit_job_with_details(request(), Some(details)).unwrap();
    assert_eq!(fake.detail_calls(), 1, "the caller's details replace the request");
    assert_eq!(fake.submissions().len(), 2);
}

#[test]
fn nothing_in_common_is_a_typed_unsupported_format_that_names_both_sides() {
    let fake = FakeBackend::new("fake").with_printer("p", true).with_formats(vec![PwgRaster]);
    let error = setup(&fake, Some(Rasteriser::making(&[Jpeg]))).submit_job(request()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFormat);
    assert!(error.message().contains("image/pwg-raster") && error.message().contains("image/jpeg"), "{}", error.message());
    let error = setup(&fake, None).submit_job(request()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFormat);
    assert!(fake.submissions().is_empty());
}

#[test]
fn a_converter_failure_is_a_prepare_error_that_is_safe_to_repeat() {
    let fake = FakeBackend::new("fake").with_printer("p", true).with_formats(vec![PwgRaster]);
    let converter = Arc::new(Rasteriser {
        targets: vec![PwgRaster],
        fail: Some(PrintError::new(ErrorKind::Io, "cannot read the page")),
        ..Rasteriser::default()
    });
    let error = setup(&fake, Some(converter)).submit_job(request()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!((error.phase(), error.retry_safety()), (Some(Phase::Prepare), RetrySafety::Safe));
    assert!(fake.submissions().is_empty());
}
