//! Typed views of what a printer reports: description, status, capabilities, job state.

use super::ticket::{Compression, Orientation, Quality, Sides};

/// A job state (`JobState`); vendor states arrive as `Other`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Pending,
    PendingHeld,
    Processing,
    ProcessingStopped,
    Started,
    Terminating,
    Completed,
    Canceled,
    Aborted,
    Other(String),
}

impl JobState {
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "Pending" => Self::Pending,
            "Pending-Held" => Self::PendingHeld,
            "Processing" => Self::Processing,
            "ProcessingStopped" => Self::ProcessingStopped,
            "Started" => Self::Started,
            "Terminating" => Self::Terminating,
            "Completed" => Self::Completed,
            "Canceled" => Self::Canceled,
            "Aborted" => Self::Aborted,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Will the job never change state again?
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Aborted)
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Pending => "Pending",
            Self::PendingHeld => "Pending-Held",
            Self::Processing => "Processing",
            Self::ProcessingStopped => "ProcessingStopped",
            Self::Started => "Started",
            Self::Terminating => "Terminating",
            Self::Completed => "Completed",
            Self::Canceled => "Canceled",
            Self::Aborted => "Aborted",
            Self::Other(s) => s,
        }
    }
}

/// A job state reason (`JobStateReason`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobReason {
    None,
    CompressionError,
    DocumentAccessError,
    DocumentFormatError,
    DocumentTimeoutError,
    DocumentTransferError,
    JobCanceledAtDevice,
    JobCanceledByUser,
    JobCompletedSuccessfully,
    JobCompletedWithErrors,
    JobCompletedWithWarnings,
    JobIncoming,
    JobPasswordWait,
    JobPrinting,
    PrinterStopped,
    UnsupportedCompression,
    UnsupportedDocumentFormat,
    Other(String),
}

impl JobReason {
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "None" => Self::None,
            "CompressionError" => Self::CompressionError,
            "DocumentAccessError" => Self::DocumentAccessError,
            "DocumentFormatError" => Self::DocumentFormatError,
            "DocumentTimeoutError" => Self::DocumentTimeoutError,
            "DocumentTransferError" => Self::DocumentTransferError,
            "JobCanceledAtDevice" => Self::JobCanceledAtDevice,
            "JobCanceledByUser" => Self::JobCanceledByUser,
            "JobCompletedSuccessfully" => Self::JobCompletedSuccessfully,
            "JobCompletedWithErrors" => Self::JobCompletedWithErrors,
            "JobCompletedWithWarnings" => Self::JobCompletedWithWarnings,
            "JobIncoming" => Self::JobIncoming,
            "JobPasswordWait" => Self::JobPasswordWait,
            "JobPrinting" => Self::JobPrinting,
            "PrinterStopped" => Self::PrinterStopped,
            "UnsupportedCompression" => Self::UnsupportedCompression,
            "UnsupportedDocumentFormat" => Self::UnsupportedDocumentFormat,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl JobReason {
    /// The reason as the printer spells it (vendor values as received).
    pub fn name(&self) -> &str {
        match self {
            Self::None => "None",
            Self::CompressionError => "CompressionError",
            Self::DocumentAccessError => "DocumentAccessError",
            Self::DocumentFormatError => "DocumentFormatError",
            Self::DocumentTimeoutError => "DocumentTimeoutError",
            Self::DocumentTransferError => "DocumentTransferError",
            Self::JobCanceledAtDevice => "JobCanceledAtDevice",
            Self::JobCanceledByUser => "JobCanceledByUser",
            Self::JobCompletedSuccessfully => "JobCompletedSuccessfully",
            Self::JobCompletedWithErrors => "JobCompletedWithErrors",
            Self::JobCompletedWithWarnings => "JobCompletedWithWarnings",
            Self::JobIncoming => "JobIncoming",
            Self::JobPasswordWait => "JobPasswordWait",
            Self::JobPrinting => "JobPrinting",
            Self::PrinterStopped => "PrinterStopped",
            Self::UnsupportedCompression => "UnsupportedCompression",
            Self::UnsupportedDocumentFormat => "UnsupportedDocumentFormat",
            Self::Other(s) => s,
        }
    }
}

/// `JobStatus` (also the shape of a `JobSummary`'s counters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobStatus {
    pub job_id: u32,
    pub state: JobState,
    pub reasons: Vec<JobReason>,
    pub k_octets_processed: u64,
    pub sheets_completed: u64,
    pub documents: u64,
}

