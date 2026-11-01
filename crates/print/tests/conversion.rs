//! Format negotiation and conversion through the registry, with a test-only backend that can say
//! which formats each printer takes and a scripted converter.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use craft_print::backends::Backend;
use craft_print::{
    BackendClass, CancelToken, ColourMode, ConversionPolicy, ConvertContext, Document, DocumentConverter, DocumentFormat, ErrorKind, Job, JobOptions,
    OwnedDocument, Phase, PrintError, PrinterDetails, PrinterRef, Registry, Resolution, RetrySafety, SubmitRequest, Submitted, UserConfirmed,
    choose_resolution,
};

#[derive(Clone, Debug, PartialEq)]
struct Seen {
    format: DocumentFormat,
    bytes: Vec<u8>,
    resolution: Option<Resolution>,
}

/// One printer, `dev`. `backend_formats` is `accepted_formats`; with `per_printer` the printer's own
/// list is `details.formats`.
#[derive(Clone)]
struct Device {
    backend_formats: &'static [DocumentFormat],
    per_printer: bool,
    network: bool,
    details: PrinterDetails,
    seen: Arc<Mutex<Vec<Seen>>>,
    detail_calls: Arc<AtomicU32>,
}

impl Device {
    /// A local device whose backend takes `backend_formats`, with default printer details.
    fn new(backend_formats: &'static [DocumentFormat]) -> Self {
        Self {
            backend_formats,
            per_printer: false,
            network: false,
            details: PrinterDetails::default(),
            seen: Arc::default(),
            detail_calls: Arc::default(),
        }
    }

    /// Report `details` (resolutions, colour, the per-printer format list) for the printer.
    fn with_details(mut self, details: PrinterDetails) -> Self {
        self.details = details;
        self
    }

    /// Make the printer's own `details.formats`, not the backend list, decide what it takes.
    fn per_printer(mut self) -> Self {
        self.per_printer = true;
        self
    }

    /// Report the backend as a network backend.
    fn network(mut self) -> Self {
        self.network = true;
        self
    }

    /// What each submit received, in order.
    fn submitted(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// How many times the printer's details were asked for.
    fn detail_calls(&self) -> u32 {
        self.detail_calls.load(Ordering::SeqCst)
    }
}

impl Backend for Device {
    fn name(&self) -> &'static str {
        "dev"
    }
    fn class(&self) -> BackendClass {
        if self.network { BackendClass::Network } else { BackendClass::Local }
    }
    fn accepted_formats(&self) -> &[DocumentFormat] {
        self.backend_formats
    }
    fn formats_per_printer(&self) -> bool {
        self.per_printer
    }
    /// The one default printer, `p`.
    fn list_printers(&self) -> Result<Vec<PrinterRef>, PrintError> {
        Ok(vec![PrinterRef { id: "p".into(), name: "P".into(), default: true, two_sided: None }])
    }
    /// The configured details under the asked-for id; each call is counted.
    fn printer_details(&self, native_id: &str) -> Result<PrinterDetails, PrintError> {
        self.detail_calls.fetch_add(1, Ordering::SeqCst);
        Ok(PrinterDetails { id: native_id.into(), name: native_id.into(), ..self.details.clone() })
    }
    /// Record the format, bytes and resolution that arrived, and accept.
    fn submit(&self, request: SubmitRequest<'_>) -> Result<Submitted, PrintError> {
        self.seen.lock().unwrap().push(Seen {
            format: request.document.format,
            bytes: request.document.bytes.to_vec(),
            resolution: request.options.resolution,
        });
        Ok(Submitted::new("dev", "accepted"))
    }
}

/// Turns anything into the `targets` it is told to make, bytes `converted:<target>`.
struct Converter {
    targets: Vec<DocumentFormat>,
    fail: Option<PrintError>,
    wrong_format: bool,
    cancel_while_converting: bool,
    calls: Mutex<Vec<(DocumentFormat, DocumentFormat, ConvertContext)>>,
}

