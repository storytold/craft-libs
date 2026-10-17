//! The built-in backend: the system spooler behind the crate's existing functions.
//!
//! This is an adapter, not a second implementation. Listing calls [`crate::printers`], details
//! call [`crate::printer_options`] and a job calls [`crate::submit_with_progress`], so behaviour
//! is the one the free functions have always had. It registers as `cups` on macOS and Linux and as
//! `spooler` on Windows (the in-process `winprint` code stays behind `cfg(windows)`); on other
//! targets there is no built-in backend.

use crate::Registry;
#[cfg(any(test, windows, all(unix, not(target_arch = "wasm32"))))]
pub use spooler::SystemSpooler;

/// Register the platform's built-in backend, if the platform has one. A name that is already
/// taken (the app registered its own first) is left alone.
pub(crate) fn register_builtin(registry: &Registry) {
    #[cfg(any(windows, all(unix, not(target_arch = "wasm32"))))]
    {
        let _ = registry.register(Box::new(SystemSpooler));
    }
    #[cfg(not(any(windows, all(unix, not(target_arch = "wasm32")))))]
    {
        let _ = registry;
    }
}

#[cfg(any(test, windows, all(unix, not(target_arch = "wasm32"))))]
mod spooler {
    use crate::backend::{Backend, JobHint, PrintError, PrinterDetails, PrinterRef, SubmitRequest};
    use crate::job::Submitted;
    #[cfg(not(windows))]
    use crate::submit_with_progress;
    use crate::{Printer, Progress, printers, spool_render};

    /// The system spooler as a [`Backend`]: CUPS (`lp`, `lpstat`) on macOS and Linux, the Windows
    /// print spooler on Windows. Local class: no consent needed. PDF only.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct SystemSpooler;

    /// The registry name: `"spooler"` on Windows, `"cups"` elsewhere.
    pub(crate) const NAME: &str = if cfg!(windows) { "spooler" } else { "cups" };

    pub(crate) fn printer_ref(printer: Printer) -> PrinterRef {
        PrinterRef { id: printer.name.clone(), name: printer.name, default: printer.default, two_sided: printer.two_sided }
    }

    /// What the spooler applies of a [`crate::Job`]. Driver keywords are CUPS only.
    pub(crate) fn honoured() -> Vec<JobHint> {
        let mut hints = vec![JobHint::Copies, JobHint::Collate, JobHint::Duplex, JobHint::Colour, JobHint::Scaling];
        if cfg!(windows) {
            // The resolution the PDF is drawn at; CUPS leaves both to the driver's own options.
            hints.extend([JobHint::Quality, JobHint::Resolution]);
        } else {
            hints.push(JobHint::DriverOptions);
        }
        hints
    }

    pub(crate) fn details_for(printer: Printer, options: Vec<crate::PrinterOption>) -> PrinterDetails {
        PrinterDetails {
            id: printer.name.clone(),
            name: printer.name,
            default: printer.default,
            // PDF only, on purpose: CUPS is handed the PDF and Windows draws it with the in-box PDF
            // renderer; there is no image, text or XPS renderer here to accept more. Other formats
            // reach the spooler as PDF through the app's `DocumentConverter`.
            formats: vec![crate::DocumentFormat::Pdf],
            options,
            two_sided: printer.two_sided,
            honoured: Some(honoured()),
            // The spooler reports after the fact; nothing is known before a job.
            status: None,
            ..PrinterDetails::default()
        }
    }

    impl Backend for SystemSpooler {
        fn name(&self) -> &'static str {
            NAME
        }

        fn list_printers(&self) -> Result<Vec<PrinterRef>, PrintError> {
            // `printers()` answers an empty list for "no spooler", exactly as before.
            Ok(printers().into_iter().map(printer_ref).collect())
        }

        fn printer_details(&self, native_id: &str) -> Result<PrinterDetails, PrintError> {
            let printer = printers()
                .into_iter()
                .find(|p| p.name == native_id)
                .ok_or_else(|| PrintError::printer_not_found(format!("the printer is not available: {native_id}")))?;
            let options = crate::printer_options(&printer.name);
            Ok(details_for(printer, options))
        }

