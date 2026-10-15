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

use winprint::printer::{PrinterDevice, WinPdfPrinter};
use winprint::ticket::document::{NS_PSK, OwnedName, PrintFeature, PrintTicketDocument, reader::ParsableXmlDocument as _};
use winprint::ticket::{FeatureOptionPack as _, PageMediaSize, PrintCapabilities, PrintTicket, PrintTicketBuilder};

use crate::sheets::{self, Fit, SAME};
use crate::ticket::{self, Paper};
use crate::{Error, Job, Printer, Scaling};

fn spool_error(e: impl std::fmt::Display) -> Error {
    Error::Spool(e.to_string())
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
/// driver without one drops two-sided from the ticket and prints its own way.
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
fn driver_ticket(device: &PrinterDevice, job: &Job, sheet: (f64, f64), landscape: bool) -> Result<(PrintTicket, (u32, u32)), Error> {
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
    let validated = builder.build().map_err(spool_error)?;
    let xml = std::str::from_utf8(validated.get_xml()).map_err(spool_error)?;
    let paper = ticket::verify_ticket(xml, wanted, landscape)?;
    Ok((validated, paper))
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
    let read = sheets::read(pdf)?;
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
        let (ticket, paper) = driver_ticket(&device, job, first, landscape)?;
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
        WinPdfPrinter::new(device)
            .print_with_progress(&path, ticket, |processed, total| {
                // Page acceptance is distinct from terminal job success. The callback
                // drains on the submitter's thread, so it can retain non-Send UI state.
                let _ = sender.send((processed as usize, total as usize));
            })
            .map_err(|e| {
                let mut message = e.to_string();
                let mut cause = std::error::Error::source(&e);
                while let Some(source) = cause {
                    message.push_str(&format!(": {source}"));
                    cause = source.source();
                }
                spool_error(message)
            })
    });
    drop(dir);
    result?;
    Ok(format!("spooled {count} sheet(s)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_printer_never_falls_back_to_another_queue() {
        assert!(select_device(Vec::new(), Some("missing"), Some("default")).is_err());
        assert!(select_device(Vec::new(), None, None).is_err());
    }

    /// What a real driver makes of a job's ticket: Letter, portrait, two copies, two-sided on
    /// the long edge, grayscale. Sends no job. `CRAFT_PRINT_TEST_PRINTER` names the queue;
    /// `CRAFT_PRINT_TEST_TICKET_OUTPUT` keeps the validated ticket, to see which options it kept.
    #[test]
    #[ignore = "needs CRAFT_PRINT_TEST_PRINTER naming an installed printer; sends no job"]
    fn an_installed_driver_validates_the_ticket() {
        let name = std::env::var("CRAFT_PRINT_TEST_PRINTER").unwrap();
        let device = select_device(PrinterDevice::all().unwrap(), Some(&name), None).unwrap();
        let job = Job { copies: 2, duplex: Some(crate::Duplex::LongEdge), grayscale: true, ..Job::default() };
        let (ticket, paper) = driver_ticket(&device, &job, (612.0, 792.0), false).unwrap();
        assert_eq!(paper, (215_900, 279_400));
        eprintln!("unprintable margin: {} pt", imageable_margin(&device, &ticket));
        if let Ok(path) = std::env::var("CRAFT_PRINT_TEST_TICKET_OUTPUT") {
            std::fs::write(path, ticket.get_xml()).unwrap();
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
}
