//! The drawn route on Windows: the job's ticket validated by the driver as for the standard
//! route, converted to the DEVMODE a GDI job starts with, and every page drawn by the caller's
//! [`SheetPainter`] into one GDI document.

use winprint::printer::PrinterDevice;
use winprint::ticket::PrintTicket;

use super::{default_printer, driver_offers, driver_ticket, imageable_margin, print_ticket_provider, select_device, spool_error};
use crate::drawn::{DrawnSheet, SheetPainter, gdi_error, page_order};
use crate::sheets::{self, Fit, SAME};
use crate::ticket;
use crate::{Cancel, Error, Job, Scaling, rgb};

/// What the driver made of the job: its validated ticket, the paper (µm) and whether the pages
/// print on both sides.
struct Validated {
    ticket: PrintTicket,
    paper: (u32, u32),
    two_sided: bool,
}

/// The job's ticket for one copy: the application draws the copies, so the driver is asked for
/// one and never collates.
fn validate(device: &PrinterDevice, job: &Job, count: usize, first: (f64, f64), landscape: bool) -> Result<Validated, Error> {
    let single = Job { copies: 1, collate: false, ..job.clone() };
    let (ticket, paper) = driver_ticket(device, &single, first, landscape)?;
    let xml = std::str::from_utf8(ticket.get_xml()).map_err(spool_error)?;
    if ticket::verify_job_options(xml, &single, count, ticket::Offered::default()).is_err() {
        ticket::verify_job_options(xml, &single, count, driver_offers(device))?;
    }
    let two_sided = match ticket::one_job_sides(xml) {
        Some(two_sided) => two_sided,
        None if job.copies > 1 => {
            return Err(spool_error(
                "the driver's settings print several pages per sheet, bind or staple, which would join the copies; print one copy at a time",
            ));
        }
        None => false,
    };
    Ok(Validated { ticket, paper, two_sided })
}

/// The DEVMODE the GDI job starts with, converted from the validated ticket.
fn dev_mode(device: &PrinterDevice, ticket: &PrintTicket) -> Result<rgb::DevMode, Error> {
    let bytes = {
        let _provider = print_ticket_provider();
        ticket.to_dev_mode(device).map_err(spool_error)?
    };
    let whole = crate::whole_dev_mode(bytes).ok_or_else(|| spool_error("the driver's settings are shorter than they say"))?;
    rgb::DevMode::from_bytes(whole).map_err(gdi_error)
}

/// The PDF as it prints: every sheet turned, and for shrink-to-fit reframed, onto the paper.
fn fitted(
    pdf: &[u8],
    read: &sheets::Sheets<'_>,
    device: &PrinterDevice,
    v: &Validated,
    job: &Job,
    first: (f64, f64),
    landscape: bool,
) -> Result<Vec<u8>, Error> {
    let (fit, paper) = match job.scaling {
        Scaling::Exact => (Fit::Exact, first),
        Scaling::ShrinkToFit => {
            let (short, long) = (ticket::points(v.paper.0.min(v.paper.1)), ticket::points(v.paper.0.max(v.paper.1)));
            (Fit::Shrink { margin: imageable_margin(device, &v.ticket) }, if landscape { (long, short) } else { (short, long) })
        }
    };
    let edits = sheets::plan(&read.sheets, paper, fit)?;
    if edits.is_empty() { Ok(pdf.to_vec()) } else { read.update(&edits) }
}

/// Sends `order`'s pages, each sheet drawn by `painter` and a `None` left blank.
fn send(
    document: &mut rgb::Document,
    order: &[Option<usize>],
    painter: &mut dyn SheetPainter,
    sender: &std::sync::mpsc::SyncSender<(usize, usize)>,
) -> Result<(), Error> {
    for (sent, sheet) in order.iter().enumerate() {
        let page = document.begin_page(None).map_err(gdi_error)?;
        if let Some(index) = *sheet {
            painter.paint(&DrawnSheet { index, page }, document)?;
        }
        document.end_page().map_err(gdi_error)?;
        let _ = sender.send((sent + 1, order.len()));
    }
    Ok(())
}

/// [`crate::submit_drawn`] on Windows.
pub(crate) fn submit(
    pdf: &[u8],
    job: &Job,
    painter: &mut dyn SheetPainter,
    output_file: Option<&str>,
    progress: &mut dyn FnMut(usize, usize),
    cancel: &Cancel,
) -> Result<String, Error> {
    let read = sheets::read(pdf)?;
    read.check_print_annotations()?;
    let count = read.sheets.len();
    let first = read.sheets.first().map(sheets::Sheet::size).ok_or_else(|| spool_error("the print-ready PDF has no pages"))?;
    let landscape = first.0 > first.1 + SAME;
    if job.scaling == Scaling::Exact {
        sheets::plan(&read.sheets, first, Fit::Exact)?;
    }
    let device = select_device(PrinterDevice::all().map_err(spool_error)?, job.printer.as_deref(), default_printer().as_deref())?;
    let read = &read;
    let sent = crate::progress::run(progress, move |sender| {
        let validated = validate(&device, job, count, first, landscape)?;
        let order = page_order(count, job.copies, job.collate, validated.two_sided)
            .ok_or_else(|| spool_error(format!("the job would send more than {} pages", crate::MAX_DRAWN_PAGES)))?;
        let settings = dev_mode(&device, &validated.ticket)?;
        painter.open(&fitted(pdf, read, &device, &validated, job, first, landscape)?)?;
        cancel.start_sending()?;
        let start = rgb::DocumentJob {
            printer: device.name(),
            settings: Some(&settings),
            title: &job.title,
            output_file,
            color_policy: rgb::ColorPolicy::DriverManaged,
            manual_copies: true,
        };
        let mut document = rgb::Document::start(&start).map_err(gdi_error)?;
        send(&mut document, &order, painter, &sender)?;
        document.finish().map_err(gdi_error)?;
        Ok(order.len())
    })?;
    Ok(format!("drew and spooled {count} sheet(s) as {sent} page(s)"))
}
