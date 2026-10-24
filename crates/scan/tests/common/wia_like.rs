//! A WIA-shaped backend and the WIA knowledge that used to sit in the core, kept here as the
//! reference for the app's real Windows backend: the HRESULT name and kind tables, the sentences
//! WIA and the WSD driver are known to produce, the document-handling status bits, and the rules
//! that sharpen a vague transfer failure with a status read.
//!
//! None of this is shipped API. The core knows only [`NativeCode`] and [`InterruptKind`]; this
//! module is what a Windows backend does with them.

use craft_scan::{
    Acquired, Backend, Connection, HandlingStatus, InterruptKind, NativeCode, Page, ScanError, ScanSettings, Scanner, ScannerRef, Source,
    interrupt_kind,
};

use super::{bmp_24, scanner};

pub const BUSY_OR_FEEDER_EMPTY: &str = craft_scan::BUSY_OR_FEEDER_EMPTY;

/// HRESULTs with a name Microsoft documents (the WIA error code list), by that exact name. Codes
/// classified but not listed by Microsoft (0x80210010 to 0x80210014: the Windows WSD driver was
/// measured returning 0x80210010 for a cover and 0x80210011 for the lamp) have no name here.
pub const CODE_NAMES: [(u32, &str); 22] = [
    (0x8021_0001, "WIA_ERROR_GENERAL_ERROR"),
    (0x8021_0002, "WIA_ERROR_PAPER_JAM"),
    (0x8021_0003, "WIA_ERROR_PAPER_EMPTY"),
    (0x8021_0004, "WIA_ERROR_PAPER_PROBLEM"),
    (0x8021_0005, "WIA_ERROR_OFFLINE"),
    (0x8021_0006, "WIA_ERROR_BUSY"),
    (0x8021_0007, "WIA_ERROR_WARMING_UP"),
    (0x8021_0008, "WIA_ERROR_USER_INTERVENTION"),
    (0x8021_0009, "WIA_ERROR_ITEM_DELETED"),
    (0x8021_000A, "WIA_ERROR_DEVICE_COMMUNICATION"),
    (0x8021_000B, "WIA_ERROR_INVALID_COMMAND"),
    (0x8021_000C, "WIA_ERROR_INCORRECT_HARDWARE_SETTING"),
    (0x8021_000D, "WIA_ERROR_DEVICE_LOCKED"),
    (0x8021_000E, "WIA_ERROR_EXCEPTION_IN_DRIVER"),
    (0x8021_000F, "WIA_ERROR_INVALID_DRIVER_RESPONSE"),
    (0x8021_0015, "WIA_S_NO_DEVICE_AVAILABLE"),
    (0x8021_0016, "WIA_ERROR_COVER_OPEN"),
    (0x8021_0017, "WIA_ERROR_LAMP_OFF"),
    (0x8021_0020, "WIA_ERROR_MULTI_FEED"),
    (0x8021_0021, "WIA_ERROR_MAXIMUM_PRINTER_ENDORSER_COUNTER"),
    (0x8000_4005, "E_FAIL"),
    (0x8007_0057, "E_INVALIDARG"),
];

/// The symbolic name of a WIA HRESULT, if the table has one.
pub fn code_name(code: u32) -> Option<&'static str> {
    CODE_NAMES.iter().find(|(known, _)| *known == code).map(|(_, name)| *name)
}

/// The kind each classified HRESULT stands for.
pub const KIND_BY_CODE: [(u32, InterruptKind); 19] = [
    (0x8021_0002, InterruptKind::Jam),
    (0x8021_0004, InterruptKind::PaperProblem),
    (0x8021_0010, InterruptKind::CoverOpen),
    (0x8021_0014, InterruptKind::MultiFeed),
    (0x8021_0003, InterruptKind::FeederEmpty),
    (0x8021_0005, InterruptKind::Offline),
    (0x8021_000A, InterruptKind::Offline),
    (0x8021_0008, InterruptKind::Attention),
    (0x8021_0001, InterruptKind::Offline),
    (0x8021_0006, InterruptKind::Busy),
    (0x8021_0007, InterruptKind::Busy),
    (0x8021_000D, InterruptKind::Busy),
    (0x8021_0011, InterruptKind::DeviceError),
    (0x8021_000E, InterruptKind::DeviceError),
    (0x8021_0013, InterruptKind::Busy),
    (0x8021_0015, InterruptKind::Offline),
    (0x8021_0016, InterruptKind::CoverOpen),
    (0x8021_0017, InterruptKind::DeviceError),
    (0x8021_0020, InterruptKind::MultiFeed),
];

