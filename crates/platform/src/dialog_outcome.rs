//! Dialog results with evidence: an empty rfd answer does not establish user cancellation.
/// Host assessment of whether a native backend is available; unknown is not proven absence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capability {
    /// The host has confirmed that a backend is available.
    Available,
    /// Proven backend absence, with a diagnostic suitable for the host.
    Unavailable(String),
    /// Inconclusive capability evidence; the host should still permit a picker attempt.
    Unknown(String),
}
/// The layer that prevented a dialog request from completing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The host proved that no backend is available.
    Unavailable,
    /// Request transport or background execution failed.
    Worker,
    /// The picker backend reported or encountered a failure.
    Backend,
}
/// A classified dialog failure with a host-facing diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogError {
    /// The failing layer.
    pub kind: ErrorKind,
    /// Diagnostic text for the host to present or log.
    pub message: String,
}
impl DialogError {
    /// Construct a failure without inferring cancellation from the diagnostic.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}
impl std::fmt::Display for DialogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for DialogError {}
/// Selection, explicit cancellation, failure, or an answer whose cause is unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<T> {
    /// A value returned by the picker.
    Selected(T),
    /// Explicit app/backend cancellation; never inferred from rfd's empty answer.
    Cancelled,
    /// A transport, capability or backend failure.
    Failed(DialogError),
    /// No selection; rfd cannot say whether the user cancelled or its backend failed.
    Indeterminate,
}
impl<T> Outcome<T> {
    /// Preserve a selection; an empty rfd answer is indeterminate, never proven cancellation.
    pub fn from_rfd(value: Option<T>) -> Self {
        value.map_or(Self::Indeterminate, Self::Selected)
    }
    /// Transform only a selected value, preserving all other outcomes unchanged.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Outcome<U> {
        match self {
            Self::Selected(value) => Outcome::Selected(map(value)),
            Self::Cancelled => Outcome::Cancelled,
            Self::Failed(error) => Outcome::Failed(error),
            Self::Indeterminate => Outcome::Indeterminate,
        }
    }
    /// Construct a backend failure with the supplied diagnostic.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(DialogError::new(ErrorKind::Backend, message))
    }
}
impl<T> Outcome<Vec<T>> {
    /// Treat missing and empty multi-selection answers as indeterminate.
    pub fn from_rfd_files(value: Option<Vec<T>>) -> Self {
        Self::from_rfd(value.filter(|paths| !paths.is_empty()))
    }
}