impl Converter {
    /// A converter that makes `targets` from PDF and never fails.
    fn making(targets: &[DocumentFormat]) -> Arc<Self> {
        Arc::new(Self { targets: targets.to_vec(), fail: None, wrong_format: false, cancel_while_converting: false, calls: Mutex::default() })
    }

    /// Every conversion asked for: source format, target and context.
    fn calls(&self) -> Vec<(DocumentFormat, DocumentFormat, ConvertContext)> {
        self.calls.lock().unwrap().clone()
    }
}

impl DocumentConverter for Converter {
    /// The configured targets for PDF; nothing for any other format.
    fn targets(&self, from: DocumentFormat) -> Vec<DocumentFormat> {
        if from == DocumentFormat::Pdf { self.targets.clone() } else { Vec::new() }
    }
    /// Record the call, then cancel the context's token, fail, or return the wrong format if so
    /// configured; otherwise `converted:<mime>` in the target format.
    fn convert(&self, document: Document<'_>, target: DocumentFormat, context: &ConvertContext) -> Result<OwnedDocument, PrintError> {
        self.calls.lock().unwrap().push((document.format, target, context.clone()));
        if self.cancel_while_converting {
            context.cancel.cancel();
        }
        if let Some(error) = &self.fail {
            return Err(error.clone());
        }
        let format = if self.wrong_format { DocumentFormat::Png } else { target };
        Ok(OwnedDocument::new(format, format!("converted:{}", target.mime()).into_bytes()))
    }
}

/// A registry holding a clone of `device` and, if given, `converter`.
fn registry_with(device: &Device, converter: Option<Arc<Converter>>) -> Registry {
    let registry = Registry::new();
    registry.register(Box::new(device.clone())).unwrap();
    if let Some(converter) = converter {
        registry.set_converter(converter);
    }
    registry
}

/// A PDF submit to `dev:p` with no consent attached.
fn pdf_request<'a>() -> SubmitRequest<'a> {
    SubmitRequest::new(Document::pdf(b"%PDF-1.7"), Job { printer: Some("dev:p".into()), ..Job::default() })
}

use DocumentFormat::{Jpeg, Pdf, PwgRaster};

/// Colour printer details advertising these square resolutions (dpi).
fn res(details_resolutions: &[u32]) -> PrinterDetails {
    PrinterDetails {
        resolutions: details_resolutions.iter().map(|d| Resolution::square(*d)).collect(),
        colour: Some(true),
        ..PrinterDetails::default()
    }
}

#[test]
fn a_format_the_backend_accepts_goes_as_it_is_and_the_converter_is_not_asked() {
    let device = Device::new(&[Pdf, PwgRaster]);
    let converter = Converter::making(&[PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));
    let submitted = registry.submit_job(pdf_request()).unwrap();
    assert_eq!(submitted.conversion, None);
    assert!(converter.calls().is_empty());
    assert_eq!(device.submitted(), [Seen { format: Pdf, bytes: b"%PDF-1.7".to_vec(), resolution: None }]);
    assert_eq!(device.detail_calls(), 0, "an exact backend list costs no extra request");
}

#[test]
fn another_format_is_converted_at_the_nearest_listed_resolution() {
    let device = Device::new(&[PwgRaster]).with_details(res(&[100, 250, 600]));
    let converter = Converter::making(&[Jpeg, PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));
    let submitted = registry.submit_job(pdf_request()).unwrap();

    let calls = converter.calls();
    assert_eq!(calls.len(), 1);
    let (from, target, context) = &calls[0];
    assert_eq!((*from, *target), (Pdf, PwgRaster));
    assert_eq!(context.resolution, Resolution::square(250));
    assert!(context.colour);
    assert_eq!(context.printer.as_deref(), Some("dev:p"));
    // The printer received the converted bytes, and the raster's resolution travels in the job.
    assert_eq!(
        device.submitted(),
        [Seen { format: PwgRaster, bytes: b"converted:image/pwg-raster".to_vec(), resolution: Some(Resolution::square(250)) }]
    );
    let conversion = submitted.conversion.unwrap();
    assert_eq!((conversion.from, conversion.to, conversion.resolution, conversion.bytes), (Pdf, PwgRaster, Resolution::square(250), 26));
}

