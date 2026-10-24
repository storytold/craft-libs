//! When a Windows job is printed through GDI after the Direct2D/XPS path failed: a decision on
//! the failure's HRESULT, the pages Direct2D had accepted and whether the XPS package had been
//! closed, so it is tested on every platform. `windows.rs` finds those and runs the GDI job.
//!
//! A retry is safe only when nothing of the XPS job can print: winprint cancels an unfinished
//! XPS job (its print context, dropped before `WinPdfPrinter::print_with_progress` returns,
//! calls `IPrintDocumentPackageTarget::Cancel` unless the job completed), and the GDI job
//! starts only after that call has returned. A failure while that context is being created
//! never gets here (`windows.rs` reads no HRESULT from it): nothing would cancel the XPS job.

/// `E_NOINTERFACE` before Direct2D accepted a page: the driver took nothing through XPS. Not
/// seen with this code on a real driver yet (the Brother report first read as this code was
/// `E_PRINTTICKET_FORMAT`, whose localized text resembles it); kept because it is safe to retry.
const E_NOINTERFACE: u32 = 0x8000_4002;
/// `E_PRINTTICKET_FORMAT` (prntvpt.h): the print system rejected the job's PrintTicket while
/// the XPS package was being written. A `FACILITY_ITF` code that localized Windows reported with
/// an "interface not supported" text ("Interfaccia non supportata.", "Schnittstelle nicht unterstützt").
/// Seen at `ID2D1PrintControl::Close` of a one-page job with the Brother MFC-L3760CDW series
/// driver, which then printed the job through GDI; the same code is reported for a one-page job
/// with the Brother MFC-L2710DN series driver (storytold/pdfcraft#3032). The likely cause is the
/// one upstream ArcticLampyrid/winprint.rs#3 reports for a Brother driver with the same code
/// ("Duplicate definitions exist for feature or parameter JobPrintSpeed"): the driver's
/// PrintCapabilities define a vendor `JobPrintSpeed` and Windows adds its own. The same ticket
/// still becomes a DEVMODE, which GDI prints with.
const E_PRINTTICKET_FORMAT: u32 = 0x8004_0003;

/// Whether the job goes to GDI after the Direct2D/XPS attempt failed with `hresult`, once
/// Direct2D had accepted `pages_accepted` pages; `package_closed` when the failure is the
/// spooler's report on a job whose package had been closed (written in full).
///
/// - `E_NOINTERFACE`: only before any page was accepted, when the driver refused the XPS path
///   itself. Nothing reached the XPS job, so nothing can print twice.
/// - `E_PRINTTICKET_FORMAT`: with at most one page accepted, as long as the package wasn't
///   closed. The rejected ticket is the job's, the same for every page (craft-print gives
///   Direct2D no page tickets), so it is expected to be refused at the first hand-over to the
///   package: seen at `Close` of a one-page job; inferred, not seen, at `AddPage` of page 2 of a
///   longer job (one page accepted). winprint asks the spooler to cancel the XPS job. With two
///   or more pages accepted, earlier pages may already be spooling (or printing, on a queue
///   that starts while spooling): it is reported, not retried, rather than risk printing them
///   twice.
/// - A closed package never: the spooler had the whole job, and some of it may have printed.
///
/// Any other failure is the job's own: an offline printer or a paused queue, cancellation
/// (`D2DERR_PRINT_JOB_CLOSED` means the job had already ended, which cancelling it also
/// causes), timeouts, ticket errors, a printer with no XPS format
/// (`D2DERR_PRINT_FORMAT_NOT_SUPPORTED`, which comes from creating the print control).
pub(crate) fn falls_back_to_gdi(hresult: u32, pages_accepted: u32, package_closed: bool) -> bool {
    if package_closed {
        return false;
    }
    match hresult {
        E_PRINTTICKET_FORMAT => pages_accepted <= 1,
        E_NOINTERFACE => pages_accepted == 0,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_driver_refusing_the_xps_path_before_any_page_falls_back() {
        assert!(falls_back_to_gdi(0x8000_4002, 0, false));
    }

    #[test]
    fn a_job_with_an_accepted_page_never_prints_again() {
        for accepted in [1, 3, u32::MAX] {
            assert!(!falls_back_to_gdi(0x8000_4002, accepted, false), "{accepted}");
        }
    }

    #[test]
    fn a_rejected_print_ticket_falls_back_at_the_first_hand_over_only() {
        // No page yet; Close of a one-page job and AddPage of page 2 (1).
        for accepted in [0, 1] {
            assert!(falls_back_to_gdi(0x8004_0003, accepted, false), "{accepted}");
        }
        // Later, earlier pages may already be spooling: never printed twice.
        for accepted in [2, 250, u32::MAX] {
            assert!(!falls_back_to_gdi(0x8004_0003, accepted, false), "{accepted}");
        }
    }

    #[test]
    fn a_failure_reported_after_the_package_was_closed_never_falls_back() {
        for hresult in [0x8004_0003, 0x8000_4002] {
            for accepted in [0, 1, 5] {
                assert!(!falls_back_to_gdi(hresult, accepted, true), "{hresult:#010x} {accepted}");
            }
        }
    }

    #[test]
    fn other_failures_never_fall_back() {
        for hresult in [
            0x8899_002C, // D2DERR_PRINT_FORMAT_NOT_SUPPORTED: from CreatePrintControl, never seen
            0x8899_002B, // D2DERR_PRINT_JOB_CLOSED: the job had ended, e.g. canceled
            0x8007_04C7, // HRESULT_FROM_WIN32(ERROR_CANCELLED)
            0x8007_05B4, // HRESULT_FROM_WIN32(ERROR_TIMEOUT)
            0x8007_0709, // HRESULT_FROM_WIN32(ERROR_INVALID_PRINTER_NAME)
            0x8007_000E, // E_OUTOFMEMORY
            0x8007_0057, // E_INVALIDARG
            0x8000_4005, // E_FAIL
            0x8004_0004, // E_PRINTCAPABILITIES_FORMAT
            0x8004_0005, // E_DELTA_PRINTTICKET_FORMAT
            0,
        ] {
            for accepted in [0, 1] {
                assert!(!falls_back_to_gdi(hresult, accepted, false), "{hresult:#010x} {accepted}");
            }
        }
    }
}