/// One row of GetActiveJobs / GetJobHistory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobSummary {
    pub status: JobStatus,
    pub job_name: String,
    pub user_name: String,
}

/// The answer to GetJobElements for the requested sections.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct JobElements {
    pub status: Option<JobStatus>,
    /// The printer returned a `PrintTicket` section (its effective ticket).
    pub has_ticket: bool,
    /// The effective ticket as far as the reply states it (`must_honor` is not read back).
    pub ticket: Option<super::ticket::PrintTicket>,
    /// Sections the printer marked `Valid="false"`.
    pub invalid: Vec<String>,
}

/// `PrinterDescription`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PrinterDescription {
    pub name: String,
    pub color_supported: Option<bool>,
    pub multiple_documents: Option<bool>,
    pub pages_per_minute: Option<u32>,
}

/// `PrinterStatus`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PrinterStatus {
    /// `Idle`, `Processing`, `Stopped` or a vendor value.
    pub state: String,
    pub queued_jobs: Option<u32>,
    pub reasons: Vec<String>,
}

/// What the printer says it accepts (`PrinterCapabilities`). An empty list means the printer did
/// not state that dimension, which is different from "nothing is allowed".
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Document formats, spelled as listed.
    pub formats: Vec<String>,
    pub compressions: Vec<String>,
    pub media_sizes: Vec<String>,
    pub media_types: Vec<String>,
    pub orientations: Vec<String>,
    pub qualities: Vec<String>,
    pub sides: Vec<String>,
    /// Allowed copies (min, max).
    pub copies: Option<(u32, u32)>,
    /// Allowed resolutions in dpi.
    pub resolutions: Vec<(u32, u32)>,
}

impl Capabilities {
    /// The printer's spelling of `format` if it lists it: exact match first, then ASCII case-insensitive.
    pub fn accepts_format(&self, format: &str) -> Option<&str> {
        self.formats
            .iter()
            .find(|f| f.as_str() == format)
            .or_else(|| self.formats.iter().find(|f| f.eq_ignore_ascii_case(format)))
            .map(String::as_str)
    }

    /// Can documents be sent uncompressed? True when the printer lists `None` or lists nothing.
    pub fn accepts_no_compression(&self) -> bool {
        self.compressions.is_empty() || self.compressions.iter().any(|c| c == Compression::None.token())
    }

    pub fn supports_sides(&self, s: Sides) -> Option<bool> {
        listed(&self.sides, s.token())
    }

    pub fn supports_orientation(&self, o: Orientation) -> Option<bool> {
        listed(&self.orientations, o.token())
    }

    pub fn supports_quality(&self, q: Quality) -> Option<bool> {
        listed(&self.qualities, q.token())
    }

    pub fn supports_media(&self, name: &str) -> Option<bool> {
        listed(&self.media_sizes, name)
    }

    pub fn supports_copies(&self, n: u32) -> Option<bool> {
        self.copies.map(|(lo, hi)| (lo.max(1)..=hi).contains(&n))
    }

    pub fn supports_resolution(&self, r: (u32, u32)) -> Option<bool> {
        if self.resolutions.is_empty() { None } else { Some(self.resolutions.contains(&r)) }
    }
}

fn listed(list: &[String], token: &str) -> Option<bool> {
    if list.is_empty() { None } else { Some(list.iter().any(|v| v == token)) }
}

/// Which sections GetPrinterElements asked for and which the printer knew.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PrinterElements {
    pub description: Option<PrinterDescription>,
    pub status: Option<PrinterStatus>,
    pub capabilities: Option<Capabilities>,
    /// Sections marked `Valid="false"`.
    pub invalid: Vec<String>,
}

/// The sections of GetPrinterElements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Description,
    Configuration,
    Status,
    DefaultPrintTicket,
    Capabilities,
}

impl Section {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Description => "PrinterDescription",
            Self::Configuration => "PrinterConfiguration",
            Self::Status => "PrinterStatus",
            Self::DefaultPrintTicket => "DefaultPrintTicket",
            Self::Capabilities => "PrinterCapabilities",
        }
    }
}
