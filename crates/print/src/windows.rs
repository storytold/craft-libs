//! The Windows spooler, in this process: no PowerShell, script or other program is started, so
//! printing works where scripts are locked down (App Control for Business or AppLocker run
//! PowerShell in Constrained Language Mode; storytold/pdfcraft#1107).
//!
//! The printers come from `EnumPrinters` (local queues and the user's connections, such as
//! `\\server\queue`), the default from `GetDefaultPrinter`, and whether each driver takes
//! two-sided settings from its PrintCapabilities. A job is drawn by the in-box
//! `Windows.Data.Pdf` with Direct2D into the XPS print API, and its options (copies,
//! collation, two-sided, colour, paper, orientation, no driver scaling) travel as a PrintTicket
//! that the driver validates before anything is sent. `winprint` and `winsafe` wrap the Win32
//! and WinRT calls, so there is no `unsafe` here.
//!
//! Vector content (paths, text) stays vector in the job. `Windows.Data.Pdf` draws the images of a
//! page at the page's destination size, by default 96 dpi whatever the images in the PDF are.
//! The pages are drawn with a destination at the job's resolution instead (the one in the ticket,
//! no ceiling, a page is always drawn whole); the job's resolution and quality hints choose the
//! page resolution the ticket asks for ([`crate::spool_render`]). The renderer
//! hands over an image at the smaller of its own size and about 4 million pixels (measured: an
//! A4 page's image arrives at about 211 dpi however large the source and the destination are);
//! tiling the page does not get past that, since every tile gets the whole image at that size.

use std::io::Write as _;

use winprint::printer::{PrinterDevice, WinPdfPrinter};
use winprint::ticket::document::{NS_PSK, OwnedName, PrintFeature, PrintTicketDocument, reader::ParsableXmlDocument as _};
use winprint::ticket::{FeatureOptionPack as _, PageMediaSize, PrintCapabilities, PrintTicket, PrintTicketBuilder};

use crate::sheets::{self, Fit, SAME};
use crate::spool_render::{self, RenderChoice};
use crate::ticket::{self, Paper};
use crate::trace::JobTrace;
use crate::{Error, Job, Printer, Scaling};

fn spool_error(e: impl std::fmt::Display) -> Error {
    Error::Spool(e.to_string())
}

/// An error with its whole chain of causes: "Render error: The parameter is incorrect. (0x80070057)".
fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut message = e.to_string();
    let mut cause = e.source();
    while let Some(source) = cause {
        message.push_str(&format!(": {source}"));
        cause = source.source();
    }
    message
}

/// The user's default printer; `None` when there is none.
fn default_printer() -> Option<String> {
    winsafe::GetDefaultPrinter().ok().filter(|n| !n.is_empty())
}

pub(crate) fn printers() -> Vec<Printer> {
    match PrinterDevice::all() {
        Ok(devices) => crate::printer_list(devices.iter().map(|d| (d.name().to_string(), two_sided(d))), default_printer().as_deref()),
        Err(_) => Vec::new(),
    }
}

/// Whether the printer's driver takes two-sided settings: its PrintCapabilities offer a two-sided
/// option for the feature the job's ticket sets. `None` when the capabilities can't be read. A
/// driver without one cannot confirm an explicit two-sided request, which is refused before submission.
fn two_sided(device: &PrinterDevice) -> Option<bool> {
    let capabilities = PrintCapabilities::fetch(device).ok()?;
    let duplex = OwnedName::qualified("JobDuplexAllDocumentsContiguously", NS_PSK, Some("psk"));
    Some(capabilities.options_for_feature(duplex).any(|o| o.name.as_ref().is_some_and(|n| n.local_name.starts_with("TwoSided"))))
}

/// The queue the job goes to: the one asked for, else the default. Never another one: a job
/// for a queue that has gone must not come out of a different printer.
fn select_device(devices: Vec<PrinterDevice>, requested: Option<&str>, default: Option<&str>) -> Result<PrinterDevice, Error> {
    let name = requested.or(default).ok_or_else(|| spool_error("there is no default printer; choose a printer"))?;
    devices
        .into_iter()
        .find(|p| p.name() == name)
        .ok_or_else(|| spool_error(format!("the printer is not available: {name} (reopen Print to refresh the list)")))
}