/// The interrupt kind a classified HRESULT stands for.
pub fn kind_for_hresult(code: u32) -> Option<InterruptKind> {
    KIND_BY_CODE.iter().find(|(known, _)| *known == code).map(|(_, kind)| *kind)
}

/// Plain sentence for a WIA HRESULT.
pub fn wia_error_message(code: u32) -> Option<&'static str> {
    match code {
        0x8021_0001 => Some("The scanner could not be opened. If it is a network scanner, remove it in Windows and add it again while it is on."),
        0x8021_0002 => Some("The scanner has a paper jam."),
        0x8021_0003 => Some("The feeder has no paper."),
        0x8021_0004 => Some("The scanner has a paper problem."),
        0x8021_0005 => Some(
            "The scanner is offline. Check that it is powered on and connected. For a network scanner, remove it in Windows and add it again while it is on.",
        ),
        0x8021_0006 => Some("The scanner is busy. Close any other program that is using it, or wait for the current scan to finish, then try again."),
        0x8021_0007 => Some("The scanner is warming up. Try again in a moment."),
        0x8021_0008 => Some("The scanner needs attention. Check it, then try again."),
        0x8021_0009 => Some("The scanner item was removed."),
        0x8021_000A => Some("The computer could not talk to the scanner. Check the cable and power, then try again."),
        0x8021_000B => Some("The scanner rejected that command."),
        0x8021_000C => Some("A scanner setting was not accepted."),
        0x8021_000D => Some("The scanner is locked. Close any other program that is using it, then try again."),
        0x8021_000E => Some("The scanner driver failed."),
        0x8021_000F => Some("The scanner driver gave an unexpected response."),
        0x8021_0010 | 0x8021_0016 => Some("A cover on the scanner is open."),
        0x8021_0011 | 0x8021_0017 => Some("The scanner lamp is off."),
        0x8021_0012 => Some("The scan could not be saved to that destination."),
        0x8021_0013 => Some("The network scanner could not be reserved."),
        0x8021_0014 | 0x8021_0020 => Some("The feeder took more than one sheet."),
        0x8021_0015 => Some("No scanner is available. Check that it is powered on and connected."),
        0x8021_0021 => Some("The printer endorser counter is at its maximum."),
        _ => None,
    }
}

/// The kind of a WIA sentence: the table first, then the core's neutral fallback.
pub fn wia_interrupt_kind(message: &str) -> InterruptKind {
    if message.contains(BUSY_OR_FEEDER_EMPTY) {
        return InterruptKind::BusyOrFeederEmpty;
    }
    for (code, kind) in KIND_BY_CODE {
        if let Some(sentence) = wia_error_message(code)
            && message.contains(sentence)
        {
            return kind;
        }
    }
    interrupt_kind(message)
}

/// Map WS-Scan SOAP text or driver messages to the same sentences as [`wia_error_message`].
fn transfer_fault_sentence(message: &str) -> Option<&'static str> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("mediajam") || lower.contains("clienterrormediajam") {
        return wia_error_message(0x8021_0002);
    }
    if lower.contains("notacceptingjobs") || lower.contains("servererrornotacceptingjobs") {
        return wia_error_message(0x8021_0005);
    }
    if lower.contains("noimagesavailable") || lower.contains("clienterrornoimagesavailable") {
        return wia_error_message(0x8021_0003);
    }
    if lower.contains("coveropen") || lower.contains("clienterrorcoveropen") {
        return wia_error_message(0x8021_0010);
    }
    if lower.contains("multifeed") || lower.contains("clienterrordocumentfeederror") {
        return wia_error_message(0x8021_0014);
    }
    None
}

