//! The Windows spooler, in this process: no PowerShell, script or other program is started, so
//! printing works where scripts are locked down (App Control for Business or AppLocker run
//! PowerShell in Constrained Language Mode; storytold/pdfcraft#1107).
//!
//! The printers come from `EnumPrinters` (local queues and the user's connections, such as
//! `\\server\queue`), the default from `GetDefaultPrinter`, and whether each driver takes
//! two-sided settings from its PrintCapabilities. A job is drawn by the in-box
//! `Windows.Data.Pdf` with Direct2D into the XPS print API, as vectors, and its options (copies,
//! collation, two-sided, colour, paper, orientation, no driver scaling) travel as a PrintTicket
//! that the driver validates before anything is sent. `winprint` and `winsafe` wrap the Win32
//! and WinRT calls, so there is no `unsafe` here.

use std::io::Write as _;
use std::sync::{Mutex, MutexGuard, PoisonError};

use winprint::printer::{PrinterDevice, WinPdfPrinter};
use winprint::ticket::document::{NS_PSK, OwnedName, PrintCapabilitiesDocument, PrintFeature, PrintTicketDocument, reader::ParsableXmlDocument as _};
use winprint::ticket::{FeatureOptionPack as _, PageMediaSize, PrintCapabilities, PrintTicket, PrintTicketBuilder};

use crate::sheets::{self, Fit, SAME};
use crate::ticket::{self, Paper};
use crate::{Cancel, Error, Job, Printer, Scaling};

fn spool_error(e: impl std::fmt::Display) -> Error {
    Error::Spool(e.to_string())
}

/// The user's default printer; `None` when there is none.
fn default_printer() -> Option<String> {
    winsafe::GetDefaultPrinter().ok().filter(|n| !n.is_empty())
}

/// Keeps two threads of this process out of the Windows PrintTicket provider (`PTOpenProvider`
/// and the calls on its handle) at the same time. When two threads make the process's first calls
/// together, winspool can keep a pointer to a device capabilities cache it has freed, and a later
/// call crashes the process.
static PRINT_TICKET_PROVIDER: Mutex<()> = Mutex::new(());

/// Holds [`PRINT_TICKET_PROVIDER`] until the guard is dropped. A lock that a panicking thread
/// held is taken like any other.
fn print_ticket_provider() -> MutexGuard<'static, ()> {
    PRINT_TICKET_PROVIDER.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn printers() -> Vec<Printer> {
    spooler_list(two_sided)
}

/// The queues and the default without asking any driver: a networked printer asleep in power
/// save can hold its driver's PrintCapabilities query for a long time, and the Print dialog
/// must not wait on it to show the queue (or any other).
pub(crate) fn printer_queues() -> Vec<Printer> {
    spooler_list(|_| None)
}

/// The spooler's queues (`EnumPrinters`, local queues and connections) with the default marked,
/// each given the two-sided support `two_sided` reports for it; empty when the spooler can't list
/// them.
fn spooler_list(two_sided: impl Fn(&PrinterDevice) -> Option<bool>) -> Vec<Printer> {
    match PrinterDevice::all() {
        Ok(devices) => crate::printer_list(devices.iter().map(|d| (d.name().to_string(), two_sided(d))), default_printer().as_deref()),
        Err(_) => Vec::new(),
    }
}

/// [`two_sided`] for the queue named `name` only; `None` when no queue has that name.
pub(crate) fn two_sided_named(name: &str) -> Option<bool> {
    PrinterDevice::all().ok()?.iter().find(|d| d.name() == name).and_then(two_sided)
}

/// Whether the driver of the queue `name` renders XPS itself (`PRINTER_DRIVER_XPS`); `None`
/// when no queue has that name or its driver can't be read.
pub(crate) fn xps_driver_named(name: &str) -> Option<bool> {
    PrinterDevice::all().ok()?.iter().find(|d| d.name() == name)?.is_xps_driver().ok()
}