/// The ticket the driver accepted: [`ticket::ticket_xml`] with the driver's own paper option for
/// `sheet` (its exact option, so a vendor's sizes survive; a ticket with dimensions only can come
/// back as Letter) and its own "no scaling" option, merged and validated against the printer's
/// capabilities. For exact size the driver must have that paper; for shrink-to-fit without one,
/// its default paper is used. Returns the ticket and its paper in micrometres. Refused when the
/// driver changed the paper, the orientation or the scaling: then nothing is sent, rather than a
/// job at the wrong size.
///
/// `choice` picks the page resolution among the driver's; the returned note says when the one
/// used is not the one asked for.
#[allow(clippy::type_complexity)]
fn driver_ticket(
    device: &PrinterDevice,
    job: &Job,
    sheet: (f64, f64),
    landscape: bool,
    choice: &RenderChoice,
) -> Result<(PrintTicket, (u32, u32), Option<String>), Error> {
    let capabilities = PrintCapabilities::fetch(device).map_err(spool_error)?;
    let medias: Vec<PageMediaSize> = capabilities.page_media_sizes().collect();
    let papers: Vec<Paper<'_>> = medias
        .iter()
        .map(|m| Paper {
            width: m.size().width_in_micron(),
            height: m.size().height_in_micron(),
            name: m.display_name(),
            keyword: m.option().name.as_ref().is_some_and(|n| n.namespace_ref() == Some(NS_PSK)),
        })
        .collect();
    let media = ticket::matching_paper(&papers, sheet.0, sheet.1).and_then(|i| medias.get(i));
    let wanted = match (media, job.scaling) {
        (Some(m), _) => Some((m.size().width_in_micron(), m.size().height_in_micron())),
        (None, Scaling::ShrinkToFit) => None,
        (None, Scaling::Exact) => {
            return Err(spool_error(format!(
                "{} has no {:.2} × {:.2} inch paper; choose a paper size it has, or add this one in its driver",
                device.name(),
                sheet.0 / 72.0,
                sheet.1 / 72.0
            )));
        }
    };
    let requested = PrintTicket::from_xml(ticket::ticket_xml(job, landscape));
    let mut document = PrintTicketDocument::parse_from_bytes(requested.get_xml()).map_err(spool_error)?;
    if let Some(media) = media {
        let media_ticket: PrintTicket = media.clone().into();
        let media_document = PrintTicketDocument::parse_from_bytes(media_ticket.get_xml()).map_err(spool_error)?;
        document.features.extend(media_document.features);
        document.parameter_inits.extend(media_document.parameter_inits);
    }
    let scaling_name = OwnedName::qualified("PageScaling", NS_PSK, Some("psk"));
    if let Some(no_scaling) = capabilities.options_for_feature(scaling_name.clone()).find(|o| o.name.as_ref().is_some_and(|n| n.local_name == "None"))
    {
        document.features.retain(|f| !(f.name.local_name == "PageScaling" && f.name.namespace_ref() == Some(NS_PSK)));
        document.features.push(PrintFeature { name: scaling_name, properties: Vec::new(), options: vec![no_scaling.clone()], features: Vec::new() });
    }
    let mut builder = PrintTicketBuilder::new(device).map_err(spool_error)?;
    builder.merge(PrintTicket::from(document)).map_err(spool_error)?;
    let mut resolutions: Vec<_> = capabilities.page_resolutions().collect();
    let offered: Vec<(u32, u32)> = resolutions.iter().map(|r| r.dpi()).collect();
    let mut note = None;
    if let Some(pick) = spool_render::pick_resolution(&offered, choice) {
        note = pick.note;
        if let Some(index) = pick.index {
            builder.merge(resolutions.swap_remove(index)).map_err(spool_error)?;
        }
    }
    let validated = builder.build().map_err(spool_error)?;
    let xml = std::str::from_utf8(validated.get_xml()).map_err(spool_error)?;
    let paper = ticket::verify_ticket(xml, wanted, landscape)?;
    Ok((validated, paper, note))
}

/// The widest unprintable margin of the ticket's paper, in points; 0 when the driver doesn't say.
fn imageable_margin(device: &PrinterDevice, ticket: &PrintTicket) -> f64 {
    PrintCapabilities::fetch_xml_for_ticket(device, Some(ticket))
        .ok()
        .and_then(|xml| String::from_utf8(xml).ok())
        .and_then(|xml| ticket::imageable_margin(&xml))
        .map_or(0.0, ticket::points)
}

