//! A SANE-shaped fake. Options-style capabilities (nothing published as a list unless the device
//! has one), no preflight status, frames delivered as JPEG, and an empty feeder that is learned
//! only when the scan fails with `SANE_STATUS_NO_DOCS`.

use craft_scan::{
    Acquired, Backend, Cause, Color, Connection, ImageChoices, InterruptKind, NativeCode, Page, ScanError, ScanSettings, Scanner, ScannerRef, Source,
};

use super::{jpeg_gray, scanner};

pub const SANE_ID: &str = "pixma:libusb:001:004";

pub const NO_DOCS: NativeCode = NativeCode::new("sane", 6, Some("SANE_STATUS_NO_DOCS"));
pub const DEVICE_BUSY: NativeCode = NativeCode::new("sane", 3, Some("SANE_STATUS_DEVICE_BUSY"));

pub struct SaneLike {
    /// Sheets in the feeder.
    pub loaded: usize,
    pub fail_listing: bool,
}

impl SaneLike {
    pub fn new(loaded: usize) -> Self {
        Self { loaded, fail_listing: false }
    }
}

impl Backend for SaneLike {
    fn name(&self) -> &'static str {
        "sane"
    }

    fn scanner_list(&self) -> Result<Vec<ScannerRef>, ScanError> {
        if self.fail_listing {
            return Err(ScanError::native_error(InterruptKind::DeviceError, DEVICE_BUSY, "sane_get_devices failed"));
        }
        let mut row = ScannerRef::new(SANE_ID.into(), "Fake SANE scanner".into(), None);
        row.connection = Connection::Usb;
        Ok(vec![row])
    }

    fn scanner_details(&self, native_id: &str) -> Result<Scanner, ScanError> {
        if native_id != SANE_ID {
            return Err(ScanError::Failed(format!("sane: no such device {native_id}")));
        }
        let mut scanner = scanner(SANE_ID, "Fake SANE scanner");
        scanner.feeder = true;
        scanner.connection = Connection::Usb;
        // The feeder's paper cannot be read before a scan: not reported, which is not "empty".
        scanner.feeder_ready = None;
        let choices = ImageChoices { dpi: vec![100, 200, 300, 600], colors: vec![Color::Gray, Color::Rgb], ..ImageChoices::default() };
        // The flatbed publishes a resolution list; the feeder's options are not listed at all.
        scanner.glass = Some(choices);
        scanner.feeder_choices = None;
        Ok(scanner)
    }

    fn scanners(&self) -> Result<Vec<Scanner>, ScanError> {
        if self.fail_listing {
            return Err(ScanError::native_error(InterruptKind::DeviceError, DEVICE_BUSY, "sane_get_devices failed"));
        }
        Ok(vec![self.scanner_details(SANE_ID)?])
    }

    // `status` stays the default: `None`, this backend cannot preflight.

    fn diagnostic_dump(&self, _filter: &str) -> Result<String, ScanError> {
        Ok(format!("device '{SANE_ID}'\n  --resolution 100..600dpi [200]\n  --source Flatbed|ADF\n"))
    }

    fn acquire(&self, settings: &ScanSettings) -> Result<Acquired, ScanError> {
        let feeder = !matches!(settings.source, Source::Glass);
        let wanted = if settings.pages == 0 { usize::MAX } else { settings.pages as usize };
        let available = if feeder { self.loaded } else { 1 };
        let mut pages = Vec::new();
        for _ in 0..wanted.min(available) {
            // One JPEG per frame, line by line on the wire in a real driver.
            let page = Page {
                bytes: jpeg_gray(16, 8),
                format: "jpeg",
                width: 16,
                height: 8,
                dpi: settings.dpi,
                dpi_y: settings.dpi,
                requested_dpi: settings.dpi,
                thresholded: false,
                length: Default::default(),
            };
            if let Some(progress) = &settings.progress {
                progress.push_page(page.clone());
            }
            pages.push(page);
        }
        let ran_out = feeder && settings.pages != 0 && pages.len() < wanted;
        if !ran_out {
            return Ok(Acquired { source: if feeder { Source::Feeder } else { Source::Glass }, pages });
        }
        // The next frame fails with NO_DOCS: that is how this backend learns the feeder is empty.
        let no_docs = ScanError::native_error(InterruptKind::FeederEmpty, NO_DOCS, "Document feeder out of documents");
        if pages.is_empty() {
            return Err(no_docs);
        }
        Err(ScanError::Incomplete {
            message: craft_scan::feeder_shortfall_message(wanted, pages.len()),
            acquired: Acquired { source: Source::Feeder, pages },
            cause: Some(Cause { kind: InterruptKind::FeederEmpty, native: Some(NO_DOCS) }),
        })
    }
}