#[cfg(test)]
thread_local! {
    /// The driver capability queries printer discovery made on this thread, so a test can show
    /// that [`printer_queues`] makes none.
    pub(crate) static CAPABILITY_QUERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether the printer's driver takes two-sided settings: its PrintCapabilities offer a two-sided
/// option for the feature the job's ticket sets. `None` when the capabilities can't be read. A
/// driver without one cannot confirm an explicit two-sided request, which is refused before submission.
fn two_sided(device: &PrinterDevice) -> Option<bool> {
    #[cfg(test)]
    CAPABILITY_QUERIES.with(|n| n.set(n.get().saturating_add(1)));
    let xml = {
        let _provider = print_ticket_provider();
        PrintCapabilities::fetch_xml_for_ticket(device, None)
    }
    .ok()?;
    ticket::two_sided(std::str::from_utf8(&xml).ok()?)
}

/// What the driver's PrintCapabilities offer for the job options [`ticket::verify_job_options`]
/// checks; all unknown when they can't be read.
fn driver_offers(device: &PrinterDevice) -> ticket::Offered {
    let xml = {
        let _provider = print_ticket_provider();
        PrintCapabilities::fetch_xml_for_ticket(device, None)
    };
    xml.ok().and_then(|xml| String::from_utf8(xml).ok()).map_or_else(ticket::Offered::default, |xml| ticket::offered(&xml))
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

/// The ticket the driver accepted: [`ticket::ticket_xml_with`] with the driver's own paper option for
/// `sheet` (its exact option, so a vendor's sizes survive; a ticket with dimensions only can come
/// back as Letter) and its own "no scaling" option, merged and validated against the printer's
/// capabilities. For exact size the driver must have that paper; for shrink-to-fit without one,
/// its default paper is used. Returns the ticket and its paper in micrometres. Refused when the
/// driver changed the paper, the orientation or the scaling: then nothing is sent, rather than a
/// job at the wrong size.
/// The title of a test job that [`HELD_DRIVER`] holds in its capability query.
#[cfg(test)]
pub(crate) const HELD_TITLE: &str = "held by its driver";
/// The title of a test job that [`HELD_DRIVER`] holds in its DEVMODE conversion, the last driver
/// call before it is sent.
#[cfg(test)]
pub(crate) const HELD_CONVERSION_TITLE: &str = "held in its conversion";

/// A test stands in for a driver that doesn't answer (a sleeping printer): the job titled
/// [`HELD_TITLE`] or [`HELD_CONVERSION_TITLE`] says it has reached that driver call, then waits
/// there, holding the provider like a real call, until released.
#[cfg(test)]
#[allow(clippy::type_complexity)]
pub(crate) static HELD_DRIVER: std::sync::Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>> = std::sync::Mutex::new(None);

/// Holds the job titled `title` in [`HELD_DRIVER`], once, if a test set it.
#[cfg(test)]
fn hold(job: &Job, title: &str) {
    if job.title == title
        && let Some((entered, release)) = HELD_DRIVER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    {
        let _ = entered.send(());
        let _ = release.recv();
    }
}

fn driver_ticket(device: &PrinterDevice, job: &Job, sheet: (f64, f64), landscape: bool) -> Result<(PrintTicket, (u32, u32)), Error> {
    let capabilities_xml = {
        let _provider = print_ticket_provider();
        #[cfg(test)]
        hold(job, HELD_TITLE);
        PrintCapabilities::fetch_xml_for_ticket(device, None)
    }
    .map_err(spool_error)?;
    let capabilities = PrintCapabilities { document: PrintCapabilitiesDocument::parse_from_bytes(&capabilities_xml).map_err(spool_error)? };
    let capabilities_xml = std::str::from_utf8(&capabilities_xml).map_err(spool_error)?;
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
    let document_only = ticket::document_duplex_only(capabilities_xml, job.duplex);
    let requested = PrintTicket::from_xml(ticket::ticket_xml_with(job, landscape, document_only, ticket::grey_option(capabilities_xml)));
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
    let built = {
        let _provider = print_ticket_provider();
        // The builder closes its provider when it is dropped, before the guard.
        let mut builder = PrintTicketBuilder::new(device).map_err(spool_error)?;
        builder.merge(PrintTicket::from(document)).map_err(spool_error)?;
        builder.build().map_err(spool_error)?
    };
    let validated = in_drivers_spelling(device, built);
    let xml = std::str::from_utf8(validated.get_xml()).map_err(spool_error)?;
    let paper = ticket::verify_ticket(xml, wanted, landscape)?;
    Ok((validated, paper))
}

/// The ticket with the driver's vendor namespaces under the prefixes the driver declares them with
/// ([`ticket::respell`]). Windows' merge renames them (`brpsk` to `ns0000`) when the merged ticket
/// uses one of the driver's names, such as its own paper option, and a Brother driver then fails
/// every page of the job with 0x80040003. The driver's prefixes are those of its own default ticket
/// (an empty one, validated). The ticket as it is when they can't be read.
fn in_drivers_spelling(device: &PrinterDevice, ticket: PrintTicket) -> PrintTicket {
    let spelled = (|| {
        let _provider = print_ticket_provider();
        let mut builder = PrintTicketBuilder::new(device).ok()?;
        builder.merge(PrintTicket::from_xml(ticket::empty_ticket())).ok()?;
        let own = builder.build().ok()?;
        ticket::respell(std::str::from_utf8(ticket.get_xml()).ok()?, std::str::from_utf8(own.get_xml()).ok()?)
    })();
    spelled.map_or(ticket, PrintTicket::from_xml)
}

/// The widest unprintable margin of the ticket's paper, in points; 0 when the driver doesn't say.
fn imageable_margin(device: &PrinterDevice, ticket: &PrintTicket) -> f64 {
    let xml = {
        let _provider = print_ticket_provider();
        PrintCapabilities::fetch_xml_for_ticket(device, Some(ticket))
    };
    xml.ok().and_then(|xml| String::from_utf8(xml).ok()).and_then(|xml| ticket::imageable_margin(&xml)).map_or(0.0, ticket::points)
}

/// Spool a print-ready PDF. The sheets are read first ([`sheets::read`]); everything that talks
/// to the driver runs on a thread of its own, so WinRT and COM get an apartment of their own
/// whoever calls. Windows.Data.Pdf reads from a path, so the job goes to a file in a private
/// temporary folder (an unpredictable name, only this user's) for as long as the spooler takes
/// it, and the folder is removed however the job ended. Windows answers once the job is in the
/// queue: a paused queue or an offline printer keeps the job there, as with any program.
pub(crate) fn submit_with_progress(pdf: &[u8], job: &Job, progress: &mut dyn FnMut(usize, usize), cancel: &Cancel) -> Result<String, Error> {
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
    // The worker is joined before `dir` is dropped: the spooler reads the file.
    let result = crate::progress::run(progress, move |sender| {
        let (mut ticket, mut paper) = driver_ticket(&device, job, first, landscape)?;
        let xml = std::str::from_utf8(ticket.get_xml()).map_err(spool_error)?;
        let mut repeat = None;
        // Jobs of one copy each, for collated copies that can't be one job.
        let mut jobs = 1;
        // The capabilities are read when the ticket lacks a requested value: they tell a driver
        // that dropped it from one that never had the feature. Unreadable stays strict. A driver
        // can also keep a collation it doesn't offer, so collated copies always read them.
        let checked = ticket::verify_job_options(xml, job, count, ticket::Offered::default());
        if checked.is_err() || ticket::collated_copies(job, count) {
            let offered = driver_offers(&device);
            if ticket::repeats_collated(job, count, offered) {
                let single = Job { copies: 1, ..job.clone() };
                (ticket, paper) = driver_ticket(&device, &single, first, landscape)?;
                let xml = std::str::from_utf8(ticket.get_xml()).map_err(spool_error)?;
                ticket::verify_job_options(xml, &single, count, offered)?;
                // Two-sided as the driver will print it, the queue's default included.
                let copies = job.copies.clamp(1, 999);
                match ticket::one_job_sides(xml) {
                    Some(two_sided) => repeat = Some(sheets::Repeat { copies, two_sided }),
                    None => jobs = copies,
                }
            } else if checked.is_err() {
                ticket::verify_job_options(xml, job, count, offered)?;
            }
        }
        let (fit, paper) = match job.scaling {
            Scaling::Exact => (Fit::Exact, first),
            Scaling::ShrinkToFit => {
                let (short, long) = (ticket::points(paper.0.min(paper.1)), ticket::points(paper.0.max(paper.1)));
                (Fit::Shrink { margin: imageable_margin(&device, &ticket) }, if landscape { (long, short) } else { (short, long) })
            }
        };
        let edits = sheets::plan(&read.sheets, paper, fit)?;
        let mut updated = None;
        if let Some(r) = repeat {
            updated = read.repeated(&edits, r)?;
            if updated.is_none() {
                (repeat, jobs) = (None, r.copies);
            }
        }
        if updated.is_none() && !edits.is_empty() {
            updated = Some(read.update(&edits)?);
        }
        // The last point before anything reaches the spooler. Until then the job may wait on a
        // driver (a sleeping printer can hold one for long), its own or another job's holding the
        // provider, and a job cancelled meanwhile stops here and is never sent. winprint converts
        // the ticket to a DEVMODE through the driver before it creates the native job, so that
        // conversion is done here first, while the job can still be cancelled; the provider stays
        // held until the first copy's own conversion has run, so no other job comes between them.
        let mut first = Some(print_ticket_provider());
        #[cfg(test)]
        hold(job, HELD_CONVERSION_TITLE);
        ticket.to_dev_mode(&device).map_err(spool_error)?;
        cancel.start_sending()?;
        // Written once sending: a cancelled job leaves no copy of the document behind.
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(written)?;
        file.write_all(updated.as_deref().unwrap_or(pdf)).map_err(written)?;
        drop(file);
        let printer = WinPdfPrinter::new(device);
        let jobs = jobs as usize;
        for copy in 0..jobs {
            // winprint converts the ticket to a DEVMODE through the provider before each job's
            // first progress report, which releases the guard.
            let mut provider = first.take().or_else(|| Some(print_ticket_provider()));
            printer
                .print_with_progress(&path, ticket.clone(), |processed, total| {
                    drop(provider.take());
                    // Page acceptance is distinct from terminal job success. The callback
                    // drains on the submitter's thread, so it can retain non-Send UI state.
                    let (processed, total) = (processed as usize, total as usize);
                    let _ = sender.send((copy * total + processed, jobs * total));
                })
                .map_err(|e| {
                    let mut message = e.to_string();
                    let mut cause = std::error::Error::source(&e);
                    while let Some(source) = cause {
                        message.push_str(&format!(": {source}"));
                        cause = source.source();
                    }
                    if copy > 0 {
                        message = format!("{copy} of {jobs} copies were sent, each as a job of its own, and the next was not: {message}");
                    }
                    spool_error(message)
                })?;
        }
        Ok((repeat, jobs))
    });
    drop(dir);
    Ok(match result? {
        (Some(repeat), _) => format!("spooled {count} sheet(s) {} times, collated", repeat.copies),
        (None, jobs) if jobs > 1 => format!("spooled {count} sheet(s) as {jobs} jobs, one per collated copy"),
        _ => format!("spooled {count} sheet(s)"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exact option support determines scope, including asymmetric duplex and simplex drivers.
    #[test]
    fn simplex_job_feature_does_not_hide_document_duplex() {
        for (job_option, document_option, duplex, expected) in [
            ("OneSided", "TwoSidedLongEdge", Some(crate::Duplex::LongEdge), true),
            ("OneSided", "TwoSidedShortEdge", Some(crate::Duplex::ShortEdge), true),
            ("TwoSidedLongEdge", "TwoSidedShortEdge", Some(crate::Duplex::ShortEdge), true),
            ("TwoSidedShortEdge", "TwoSidedLongEdge", Some(crate::Duplex::LongEdge), true),
            ("TwoSidedLongEdge", "OneSided", Some(crate::Duplex::Off), true),
            ("TwoSidedLongEdge", "TwoSidedLongEdge", Some(crate::Duplex::LongEdge), false),
            ("OneSided", "TwoSidedLongEdge", None, false),
            ("OneSided", "OneSided", Some(crate::Duplex::Off), false),
        ] {
            let xml = format!(
                r#"<psf:PrintCapabilities version="1" xmlns:psf="http://schemas.microsoft.com/windows/2003/08/printing/printschemaframework" xmlns:psk="{NS_PSK}">
                <psf:Feature name="psk:JobDuplexAllDocumentsContiguously"><psf:Option name="psk:{job_option}"/></psf:Feature>
                <psf:Feature name="psk:DocumentDuplex"><psf:Option name="psk:{document_option}"/></psf:Feature>
                </psf:PrintCapabilities>"#
            );
            assert_eq!(ticket::document_duplex_only(&xml, duplex), expected, "job={job_option}, document={document_option}");
            let job = Job { duplex, ..Job::default() };
            let ticket = ticket::ticket_xml_for_duplex(&job, false, ticket::document_duplex_only(&xml, duplex));
            assert_eq!(ticket.contains("psk:DocumentDuplex"), expected);
        }
    }

    /// Vendor options with standard-looking local names cannot claim Print Schema support.
    #[test]
    fn duplex_scope_requires_standard_option_namespaces() {
        for (job_options, document_options, expected) in [
            ("<psf:Option name='vendor:TwoSidedLongEdge'/>", "<psf:Option name='psk:TwoSidedLongEdge'/>", true),
            ("<psf:Option name='psk:OneSided'/>", "<psf:Option name='vendor:TwoSidedLongEdge'/>", false),
            (
                "<psf:Option name='vendor:TwoSidedLongEdge'/><psf:Option name='psk:TwoSidedLongEdge'/>",
                "<psf:Option name='psk:TwoSidedLongEdge'/>",
                false,
            ),
        ] {
            let xml = format!(
                r#"<psf:PrintCapabilities version="1" xmlns:psf="http://schemas.microsoft.com/windows/2003/08/printing/printschemaframework" xmlns:psk="{NS_PSK}" xmlns:vendor="urn:vendor">
                <psf:Feature name="psk:JobDuplexAllDocumentsContiguously">{job_options}</psf:Feature>
                <psf:Feature name="psk:DocumentDuplex">{document_options}</psf:Feature>
                </psf:PrintCapabilities>"#
            );
            assert_eq!(ticket::document_duplex_only(&xml, Some(crate::Duplex::LongEdge)), expected);
        }
    }

    /// Missing queues fail instead of sending private output to a different printer.
    #[test]
    fn a_missing_printer_never_falls_back_to_another_queue() {
        assert!(select_device(Vec::new(), Some("missing"), Some("default")).is_err());
        assert!(select_device(Vec::new(), None, None).is_err());
    }

    #[test]
    fn one_thread_at_a_time_is_in_the_print_ticket_provider() {
        use std::sync::Barrier;
        use std::sync::atomic::{AtomicUsize, Ordering};
        const THREADS: usize = 12;
        let inside = AtomicUsize::new(0);
        let most = AtomicUsize::new(0);
        let start = Barrier::new(THREADS);
        std::thread::scope(|s| {
            for _ in 0..THREADS {
                s.spawn(|| {
                    start.wait();
                    for _ in 0..20 {
                        let _provider = print_ticket_provider();
                        most.fetch_max(inside.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(most.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_panic_inside_the_print_ticket_provider_does_not_lock_it_out() {
        let panicked = std::thread::spawn(|| {
            let _provider = print_ticket_provider();
            panic!("a driver call panicked");
        })
        .join();
        assert!(panicked.is_err());
        drop(print_ticket_provider());
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
        let (ticket, paper) = driver_ticket(&device, &job, (612.0, 792.0), false).unwrap();
        ticket::verify_job_options(std::str::from_utf8(ticket.get_xml()).unwrap(), &job, 1, ticket::Offered::default()).unwrap();
        assert_eq!(paper, (215_900, 279_400));
        // The ticket names the driver's vendor namespaces as the driver does (the merge renamed them).
        // `respell` returning `None` would not show it: it also does when it refused the ticket.
        let own = {
            let _provider = print_ticket_provider();
            let mut own = PrintTicketBuilder::new(&device).unwrap();
            own.merge(PrintTicket::from_xml(ticket::empty_ticket())).unwrap();
            own.build().unwrap()
        };
        let unlike =
            ticket::prefixes_unlike_the_drivers(std::str::from_utf8(ticket.get_xml()).unwrap(), std::str::from_utf8(own.get_xml()).unwrap()).unwrap();
        assert!(unlike.is_empty(), "vendor namespaces under another prefix than the driver's (namespace, ticket, driver): {unlike:?}");
        eprintln!("unprintable margin: {} pt", imageable_margin(&device, &ticket));
        if let Ok(path) = std::env::var("CRAFT_PRINT_TEST_TICKET_OUTPUT") {
            std::fs::write(path, ticket.get_xml()).unwrap();
        }
        let capabilities = {
            let _provider = print_ticket_provider();
            PrintCapabilities::fetch(&device)
        }
        .unwrap();
        for (feature, wanted, requested) in [
            ("PageOutputColor", "Monochrome", Job { grayscale: true, ..Job::default() }),
            ("JobDuplexAllDocumentsContiguously", "TwoSidedLongEdge", Job { duplex: Some(crate::Duplex::LongEdge), ..Job::default() }),
            ("DocumentCollate", "Collated", Job { copies: 2, ..Job::default() }),
        ] {
            let (ticket, _) = driver_ticket(&device, &requested, (612.0, 792.0), false).unwrap();
            let retained = ticket::verify_job_options(std::str::from_utf8(ticket.get_xml()).unwrap(), &requested, 3, ticket::Offered::default());
            eprintln!("{feature}={wanted}: {retained:?}");
            let offered =
                [feature, if feature == "JobDuplexAllDocumentsContiguously" { "DocumentDuplex" } else { feature }].into_iter().any(|name| {
                    capabilities.options_for_feature(OwnedName::qualified(name, NS_PSK, Some("psk"))).any(|o| {
                        o.name.as_ref().is_some_and(|n| {
                            n.namespace_ref() == Some(NS_PSK) && n.local_name == wanted
                                || (feature == "PageOutputColor" && n.local_name == "Grayscale")
                        })
                    })
                });
            if !offered {
                assert!(retained.is_err(), "unsupported explicit option must not be silently dropped");
            }
        }
    }

    /// A real job, for checking a printer by hand (paper, or a paused queue's spool file):
    /// `CRAFT_PRINT_TEST_FILE` printed on `CRAFT_PRINT_TEST_PRINTER`, shrink-to-fit when
    /// `CRAFT_PRINT_TEST_SCALING=shrink`, two-sided on the long edge when
    /// `CRAFT_PRINT_TEST_DUPLEX=long`.
    #[test]
    #[ignore = "sends a real job: needs CRAFT_PRINT_TEST_PRINTER and CRAFT_PRINT_TEST_FILE"]
    fn an_installed_printer_prints_a_file() {
        let pdf = std::fs::read(std::env::var("CRAFT_PRINT_TEST_FILE").unwrap()).unwrap();
        let job = Job {
            printer: Some(std::env::var("CRAFT_PRINT_TEST_PRINTER").unwrap()),
            scaling: if std::env::var("CRAFT_PRINT_TEST_SCALING").as_deref() == Ok("shrink") { Scaling::ShrinkToFit } else { Scaling::Exact },
            duplex: (std::env::var("CRAFT_PRINT_TEST_DUPLEX").as_deref() == Ok("long")).then_some(crate::Duplex::LongEdge),
            title: "craft-print test".into(),
            ..Job::default()
        };
        let sent = crate::submit(&pdf, &job);
        eprintln!("{sent:?}");
        assert!(sent.is_ok());
    }

    /// A colour chosen in the driver's own preferences (Properties…) reaches the job when the job
    /// doesn't ask for grayscale: the validated ticket has the colour of the user's preferences,
    /// whatever they are (storytold/pdfcraft#2422; it was always Color before). Set the queue to
    /// black and white in its preferences to see the case that failed. Sends no job.
    #[test]
    #[ignore = "needs CRAFT_PRINT_TEST_PRINTER naming an installed printer; sends no job"]
    fn the_preferences_colour_reaches_a_job_without_grayscale() {
        fn colour(ticket: &PrintTicket) -> Option<String> {
            let xml = std::str::from_utf8(ticket.get_xml()).ok()?;
            let document = roxmltree::Document::parse(xml).ok()?;
            let feature = document
                .root_element()
                .children()
                .find(|n| n.has_tag_name((ticket::NS_PSF, "Feature")) && n.attribute("name").is_some_and(|a| a.ends_with(":PageOutputColor")))?;
            feature.children().find(|n| n.has_tag_name((ticket::NS_PSF, "Option")))?.attribute("name").map(str::to_owned)
        }
        let name = std::env::var("CRAFT_PRINT_TEST_PRINTER").unwrap();
        let device = select_device(PrinterDevice::all().unwrap(), Some(&name), None).unwrap();
        let preferences = {
            let _provider = print_ticket_provider();
            let mut builder = PrintTicketBuilder::new(&device).unwrap();
            builder.merge(PrintTicket::from_xml(winprint::ticket::DEFAULT_PRINT_TICKET_XML.as_bytes().to_vec())).unwrap();
            colour(&builder.build().unwrap())
        };
        eprintln!("the preferences' colour: {preferences:?}");
        let (ticket, _) = driver_ticket(&device, &Job::default(), (595.28, 841.89), false).unwrap();
        assert_eq!(colour(&ticket), preferences, "a job without grayscale keeps the preferences' colour");
    }
}
