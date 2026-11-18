//! How a response message says it went, and why not ([MS-OXWSCDATA]), and the waits a busy
//! server asks for.

use std::fmt;

/// The most a server may ask the client to wait, in milliseconds (five minutes); more is
/// taken as this.
pub const MAX_BACK_OFF_MS: u32 = 300_000;
/// The shortest wait a busy server is given (one second): a server that says 0 is not to be
/// asked again at once.
pub const MIN_BACK_OFF_MS: u32 = 1_000;
/// How long to wait when a server is busy and does not say for how long (one second).
pub const DEFAULT_BACK_OFF_MS: u32 = 1_000;
/// The longest response code kept.
const MAX_CODE: usize = 64;

/// How a response message says it went (`ResponseClass`, [MS-OXWSCDATA]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseClass {
    /// It was done.
    Success,
    /// It was done, with a remark.
    Warning,
    /// It was not done.
    Error,
}

impl ResponseClass {
    /// The class a document names; none for a word the schema does not have.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Success" => Some(Self::Success),
            "Warning" => Some(Self::Warning),
            "Error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// Why a response message says no (`ResponseCodeType`, [MS-OXWSCDATA]): the ones the client
/// acts on by name, and any other as the identifier it was.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResponseCode {
    /// `NoError`.
    NoError,
    /// `ErrorServerBusy`: try again after the back-off.
    ServerBusy,
    /// `ErrorAccessDenied`.
    AccessDenied,
    /// `ErrorFolderNotFound`.
    FolderNotFound,
    /// `ErrorItemNotFound`.
    ItemNotFound,
    /// `ErrorNonExistentMailbox`.
    NonExistentMailbox,
    /// `ErrorInvalidSyncStateData`: the sync state is no longer known; read afresh.
    InvalidSyncState,
    /// `ErrorInvalidPropertyRequest`: the server doesn't know a property that was asked for.
    InvalidPropertyRequest,
    /// `ErrorSchemaValidation`: the request did not fit the schema (a bug here).
    SchemaValidation,
    /// `ErrorInvalidServerVersion`: the server does not know the version asked for.
    InvalidServerVersion,
    /// `ErrorInvalidRequest`.
    InvalidRequest,
    /// `ErrorInternalServerError` or `ErrorInternalServerTransientError`.
    InternalServerError,
    /// `ErrorMailboxStoreUnavailable` or `ErrorMailboxMoveInProgress`: the mailbox cannot be
    /// reached for now.
    MailboxUnavailable,
    /// `ErrorTimeoutExpired`.
    TimeoutExpired,
    /// Any other code, as the short identifier it was.
    Other(String),
}

impl ResponseCode {
    /// The code a document names. A name that is not a short identifier (letters and digits)
    /// is no code and reads as `Other("")`.
    pub fn from_name(name: &str) -> Self {
        match name.trim() {
            "NoError" => Self::NoError,
            "ErrorServerBusy" => Self::ServerBusy,
            "ErrorAccessDenied" => Self::AccessDenied,
            "ErrorFolderNotFound" => Self::FolderNotFound,
            "ErrorItemNotFound" => Self::ItemNotFound,
            "ErrorNonExistentMailbox" => Self::NonExistentMailbox,
            "ErrorInvalidSyncStateData" => Self::InvalidSyncState,
            "ErrorInvalidPropertyRequest" => Self::InvalidPropertyRequest,
            "ErrorSchemaValidation" => Self::SchemaValidation,
            "ErrorInvalidServerVersion" => Self::InvalidServerVersion,
            "ErrorInvalidRequest" => Self::InvalidRequest,
            "ErrorInternalServerError" | "ErrorInternalServerTransientError" => Self::InternalServerError,
            "ErrorMailboxStoreUnavailable" | "ErrorMailboxMoveInProgress" => Self::MailboxUnavailable,
            "ErrorTimeoutExpired" => Self::TimeoutExpired,
            other if !other.is_empty() && other.len() <= MAX_CODE && other.bytes().all(|b| b.is_ascii_alphanumeric()) => {
                Self::Other(other.to_owned())
            }
            _ => Self::Other(String::new()),
        }
    }
}

impl fmt::Display for ResponseCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoError => f.write_str("NoError"),
            Self::ServerBusy => f.write_str("ErrorServerBusy"),
            Self::AccessDenied => f.write_str("ErrorAccessDenied"),
            Self::FolderNotFound => f.write_str("ErrorFolderNotFound"),
            Self::ItemNotFound => f.write_str("ErrorItemNotFound"),
            Self::NonExistentMailbox => f.write_str("ErrorNonExistentMailbox"),
            Self::InvalidSyncState => f.write_str("ErrorInvalidSyncStateData"),
            Self::InvalidPropertyRequest => f.write_str("ErrorInvalidPropertyRequest"),
            Self::SchemaValidation => f.write_str("ErrorSchemaValidation"),
            Self::InvalidServerVersion => f.write_str("ErrorInvalidServerVersion"),
            Self::InvalidRequest => f.write_str("ErrorInvalidRequest"),
            Self::InternalServerError => f.write_str("ErrorInternalServerError"),
            Self::MailboxUnavailable => f.write_str("ErrorMailboxStoreUnavailable"),
            Self::TimeoutExpired => f.write_str("ErrorTimeoutExpired"),
            Self::Other(code) if code.is_empty() => f.write_str("an unnamed error"),
            Self::Other(code) => f.write_str(code),
        }
    }
}