#[test]
fn the_callers_resolution_beats_the_printers_list_and_a_missing_list_means_the_target() {
    let device = Device::new(&[PwgRaster]).with_details(res(&[100, 250]));
    let converter = Converter::making(&[PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));
    registry.submit_job(pdf_request().with_options(JobOptions::new().with_resolution(Resolution::new(600, 300)))).unwrap();
    assert_eq!(converter.calls()[0].2.resolution, Resolution::new(600, 300));
    assert_eq!(device.submitted()[0].resolution, Some(Resolution::new(600, 300)));

    let device = Device::new(&[PwgRaster]);
    let converter = Converter::making(&[PwgRaster]);
    registry_with(&device, Some(converter.clone())).submit_job(pdf_request()).unwrap();
    assert_eq!(converter.calls()[0].2.resolution, Resolution::square(300), "nothing published: the policy's 300 dpi");
}

#[test]
fn the_policy_sets_the_preferred_format_and_the_target_dpi() {
    let device = Device::new(&[Jpeg, PwgRaster]).with_details(res(&[300, 600]));
    let converter = Converter::making(&[Jpeg, PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));
    registry.submit_job(pdf_request()).unwrap();
    assert_eq!(converter.calls()[0].1, PwgRaster, "the default preference puts raster before JPEG");

    registry.set_conversion_policy(ConversionPolicy { preference: vec![Jpeg, PwgRaster], target_dpi: 600 });
    registry.submit_job(pdf_request()).unwrap();
    let calls = converter.calls();
    assert_eq!((calls[1].1, calls[1].2.resolution), (Jpeg, Resolution::square(600)));
    assert_eq!(device.submitted()[1].resolution, None, "only a raster target changes the job's resolution");
}

#[test]
fn colour_is_off_for_a_mono_request_or_a_mono_printer() {
    let device = Device::new(&[PwgRaster]).with_details(res(&[300]));
    let converter = Converter::making(&[PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));
    registry.submit_job(pdf_request().with_options(JobOptions::new().with_colour(ColourMode::Mono))).unwrap();
    assert!(!converter.calls()[0].2.colour);

    let mono_printer = Device::new(&[PwgRaster]).with_details(PrinterDetails { colour: Some(false), ..PrinterDetails::default() });
    let converter = Converter::making(&[PwgRaster]);
    registry_with(&mono_printer, Some(converter.clone())).submit_job(pdf_request()).unwrap();
    assert!(!converter.calls()[0].2.colour);
}

#[test]
fn a_backend_with_per_printer_formats_is_asked_once_and_not_when_details_are_passed() {
    let details = PrinterDetails { formats: vec![PwgRaster], resolutions: vec![Resolution::square(600)], ..PrinterDetails::default() };
    let device = Device::new(&[Pdf, PwgRaster, Jpeg]).per_printer().with_details(details.clone());
    let converter = Converter::making(&[PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));

    // The backend names PDF, this printer does not list it: converted, after one details request.
    let submitted = registry.submit_job(pdf_request()).unwrap();
    assert_eq!(device.detail_calls(), 1);
    assert_eq!(submitted.conversion.unwrap().to, PwgRaster);
    assert_eq!(converter.calls()[0].2.resolution, Resolution::square(600));

    // The caller has the details already: no request at all.
    registry.submit_job_with_details(pdf_request(), Some(details)).unwrap();
    assert_eq!(device.detail_calls(), 1);
    assert_eq!(device.submitted().len(), 2);

    // A printer that lists PDF takes it as it is (still one request to know).
    let pdf_printer = Device::new(&[Pdf, PwgRaster]).per_printer().with_details(PrinterDetails { formats: vec![Pdf], ..PrinterDetails::default() });
    let converter = Converter::making(&[PwgRaster]);
    let registry = registry_with(&pdf_printer, Some(converter.clone()));
    assert_eq!(registry.submit_job(pdf_request()).unwrap().conversion, None);
    assert_eq!((pdf_printer.detail_calls(), converter.calls().len()), (1, 0));

    // Without a converter there is nothing to decide, so nothing is asked.
    let plain = Device::new(&[Pdf, PwgRaster]).per_printer();
    registry_with(&plain, None).submit_job(pdf_request()).unwrap();
    assert_eq!(plain.detail_calls(), 0);

    // A printer that publishes no list is treated as taking what the backend names.
    let unlisted = Device::new(&[Pdf, PwgRaster]).per_printer();
    assert_eq!(registry_with(&unlisted, Some(Converter::making(&[PwgRaster]))).submit_job(pdf_request()).unwrap().conversion, None);
}