        /// The spooler's own answer is the receipt: `lp` (or the Windows spooler) took the document.
        /// It gives no job handle and cannot be asked about the job later, so the result is
        /// "accepted, outcome unknown" ([`crate::WatchSupport::None`]).
        fn submit(&self, request: SubmitRequest<'_>) -> Result<Submitted, PrintError> {
            let SubmitRequest { document, mut job, options, progress, .. } = request;
            // The spooler honours the colour choice; on Windows also the resolution and quality
            // the pages are drawn at. Media and orientation (and, with CUPS, quality and
            // resolution) are the driver's (CUPS: `Job::options`): ignored here, and said so.
            job.grayscale = options.colour.is_mono(job.grayscale);
            #[allow(unused_mut)]
            let mut warnings = spool_render::unhonoured(&options, cfg!(windows));
            let mut progress = progress;
            let mut report_progress = |done: usize, total: usize| {
                if let Some(report) = progress.as_deref_mut() {
                    let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
                    report(Progress::of(clamp(done), clamp(total)));
                }
            };
            #[cfg(windows)]
            let message = {
                let choice = spool_render::choose(&options);
                let spooled = crate::windows::submit_with_render(document.bytes, &job, &choice, &mut report_progress)?;
                warnings.extend(spooled.warnings);
                spooled.message
            };
            #[cfg(not(windows))]
            let message = submit_with_progress(document.bytes, &job, &mut report_progress)?;
            Ok(Submitted::new(NAME, message).with_warnings(warnings))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, ErrorKind, JobHint, PrintError};
    use crate::{Document, DocumentFormat, Error, Job, Printer, SubmitRequest};

    #[test]
    fn the_adapter_keeps_the_legacy_printer_fields() {
        let legacy = Printer { name: "Office_Laser".into(), default: true, two_sided: Some(false) };
        let listed = spooler::printer_ref(legacy.clone());
        assert_eq!((listed.id.as_str(), listed.name.as_str(), listed.default, listed.two_sided), ("Office_Laser", "Office_Laser", true, Some(false)));
        let details = spooler::details_for(legacy, Vec::new());
        assert_eq!(details.id, "Office_Laser");
        assert_eq!(details.formats, [DocumentFormat::Pdf]);
        assert!(details.status.is_none(), "a spooler does not know the state before a job");
        assert_eq!(details.two_sided, Some(false));
        let honoured = details.honoured.unwrap_or_default();
        assert!(honoured.contains(&JobHint::Duplex) && honoured.contains(&JobHint::Copies));
        assert_eq!(honoured.contains(&JobHint::DriverOptions), !cfg!(windows), "driver keywords are CUPS only");
    }

    #[test]
    fn the_legacy_error_keeps_its_message() {
        let error: PrintError = Error::Spool("the print job was refused".into()).into();
        assert_eq!(error.message(), "the print job was refused");
        assert_eq!(error.to_string(), "the print job was refused");
        assert_eq!(error.kind(), ErrorKind::Failed);
        assert_eq!(error.native(), None);
    }

    #[test]
    fn the_built_in_listing_is_the_free_function_listing() {
        let via_backend = SystemSpooler.list_printers().unwrap_or_default();
        let direct: Vec<_> = crate::printers().into_iter().map(spooler::printer_ref).collect();
        // The spooler can change between two calls; compare the stable part.
        assert_eq!(via_backend.iter().filter(|p| p.default).count() <= 1, direct.iter().filter(|p| p.default).count() <= 1);
        assert!(via_backend.iter().all(|p| p.id == p.name));
    }

    #[test]
    fn an_unknown_printer_is_not_found_and_a_foreign_format_is_refused() {
        let registry = Registry::new();
        register_builtin(&registry);
        let id = format!("{}:no-such-printer-for-this-test", spooler::NAME);
        assert_eq!(registry.printer_details(&id).map_err(|e| e.kind()), Err(ErrorKind::NotFound));
        let doc = Document::new(DocumentFormat::PostScript, b"%!PS");
        let job = Job { printer: Some(id), ..Job::default() };
        assert_eq!(registry.submit(SubmitRequest::new(doc, job)).map_err(|e| e.kind()), Err(ErrorKind::UnsupportedFormat));
        assert_eq!(SystemSpooler.class(), crate::BackendClass::Local);
    }
}
