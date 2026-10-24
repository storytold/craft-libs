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
use crate::{Error, Job, Printer, PrinterPaper, Scaling};

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
/// driver without one cannot confirm an explicit two-sided request, which is refused before submission.
fn two_sided(device: &PrinterDevice) -> Option<bool> {
    let capabilities = capabilities_for(device, user_ticket(device).as_ref()).ok()?;
    let duplex = OwnedName::qualified("JobDuplexAllDocumentsContiguously", NS_PSK, Some("psk"));
    Some(capabilities.options_for_feature(duplex).any(|o| o.name.as_ref().is_some_and(|n| n.local_name.starts_with("TwoSided"))))
}

/// [`crate::printer_papers`]: the driver's page media sizes as it has them, keyed by their
/// Print Schema or vendor name and named by their display name, the driver's current default
/// marked. Roll media (a side of 0) and sizes beyond PDF's 200-inch page limit are left out, as
/// are repeated keywords.
pub(crate) fn printer_papers(printer: &str) -> Vec<PrinterPaper> {
    const MAX_SIDE: f64 = 14_400.0;
    let Ok(devices) = PrinterDevice::all() else { return Vec::new() };
    let Some(device) = devices.iter().find(|d| d.name() == printer) else { return Vec::new() };
    let base = user_ticket(device);
    let Ok(capabilities) = capabilities_for(device, base.as_ref()) else { return Vec::new() };
    let default_key = default_media_keyword(device, base.as_ref(), &capabilities);
    let mut out: Vec<PrinterPaper> = Vec::new();
    for media in capabilities.page_media_sizes() {
        if out.len() >= crate::MAX_PRINTER_PAPERS {
            break;
        }
        let Some(key) = media.option().name.as_ref().map(|n| n.local_name.clone()).filter(|k| !k.is_empty()) else { continue };
        let size = media.size();
        let (w, h) = (ticket::points(size.width_in_micron()), ticket::points(size.height_in_micron()));
        if size.is_roll() || !(1.0..=MAX_SIDE).contains(&w) || !(1.0..=MAX_SIDE).contains(&h) || out.iter().any(|p| p.key == key) {
            continue;
        }
        let label = media.display_name().filter(|l| !l.trim().is_empty()).map_or_else(|| key.clone(), |l| l.trim().to_string());
        let default = default_key.as_deref() == Some(key.as_str());
        out.push(PrinterPaper { key, label, size: (w, h), default });
    }
    out
}

/// The signed-in user's own settings for the printer as a job ticket: what they set in its
/// Printing Preferences (`PrintTicket::user_default`). Windows' Print Ticket calls otherwise work
/// from the printer's Printing Defaults, which can hold another label stock, another darkness and
/// no cut (a Zebra ZD500 whose defaults said 4.09 × 3.00 in, darkness 10, while the user had set
/// 2.25 × 1.25 in, darkness 22); every ticket and capabilities request here starts from this one.
/// `None` when the driver won't say, and then the printer's defaults apply as before.
fn user_ticket(device: &PrinterDevice) -> Option<PrintTicket> {
    PrintTicket::user_default(device).ok()
}

/// The driver's capabilities for the user's settings, else for the printer's defaults.
fn capabilities_for(device: &PrinterDevice, base: Option<&PrintTicket>) -> Result<PrintCapabilities, Error> {
    match base {
        Some(base) => PrintCapabilities::fetch_for_ticket(device, base).or_else(|_| PrintCapabilities::fetch(device)),
        None => PrintCapabilities::fetch(device),
    }
    .map_err(spool_error)
}

/// [`crate::is_label_printer`]: the driver's PageOutputColor options and the paper of the user's
/// settings ([`ticket::label_shaped`]).
pub(crate) fn is_label_printer(printer: &str) -> bool {
    let Ok(devices) = PrinterDevice::all() else { return false };
    let Some(device) = devices.iter().find(|d| d.name() == printer) else { return false };
    let base = user_ticket(device);
    let Ok(capabilities) = capabilities_for(device, base.as_ref()) else { return false };
    let colour = OwnedName::qualified("PageOutputColor", NS_PSK, Some("psk"));
    let colours: Vec<String> = capabilities.options_for_feature(colour).filter_map(|o| o.name.as_ref().map(|n| n.local_name.clone())).collect();
    let colours: Vec<&str> = colours.iter().map(String::as_str).collect();
    let paper = base.as_ref().and_then(|t| std::str::from_utf8(t.get_xml()).ok()).and_then(ticket::media_size);
    ticket::label_shaped(&colours, paper)
}