#[test]
fn no_common_format_is_a_typed_error_that_lists_both_sides() {
    let device = Device::new(&[PwgRaster]);
    let converter = Converter::making(&[Jpeg]);
    let error = registry_with(&device, Some(converter.clone())).submit_job(pdf_request()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFormat);
    assert_eq!(error.retry_safety(), RetrySafety::Safe);
    for needle in ["application/pdf", "image/pwg-raster", "image/jpeg", "dev"] {
        assert!(error.message().contains(needle), "{needle} missing from {:?}", error.message());
    }
    assert!(device.submitted().is_empty() && converter.calls().is_empty());

    // The converter cannot start from this format at all.
    let error = registry_with(&device, Some(Converter::making(&[PwgRaster])))
        .submit_job(SubmitRequest::new(Document::new(DocumentFormat::Tiff, b"II*\0"), Job { printer: Some("dev:p".into()), ..Job::default() }))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFormat);
    assert!(error.message().contains("nothing from this format") && error.message().contains("image/tiff"), "{}", error.message());

    // No converter registered: the same kind, with the accepted list.
    let error = registry_with(&device, None).submit_job(pdf_request()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFormat);
    assert!(error.message().contains("it accepts: image/pwg-raster"), "{}", error.message());
}

#[test]
fn a_converter_failure_is_reported_as_before_anything_was_sent() {
    let device = Device::new(&[PwgRaster]);
    let mut failing = Converter::making(&[PwgRaster]);
    Arc::get_mut(&mut failing).unwrap().fail = Some(PrintError::new(ErrorKind::Failed, "the renderer gave up"));
    let error = registry_with(&device, Some(failing.clone())).submit_job(pdf_request()).unwrap_err();
    assert_eq!((error.kind(), error.message()), (ErrorKind::Failed, "the renderer gave up"));
    assert_eq!((error.phase(), error.retry_safety()), (Some(Phase::Prepare), RetrySafety::Safe));
    assert!(!error.outcome_unknown() && device.submitted().is_empty());

    let mut wrong = Converter::making(&[PwgRaster]);
    Arc::get_mut(&mut wrong).unwrap().wrong_format = true;
    let error = registry_with(&device, Some(wrong)).submit_job(pdf_request()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Failed);
    assert!(error.message().contains("image/png") && error.message().contains("image/pwg-raster"), "{}", error.message());
    assert!(device.submitted().is_empty());
}

#[test]
fn a_job_cancelled_during_the_conversion_is_not_sent() {
    let device = Device::new(&[PwgRaster]);
    let mut converter = Converter::making(&[PwgRaster]);
    Arc::get_mut(&mut converter).unwrap().cancel_while_converting = true;
    let token = CancelToken::new();
    let error = registry_with(&device, Some(converter)).submit_job(pdf_request().with_cancel(token.clone())).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled);
    assert!(token.is_cancelled() && device.submitted().is_empty());
}

#[test]
fn a_network_backend_is_not_contacted_or_converted_for_without_consent() {
    let details = PrinterDetails { formats: vec![PwgRaster], ..PrinterDetails::default() };
    let device = Device::new(&[Pdf, PwgRaster]).network().per_printer().with_details(details);
    let converter = Converter::making(&[PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));
    let error = registry.submit_job(pdf_request()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ConsentRequired);
    assert_eq!((device.detail_calls(), converter.calls().len(), device.submitted().len()), (0, 0, 0));

    registry.submit_job(pdf_request().with_consent(UserConfirmed::interactive())).unwrap();
    assert_eq!((device.detail_calls(), converter.calls().len(), device.submitted().len()), (1, 1, 1));
}

