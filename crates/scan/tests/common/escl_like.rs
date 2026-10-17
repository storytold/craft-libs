//! An eSCL-shaped fake. The device reports a `ScannerStatus` with an `AdfState` before a scan,
//! serves each document as a JPEG, and ids are service names. Its native error codes are HTTP
//! statuses.

use craft_scan::{
    Acquired, Backend, Cause, Connection, HandlingStatus, InterruptKind, NativeCode, Page, ScanError, ScanSettings, Scanner, ScannerRef, Source,
    preflight,
};

use super::{jpeg_gray, scanner};

pub const ESCL_ID: &str = "scanner-1";
pub const SERVICE_UNAVAILABLE: NativeCode = NativeCode::new("http", 503, Some("Service Unavailable"));

/// The `AdfState` values of the eSCL `ScannerStatus` document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdfState {
    Loaded,
    Empty,
    Jam,
    DoorOpen,
    Multipick,
}

pub struct EsclLike {
    pub adf: AdfState,
    /// Documents the ADF serves before the job ends.
    pub sheets: usize,
    /// The ADF jams after this many documents.
    pub jam_after: Option<usize>,
    pub fail_listing: bool,
}

impl EsclLike {
    pub fn new(adf: AdfState, sheets: usize) -> Self {
        Self { adf, sheets, jam_after: None, fail_listing: false }
    }

    fn handling(&self) -> HandlingStatus {
        HandlingStatus {
            feeder_ready: Some(self.adf == AdfState::Loaded),
            jam: self.adf == AdfState::Jam,
            cover_open: self.adf == AdfState::DoorOpen,
            multiple_feed: self.adf == AdfState::Multipick,
            ..HandlingStatus::default()
        }
    }
}

impl Backend for EsclLike {
    fn name(&self) -> &'static str {
        "escl"
    }

    fn scanner_list(&self) -> Result<Vec<ScannerRef>, ScanError> {
        if self.fail_listing {
            return Err(ScanError::native_error(InterruptKind::Offline, SERVICE_UNAVAILABLE, "no eSCL service answered"));
        }
        let mut row = ScannerRef::new(ESCL_ID.into(), "Fake eSCL scanner".into(), None);
        row.connection = Connection::Other;
        Ok(vec![row])
    }

    fn scanner_details(&self, native_id: &str) -> Result<Scanner, ScanError> {
        if native_id != ESCL_ID {
            return Err(ScanError::native_error(InterruptKind::Offline, NativeCode::new("http", 404, Some("Not Found")), "no such service"));
        }
        let mut scanner = scanner(ESCL_ID, "Fake eSCL scanner");
        scanner.feeder = true;
        scanner.connection = Connection::Other;
        scanner.feeder_ready = Some(self.adf == AdfState::Loaded);
        scanner.handling_status = self.handling();
        Ok(scanner)
    }

    fn scanners(&self) -> Result<Vec<Scanner>, ScanError> {
        if self.fail_listing {
            return Err(ScanError::native_error(InterruptKind::Offline, SERVICE_UNAVAILABLE, "no eSCL service answered"));
        }
        Ok(vec![self.scanner_details(ESCL_ID)?])
    }

    fn status(&self, _native_id: &str) -> Option<HandlingStatus> {
        Some(self.handling())
    }

    fn diagnostic_dump(&self, _filter: &str) -> Result<String, ScanError> {
        Ok(format!("ScannerCapabilities for {ESCL_ID}\nAdfState: {:?}\n", self.adf))
    }

    fn acquire(&self, settings: &ScanSettings) -> Result<Acquired, ScanError> {
        let feeder = !matches!(settings.source, Source::Glass);
        let source = if feeder { Source::Feeder } else { Source::Glass };
        preflight(source, Some(self.handling()))?;
        let wanted = if settings.pages == 0 { usize::MAX } else { settings.pages as usize };
        let available = if feeder { self.sheets } else { 1 };
        let mut pages = Vec::new();
        for index in 0..wanted.min(available) {
            if self.jam_after == Some(index) {
                let jam = ScanError::native_error(InterruptKind::Jam, SERVICE_UNAVAILABLE, "ADF jam detected");
                if pages.is_empty() {
                    return Err(jam);
                }
                return Err(ScanError::Incomplete {
                    message: jam.to_string(),
                    acquired: Acquired { source, pages },
                    cause: Some(Cause { kind: InterruptKind::Jam, native: Some(SERVICE_UNAVAILABLE) }),
                });
            }
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
        Ok(Acquired { source, pages })
    }
}