/// Text for a Windows error from WIA. Known codes become sentences; unknown ones keep the hex.
pub fn describe_windows_error(code: u32, message: &str) -> String {
    if code == 0x8021_0004
        && let Some(sentence) = transfer_fault_sentence(message.trim())
    {
        return sentence.to_string();
    }
    if let Some(text) = wia_error_message(code) {
        return text.to_string();
    }
    let trimmed = message.trim();
    if !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("ok")
        && trimmed != "The operation completed successfully."
        && !trimmed.eq_ignore_ascii_case("Unspecified error")
    {
        if let Some(sentence) = transfer_fault_sentence(trimmed) {
            return sentence.to_string();
        }
        return trimmed.to_string();
    }
    if code == 0x8000_4005 {
        return "The scan was interrupted. Check for a paper jam, an open cover, or that the scanner stayed online.".into();
    }
    if (0x8021_0000..=0x8021_FFFF).contains(&code) {
        return format!("The scanner reported an error ({code:#010x}).");
    }
    format!("The scan failed ({code:#010x}).")
}

/// An error for an HRESULT, with the sentence for it and the cause the code stands for. The sentence
/// decides only when the code is unknown or is the vague `PAPER_PROBLEM`, which the driver's own
/// text can sharpen into a jam or a double feed.
pub fn error_from_hresult(code: u32, driver_message: &str) -> ScanError {
    let message = describe_windows_error(code, driver_message);
    let from_message = wia_interrupt_kind(&message);
    let kind = match kind_for_hresult(code) {
        Some(_) if code == 0x8021_0004 => from_message,
        Some(kind) => kind,
        None => from_message,
    };
    ScanError::native_error(kind, NativeCode::hresult(code, code_name(code)), message)
}

/// Bits from `WIA_DPS_DOCUMENT_HANDLING_STATUS`.
pub fn status_from_bits(bits: i32) -> HandlingStatus {
    let bits = u32::try_from(bits).unwrap_or(0);
    HandlingStatus {
        feeder_ready: Some(bits & 0x1 != 0),
        flat_ready: bits & 0x2 != 0,
        duplex_ready: bits & 0x4 != 0,
        cover_open: bits & 0x8 != 0,
        path_cover_open: bits & 0x10 != 0,
        jam: bits & 0x20 != 0,
        multiple_feed: bits & 0x200 != 0,
        attention: bits & 0x400 != 0,
        lamp_error: bits & 0x800 != 0,
    }
}

/// WIA document-handling capability bits: feeder, flatbed, duplex.
pub fn handling_caps(bits: i32) -> (bool, bool, bool) {
    (bits & 0x1 != 0, bits & 0x2 != 0, bits & 0x4 != 0)
}

/// WIA's wording for the status bits that block a scan.
pub fn acquire_block_reason(status: HandlingStatus) -> Option<&'static str> {
    if status.jam {
        return wia_error_message(0x8021_0002);
    }
    if status.cover_open || status.path_cover_open {
        return wia_error_message(0x8021_0010);
    }
    if status.multiple_feed {
        return wia_error_message(0x8021_0014);
    }
    if status.lamp_error {
        return wia_error_message(0x8021_0011);
    }
    None
}

/// The sentence for a paper-path fault the bits name, in the order jam, cover, double feed.
pub fn paper_path_reason(status: HandlingStatus) -> Option<&'static str> {
    if status.jam {
        return wia_error_message(0x8021_0002);
    }
    if status.cover_open || status.path_cover_open {
        return wia_error_message(0x8021_0010);
    }
    if status.multiple_feed {
        return wia_error_message(0x8021_0014);
    }
    None
}

/// The same failure restated as `sentence` by a status read; the native code stays.
fn restated(error: &ScanError, sentence: &str) -> ScanError {
    error.restated(wia_interrupt_kind(sentence), sentence)
}

/// A Windows failure that does not name the fault. A later status read can.
pub fn generic_scan_failure(error: &ScanError) -> bool {
    let text = error.to_string();
    text == "The scanner reported an error."
        || text.starts_with("The scanner reported an error (")
        || text.starts_with("The scan failed (")
        || text == "Unspecified error"
        || text == "The parameter is incorrect."
        || text.starts_with("The scan was interrupted.")
}

