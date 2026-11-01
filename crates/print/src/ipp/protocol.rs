//! IPP operation ids, status codes and the enums of RFC 8011 this backend reads.

/// The operations this backend sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    PrintJob,
    ValidateJob,
    CreateJob,
    SendDocument,
    CancelJob,
    GetJobAttributes,
    GetJobs,
    GetPrinterAttributes,
}

impl Operation {
    pub const ALL: [Operation; 8] = [
        Self::PrintJob,
        Self::ValidateJob,
        Self::CreateJob,
        Self::SendDocument,
        Self::CancelJob,
        Self::GetJobAttributes,
        Self::GetJobs,
        Self::GetPrinterAttributes,
    ];

    pub const fn code(self) -> u16 {
        match self {
            Self::PrintJob => 0x0002,
            Self::ValidateJob => 0x0004,
            Self::CreateJob => 0x0005,
            Self::SendDocument => 0x0006,
            Self::CancelJob => 0x0008,
            Self::GetJobAttributes => 0x0009,
            Self::GetJobs => 0x000A,
            Self::GetPrinterAttributes => 0x000B,
        }
    }

    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.code() == code)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::PrintJob => "Print-Job",
            Self::ValidateJob => "Validate-Job",
            Self::CreateJob => "Create-Job",
            Self::SendDocument => "Send-Document",
            Self::CancelJob => "Cancel-Job",
            Self::GetJobAttributes => "Get-Job-Attributes",
            Self::GetJobs => "Get-Jobs",
            Self::GetPrinterAttributes => "Get-Printer-Attributes",
        }
    }

    /// Repeating the request changes nothing the first one did not.
    pub const fn is_idempotent(self) -> bool {
        matches!(self, Self::ValidateJob | Self::GetJobAttributes | Self::GetJobs | Self::GetPrinterAttributes | Self::CancelJob)
    }
}

/// An IPP status code (RFC 8011 section 4.1.6 and PWG 5100.x).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StatusCode(pub u16);

macro_rules! statuses {
    ($($const:ident = $code:literal, $name:literal;)*) => {
        impl StatusCode {
            $(pub const $const: StatusCode = StatusCode($code);)*
            /// Every status code with a name here.
            pub const KNOWN: &'static [StatusCode] = &[$(StatusCode($code)),*];
            /// The registered name (`client-error-not-found`), when this code has one.
            pub fn name(self) -> Option<&'static str> {
                match self.0 { $($code => Some($name),)* _ => None }
            }
        }
    };
}

statuses! {
    OK = 0x0000, "successful-ok";
    OK_IGNORED = 0x0001, "successful-ok-ignored-or-substituted-attributes";
    OK_CONFLICTING = 0x0002, "successful-ok-conflicting-attributes";
    BAD_REQUEST = 0x0400, "client-error-bad-request";
    FORBIDDEN = 0x0401, "client-error-forbidden";
    NOT_AUTHENTICATED = 0x0402, "client-error-not-authenticated";
    NOT_AUTHORIZED = 0x0403, "client-error-not-authorized";
    NOT_POSSIBLE = 0x0404, "client-error-not-possible";
    TIMEOUT = 0x0405, "client-error-timeout";
    NOT_FOUND = 0x0406, "client-error-not-found";
    GONE = 0x0407, "client-error-gone";
    REQUEST_ENTITY_TOO_LARGE = 0x0408, "client-error-request-entity-too-large";
    REQUEST_VALUE_TOO_LONG = 0x0409, "client-error-request-value-too-long";
    DOCUMENT_FORMAT_NOT_SUPPORTED = 0x040A, "client-error-document-format-not-supported";
    ATTRIBUTES_OR_VALUES_NOT_SUPPORTED = 0x040B, "client-error-attributes-or-values-not-supported";
    URI_SCHEME_NOT_SUPPORTED = 0x040C, "client-error-uri-scheme-not-supported";
    CHARSET_NOT_SUPPORTED = 0x040D, "client-error-charset-not-supported";
    CONFLICTING_ATTRIBUTES = 0x040E, "client-error-conflicting-attributes";
    COMPRESSION_NOT_SUPPORTED = 0x040F, "client-error-compression-not-supported";
    COMPRESSION_ERROR = 0x0410, "client-error-compression-error";
    DOCUMENT_FORMAT_ERROR = 0x0411, "client-error-document-format-error";
    DOCUMENT_ACCESS_ERROR = 0x0412, "client-error-document-access-error";
    ATTRIBUTES_NOT_SETTABLE = 0x0413, "client-error-attributes-not-settable";
    IGNORED_ALL_SUBSCRIPTIONS = 0x0414, "client-error-ignored-all-subscriptions";
    TOO_MANY_SUBSCRIPTIONS = 0x0415, "client-error-too-many-subscriptions";
    IGNORED_ALL_NOTIFICATIONS = 0x0416, "client-error-ignored-all-notifications";
    PRINT_SUPPORT_FILE_NOT_FOUND = 0x0417, "client-error-print-support-file-not-found";
    DOCUMENT_PASSWORD_ERROR = 0x0418, "client-error-document-password-error";
    DOCUMENT_PERMISSION_ERROR = 0x0419, "client-error-document-permission-error";
    DOCUMENT_SECURITY_ERROR = 0x041A, "client-error-document-security-error";
    DOCUMENT_UNPRINTABLE_ERROR = 0x041B, "client-error-document-unprintable-error";
    INTERNAL_ERROR = 0x0500, "server-error-internal-error";
    OPERATION_NOT_SUPPORTED = 0x0501, "server-error-operation-not-supported";
    SERVICE_UNAVAILABLE = 0x0502, "server-error-service-unavailable";
    VERSION_NOT_SUPPORTED = 0x0503, "server-error-version-not-supported";
    DEVICE_ERROR = 0x0504, "server-error-device-error";
    TEMPORARY_ERROR = 0x0505, "server-error-temporary-error";
    NOT_ACCEPTING_JOBS = 0x0506, "server-error-not-accepting-jobs";
    BUSY = 0x0507, "server-error-busy";
    JOB_CANCELED = 0x0508, "server-error-job-canceled";
    MULTIPLE_DOCUMENT_JOBS_NOT_SUPPORTED = 0x0509, "server-error-multiple-document-jobs-not-supported";
    PRINTER_IS_DEACTIVATED = 0x050A, "server-error-printer-is-deactivated";
    TOO_MANY_JOBS = 0x050B, "server-error-too-many-jobs";
    TOO_MANY_DOCUMENTS = 0x050C, "server-error-too-many-documents";
}