#[test]
fn the_registry_can_drop_its_converter() {
    let device = Device::new(&[PwgRaster]);
    let registry = registry_with(&device, Some(Converter::making(&[PwgRaster])));
    assert!(registry.has_converter());
    registry.clear_converter();
    assert!(!registry.has_converter());
    assert_eq!(registry.submit_job(pdf_request()).unwrap_err().kind(), ErrorKind::UnsupportedFormat);
}

#[test]
fn the_resolution_rule_is_public() {
    let list = [Resolution::square(150), Resolution::square(600)];
    assert_eq!(choose_resolution(&list, 300), Resolution::square(150));
    assert_eq!(choose_resolution(&list, 500), Resolution::square(600));
}

/// A converter the way an app writes one: render pixels, pack them with the public PWG encoder.
struct BlankPages;

impl DocumentConverter for BlankPages {
    /// PWG raster from PDF only.
    fn targets(&self, from: DocumentFormat) -> Vec<DocumentFormat> {
        if from == DocumentFormat::Pdf { vec![DocumentFormat::PwgRaster] } else { Vec::new() }
    }
    /// One blank page a tenth of an inch square at the context's resolution, in colour or grey as asked,
    /// packed with the public PWG encoder.
    fn convert(&self, _document: Document<'_>, target: DocumentFormat, context: &ConvertContext) -> Result<OwnedDocument, PrintError> {
        let (w, h) = (context.resolution.x_dpi / 10, context.resolution.y_dpi / 10);
        let page = craft_print::pwg::RasterPage {
            width: w,
            height: h,
            gray: !context.colour,
            pixels: vec![255; (w * h) as usize * if context.colour { 3 } else { 1 }],
        };
        let options =
            craft_print::pwg::PwgOptions { dpi: (context.resolution.x_dpi, context.resolution.y_dpi), duplex: craft_print::pwg::Duplex::Off };
        let bytes = craft_print::pwg::encode(&[page], &options).map_err(|e| PrintError::new(ErrorKind::Failed, e.to_string()))?;
        Ok(OwnedDocument::new(target, bytes))
    }
}

#[test]
fn an_app_converter_can_pack_pages_with_the_public_pwg_encoder() {
    let device = Device::new(&[PwgRaster]).with_details(res(&[200]));
    let registry = Registry::new();
    registry.register(Box::new(device.clone())).unwrap();
    registry.set_converter(Arc::new(BlankPages));
    let submitted = registry.submit_job(pdf_request()).unwrap();
    let sent = &device.submitted()[0];
    assert_eq!(sent.format, PwgRaster);
    assert_eq!(&sent.bytes[..4], b"RaS2");
    assert_eq!(submitted.conversion.unwrap().bytes, sent.bytes.len());
    assert_eq!(sent.resolution, Some(Resolution::square(200)));
}

#[test]
fn the_default_printers_details_are_used_when_the_request_names_no_printer() {
    let device = Device::new(&[PwgRaster]).with_details(res(&[100, 250, 600]));
    let converter = Converter::making(&[PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));
    registry.submit_job(SubmitRequest::new(Document::pdf(b"%PDF-1.7"), Job::default())).unwrap();
    assert_eq!(converter.calls()[0].2.resolution, Resolution::square(250));
}

#[test]
fn per_printer_formats_are_read_for_the_default_printer_too() {
    let details = PrinterDetails { formats: vec![PwgRaster], ..PrinterDetails::default() };
    let device = Device::new(&[Pdf, PwgRaster]).per_printer().with_details(details);
    let converter = Converter::making(&[PwgRaster]);
    let registry = registry_with(&device, Some(converter.clone()));
    let submitted = registry.submit_job(SubmitRequest::new(Document::pdf(b"%PDF-1.7"), Job::default())).unwrap();
    assert_eq!(submitted.conversion.unwrap().to, PwgRaster);
    assert_eq!(device.detail_calls(), 1);
}