/// Whether the error carries the sentence for a busy device.
fn is_busy(error: &ScanError) -> bool {
    wia_error_message(0x8021_0006).is_some_and(|sentence| error.to_string().contains(sentence))
}

/// Whether the error carries the sentence for a paper problem.
fn is_paper_problem(error: &ScanError) -> bool {
    wia_error_message(0x8021_0004).is_some_and(|sentence| error.to_string().contains(sentence))
}

/// When Download returns a generic HRESULT, prefer a jam, cover, double-feed, lamp, or attention
/// bit read after the call; on the feeder a clear `FEED_READY` with no block bit is an empty tray.
pub fn explain_transfer_failure(
    error: ScanError,
    status: HandlingStatus,
    feeder: bool,
    pages_transferred: usize,
    pages_requested: usize,
) -> ScanError {
    if feeder && is_busy(&error) {
        // An empty feeder is invisible to the WIA status: the driver starts the scan, then answers
        // BUSY. A bit that names a cause wins; with none set, say both.
        let named = acquire_block_reason(status).or_else(|| status.attention.then(|| wia_error_message(0x8021_0008)).flatten());
        return restated(&error, named.unwrap_or(BUSY_OR_FEEDER_EMPTY));
    }
    if is_paper_problem(&error) {
        return match paper_path_reason(status) {
            Some(reason) => restated(&error, reason),
            None => error,
        };
    }
    if !generic_scan_failure(&error) {
        return error;
    }
    if let Some(reason) = acquire_block_reason(status) {
        return restated(&error, reason);
    }
    if feeder && pages_requested == 0 && pages_transferred > 0 {
        return restated(&error, wia_error_message(0x8021_0003).unwrap_or("The feeder has no paper."));
    }
    if status.attention
        && let Some(reason) = wia_error_message(0x8021_0008)
    {
        // A door interlock or a full device store clears `FEED_READY` too; the sentence does not
        // guess which one.
        return restated(&error, reason);
    }
    if feeder && status.feeder_ready != Some(true) {
        let empty_tray = pages_transferred == 0
            || (pages_requested > 0 && pages_transferred >= pages_requested)
            || (pages_requested == 0 && pages_transferred > 0);
        if empty_tray {
            return restated(&error, wia_error_message(0x8021_0003).unwrap_or("The feeder has no paper."));
        }
    }
    error
}

/// Banded transfer is the wrong mode when the driver rejects it without naming a device fault.
pub fn retry_with_file_transfer(error: &ScanError) -> bool {
    generic_scan_failure(error)
}

/// Whether feeder acquire should refuse before transfer when `FEED_READY` is clear. Inbox WSD
/// drivers often omit the bit even when the feeder has sheets loaded.
pub fn feeder_empty_blocks_acquire(source: Source, feeder_ready: bool, wsd_port: bool) -> bool {
    source == Source::Feeder && !feeder_ready && !wsd_port
}

/// WIA `IPS_PAGES` / `DPS_PAGES` for inbox WSD when the user asked for the whole feeder (`0`).
pub fn wsd_feeder_ticket_pages(page_count: i32) -> i32 {
    if page_count == 0 { i32::try_from(craft_scan::MAX_PAGES).unwrap_or(i32::MAX) } else { page_count }
}

/// A WIA-shaped fake: native ids like `{GUID}\0000`, HRESULT errors, BMP pages, and a status read
/// from the document-handling bits.
pub struct WiaLike {
    /// The `WIA_DPS_DOCUMENT_HANDLING_STATUS` bits it reports.
    pub bits: i32,
    pub fail_listing: bool,
}

pub const WIA_ID: &str = r"{00000000-0000-4000-8000-000000000001}\0000";

impl WiaLike {
    /// A scanner with the feeder and flatbed ready (status bits 0x3) that lists normally.
    pub fn new() -> Self {
        Self { bits: 0x3, fail_listing: false }
    }
}