/// The keyword of the driver's current default paper among `capabilities`' papers: the user's
/// settings' paper when the capabilities have it, else an empty ticket merged and validated, which
/// comes back complete at the printer's defaults (the stock a label printer is set up for, an
/// office printer's tray paper); the capabilities fall back to the printer's defaults when the
/// user's can't be read, and then so does the default. `None` when the driver won't say.
fn default_media_keyword(device: &PrinterDevice, base: Option<&PrintTicket>, capabilities: &PrintCapabilities) -> Option<String> {
    let offered = |key: &str| capabilities.page_media_sizes().any(|m| m.option().name.as_ref().is_some_and(|n| n.local_name == key));
    if let Some(key) = base.and_then(|t| std::str::from_utf8(t.get_xml()).ok()).and_then(ticket::media_keyword)
        && offered(&key)
    {
        return Some(key);
    }
    let mut builder = PrintTicketBuilder::new(device).ok()?;
    builder.merge(PrintTicket::from_xml(winprint::ticket::DEFAULT_PRINT_TICKET_XML)).ok()?;
    let validated = builder.build().ok()?;
    std::str::from_utf8(validated.get_xml()).ok().and_then(ticket::media_keyword)
}

/// What [`driver_ticket`] settled on.
struct DriverTicket {
    /// The ticket the job is printed with.
    ticket: PrintTicket,
    /// The driver's merged and validated ticket: what the driver kept of the job's options
    /// (copies, collation, two-sided, colour), held to them by `ticket::verify_job_options`
    /// whichever ticket is printed with.
    validated: PrintTicket,
    /// The paper, in micrometres.
    paper: (u32, u32),
    /// Whether the driver has a two-sided feature at all.
    duplex_offered: bool,
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
fn driver_ticket(device: &PrinterDevice, job: &Job, sheet: (f64, f64), landscape: bool) -> Result<DriverTicket, Error> {
    let base = user_ticket(device);
    let capabilities = capabilities_for(device, base.as_ref())?;
    // Whether the driver has a two-sided feature at all: a label printer's doesn't, and its
    // answer to a one-sided request is then not judged (see `ticket::verify_job_options`).
    let duplex_offered =
        capabilities.options_for_feature(OwnedName::qualified("JobDuplexAllDocumentsContiguously", NS_PSK, Some("psk"))).next().is_some();
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
    // Which way the sheet lies on the paper: turned when its shape differs from the paper's as the
    // driver feeds it. Office papers are upright, so a sheet wider than tall is turned; a label
    // stock defined across the feed (a Zebra's 2.75 × 1.25 in) is wider than tall itself, so a
    // sheet of that shape lies on it as it is and must not be turned, and an upright sheet must.
    // A sheet that matches none of the driver's papers (shrink to fit) is printed on the driver's
    // default paper, so the turn is judged against that one.
    let fed = wanted.or_else(|| {
        let key = default_media_keyword(device, base.as_ref(), &capabilities)?;
        let m = medias.iter().find(|m| m.option().name.as_ref().is_some_and(|n| n.local_name == key))?;
        Some((m.size().width_in_micron(), m.size().height_in_micron()))
    });
    let turned = ticket::turned(landscape, fed);
    let requested = PrintTicket::from_xml(ticket::ticket_xml(job, turned));
    let mut document = PrintTicketDocument::parse_from_bytes(requested.get_xml()).map_err(spool_error)?;
    if let Some(media) = media {
        let media_ticket: PrintTicket = media.clone().into();
        let media_document = PrintTicketDocument::parse_from_bytes(media_ticket.get_xml()).map_err(spool_error)?;
        document.features.extend(media_document.features);
        document.parameter_inits.extend(media_document.parameter_inits);
    }
    let scaling_name = OwnedName::qualified("PageScaling", NS_PSK, Some("psk"));
    let scaling_offered: Vec<_> = capabilities.options_for_feature(scaling_name.clone()).collect();
    let is_page_scaling = |f: &PrintFeature| f.name.local_name == "PageScaling" && f.name.namespace_ref() == Some(NS_PSK);
    if scaling_offered.is_empty() {
        // A driver without a PageScaling feature (Zebra's ZDesigner label drivers) is not asked
        // for one: asked for None, it answers CustomSquare, which the check below would refuse,
        // though it cannot scale by a feature it doesn't have.
        document.features.retain(|f| !is_page_scaling(f));
    } else if let Some(no_scaling) = scaling_offered.iter().find(|o| o.name.as_ref().is_some_and(|n| n.local_name == "None")) {
        document.features.retain(|f| !is_page_scaling(f));
        document.features.push(PrintFeature {
            name: scaling_name,
            properties: Vec::new(),
            options: vec![(*no_scaling).clone()],
            features: Vec::new(),
        });
    }
    let requested_xml = PrintTicket::from(document).into_xml();
    // The job starts from the user's own settings (stock, darkness, cutter: the driver's private
    // features ride along) and the job's options are merged over them.
    let mut builder = PrintTicketBuilder::new(device).map_err(spool_error)?;
    if let Some(base) = &base {
        builder.merge(base.clone()).map_err(spool_error)?;
    }
    builder.merge(PrintTicket::from_xml(requested_xml.clone())).map_err(spool_error)?;
    let validated = builder.build().map_err(spool_error)?;
    let xml = std::str::from_utf8(validated.get_xml()).map_err(spool_error)?;
    // The paper the job comes out on is the one in the DEVMODE winprint prints with: the ticket
    // converted on the user's own settings and settled by the driver. The merged ticket is built
    // on the printer's defaults instead, and a driver whose one "User defined" stock is set in
    // the user's Printing Preferences but not in the printer's Printing Defaults (Zebra's) names
    // the defaults' size there while printing on the preferences'. So when the driver settles on
    // the paper asked for, the ticket asked for is the one printed with, and the merged ticket is
    // only held to the rest; otherwise the merged ticket is held to the paper as well. The ticket
    // asked for is laid over the user's settings, so their private features (a cutter, a
    // darkness, a finisher) are printed with too: the spooler merges a job's ticket onto the
    // printer's defaults, not the user's.
    let requested = PrintTicket::from_xml(requested_xml);
    let requested = match &base {
        Some(base) => overlay(base, &requested)?,
        None => requested,
    };
    let settled = requested.to_dev_mode(device).ok().and_then(|dev_mode| ticket::devmode_paper(&dev_mode));
    let same = |a: (u32, u32), b: (u32, u32)| a.0.abs_diff(b.0) <= ticket::PAPER_TOLERANCE && a.1.abs_diff(b.1) <= ticket::PAPER_TOLERANCE;
    if let (Some((size, settled_landscape)), Some(want)) = (settled, wanted)
        && (same(size, want) || same(size, (want.1, want.0)))
        && settled_landscape == turned
    {
        ticket::verify_ticket(xml, None, turned, !scaling_offered.is_empty())?;
        return Ok(DriverTicket { ticket: requested, validated, paper: size, duplex_offered });
    }
    let paper = ticket::verify_ticket(xml, wanted, turned, !scaling_offered.is_empty())?;
    Ok(DriverTicket { ticket: validated.clone(), validated, paper, duplex_offered })
}

/// `over` laid over `base`: each of `over`'s features, parameters and properties replaces
/// `base`'s of the same name, and the rest of `base` is kept.
fn overlay(base: &PrintTicket, over: &PrintTicket) -> Result<PrintTicket, Error> {
    let same = |a: &OwnedName, b: &OwnedName| a.local_name == b.local_name && a.namespace == b.namespace;
    let mut document = PrintTicketDocument::parse_from_bytes(base.get_xml()).map_err(spool_error)?;
    let top = PrintTicketDocument::parse_from_bytes(over.get_xml()).map_err(spool_error)?;
    document.features.retain(|f| !top.features.iter().any(|t| same(&t.name, &f.name)));
    document.features.extend(top.features);
    document.parameter_inits.retain(|p| !top.parameter_inits.iter().any(|t| same(&t.name, &p.name)));
    document.parameter_inits.extend(top.parameter_inits);
    document.properties.retain(|p| !top.properties.iter().any(|t| same(&t.name, &p.name)));
    document.properties.extend(top.properties);
    Ok(PrintTicket::from(document))
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
        let DriverTicket { ticket, validated, paper, duplex_offered } = driver_ticket(&device, job, first, landscape)?;
        let xml = std::str::from_utf8(validated.get_xml()).map_err(spool_error)?;
        ticket::verify_job_options(xml, job, count, duplex_offered)?;
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
        let DriverTicket { ticket, validated, paper, duplex_offered } = driver_ticket(&device, &job, (612.0, 792.0), false).unwrap();
        ticket::verify_job_options(std::str::from_utf8(validated.get_xml()).unwrap(), &job, 1, duplex_offered).unwrap();
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
            let DriverTicket { validated, duplex_offered, .. } = driver_ticket(&device, &requested, (612.0, 792.0), false).unwrap();
            let retained = ticket::verify_job_options(std::str::from_utf8(validated.get_xml()).unwrap(), &requested, 3, duplex_offered);
            eprintln!("{feature}={wanted}: {retained:?}");
            let offered = capabilities
                .options_for_feature(OwnedName::qualified(feature, NS_PSK, Some("psk")))
                .any(|o| o.name.as_ref().is_some_and(|n| n.local_name == wanted || (feature == "PageOutputColor" && n.local_name == "Grayscale")));
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
}
