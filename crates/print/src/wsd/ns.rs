//! Namespaces and the operation table (names and action URIs, from the WS-Print service description).
//!
//! The namespaces every WSD service shares come from the WSD core; `WPRT` and the actions are print-only.

pub use super::adapter::{ANONYMOUS, FAULT_ACTION, SOAP, WSA, WSA_FINAL, XOP};

/// The WS-Print schema namespace (`wprt`).
pub const WPRT: &str = "http://schemas.microsoft.com/windows/2006/08/wdp/print";

/// The operations this client sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    GetPrinterElements,
    CreatePrintJob,
    SendDocument,
    CancelJob,
    GetJobElements,
    GetActiveJobs,
    GetJobHistory,
}

impl Action {
    /// The operation name, also the local name of the request element (`<Name>Request`) and the
    /// stem of the reply (`<Name>Response`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::GetPrinterElements => "GetPrinterElements",
            Self::CreatePrintJob => "CreatePrintJob",
            Self::SendDocument => "SendDocument",
            Self::CancelJob => "CancelJob",
            Self::GetJobElements => "GetJobElements",
            Self::GetActiveJobs => "GetActiveJobs",
            Self::GetJobHistory => "GetJobHistory",
        }
    }

    /// The `wsa:Action` (and SOAPAction) of the request.
    pub fn uri(self) -> String {
        format!("{WPRT_ACTIONS}/{}", self.name())
    }

    /// The `wsa:Action` of the reply.
    pub fn response_uri(self) -> String {
        format!("{WPRT_ACTIONS}/{}Response", self.name())
    }
}

/// The events a WS-Print service pushes (each is also its `wsa:Action` and its body element).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    PrinterElementsChange,
    PrinterStatusSummary,
    PrinterStatusCondition,
    PrinterStatusConditionCleared,
    JobStatus,
    JobEndState,
}

impl Event {
    pub const ALL: [Event; 6] = [
        Self::PrinterElementsChange,
        Self::PrinterStatusSummary,
        Self::PrinterStatusCondition,
        Self::PrinterStatusConditionCleared,
        Self::JobStatus,
        Self::JobEndState,
    ];

    /// The element name, `<Name>Event`.
    pub const fn element(self) -> &'static str {
        match self {
            Self::PrinterElementsChange => "PrinterElementsChangeEvent",
            Self::PrinterStatusSummary => "PrinterStatusSummaryEvent",
            Self::PrinterStatusCondition => "PrinterStatusConditionEvent",
            Self::PrinterStatusConditionCleared => "PrinterStatusConditionClearedEvent",
            Self::JobStatus => "JobStatusEvent",
            Self::JobEndState => "JobEndStateEvent",
        }
    }

    /// The `wsa:Action` of the notification (and the WS-Eventing action filter value).
    pub fn uri(self) -> String {
        format!("{WPRT_ACTIONS}/{}", self.element())
    }

    /// The event an action URI names, if it is one of the six.
    pub fn from_action(action: &str) -> Option<Self> {
        let name = action.strip_prefix(WPRT_ACTIONS)?.strip_prefix('/')?;
        Self::ALL.into_iter().find(|e| e.element() == name)
    }

    /// Whether the event is about one job (its body carries a `JobId`).
    pub const fn is_job_event(self) -> bool {
        matches!(self, Self::JobStatus | Self::JobEndState)
    }
}

const WPRT_ACTIONS: &str = "http://schemas.microsoft.com/windows/2006/08/wdp/print";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_uris_follow_the_operation_name() {
        assert_eq!(Action::CreatePrintJob.uri(), "http://schemas.microsoft.com/windows/2006/08/wdp/print/CreatePrintJob");
        assert_eq!(Action::CancelJob.response_uri(), "http://schemas.microsoft.com/windows/2006/08/wdp/print/CancelJobResponse");
        assert_eq!(Action::GetActiveJobs.name(), "GetActiveJobs");
    }
}