impl Backend for WiaLike {
    fn name(&self) -> &'static str {
        "wia"
    }

    fn scanner_list(&self) -> Result<Vec<ScannerRef>, ScanError> {
        if self.fail_listing {
            return Err(error_from_hresult(0x8021_0015, ""));
        }
        Ok(vec![ScannerRef::new(WIA_ID.into(), "Fake WIA scanner".into(), Some("urn:uuid:00000000-0000-4000-8000-000000000001".into()))])
    }

    fn scanner_details(&self, native_id: &str) -> Result<Scanner, ScanError> {
        if native_id != WIA_ID {
            return Err(error_from_hresult(0x8021_0001, ""));
        }
        let (feeder, flatbed, duplex) = handling_caps(self.bits);
        let mut scanner = scanner(WIA_ID, "Fake WIA scanner");
        scanner.feeder = feeder;
        scanner.flatbed = flatbed;
        scanner.duplex = duplex;
        // The handling bits were read, so a unit missing from them is absent.
        scanner.reported.units = true;
        scanner.connection = Connection::Wsd;
        scanner.feeder_ready = Some(self.bits & 0x1 != 0);
        scanner.handling_status = status_from_bits(self.bits);
        Ok(scanner)
    }

    fn scanners(&self) -> Result<Vec<Scanner>, ScanError> {
        if self.fail_listing {
            return Err(error_from_hresult(0x8021_0015, ""));
        }
        Ok(vec![self.scanner_details(WIA_ID)?])
    }

    fn status(&self, _native_id: &str) -> Option<HandlingStatus> {
        Some(status_from_bits(self.bits))
    }

    fn diagnostic_dump(&self, _filter: &str) -> Result<String, ScanError> {
        Ok(format!("device {WIA_ID}\n  WIA_DPS_DOCUMENT_HANDLING_STATUS = {:#x}\n", self.bits))
    }

    fn acquire(&self, settings: &ScanSettings) -> Result<Acquired, ScanError> {
        if settings.source == Source::Feeder && settings.pages > 1 {
            // One sheet in the tray: the first page arrives, then the driver answers busy.
            let page = Page {
                bytes: bmp_24(2, 2),
                format: "bmp",
                width: 2,
                height: 2,
                dpi: settings.dpi,
                dpi_y: settings.dpi,
                requested_dpi: settings.dpi,
                thresholded: false,
                length: Default::default(),
            };
            let busy = explain_transfer_failure(error_from_hresult(0x8021_0006, ""), status_from_bits(0x1), true, 1, settings.pages as usize);
            let stop = craft_scan::after_transfer_checked(settings.pages as usize, true, 1, false, Some(busy)).err();
            return Err(match stop {
                Some(error) => ScanError::Incomplete {
                    message: error.to_string(),
                    acquired: Acquired { source: Source::Feeder, pages: vec![page] },
                    cause: error.cause(),
                },
                None => ScanError::Failed("unexpected".into()),
            });
        }
        let page = Page {
            bytes: bmp_24(2, 2),
            format: "bmp",
            width: 2,
            height: 2,
            dpi: settings.dpi,
            dpi_y: settings.dpi,
            requested_dpi: settings.dpi,
            thresholded: false,
            length: Default::default(),
        };
        Ok(Acquired { source: Source::Glass, pages: vec![page] })
    }
}

/// [`craft_scan::after_transfer`] with the Windows clarification: a generic failure (an `E_FAIL`
/// the driver gave no reason for) after some pages is a stack that stopped part-way, and with no
/// pages a page the scanner did not deliver. Other faults keep their own sentence.
pub fn after_transfer(page_count: usize, cancelled: bool, download_error: Option<ScanError>) -> Result<(), ScanError> {
    let download_error = download_error.map(|error| {
        if cancelled && page_count == 0 || !generic_scan_failure(&error) {
            return error;
        }
        if page_count > 0 {
            restated(&error, "The scan stopped before all pages arrived. Check for a paper jam, an open cover, or that the scanner stayed online.")
        } else {
            restated(
                &error,
                "The scanner did not deliver a usable page. Check that it stayed online; for a network scanner, remove it in Windows and add it again while it is on if this keeps happening.",
            )
        }
    });
    craft_scan::after_transfer(page_count, cancelled, download_error)
}