/// Spool a print-ready PDF. The sheets are read first ([`sheets::read`]); everything that talks
/// to the driver runs on a thread of its own, so WinRT and COM get an apartment of their own
/// whoever calls. Windows.Data.Pdf reads from a path, so the job goes to a file in a private
/// temporary folder (an unpredictable name, only this user's) for as long as the spooler takes
/// it, and the folder is removed however the job ended. Windows answers once the job is in the
/// queue: a paused queue or an offline printer keeps the job there, as with any program.
pub(crate) fn submit_with_progress(pdf: &[u8], job: &Job, progress: &mut dyn FnMut(usize, usize)) -> Result<String, Error> {
    submit_with_render(pdf, job, &RenderChoice::default(), progress).map(|spooled| spooled.message)
}

/// What the spooler accepted: the message for people and what was changed on the way.
pub(crate) struct Spooled {
    pub(crate) message: String,
    pub(crate) warnings: Vec<String>,
}

/// [`submit_with_progress`] with the render resolution chosen by `choice`; its notes and any
/// difference between the resolution asked for and the one used come back as warnings.
pub(crate) fn submit_with_render(pdf: &[u8], job: &Job, choice: &RenderChoice, progress: &mut dyn FnMut(usize, usize)) -> Result<Spooled, Error> {
    let trace = JobTrace::begin();
    trace.line(|| format!("submit: {} bytes, printer {:?}, scaling {:?}, copies {}", pdf.len(), job.printer, job.scaling, job.copies));
    let read = sheets::read(pdf)?;
    read.check_print_annotations()?;
    let count = read.sheets.len();
    let first = read.sheets.first().map(sheets::Sheet::size).ok_or_else(|| spool_error("the print-ready PDF has no pages"))?;
    let landscape = first.0 > first.1 + SAME;
    // Exact size: every sheet must be the first one's size, either way round; checked before
    // the driver is asked anything.
    if job.scaling == Scaling::Exact {
        sheets::plan(&read.sheets, first, Fit::Exact)?;
    }
    let device = select_device(PrinterDevice::all().map_err(spool_error)?, job.printer.as_deref(), default_printer().as_deref())?;
    let written = |e: std::io::Error| spool_error(format!("the print job could not be written: {e}"));
    let dir = tempfile::Builder::new().prefix("craft-print-").tempdir().map_err(written)?;
    let path = dir.path().join(ticket::job_filename(&job.title));
    let read = &read;
    let trace = &trace;
    // The worker is joined before `dir` is dropped: the spooler reads the file.
    let result = crate::progress::run(progress, move |sender| {
        let (ticket, paper, resolution_note) = driver_ticket(&device, job, first, landscape, choice)?;
        trace.line(|| format!("ticket accepted by {}: paper {}x{} um, landscape {landscape}", device.name(), paper.0, paper.1));
        let xml = std::str::from_utf8(ticket.get_xml()).map_err(spool_error)?;
        ticket::verify_job_options(xml, job, count)?;
        let (fit, paper) = match job.scaling {
            Scaling::Exact => (Fit::Exact, first),
            Scaling::ShrinkToFit => {
                let (short, long) = (ticket::points(paper.0.min(paper.1)), ticket::points(paper.0.max(paper.1)));
                (Fit::Shrink { margin: imageable_margin(&device, &ticket) }, if landscape { (long, short) } else { (short, long) })
            }
        };
        let edits = sheets::plan(&read.sheets, paper, fit)?;
        let updated = if edits.is_empty() { None } else { Some(read.update(&edits)?) };
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(written)?;
        file.write_all(updated.as_deref().unwrap_or(pdf)).map_err(written)?;
        drop(file);
        trace.line(|| {
            format!("job file written ({count} sheet(s) read); starting the spool job (StartDoc equivalent: the document package target is created)")
        });
        let printed = WinPdfPrinter::new(device)
            .print_with_progress(&path, ticket, |processed, total| {
                trace.line(|| format!("page {processed}/{total} accepted by Direct2D (the source PDF has {count} sheet(s))"));
                // Page acceptance is distinct from terminal job success. The callback
                // drains on the submitter's thread, so it can retain non-Send UI state.
                let _ = sender.send((processed as usize, total as usize));
            })
            .map_err(|e| spool_error(chain(&e)));
        match &printed {
            Ok(()) => trace
                .line(|| "the spooler reported the package complete (EndDoc equivalent: the job is in the queue, not necessarily printed)".into()),
            Err(e) => trace.line(|| format!("the job failed and was cancelled by winprint (AbortDoc equivalent): {e}")),
        }
        printed.map(|()| resolution_note)
    });
    drop(dir);
    if let Err(e) = &result {
        trace.line(|| format!("submit failed: {e}"));
    }
    let resolution_note = result?;
    trace.line(|| "submit finished".into());
    let mut warnings = choice.notes.clone();
    warnings.extend(resolution_note);
    Ok(Spooled { message: format!("spooled {count} sheet(s)"), warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_printer_never_falls_back_to_another_queue() {
        assert!(select_device(Vec::new(), Some("missing"), Some("default")).is_err());
        assert!(select_device(Vec::new(), None, None).is_err());
    }

    /// What a real driver makes of a job's ticket: Letter, portrait, two copies. Checks explicit
    /// option retention separately; unsupported options must fail before submission. Sends no job.
    /// `CRAFT_PRINT_TEST_PRINTER` names the queue;
    /// `CRAFT_PRINT_TEST_TICKET_OUTPUT` keeps the validated ticket, to see which options it kept.
    #[test]
    #[ignore = "needs CRAFT_PRINT_TEST_PRINTER naming an installed printer; sends no job"]
    fn an_installed_driver_validates_the_ticket() {
        let name = std::env::var("CRAFT_PRINT_TEST_PRINTER").unwrap();
        let device = select_device(PrinterDevice::all().unwrap(), Some(&name), None).unwrap();
        let job = Job { copies: 2, ..Job::default() };
        let (ticket, paper, _) = driver_ticket(&device, &job, (612.0, 792.0), false, &RenderChoice::default()).unwrap();
        ticket::verify_job_options(std::str::from_utf8(ticket.get_xml()).unwrap(), &job, 1).unwrap();
        assert_eq!(paper, (215_900, 279_400));
        eprintln!("unprintable margin: {} pt", imageable_margin(&device, &ticket));
        if let Ok(path) = std::env::var("CRAFT_PRINT_TEST_TICKET_OUTPUT") {
            std::fs::write(path, ticket.get_xml()).unwrap();
        }
        let capabilities = PrintCapabilities::fetch(&device).unwrap();
        for (feature, wanted, requested) in [
            ("PageOutputColor", "Monochrome", Job { grayscale: true, ..Job::default() }),
            ("JobDuplexAllDocumentsContiguously", "TwoSidedLongEdge", Job { duplex: Some(crate::Duplex::LongEdge), ..Job::default() }),
            ("DocumentCollate", "Collated", Job { copies: 2, ..Job::default() }),
        ] {
            let (ticket, _, _) = driver_ticket(&device, &requested, (612.0, 792.0), false, &RenderChoice::default()).unwrap();
            let retained = ticket::verify_job_options(std::str::from_utf8(ticket.get_xml()).unwrap(), &requested, 3);
            eprintln!("{feature}={wanted}: {retained:?}");
            let offered = capabilities
                .options_for_feature(OwnedName::qualified(feature, NS_PSK, Some("psk")))
                .any(|o| o.name.as_ref().is_some_and(|n| n.local_name == wanted || (feature == "PageOutputColor" && n.local_name == "Grayscale")));
            if !offered {
                assert!(retained.is_err(), "unsupported explicit option must not be silently dropped");
            }
        }
    }

    #[test]
    fn an_error_is_reported_with_its_whole_chain_of_causes() {
        #[derive(Debug)]
        struct Inner;
        impl std::fmt::Display for Inner {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("the parameter is incorrect")
            }
        }
        impl std::error::Error for Inner {}
        #[derive(Debug)]
        struct Outer(Inner);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("Render error")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        assert_eq!(chain(&Outer(Inner)), "Render error: the parameter is incorrect");
        assert_eq!(chain(&Inner), "the parameter is incorrect");
    }

    /// Run a PowerShell one-liner that only reads the spooler (test helper).
    fn read_spooler(script: &str) -> String {
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// Whether `queue` is one of the fakeprint fakes: a known fake name, its WSD port's printer
    /// UUID in the fakes' namespace. Nothing else is ever printed to by these tests.
    fn is_provably_fake(queue: &str) -> bool {
        if !["fakeprint mono1", "fakeprint inkjet1", "Fake WSD printer"].contains(&queue) {
            return false;
        }
        let port = read_spooler(&format!("(Get-Printer -Name '{queue}').PortName"));
        let uuid =
            read_spooler(&format!("(Get-ItemProperty 'HKLM:/SYSTEM/CurrentControlSet/Control/Print/Monitors/WSD Port/Ports/{port}').'Printer UUID'"));
        uuid.starts_with("00000000-0000-4000-8000-")
    }

    /// The Gridcraft-style job (shrink to fit and exact size) on a fake queue: it must finish, and
    /// the spooler must not keep it as an unfinished job. `CRAFT_TEST_FAKE_QUEUE` names the fake
    /// (checked first), `CRAFT_TEST_FAKE_PDF` the print-ready PDF. Prints only to fakes.
    #[test]
    #[ignore = "prints to a fakeprint queue: needs CRAFT_TEST_FAKE_QUEUE and CRAFT_TEST_FAKE_PDF"]
    fn a_job_to_a_fake_queue_finishes_and_leaves_nothing_unfinished() {
        let queue = std::env::var("CRAFT_TEST_FAKE_QUEUE").unwrap();
        assert!(is_provably_fake(&queue), "{queue} is not a provable fakeprint queue; refusing to print");
        let pdf = std::fs::read(std::env::var("CRAFT_TEST_FAKE_PDF").unwrap()).unwrap();
        for scaling in [Scaling::ShrinkToFit, Scaling::Exact] {
            let title = format!("craft-print spool test {scaling:?}");
            let job = Job { printer: Some(queue.clone()), scaling, title: title.clone(), ..Job::default() };
            let began = std::time::Instant::now();
            let sent = crate::submit(&pdf, &job);
            eprintln!("{scaling:?}: {sent:?} in {:?}", began.elapsed());
            assert!(sent.is_ok(), "{sent:?}");
            assert!(began.elapsed() < std::time::Duration::from_secs(30));
            std::thread::sleep(std::time::Duration::from_secs(5));
            let left = read_spooler(&format!(
                "Get-PrintJob -PrinterName '{queue}' | Where-Object {{ $_.DocumentName -like '{title}*' -and $_.JobStatus -notmatch 'Complete|Deleted|Printed' }} | ForEach-Object {{ \"$($_.Id) $($_.JobStatus)\" }}"
            ));
            assert!(left.is_empty(), "unfinished jobs left on the fake queue: {left}");
        }
    }

    /// A real job, for checking a printer by hand (paper, or a paused queue's spool file):
    /// `CRAFT_PRINT_TEST_FILE` printed on `CRAFT_PRINT_TEST_PRINTER`, shrink-to-fit when
    /// `CRAFT_PRINT_TEST_SCALING=shrink`, two-sided on the long edge when
    /// `CRAFT_PRINT_TEST_DUPLEX=long`. `CRAFT_PRINT_TEST_DPI` asks for a resolution and
    /// `CRAFT_PRINT_TEST_QUALITY` (`draft`, `normal`, `high`, `photo`) for a quality, as the
    /// registry's job hints do; the warnings the backend answers are printed. Print to a
    /// fake queue only: the page arrives at the device's resolution unless the hints say otherwise.
    #[test]
    #[ignore = "sends a real job: needs CRAFT_PRINT_TEST_PRINTER and CRAFT_PRINT_TEST_FILE"]
    fn an_installed_printer_prints_a_file() {
        use crate::backends::Backend as _;
        let pdf = std::fs::read(std::env::var("CRAFT_PRINT_TEST_FILE").unwrap()).unwrap();
        let job = Job {
            printer: Some(std::env::var("CRAFT_PRINT_TEST_PRINTER").unwrap()),
            scaling: if std::env::var("CRAFT_PRINT_TEST_SCALING").as_deref() == Ok("shrink") { Scaling::ShrinkToFit } else { Scaling::Exact },
            duplex: (std::env::var("CRAFT_PRINT_TEST_DUPLEX").as_deref() == Ok("long")).then_some(crate::Duplex::LongEdge),
            title: "craft-print test".into(),
            ..Job::default()
        };
        let mut options = crate::JobOptions::new();
        if let Some(dpi) = std::env::var("CRAFT_PRINT_TEST_DPI").ok().and_then(|v| v.parse().ok()) {
            options = options.with_resolution(crate::Resolution::square(dpi));
        }
        if let Some(quality) = std::env::var("CRAFT_PRINT_TEST_QUALITY").ok().and_then(|v| crate::Quality::from_name(&v)) {
            options = options.with_quality(quality);
        }
        let request = crate::SubmitRequest::new(crate::Document::pdf(&pdf), job).with_options(options);
        let sent = crate::SystemSpooler.submit(request);
        eprintln!("{sent:?}");
        assert!(sent.is_ok());
    }
}