impl StatusCode {
    /// `successful-ok` and its two variants: the operation was carried out.
    pub const fn is_success(self) -> bool {
        self.0 < 0x0100
    }

    pub const fn is_client_error(self) -> bool {
        self.0 >= 0x0400 && self.0 < 0x0500
    }

    pub const fn is_server_error(self) -> bool {
        self.0 >= 0x0500 && self.0 < 0x0600
    }
}

/// `job-state` (RFC 8011 section 5.3.7).
pub mod job_state {
    pub const PENDING: i32 = 3;
    pub const PENDING_HELD: i32 = 4;
    pub const PROCESSING: i32 = 5;
    pub const PROCESSING_STOPPED: i32 = 6;
    pub const CANCELED: i32 = 7;
    pub const ABORTED: i32 = 8;
    pub const COMPLETED: i32 = 9;
}

/// `printer-state` (RFC 8011 section 5.4.11).
pub mod printer_state {
    pub const IDLE: i32 = 3;
    pub const PROCESSING: i32 = 4;
    pub const STOPPED: i32 = 5;
}

/// `orientation-requested` (RFC 8011 section 5.2.10).
pub mod orientation {
    pub const PORTRAIT: i32 = 3;
    pub const LANDSCAPE: i32 = 4;
    pub const REVERSE_LANDSCAPE: i32 = 5;
    pub const REVERSE_PORTRAIT: i32 = 6;
}

/// `print-quality` (RFC 8011 section 5.2.13).
pub mod quality {
    pub const DRAFT: i32 = 3;
    pub const NORMAL: i32 = 4;
    pub const HIGH: i32 = 5;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_have_their_registered_ids() {
        for (op, code) in [
            (Operation::PrintJob, 2),
            (Operation::ValidateJob, 4),
            (Operation::CreateJob, 5),
            (Operation::SendDocument, 6),
            (Operation::CancelJob, 8),
            (Operation::GetJobAttributes, 9),
            (Operation::GetJobs, 10),
            (Operation::GetPrinterAttributes, 11),
        ] {
            assert_eq!(op.code(), code);
            assert_eq!(Operation::from_code(code), Some(op));
        }
        assert_eq!(Operation::from_code(0x3C), None);
        assert!(Operation::GetJobs.is_idempotent() && !Operation::PrintJob.is_idempotent() && !Operation::SendDocument.is_idempotent());
    }

    #[test]
    fn status_names_and_classes() {
        assert_eq!(StatusCode::NOT_FOUND.name(), Some("client-error-not-found"));
        assert_eq!(StatusCode(0x0999).name(), None);
        assert!(StatusCode::OK_IGNORED.is_success() && !StatusCode::BAD_REQUEST.is_success());
        assert!(StatusCode::BAD_REQUEST.is_client_error() && StatusCode::BUSY.is_server_error());
        let mut names: Vec<_> = StatusCode::KNOWN.iter().filter_map(|c| c.name()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n);
        assert_eq!(StatusCode::KNOWN.len(), 3 + 28 + 13);
    }
}
